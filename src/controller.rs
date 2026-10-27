//! Network side: follows the game's state through presence and loads player data in the background.

use crate::local_files::browser_profile_dir;
use crate::model::{Load, Lobby, Model, Phase, PlayerData, parse_coregame, parse_pregame};
use crate::presence::LoopState;
use crate::riot::{Riot, connect};
use crate::tracker::{self, Tracker, riot_id_path};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Model>>;

/// How long a failed load waits before the next tick may retry it.
const RETRY_AFTER: Duration = Duration::from_secs(10);

pub fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Model> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

fn report(shared: &Shared, e: &anyhow::Error) {
    // The footer counts a rate limit down instead.
    if e.is::<crate::riot::RateLimited>() {
        return;
    }
    lock(shared).last_error = Some((format!("{e:#}"), Instant::now()));
}

/// Ids whose load is running or done. A failed load releases its ids after `RETRY_AFTER` so a later
/// tick tries again.
type Claims = Arc<Mutex<HashSet<String>>>;

fn claim(claims: &Claims, id: &str) -> bool {
    claims.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string())
}

async fn release_after(claims: Claims, ids: Vec<String>, delay: Duration) {
    tokio::time::sleep(delay).await;
    let mut c = claims.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        c.remove(&id);
    }
}

/// Runs forever: loads names from valorant-api.com, connects to the Riot Client, follows the game,
/// and reconnects when the client goes away.
pub async fn run(shared: Shared, dump_dir: Option<PathBuf>) {
    loop {
        match crate::riot::fetch_content().await {
            Ok(c) => {
                lock(&shared).content = Arc::new(c);
                break;
            }
            Err(e) => {
                lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    loop {
        let connected = connect(dump_dir.clone()).await.and_then(|r| {
            let profile = browser_profile_dir().ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA is not set"))?;
            let tracker = Tracker::new(r.net.clone(), dump_dir.clone(), profile);
            Ok((Arc::new(r), Arc::new(tracker)))
        });
        let (riot, tracker) = match connected {
            Ok(c) => c,
            Err(e) => {
                lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        {
            let mut m = lock(&shared);
            m.me = riot.puuid.clone();
            m.region = riot.log.region.to_uppercase();
            m.last_error = None;
            m.net = Some(riot.net.clone());
        }
        let mut session = Session::new(riot, tracker, shared.clone());
        loop {
            match session.tick().await {
                Ok(()) => {}
                Err(e) if session.client_gone => {
                    lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                    break;
                }
                Err(e) => report(&shared, &e),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

struct Session {
    riot: Arc<Riot>,
    tracker: Arc<Tracker>,
    shared: Shared,
    pregame_id: Option<String>,
    core_id: Option<String>,
    /// The last agent-select match seen; a new one reloads everyone.
    loaded_match: Option<String>,
    requested: Claims,
    named: Claims,
    client_gone: bool,
}

impl Session {
    fn new(riot: Arc<Riot>, tracker: Arc<Tracker>, shared: Shared) -> Self {
        Self {
            riot,
            tracker,
            shared,
            pregame_id: None,
            core_id: None,
            loaded_match: None,
            requested: Claims::default(),
            named: Claims::default(),
            client_gone: false,
        }
    }

    async fn tick(&mut self) -> Result<()> {
        let presences = match self.riot.presences().await {
            Ok(p) => p,
            Err(e) => {
                self.client_gone = true;
                return Err(e);
            }
        };
        let me = self.riot.puuid.clone();
        let state = presences.iter().find(|p| p.puuid == me).and_then(|p| p.state.clone());
        {
            let mut m = lock(&self.shared);
            m.presences = presences.into_iter().map(|p| (p.puuid.clone(), p)).collect();
        }
        match state {
            None => self.set_phase(Phase::Waiting("VALORANT isn't running".into())),
            Some(LoopState::Pregame) => self.pregame().await?,
            Some(LoopState::Ingame) => self.ingame().await?,
            Some(LoopState::Menus) | Some(LoopState::Other(_)) => self.menus(),
        }
        self.tracker.close_if_idle();
        Ok(())
    }

    fn set_phase(&self, phase: Phase) {
        lock(&self.shared).phase = phase;
    }

    fn menus(&mut self) {
        self.pregame_id = None;
        self.core_id = None;
        let party: Vec<String> = {
            let mut m = lock(&self.shared);
            m.phase = Phase::Menus;
            m.lobby = None;
            let my_party = m.my_presence().and_then(|p| p.party_id.clone());
            m.presences.values().filter(|p| p.party_id.is_some() && p.party_id == my_party).map(|p| p.puuid.clone()).collect()
        };
        let me = self.riot.puuid.clone();
        self.load_names(std::iter::once(me.clone()).chain(party.iter().cloned()).collect());
        self.load_player(me);
        for p in party {
            self.load_player(p);
        }
    }

    async fn pregame(&mut self) -> Result<()> {
        self.core_id = None;
        if self.pregame_id.is_none() {
            self.pregame_id = self.riot.pregame_match_id().await?;
        }
        let Some(id) = self.pregame_id.clone() else { return Ok(()) };
        // Stats are loaded fresh at the start of every match, never carried over from an earlier one.
        if self.loaded_match.as_deref() != Some(id.as_str()) {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).clear();
            self.loaded_match = Some(id.clone());
        }
        let Some(v) = self.riot.pregame_match(&id).await? else {
            self.pregame_id = None;
            return Ok(());
        };
        if let Some(lobby) = parse_pregame(&v, Instant::now()) {
            self.show_lobby(Phase::Pregame, lobby);
        }
        Ok(())
    }

    async fn ingame(&mut self) -> Result<()> {
        self.pregame_id = None;
        if self.core_id.is_none() {
            self.core_id = self.riot.coregame_match_id().await?;
        }
        let Some(id) = self.core_id.clone() else { return Ok(()) };
        let current = lock(&self.shared).ingame_lobby(&id).cloned();
        let lobby = match current {
            Some(l) => l,
            None => {
                let Some(v) = self.riot.coregame_match(&id).await? else { return Ok(()) };
                let Some(lobby) = parse_coregame(&v, &self.riot.puuid) else { return Ok(()) };
                lobby
            }
        };
        self.show_lobby(Phase::Ingame, lobby);
        Ok(())
    }

    fn show_lobby(&mut self, phase: Phase, lobby: Lobby) {
        let competitive = lobby.is_competitive();
        // You're never hidden from yourself.
        let named: Vec<String> =
            lobby.slots.iter().filter(|s| !s.incognito || s.puuid == self.riot.puuid).map(|s| s.puuid.clone()).collect();
        {
            let mut m = lock(&self.shared);
            m.phase = phase;
            m.lobby = Some(lobby);
        }
        if competitive {
            // Incognito players' names are never requested, so tracker.gg is never asked about them.
            self.load_names(named.clone());
            for p in named {
                self.load_player(p);
            }
        }
    }

    fn load_names(&mut self, puuids: Vec<String>) {
        let missing: Vec<String> = puuids.into_iter().filter(|p| claim(&self.named, p)).collect();
        if missing.is_empty() {
            return;
        }
        let (riot, shared, claims) = (self.riot.clone(), self.shared.clone(), self.named.clone());
        tokio::spawn(async move {
            match riot.names(&missing).await {
                Ok(names) => {
                    let mut m = lock(&shared);
                    for (id, name, tag) in names {
                        m.players.entry(id).or_default().name = Some((name, tag));
                    }
                }
                Err(e) => {
                    report(&shared, &e);
                    release_after(claims, missing, RETRY_AFTER).await;
                }
            }
        });
    }

    /// The player's Riot ID as tracker.gg's URLs spell it, once the name lookup has returned it.
    fn riot_id(&self, puuid: &str) -> Option<String> {
        lock(&self.shared).players.get(puuid).and_then(|d| d.name.as_ref()).map(|(n, t)| riot_id_path(n, t))
    }

    /// Loads a player's stats from tracker.gg. Waits for their name; a later tick calls this again.
    fn load_player(&mut self, puuid: String) {
        let Some(riot_id) = self.riot_id(&puuid) else { return };
        let Some(act) = lock(&self.shared).content.current_act(chrono::Utc::now()).map(|a| a.id.clone()) else { return };
        if !claim(&self.requested, &puuid) {
            return;
        }
        let (tracker, shared, claims) = (self.tracker.clone(), self.shared.clone(), self.requested.clone());
        tokio::spawn(async move {
            if let Err(e) = load_player(&tracker, &shared, &puuid, &riot_id, &act).await {
                report(&shared, &e);
                release_after(claims, vec![puuid], RETRY_AFTER).await;
            }
        });
    }
}

fn update(shared: &Shared, puuid: &str, f: impl FnOnce(&mut PlayerData)) {
    f(lock(shared).players.entry(puuid.to_string()).or_default());
}

/// Rank, act record and map records for one player from tracker.gg. A player
/// tracker.gg doesn't know, or whose profile is private, is shown as such and not asked about again;
/// any other failure is returned so the load is retried.
async fn load_player(tracker: &Tracker, shared: &Shared, puuid: &str, riot_id: &str, act: &str) -> Result<()> {
    fn state<T>(private: bool) -> Load<T> {
        if private { Load::Private } else { Load::Missing }
    }
    let unavailable = |private: bool| {
        update(shared, puuid, |d| {
            d.profile = state(private);
            d.maps = state(private);
        })
    };
    let v = match tracker.profile(riot_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            unavailable(false);
            return Ok(());
        }
        Err(e) if e.is::<tracker::Private>() => {
            unavailable(true);
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    // A profile shared only partly (tracker.gg's "premier" privacy) loads, but without stats, and its
    // other pages are refused.
    let shared_stats = v["data"]["segments"].as_array().is_some_and(|s| s.iter().any(|s| s["type"] == "season"));
    if !shared_stats && v["data"]["metadata"]["privacy"].as_str().is_some_and(|p| !p.eq_ignore_ascii_case("public")) {
        unavailable(true);
        return Ok(());
    }
    let profile = tracker::parse_profile(&v, act);
    drop(v);
    // Without a game this act there are no map records to ask for.
    let played = profile.acts.iter().any(|a| a.act_id == act);
    update(shared, puuid, |d| d.profile = Load::Ready(profile));
    let maps = if played { tracker.maps(riot_id, act).await } else { Ok(None) };
    let maps = private_or(maps)?.map(|v| v.map(|v| tracker::parse_maps(&v)).unwrap_or_default());
    update(shared, puuid, |d| d.maps = maps);
    Ok(())
}

/// A tracker.gg answer, with a private refusal as `Load::Private` rather than an error: it isn't
/// shown as a failure or asked again.
fn private_or<T>(result: Result<T>) -> Result<Load<T>> {
    match result {
        Ok(v) => Ok(Load::Ready(v)),
        Err(e) if e.is::<tracker::Private>() => Ok(Load::Private),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_limit_isnt_reported_as_an_error() {
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        report(&shared, &anyhow::Error::new(crate::riot::RateLimited).context("/x"));
        assert!(lock(&shared).last_error.is_none(), "the footer counts it down instead");
        report(&shared, &anyhow::anyhow!("500 from /x"));
        assert!(lock(&shared).last_error.is_some());
    }

    fn profile_body(privacy: &str) -> String {
        let season = r#"{"type":"season","attributes":{"seasonId":"a6"},"stats":{"rank":{"value":null,"metadata":{"iconUrl":"https://x/tiersv2/14.png"}},"matchesWon":{"value":3},"matchesLost":{"value":2}}}"#;
        // As seen live, a "premier" profile carries no stats.
        let segments = if privacy == "public" { season } else { "" };
        format!(r#"{{"data":{{"metadata":{{"privacy":"{privacy}"}},"segments":[{segments}]}}}}"#)
    }

    /// Seen live: a "premier" profile loads without stats, and its other pages answer 451.
    #[tokio::test]
    async fn a_partly_private_profile_shows_as_private_without_an_error() {
        let answer = crate::tracker::tests::answer;
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let tracker = Tracker::canned(vec![answer(200, None, &profile_body("premier"))]);
        load_player(&tracker, &shared, "p", "a%23b", "a6").await.unwrap();
        let m = lock(&shared);
        let d = &m.players["p"];
        // The canned tracker panics on a second request, so nothing else was asked for.
        assert_eq!(d.profile, Load::Private);
        assert_eq!(d.maps.clone().map(|_| ()), Load::Private);
    }

    #[tokio::test]
    async fn a_profile_with_stats_loads_whatever_its_privacy_label() {
        let answer = crate::tracker::tests::answer;
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let body = profile_body("public").replace(r#""privacy":"public""#, r#""privacy":"premier""#);
        let tracker = Tracker::canned(vec![answer(200, None, &body), answer(404, None, "")]);
        load_player(&tracker, &shared, "p", "a%23b", "a6").await.unwrap();
        assert!(matches!(&lock(&shared).players["p"].profile, Load::Ready(p) if p.tier == 14));
    }

    #[tokio::test]
    async fn private_maps_are_private_not_a_failure() {
        let answer = crate::tracker::tests::answer;
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let tracker = Tracker::canned(vec![answer(200, None, &profile_body("public")), answer(451, None, "")]);
        load_player(&tracker, &shared, "p", "a%23b", "a6").await.unwrap();
        let m = lock(&shared);
        let d = &m.players["p"];
        assert!(matches!(&d.profile, Load::Ready(p) if p.tier == 14));
        assert_eq!(d.maps.clone().map(|_| ()), Load::Private);
    }

    /// tracker.gg's `platformUserId` is its own id, never the Riot puuid the lobby uses.
    #[tokio::test]
    async fn a_profile_loads_though_tracker_gg_ids_the_player_differently() {
        let act = "a6";
        let body = format!(
            r#"{{"data":{{"platformInfo":{{"platformUserId":"tracker-gg-own-id"}},"segments":[{{"type":"season","attributes":{{"seasonId":"{act}"}},"stats":{{"rank":{{"value":40,"metadata":{{"iconUrl":"https://x/tiersv2/14.png"}}}},"matchesWon":{{"value":3}},"matchesLost":{{"value":2}}}}}}]}}}}"#
        );
        let answer = crate::tracker::tests::answer;
        let tracker = Tracker::canned(vec![answer(200, None, &body), answer(404, None, "")]);
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let riot_puuid = "riot-puuid";
        load_player(&tracker, &shared, riot_puuid, "a%23b", act).await.unwrap();
        let m = lock(&shared);
        let Load::Ready(profile) = &m.players[riot_puuid].profile else { panic!("the profile is shown") };
        assert_eq!((profile.tier, profile.rr), (14, Some(40)));
    }
}
