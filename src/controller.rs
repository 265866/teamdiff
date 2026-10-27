//! Network side: follows the game's state through presence and loads player data in the background.

use crate::model::{Lobby, Model, Phase, parse_coregame, parse_pregame};
use crate::presence::LoopState;
use crate::riot::{Riot, connect};
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
        let riot = match connect(dump_dir.clone()).await {
            Ok(r) => Arc::new(r),
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
        let mut session = Session::new(riot, shared.clone());
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
    shared: Shared,
    pregame_id: Option<String>,
    core_id: Option<String>,
    named: Claims,
    client_gone: bool,
}

impl Session {
    fn new(riot: Arc<Riot>, shared: Shared) -> Self {
        Self { riot, shared, pregame_id: None, core_id: None, named: Claims::default(), client_gone: false }
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
        self.load_names(std::iter::once(me).chain(party).collect());
    }

    async fn pregame(&mut self) -> Result<()> {
        self.core_id = None;
        if self.pregame_id.is_none() {
            self.pregame_id = self.riot.pregame_match_id().await?;
        }
        let Some(id) = self.pregame_id.clone() else { return Ok(()) };
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
            // Incognito players' names are never requested.
            self.load_names(named);
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
}
