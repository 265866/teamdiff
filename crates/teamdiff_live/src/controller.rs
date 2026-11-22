//! Network side: follows the game's state through presence and loads player data in the
//! background, into the model the UI draws.

use std::collections::HashSet;
use std::iter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use jiff::Timestamp;
use teamdiff_core::goldstars::{self, Definitions};
use teamdiff_core::model::{
    self, LastError, Load, Lobby, Model, Phase, PlayerData, Postgame, RiotId,
};
use teamdiff_core::presence::LoopState;
use teamdiff_core::shared::{self, Shared};
use teamdiff_core::{local_files, stats, tracker as parse};
use tokio::time;

use crate::mutex;
use crate::riot::{self, PlayerName, RateLimited, Riot};
use crate::tracker::{self, Private, Tracker, Transport};

/// Matches used for each player's K/D and HS%.
const RECENT_MATCHES: usize = 5;
/// Acts before the current one shown in a player's details.
const HISTORY_ACTS: usize = 4;
/// How long a failed load waits before the next tick may retry it.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// When you and your party are reloaded after a match. tracker.gg needs a while to add the match and
/// caches profiles for three minutes.
const REFRESH_AFTER_MATCH: Duration = Duration::from_secs(240);

/// Shows a failed request in the footer, except a rate limit, which the footer counts down
/// instead.
fn report(shared: &Shared, e: &anyhow::Error) {
    if e.is::<RateLimited>() {
        return;
    }
    shared::lock(shared).last_error = Some(LastError {
        message: format!("{e:#}"),
        at: Instant::now(),
    });
}

/// Ids whose load is running or done. A failed load releases its ids after `RETRY_AFTER` so a later
/// tick tries again.
#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        shared_ownership,
        reason = "each load runs on a task of its own and releases its ids from there when it \
                  fails, after the tick that claimed them has moved on"
    )
)]
type Claims = Arc<Mutex<HashSet<String>>>;

/// Claims `id`, unless a load already has it.
fn claim(claims: &Claims, id: &str) -> bool {
    mutex::lock(claims).insert(id.to_string())
}

/// Releases `ids` after `delay`, so a later tick loads them again.
async fn release_after(claims: &Claims, ids: Vec<String>, delay: Duration) {
    time::sleep(delay).await;
    let mut c = mutex::lock(claims);
    for id in ids {
        c.remove(&id);
    }
}

/// Lets the menus load `ids` again after `delay`, unless the next match has started by then: that
/// match loads every player once.
async fn refresh_after_match(
    shared: Shared,
    claims: Claims,
    ids: Vec<String>,
    match_id: String,
    delay: Duration,
) {
    time::sleep(delay).await;
    let current = shared::lock(&shared)
        .postgame
        .as_ref()
        .is_some_and(|p| p.match_id == match_id);
    if current {
        release_after(&claims, ids, Duration::ZERO).await;
    }
}

/// Runs forever: loads names from valorant-api.com, connects to the Riot Client, follows the game,
/// and reconnects when the client goes away. With `dump_dir`, saves the raw responses there.
pub async fn run(shared: Shared, dump_dir: Option<PathBuf>) {
    loop {
        match riot::fetch_content().await {
            Ok(c) => {
                shared::lock(&shared).content = c;
                break;
            }
            Err(e) => {
                shared::lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    loop {
        let connected = riot::connect(dump_dir.clone()).await.and_then(|r| {
            let profile = local_files::browser_profile_dir()
                .ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
            let tracker = Tracker::new(Arc::clone(&r.net), dump_dir.clone(), profile);
            Ok((Arc::new(r), Arc::new(tracker)))
        });
        let (riot, tracker) = match connected {
            Ok(c) => c,
            Err(e) => {
                shared::lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        {
            let mut m = shared::lock(&shared);
            m.me.clone_from(&riot.puuid);
            m.region = riot.log.region.to_uppercase();
            m.last_error = None;
            m.net = Some(Arc::clone(&riot.net));
        }
        if let Ok(Some(v)) = riot.goldstar_defs().await {
            shared::lock(&shared).defs = Definitions::from_response(&v);
        }
        let mut session = Session::new(riot, tracker, Arc::clone(&shared));
        loop {
            match session.tick().await {
                Ok(()) => {}
                Err(e) if session.client_gone => {
                    shared::lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                    break;
                }
                Err(e) => report(&shared, &e),
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// One connection to the Riot Client: what the last ticks saw, and which loads they started.
struct Session {
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the loads run on tasks of their own, which keep sending requests after the \
                      tick that started them returns"
        )
    )]
    riot: Arc<Riot>,
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the loads run on tasks of their own, which keep sending requests after the \
                      tick that started them returns"
        )
    )]
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
    /// Past matches whose parties were loaded.
    match_parties: Claims,
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the accolade load clears it from its own task when it fails, after the tick \
                      that set it has moved on"
        )
    )]
    goldstars_requested: Arc<AtomicBool>,
    client_gone: bool,
}

/// The player whose details are open, and the older acts to load for them.
#[derive(Debug, PartialEq, Eq)]
struct History {
    puuid: String,
    acts: Vec<String>,
}

impl Session {
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the session hands both clients to the loads it starts, each on a task of \
                      its own"
        )
    )]
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
            match_parties: Claims::default(),
            goldstars_requested: Arc::default(),
            client_gone: false,
        }
    }

    /// Reads where you are in the game and loads what that screen shows.
    async fn tick(&mut self) -> Result<()> {
        let presences = match self.riot.presences().await {
            Ok(p) => p,
            Err(e) => {
                self.client_gone = true;
                return Err(e);
            }
        };
        let me = self.riot.puuid.clone();
        let state = presences
            .iter()
            .find(|p| p.puuid == me)
            .and_then(|p| p.state.clone());
        {
            let mut m = shared::lock(&self.shared);
            m.presences = presences
                .into_iter()
                .map(|p| (p.puuid.clone(), p))
                .collect();
        }
        match state {
            None => self.set_phase(Phase::Waiting("VALORANT isn't running".into())),
            Some(LoopState::Pregame) => self.pregame().await?,
            Some(LoopState::Ingame) => self.ingame().await?,
            Some(LoopState::Menus | LoopState::Other(_)) => self.menus(),
        }
        self.load_history();
        self.tracker.close_if_idle();
        Ok(())
    }

    fn set_phase(&self, phase: Phase) {
        shared::lock(&self.shared).phase = phase;
    }

    /// In the menus: shows the results of the match that just ended, and loads you and your
    /// party.
    fn menus(&mut self) {
        self.pregame_id = None;
        if let Some(id) = self.core_id.take() {
            let competitive = shared::lock(&self.shared)
                .lobby
                .as_ref()
                .is_some_and(Lobby::is_competitive);
            if competitive {
                self.start_postgame(id);
            }
        }
        let party: Vec<String> = {
            let mut m = shared::lock(&self.shared);
            if m.phase != Phase::Postgame {
                m.phase = Phase::Menus;
            }
            m.lobby = None;
            let my_party = m.my_presence().and_then(|p| p.party_id.clone());
            m.presences
                .values()
                .filter(|p| p.party_id.is_some() && p.party_id == my_party)
                .map(|p| p.puuid.clone())
                .collect()
        };
        let me = self.riot.puuid.clone();
        self.load_names(
            iter::once(me.clone())
                .chain(party.iter().cloned())
                .collect(),
        );
        self.load_player(me);
        for p in party {
            self.load_player(p);
        }
        self.load_my_goldstars();
    }

    /// In agent select: shows your team and loads its players, fresh for every match.
    async fn pregame(&mut self) -> Result<()> {
        self.core_id = None;
        if self.pregame_id.is_none() {
            self.pregame_id = self.riot.pregame_match_id().await?;
        }
        let Some(id) = self.pregame_id.clone() else {
            return Ok(());
        };
        if self.loaded_match.as_deref() != Some(id.as_str()) {
            mutex::lock(&self.requested).clear();
            self.loaded_match = Some(id.clone());
        }
        let Some(v) = self.riot.pregame_match(&id).await? else {
            self.pregame_id = None;
            return Ok(());
        };
        if let Some(lobby) = model::parse_pregame(&v, Instant::now()) {
            self.show_lobby(Phase::Pregame, lobby);
        }
        Ok(())
    }

    /// In a match: shows both teams, loaded once per match, and loads their players.
    async fn ingame(&mut self) -> Result<()> {
        self.pregame_id = None;
        if self.core_id.is_none() {
            self.core_id = self.riot.coregame_match_id().await?;
        }
        let Some(id) = self.core_id.clone() else {
            return Ok(());
        };
        let current = shared::lock(&self.shared).ingame_lobby(&id).cloned();
        let lobby = if let Some(l) = current {
            l
        } else {
            let Some(v) = self.riot.coregame_match(&id).await? else {
                return Ok(());
            };
            let Some(lobby) = model::parse_coregame(&v, &self.riot.puuid) else {
                return Ok(());
            };
            lobby
        };
        self.show_lobby(Phase::Ingame, lobby);
        self.load_match_parties();
        Ok(())
    }

    /// Loads who partied with whom in each player's last match, so a party shows in game even when
    /// only one of its members has a public tracker.gg profile.
    fn load_match_parties(&mut self) {
        let wanted: Vec<String> = {
            let m = shared::lock(&self.shared);
            let Some(lobby) = &m.lobby else { return };
            let mut ids: Vec<String> = lobby
                .slots
                .iter()
                .filter(|s| !s.incognito)
                .filter_map(|s| m.players.get(&s.puuid)?.last_match.clone())
                .filter(|id| !m.match_parties.contains_key(id))
                .collect();
            ids.sort();
            ids.dedup();
            ids
        };
        for id in wanted {
            if !claim(&self.match_parties, &id) {
                continue;
            }
            let (tracker, shared, claims) = (
                Arc::clone(&self.tracker),
                Arc::clone(&self.shared),
                Arc::clone(&self.match_parties),
            );
            tokio::spawn(async move {
                match tracker.match_players(&id).await {
                    Ok(v) => {
                        let parties = v.map_or_default(|v| parse::parse_match_parties(&v));
                        shared::lock(&shared).match_parties.insert(id, parties);
                    }
                    Err(e) => {
                        report(&shared, &e);
                        release_after(&claims, vec![id], RETRY_AFTER).await;
                    }
                }
            });
        }
    }

    /// Shows `lobby` and, in a competitive match, loads everyone in it who isn't incognito.
    fn show_lobby(&mut self, phase: Phase, lobby: Lobby) {
        let competitive = lobby.is_competitive();
        let named: Vec<String> = lobby
            .slots
            .iter()
            .filter(|s| !s.incognito || s.puuid == self.riot.puuid)
            .map(|s| s.puuid.clone())
            .collect();
        {
            let mut m = shared::lock(&self.shared);
            m.phase = phase;
            m.postgame = None;
            m.lobby = Some(lobby);
        }
        if competitive {
            self.load_names(named.clone());
            for p in named {
                self.load_player(p);
            }
        }
    }

    /// Loads the Riot IDs of `puuids` that no load has yet.
    fn load_names(&mut self, puuids: Vec<String>) {
        let missing: Vec<String> = puuids
            .into_iter()
            .filter(|p| claim(&self.named, p))
            .collect();
        if missing.is_empty() {
            return;
        }
        let (riot, shared, claims) = (
            Arc::clone(&self.riot),
            Arc::clone(&self.shared),
            Arc::clone(&self.named),
        );
        tokio::spawn(async move {
            match riot.names(&missing).await {
                Ok(names) => {
                    let mut m = shared::lock(&shared);
                    for PlayerName { puuid, riot_id } in names {
                        m.players.entry(puuid).or_default().name = Some(riot_id);
                    }
                }
                Err(e) => {
                    report(&shared, &e);
                    release_after(&claims, missing, RETRY_AFTER).await;
                }
            }
        });
    }

    /// The player's Riot ID as tracker.gg's URLs spell it, once the name lookup has returned it.
    fn riot_id(&self, puuid: &str) -> Option<String> {
        shared::lock(&self.shared)
            .players
            .get(puuid)
            .and_then(|d| d.name.as_ref())
            .map(|RiotId { name, tag }| tracker::riot_id_path(name, tag))
    }

    /// Loads a player's stats from tracker.gg. Waits for their name; a later tick calls this again.
    fn load_player(&mut self, puuid: String) {
        let Some(riot_id) = self.riot_id(&puuid) else {
            return;
        };
        let Some(act) = shared::lock(&self.shared)
            .content
            .current_act(Timestamp::now())
            .map(|a| a.id.clone())
        else {
            return;
        };
        if !claim(&self.requested, &puuid) {
            return;
        }
        let (tracker, shared, claims) = (
            Arc::clone(&self.tracker),
            Arc::clone(&self.shared),
            Arc::clone(&self.requested),
        );
        tokio::spawn(async move {
            if let Err(e) = load_player(&tracker, &shared, &puuid, &riot_id, &act).await {
                report(&shared, &e);
                release_after(&claims, vec![puuid], RETRY_AFTER).await;
            }
        });
    }

    /// Loads older acts for the player whose details are open, once tracker.gg has their profile.
    fn load_history(&mut self) {
        let Some(History { puuid, acts }) =
            history_wanted(&shared::lock(&self.shared), Timestamp::now())
        else {
            return;
        };
        let Some(riot_id) = self.riot_id(&puuid) else {
            return;
        };
        if !claim(&self.history, &puuid) {
            return;
        }
        let (tracker, shared, claims) = (
            Arc::clone(&self.tracker),
            Arc::clone(&self.shared),
            Arc::clone(&self.history),
        );
        tokio::spawn(async move {
            for act in acts {
                let loaded = |d: &PlayerData| {
                    d.history
                        .iter()
                        .chain(d.profile.ready().into_iter().flat_map(|p| &p.acts))
                        .any(|a| a.act_id == act)
                };
                if shared::lock(&shared)
                    .players
                    .get(&puuid)
                    .is_some_and(loaded)
                {
                    continue;
                }
                match tracker.season(&riot_id, &act).await {
                    Ok(v) => {
                        if let Some(a) = v
                            .as_ref()
                            .and_then(parse::parse_season)
                            .filter(|a| a.record.games() > 0)
                        {
                            shared::lock(&shared)
                                .players
                                .entry(puuid.clone())
                                .or_default()
                                .history
                                .push(a);
                        }
                    }
                    Err(e) if e.is::<Private>() => return,
                    Err(e) => {
                        report(&shared, &e);
                        release_after(&claims, vec![puuid], RETRY_AFTER).await;
                        return;
                    }
                }
            }
        });
    }

    /// Loads your own accolades, once until a match ends or the load fails.
    fn load_my_goldstars(&mut self) {
        if self.goldstars_requested.swap(true, Ordering::Relaxed) {
            return;
        }
        let (riot, shared, requested) = (
            Arc::clone(&self.riot),
            Arc::clone(&self.shared),
            Arc::clone(&self.goldstars_requested),
        );
        tokio::spawn(async move {
            match riot.player_goldstars().await {
                Ok(v) => {
                    let mut m = shared::lock(&shared);
                    let parsed = v.map_or_default(|v| goldstars::parse_player(&v, &m.defs));
                    m.my_goldstars = Some(parsed);
                }
                Err(e) => {
                    report(&shared, &e);
                    time::sleep(RETRY_AFTER).await;
                    requested.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    /// Shows the results of the match `match_id` as Riot has them, and reloads you and your
    /// party once tracker.gg has the match.
    fn start_postgame(&mut self, match_id: String) {
        let party: Vec<String> = {
            let mut m = shared::lock(&self.shared);
            m.phase = Phase::Postgame;
            let lobby = m.lobby.take();
            m.postgame = Some(Postgame {
                match_id: match_id.clone(),
                lobby,
                ..Default::default()
            });
            let my_party = m.my_presence().and_then(|p| p.party_id.clone());
            let mates = m
                .presences
                .values()
                .filter(|p| p.party_id.is_some() && p.party_id == my_party)
                .map(|p| p.puuid.clone());
            iter::once(m.me.clone()).chain(mates).collect()
        };
        self.goldstars_requested.store(false, Ordering::Relaxed);
        tokio::spawn(postgame(
            Arc::clone(&self.riot),
            Arc::clone(&self.shared),
            match_id.clone(),
        ));
        tokio::spawn(refresh_after_match(
            Arc::clone(&self.shared),
            Arc::clone(&self.requested),
            party,
            match_id,
            REFRESH_AFTER_MATCH,
        ));
    }
}

/// The player whose details are open and the older acts to load for them: only once tracker.gg has
/// their profile, and never for an incognito player.
fn history_wanted(m: &Model, now: Timestamp) -> Option<History> {
    let puuid = m.viewing.clone()?;
    let lobby = m
        .lobby
        .as_ref()
        .or(m.postgame.as_ref().and_then(|p| p.lobby.as_ref()));
    let incognito = puuid != m.me
        && lobby.is_some_and(|l| l.slots.iter().any(|s| s.puuid == puuid && s.incognito));
    let found = m
        .players
        .get(&puuid)
        .is_some_and(|d| d.profile.ready().is_some());
    if incognito || !found {
        return None;
    }
    let current = m.content.current_act(now)?;
    let acts: Vec<String> = m
        .content
        .acts
        .iter()
        .rev()
        .skip_while(|a| a.id != current.id)
        .skip(1)
        .take(HISTORY_ACTS)
        .map(|a| a.id.clone())
        .collect();
    Some(History { puuid, acts })
}

/// Changes what is known about the player `puuid`.
fn update(shared: &Shared, puuid: &str, f: impl FnOnce(&mut PlayerData)) {
    f(shared::lock(shared)
        .players
        .entry(puuid.to_string())
        .or_default());
}

/// Rank, act record, map records and recent matches for one player from tracker.gg. A player
/// tracker.gg doesn't know, or whose profile is private, is shown as such and not asked about again;
/// any other failure is returned so the load is retried.
async fn load_player<T: Transport>(
    tracker: &Tracker<T>,
    shared: &Shared,
    puuid: &str,
    riot_id: &str,
    act: &str,
) -> Result<()> {
    let unavailable = |why: Unavailable| {
        update(shared, puuid, |d| {
            d.profile = why.load();
            d.maps = why.load();
            d.recent = why.load();
            d.sides.clear();
            d.last_match = None;
        });
    };
    let v = match tracker.profile(riot_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            unavailable(Unavailable::Missing);
            return Ok(());
        }
        Err(e) if e.is::<Private>() => {
            unavailable(Unavailable::Private);
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let shared_stats = v["data"]["segments"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s["type"] == "season"));
    if !shared_stats
        && v["data"]["metadata"]["privacy"]
            .as_str()
            .is_some_and(|p| !p.eq_ignore_ascii_case("public"))
    {
        unavailable(Unavailable::Private);
        return Ok(());
    }
    let profile = parse::parse_profile(&v, act);
    drop(v);
    let played = profile.acts.iter().any(|a| a.act_id == act);
    update(shared, puuid, |d| d.profile = Load::Ready(profile));
    let maps = if played {
        tracker.maps(riot_id, act).await
    } else {
        Ok(None)
    };
    let maps = private_or(maps)?.map(|v| v.map_or_default(|v| parse::parse_maps(&v)));
    update(shared, puuid, |d| d.maps = maps);
    let Load::Ready(v) = private_or(tracker.matches(riot_id).await)? else {
        update(shared, puuid, |d| {
            d.recent = Load::Private;
            d.sides.clear();
            d.last_match = None;
        });
        return Ok(());
    };
    let recent = v
        .as_ref()
        .map_or_default(|v| parse::parse_matches(v, RECENT_MATCHES));
    let sides = v.as_ref().map_or_default(parse::parse_match_sides);
    let last_match = v
        .as_ref()
        .and_then(|v| v["data"]["matches"][0]["attributes"]["id"].as_str())
        .map(str::to_lowercase);
    update(shared, puuid, |d| {
        d.recent = Load::Ready(recent);
        d.sides = sides;
        d.last_match = last_match;
    });
    Ok(())
}

/// Why tracker.gg has no data for a player.
#[derive(Debug, Clone, Copy)]
enum Unavailable {
    /// The player isn't on tracker.gg.
    Missing,
    /// The player's profile is private.
    Private,
}

impl Unavailable {
    /// The load state that shows it.
    fn load<T>(self) -> Load<T> {
        match self {
            Unavailable::Missing => Load::Missing,
            Unavailable::Private => Load::Private,
        }
    }
}

/// A tracker.gg answer, with a private refusal as `Load::Private` rather than an error: it isn't
/// shown as a failure or asked again.
fn private_or<T>(result: Result<T>) -> Result<Load<T>> {
    match result {
        Ok(v) => Ok(Load::Ready(v)),
        Err(e) if e.is::<Private>() => Ok(Load::Private),
        Err(e) => Err(e),
    }
}

/// After a match: waits for its details, your RR change and the accolades Riot awarded.
#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        shared_ownership,
        reason = "the postgame runs on a task of its own for up to twenty minutes, sending its \
                  requests long after the tick that started it returned"
    )
)]
async fn postgame(riot: Arc<Riot>, shared: Shared, match_id: String) {
    const POLL: Duration = Duration::from_secs(5);
    const ATTEMPTS: u32 = 120;
    let still_current = |shared: &Shared| {
        shared::lock(shared)
            .postgame
            .as_ref()
            .is_some_and(|p| p.match_id == match_id)
    };

    let mut summary = None;
    for _ in 0..ATTEMPTS {
        if !still_current(&shared) {
            return;
        }
        if let Ok(Some(m)) = riot
            .match_details(&match_id)
            .await
            .map(|v| v.as_ref().and_then(stats::parse_match))
        {
            summary = Some(Arc::new(m));
            break;
        }
        time::sleep(POLL).await;
    }
    let Some(summary) = summary else {
        shared::lock(&shared).give_up_postgame(&match_id);
        return;
    };
    {
        let mut m = shared::lock(&shared);
        if let Some(p) = m.postgame.as_mut().filter(|p| p.match_id == match_id) {
            p.summary = Some(Arc::clone(&summary));
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
            && let Some(u) = stats::parse_comp_updates(&v)
                .into_iter()
                .find(|u| u.match_id == match_id)
        {
            if let Some(p) = shared::lock(&shared)
                .postgame
                .as_mut()
                .filter(|p| p.match_id == match_id)
            {
                p.rr = Some(u);
            }
            rr_done = true;
        }
        if !accolades_done && let Ok(Some(v)) = riot.player_goldstars().await {
            let mut m = shared::lock(&shared);
            let parsed = goldstars::parse_player(&v, &m.defs);
            if let Some(found) = parsed.match_near(summary.start_ms).cloned() {
                if let Some(p) = m.postgame.as_mut().filter(|p| p.match_id == match_id) {
                    p.accolades = Some(found);
                }
                accolades_done = true;
            }
            m.my_goldstars = Some(parsed);
        }
        time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use teamdiff_core::content::Content;
    use teamdiff_core::demo;
    use teamdiff_core::tracker::MatchSide;
    use tokio::runtime::Builder;

    use super::*;
    use crate::tracker::tests::{self as canned, Canned};

    /// Runs `test` on a runtime whose clock stands still until every task waits on it.
    fn block_on<F: Future>(test: F) -> F::Output {
        Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("the test runtime should start")
            .block_on(test)
    }

    fn new_shared(model: Model) -> Shared {
        Arc::new(Mutex::new(model))
    }

    #[test]
    fn the_refresh_after_a_match_leaves_the_next_match_alone() {
        block_on(async {
            let shared = new_shared(demo::model("victory"));
            let id = shared::lock(&shared)
                .postgame
                .as_ref()
                .unwrap()
                .match_id
                .clone();
            let claims = Claims::default();
            claim(&claims, "me");
            refresh_after_match(
                Arc::clone(&shared),
                Arc::clone(&claims),
                vec!["me".into()],
                "an older match".into(),
                Duration::ZERO,
            )
            .await;
            assert!(!claim(&claims, "me"), "a later match keeps its claims");
            refresh_after_match(
                shared,
                Arc::clone(&claims),
                vec!["me".into()],
                id,
                Duration::ZERO,
            )
            .await;
            assert!(claim(&claims, "me"), "released for the menus to reload");
        });
    }

    #[test]
    fn the_controller_can_run_on_a_task_of_its_own() {
        fn spawnable<F: Future + Send + 'static>(_: &F) {}
        let task = run(new_shared(Model::new(Content::default())), None);
        spawnable(&task);
    }

    #[test]
    fn a_rate_limit_isnt_reported_as_an_error() {
        let shared = new_shared(Model::new(Content::default()));
        report(&shared, &anyhow::Error::new(RateLimited).context("/x"));
        assert!(
            shared::lock(&shared).last_error.is_none(),
            "the footer counts it down instead"
        );
        report(&shared, &anyhow!("500 from /x"));
        assert!(shared::lock(&shared).last_error.is_some());
    }

    #[test]
    fn older_acts_load_only_for_a_found_player_who_isnt_incognito() {
        let mut m = demo::model("in-game");
        let now = Timestamp::now();
        assert_eq!(history_wanted(&m, now), None, "no details open");

        m.viewing = Some("raze".into());
        let acts = ["a5", "a4", "a3", "a2"].map(String::from).to_vec();
        assert_eq!(
            history_wanted(&m, now),
            Some(History {
                puuid: "raze".into(),
                acts
            })
        );

        m.players.get_mut("raze").unwrap().profile = Load::Private;
        assert_eq!(history_wanted(&m, now), None, "private on tracker.gg");
        m.players.get_mut("raze").unwrap().profile = Load::Missing;
        assert_eq!(history_wanted(&m, now), None, "unknown to tracker.gg");

        m.viewing = Some("cypher".into());
        assert!(m.players["cypher"].profile.ready().is_some());
        assert_eq!(
            history_wanted(&m, now),
            None,
            "incognito, even with a profile loaded earlier"
        );
    }

    fn profile_body(privacy: &str) -> String {
        let season = r#"{"type":"season","attributes":{"seasonId":"a6"},"stats":{"rank":{"value":null,"metadata":{"iconUrl":"https://x/tiersv2/14.png"}},"matchesWon":{"value":3},"matchesLost":{"value":2}}}"#;
        let segments = if privacy == "public" { season } else { "" };
        format!(r#"{{"data":{{"metadata":{{"privacy":"{privacy}"}},"segments":[{segments}]}}}}"#)
    }

    /// Seen live: a "premier" profile loads without stats, and its other pages answer 451.
    #[test]
    fn a_partly_private_profile_shows_as_private_without_an_error() {
        block_on(async {
            let shared = new_shared(Model::new(Content::default()));
            let tracker =
                Canned::tracker(vec![canned::answer(200, None, &profile_body("premier"))]);
            load_player(&tracker, &shared, "p", "a%23b", "a6")
                .await
                .unwrap();
            let m = shared::lock(&shared);
            let d = &m.players["p"];
            assert_eq!(
                d.profile,
                Load::Private,
                "the canned tracker panics on a second request, so nothing else was asked for"
            );
            assert_eq!(
                (d.maps.clone().map(|_| ()), d.recent.clone().map(|_| ())),
                (Load::Private, Load::Private)
            );
        });
    }

    #[test]
    fn a_profile_with_stats_loads_whatever_its_privacy_label() {
        block_on(async {
            let shared = new_shared(Model::new(Content::default()));
            let body =
                profile_body("public").replace(r#""privacy":"public""#, r#""privacy":"premier""#);
            let tracker = Canned::tracker(vec![
                canned::answer(200, None, &body),
                canned::answer(404, None, ""),
                canned::answer(404, None, ""),
            ]);
            load_player(&tracker, &shared, "p", "a%23b", "a6")
                .await
                .unwrap();
            assert_eq!(
                shared::lock(&shared).players["p"]
                    .profile
                    .ready()
                    .map(|p| p.tier),
                Some(14)
            );
        });
    }

    #[test]
    fn private_maps_or_matches_are_private_not_a_failure() {
        block_on(async {
            let shared = new_shared(Model::new(Content::default()));
            let side = MatchSide {
                match_id: "m1".into(),
                rounds_won: 13,
                rounds_lost: 7,
            };
            shared::lock(&shared).players.insert(
                "p".into(),
                PlayerData {
                    sides: vec![side],
                    last_match: Some("m1".into()),
                    ..Default::default()
                },
            );
            let refused = || canned::answer(451, None, "");
            let tracker = Canned::tracker(vec![
                canned::answer(200, None, &profile_body("public")),
                refused(),
                refused(),
            ]);
            load_player(&tracker, &shared, "p", "a%23b", "a6")
                .await
                .unwrap();
            let m = shared::lock(&shared);
            let d = &m.players["p"];
            assert_eq!(d.profile.ready().map(|p| p.tier), Some(14));
            assert_eq!(
                (d.maps.clone().map(|_| ()), d.recent.clone().map(|_| ())),
                (Load::Private, Load::Private)
            );
            assert!(
                d.sides.is_empty() && d.last_match.is_none(),
                "an old history isn't used once it's private, though it was public in an \
                 earlier match this session"
            );
        });
    }

    /// The `platformUserId` tracker.gg returns is its own id, never the Riot puuid the lobby uses.
    #[test]
    fn a_profile_loads_though_tracker_gg_ids_the_player_differently() {
        block_on(async {
            let act = "a6";
            let body = format!(
                r#"{{"data":{{"platformInfo":{{"platformUserId":"tracker-gg-own-id"}},"segments":[{{"type":"season","attributes":{{"seasonId":"{act}"}},"stats":{{"rank":{{"value":40,"metadata":{{"iconUrl":"https://x/tiersv2/14.png"}}}},"matchesWon":{{"value":3}},"matchesLost":{{"value":2}}}}}}]}}}}"#
            );
            let tracker = Canned::tracker(vec![
                canned::answer(200, None, &body),
                canned::answer(404, None, ""),
                canned::answer(404, None, ""),
            ]);
            let shared = new_shared(Model::new(Content::default()));
            let riot_puuid = "riot-puuid";
            load_player(&tracker, &shared, riot_puuid, "a%23b", act)
                .await
                .unwrap();
            let m = shared::lock(&shared);
            let profile = m.players[riot_puuid]
                .profile
                .ready()
                .expect("the profile is shown");
            assert_eq!((profile.tier, profile.rr), (14, Some(40)));
        });
    }
}
