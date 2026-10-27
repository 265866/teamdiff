//! Network side: follows the game's state through presence.

use crate::model::{Model, Phase};
use crate::riot::{Riot, connect};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Shared = Arc<Mutex<Model>>;

pub fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Model> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs forever: connects to the Riot Client, follows the game, and reconnects when the client goes
/// away.
pub async fn run(shared: Shared) {
    loop {
        let riot = match connect().await {
            Ok(r) => Arc::new(r),
            Err(e) => {
                lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        lock(&shared).me = riot.puuid.clone();
        let mut session = Session::new(riot, shared.clone());
        loop {
            match session.tick().await {
                Ok(()) => {}
                Err(e) if session.client_gone => {
                    lock(&shared).phase = Phase::Waiting(format!("{e:#}"));
                    break;
                }
                // Other failures are retried on the next tick.
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

struct Session {
    riot: Arc<Riot>,
    shared: Shared,
    client_gone: bool,
}

impl Session {
    fn new(riot: Arc<Riot>, shared: Shared) -> Self {
        Self { riot, shared, client_gone: false }
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
            Some(_) => self.set_phase(Phase::Menus),
        }
        Ok(())
    }

    fn set_phase(&self, phase: Phase) {
        lock(&self.shared).phase = phase;
    }
}
