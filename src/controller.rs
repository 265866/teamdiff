//! Network side: follows the game's state through presence and loads player data in the background.

use crate::goldstars::{self, Definitions};
use crate::local_files::browser_profile_dir;
use crate::model::{Load, Lobby, Model, Phase, PlayerData, Postgame, parse_coregame, parse_pregame};
use crate::presence::LoopState;
use crate::riot::{Riot, connect};
use crate::stats::{parse_comp_updates, parse_match};
use crate::tracker::{self, Tracker, riot_id_path};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Model>>;

/// Matches used for each player's K/D and HS%.
const RECENT_MATCHES: usize = 5;
/// Acts before the current one shown in a player's details.
const HISTORY_ACTS: usize = 4;
/// How long a failed load waits before the next tick may retry it.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// When you and your party are reloaded after a match. tracker.gg needs a while to add the match and
/// caches profiles for three minutes.
const REFRESH_AFTER_MATCH: Duration = Duration::from_secs(240);

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

/// Lets the menus load `ids` again after `delay`, unless the next match has started by then: that
/// match loads every player once.
async fn refresh_after_match(shared: Shared, claims: Claims, ids: Vec<String>, match_id: String, delay: Duration) {
    tokio::time::sleep(delay).await;
    if lock(&shared).postgame.as_ref().is_some_and(|p| p.match_id == match_id) {
        release_after(claims, ids, Duration::ZERO).await;
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
        if let Ok(Some(v)) = riot.goldstar_defs().await {
            lock(&shared).defs = Arc::new(Definitions::from_response(&v));
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
    /// Players whose older acts were loaded for their details.
    history: Claims,
    goldstars_requested: Arc<AtomicBool>,
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
            history: Claims::default(),
            goldstars_requested: Arc::default(),
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
        self.load_history();
        self.tracker.close_if_idle();
        Ok(())
    }

    fn set_phase(&self, phase: Phase) {
        lock(&self.shared).phase = phase;
    }

    fn menus(&mut self) {
        self.pregame_id = None;
        if let Some(id) = self.core_id.take() {
            let competitive = lock(&self.shared).lobby.as_ref().is_some_and(Lobby::is_competitive);
            if competitive {
                self.start_postgame(id);
            }
        }
        let party: Vec<String> = {
            let mut m = lock(&self.shared);
            if m.phase != Phase::Postgame {
                m.phase = Phase::Menus;
            }
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
        self.load_my_goldstars();
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
            m.postgame = None;
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

    /// Loads older acts for the player whose details are open, once tracker.gg has their profile.
    fn load_history(&mut self) {
        let Some((puuid, acts)) = history_wanted(&lock(&self.shared), chrono::Utc::now()) else { return };
        let Some(riot_id) = self.riot_id(&puuid) else { return };
        if !claim(&self.history, &puuid) {
            return;
        }
        let (tracker, shared, claims) = (self.tracker.clone(), self.shared.clone(), self.history.clone());
        tokio::spawn(async move {
            for act in acts {
                let loaded =
                    |d: &PlayerData| d.history.iter().chain(d.profile.ready().into_iter().flat_map(|p| &p.acts)).any(|a| a.act_id == act);
                if lock(&shared).players.get(&puuid).is_some_and(loaded) {
                    continue;
                }
                match tracker.season(&riot_id, &act).await {
                    Ok(v) => {
                        if let Some(a) = v.as_ref().and_then(tracker::parse_season).filter(|a| a.record.games() > 0) {
                            lock(&shared).players.entry(puuid.clone()).or_default().history.push(a);
                        }
                    }
                    // Older acts the player keeps private aren't a failure; they just aren't shown.
                    Err(e) if e.is::<tracker::Private>() => return,
                    Err(e) => {
                        report(&shared, &e);
                        release_after(claims, vec![puuid], RETRY_AFTER).await;
                        return;
                    }
                }
            }
        });
    }

    fn load_my_goldstars(&mut self) {
        if self.goldstars_requested.swap(true, Ordering::Relaxed) {
            return;
        }
        let (riot, shared, requested) = (self.riot.clone(), self.shared.clone(), self.goldstars_requested.clone());
        tokio::spawn(async move {
            match riot.player_goldstars().await {
                Ok(v) => {
                    let mut m = lock(&shared);
                    let parsed = v.map(|v| goldstars::parse_player(&v, &m.defs)).unwrap_or_default();
                    m.my_goldstars = Some(parsed);
                }
                Err(e) => {
                    report(&shared, &e);
                    tokio::time::sleep(RETRY_AFTER).await;
                    requested.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    fn start_postgame(&mut self, match_id: String) {
        let party: Vec<String> = {
            let mut m = lock(&self.shared);
            m.phase = Phase::Postgame;
            let lobby = m.lobby.take();
            m.postgame = Some(Postgame { match_id: match_id.clone(), lobby, ..Default::default() });
            let my_party = m.my_presence().and_then(|p| p.party_id.clone());
            let mates = m.presences.values().filter(|p| p.party_id.is_some() && p.party_id == my_party).map(|p| p.puuid.clone());
            std::iter::once(m.me.clone()).chain(mates).collect()
        };
        self.goldstars_requested.store(false, Ordering::Relaxed);
        tokio::spawn(postgame(self.riot.clone(), self.shared.clone(), match_id.clone()));
        // Your stats change with this match; the menus reload you and your party once tracker.gg has it.
        tokio::spawn(refresh_after_match(self.shared.clone(), self.requested.clone(), party, match_id, REFRESH_AFTER_MATCH));
    }
}

/// The player whose details are open and the older acts to load for them: only once tracker.gg has
/// their profile, and never for an incognito player.
fn history_wanted(m: &Model, now: chrono::DateTime<chrono::Utc>) -> Option<(String, Vec<String>)> {
    let puuid = m.viewing.clone()?;
    let lobby = m.lobby.as_ref().or(m.postgame.as_ref().and_then(|p| p.lobby.as_ref()));
    let incognito = puuid != m.me && lobby.is_some_and(|l| l.slots.iter().any(|s| s.puuid == puuid && s.incognito));
    let found = m.players.get(&puuid).is_some_and(|d| d.profile.ready().is_some());
    if incognito || !found {
        return None;
    }
    let current = m.content.current_act(now)?;
    let acts = m.content.acts.iter().rev().skip_while(|a| a.id != current.id).skip(1).take(HISTORY_ACTS).map(|a| a.id.clone()).collect();
    Some((puuid, acts))
}

fn update(shared: &Shared, puuid: &str, f: impl FnOnce(&mut PlayerData)) {
    f(lock(shared).players.entry(puuid.to_string()).or_default());
}

/// Rank, act record, map records and recent matches for one player from tracker.gg. A player
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
            d.recent = state(private);
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
    let Load::Ready(v) = private_or(tracker.matches(riot_id).await)? else {
        update(shared, puuid, |d| d.recent = Load::Private);
        return Ok(());
    };
    let recent = v.as_ref().map(|v| tracker::parse_matches(v, RECENT_MATCHES)).unwrap_or_default();
    update(shared, puuid, |d| d.recent = Load::Ready(recent));
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

/// After a match: waits for its details, your RR change and the accolades Riot awarded.
async fn postgame(riot: Arc<Riot>, shared: Shared, match_id: String) {
    const POLL: Duration = Duration::from_secs(5);
    const ATTEMPTS: u32 = 120;
    let still_current = |shared: &Shared| lock(shared).postgame.as_ref().is_some_and(|p| p.match_id == match_id);

    let mut summary = None;
    for _ in 0..ATTEMPTS {
        if !still_current(&shared) {
            return;
        }
        if let Ok(Some(m)) = riot.match_details(&match_id).await.map(|v| v.as_ref().and_then(parse_match)) {
            summary = Some(Arc::new(m));
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    let Some(summary) = summary else {
        lock(&shared).give_up_postgame(&match_id);
        return;
    };
    {
        let mut m = lock(&shared);
        if let Some(p) = m.postgame.as_mut().filter(|p| p.match_id == match_id) {
            p.summary = Some(summary.clone());
            p.shown_at = Some(Instant::now());
        }
    }
    let (mut rr_done, mut accolades_done) = (summary.queue != "competitive", false);
    for _ in 0..ATTEMPTS {
        if !still_current(&shared) || (rr_done && accolades_done) {
            return;
        }
        if !rr_done
            && let Ok(Some(v)) = riot.my_comp_updates(3).await
            && let Some(u) = parse_comp_updates(&v).into_iter().find(|u| u.match_id == match_id)
        {
            if let Some(p) = lock(&shared).postgame.as_mut().filter(|p| p.match_id == match_id) {
                p.rr = Some(u);
            }
            rr_done = true;
        }
        if !accolades_done && let Ok(Some(v)) = riot.player_goldstars().await {
            let mut m = lock(&shared);
            let parsed = goldstars::parse_player(&v, &m.defs);
            if let Some(found) = parsed.match_near(summary.start_ms).cloned() {
                if let Some(p) = m.postgame.as_mut().filter(|p| p.match_id == match_id) {
                    p.accolades = Some(found);
                }
                accolades_done = true;
            }
            m.my_goldstars = Some(parsed);
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_refresh_after_a_match_leaves_the_next_match_alone() {
        let shared: Shared = Arc::new(Mutex::new(crate::demo::model("victory")));
        let id = lock(&shared).postgame.as_ref().unwrap().match_id.clone();
        let claims: Claims = Arc::default();
        claim(&claims, "me");
        refresh_after_match(shared.clone(), claims.clone(), vec!["me".into()], "an older match".into(), Duration::ZERO).await;
        assert!(!claim(&claims, "me"), "a later match keeps its claims");
        refresh_after_match(shared, claims.clone(), vec!["me".into()], id, Duration::ZERO).await;
        assert!(claim(&claims, "me"), "released for the menus to reload");
    }

    #[test]
    fn a_rate_limit_isnt_reported_as_an_error() {
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        report(&shared, &anyhow::Error::new(crate::riot::RateLimited).context("/x"));
        assert!(lock(&shared).last_error.is_none(), "the footer counts it down instead");
        report(&shared, &anyhow::anyhow!("500 from /x"));
        assert!(lock(&shared).last_error.is_some());
    }

    #[test]
    fn older_acts_load_only_for_a_found_player_who_isnt_incognito() {
        let mut m = crate::demo::model("in-game");
        let now = chrono::Utc::now();
        assert_eq!(history_wanted(&m, now), None, "no details open");

        m.viewing = Some("raze".into());
        let acts = ["a5", "a4", "a3", "a2"].map(String::from).to_vec();
        assert_eq!(history_wanted(&m, now), Some(("raze".into(), acts)));

        m.players.get_mut("raze").unwrap().profile = Load::Private;
        assert_eq!(history_wanted(&m, now), None, "private on tracker.gg");
        m.players.get_mut("raze").unwrap().profile = Load::Missing;
        assert_eq!(history_wanted(&m, now), None, "unknown to tracker.gg");

        m.viewing = Some("cypher".into());
        assert!(m.players["cypher"].profile.ready().is_some());
        assert_eq!(history_wanted(&m, now), None, "incognito, even with a profile loaded earlier");
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
        assert_eq!((d.maps.clone().map(|_| ()), d.recent.clone().map(|_| ())), (Load::Private, Load::Private));
    }

    #[tokio::test]
    async fn a_profile_with_stats_loads_whatever_its_privacy_label() {
        let answer = crate::tracker::tests::answer;
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let body = profile_body("public").replace(r#""privacy":"public""#, r#""privacy":"premier""#);
        let tracker = Tracker::canned(vec![answer(200, None, &body), answer(404, None, ""), answer(404, None, "")]);
        load_player(&tracker, &shared, "p", "a%23b", "a6").await.unwrap();
        assert_eq!(lock(&shared).players["p"].profile.ready().map(|p| p.tier), Some(14));
    }

    #[tokio::test]
    async fn private_maps_or_matches_are_private_not_a_failure() {
        let answer = crate::tracker::tests::answer;
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let refused = || answer(451, None, "");
        let tracker = Tracker::canned(vec![answer(200, None, &profile_body("public")), refused(), refused()]);
        load_player(&tracker, &shared, "p", "a%23b", "a6").await.unwrap();
        let m = lock(&shared);
        let d = &m.players["p"];
        assert_eq!(d.profile.ready().map(|p| p.tier), Some(14));
        assert_eq!((d.maps.clone().map(|_| ()), d.recent.clone().map(|_| ())), (Load::Private, Load::Private));
    }

    /// tracker.gg's `platformUserId` is its own id, never the Riot puuid the lobby uses.
    #[tokio::test]
    async fn a_profile_loads_though_tracker_gg_ids_the_player_differently() {
        let act = "a6";
        let body = format!(
            r#"{{"data":{{"platformInfo":{{"platformUserId":"tracker-gg-own-id"}},"segments":[{{"type":"season","attributes":{{"seasonId":"{act}"}},"stats":{{"rank":{{"value":40,"metadata":{{"iconUrl":"https://x/tiersv2/14.png"}}}},"matchesWon":{{"value":3}},"matchesLost":{{"value":2}}}}}}]}}}}"#
        );
        let answer = crate::tracker::tests::answer;
        let tracker = Tracker::canned(vec![answer(200, None, &body), answer(404, None, ""), answer(404, None, "")]);
        let shared: Shared = Arc::new(Mutex::new(Model::new(Arc::default())));
        let riot_puuid = "riot-puuid";
        load_player(&tracker, &shared, riot_puuid, "a%23b", act).await.unwrap();
        let m = lock(&shared);
        let profile = m.players[riot_puuid].profile.ready().expect("the profile is shown");
        assert_eq!((profile.tier, profile.rr), (14, Some(40)));
    }
}
