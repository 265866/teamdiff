//! Application state shared between the network task and the UI.

use crate::presence::Presence;
use crate::riot::NetStats;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Not connected; the reason is shown under "Waiting for VALORANT…".
    Waiting(String),
    Menus,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerData {
    pub name: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub phase: Phase,
    pub me: String,
    pub region: String,
    pub players: HashMap<String, PlayerData>,
    /// Latest presence per player.
    pub presences: HashMap<String, Presence>,
    /// The latest request failure and when it happened; shown briefly in the footer.
    pub last_error: Option<(String, Instant)>,
    pub net: Option<Arc<NetStats>>,
}

impl Model {
    pub fn new() -> Self {
        Self {
            phase: Phase::Waiting("Looking for the Riot Client".into()),
            me: String::new(),
            region: String::new(),
            players: HashMap::new(),
            presences: HashMap::new(),
            last_error: None,
            net: None,
        }
    }

    pub fn my_presence(&self) -> Option<&Presence> {
        self.presences.get(&self.me)
    }

    /// Your party's members, by party id, when you queued with others.
    pub fn parties(&self) -> HashMap<String, String> {
        let mine = self.my_presence().and_then(|p| p.party_id.clone());
        let mut by_party: HashMap<String, Vec<String>> = HashMap::new();
        for p in self.presences.values() {
            if let Some(id) = &p.party_id {
                by_party.entry(id.clone()).or_default().push(p.puuid.clone());
            }
        }
        by_party
            .into_iter()
            .filter(|(id, _)| Some(id) == mine.as_ref())
            .flat_map(|(id, members)| {
                let keep = members.len() > 1;
                members.into_iter().filter(move |_| keep).map(move |m| (m, id.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(puuid: &str, party: &str) -> Presence {
        Presence { puuid: puuid.into(), party_id: Some(party.into()), ..Default::default() }
    }

    #[test]
    fn parties_only_include_your_own() {
        let mut m = Model::new();
        m.me = "me".into();
        for (p, party) in [("me", "x"), ("mate", "x"), ("a", "y"), ("b", "y"), ("solo", "z")] {
            m.presences.insert(p.into(), presence(p, party));
        }
        let own = m.parties();
        assert_eq!((own.len(), own["me"].as_str(), own["mate"].as_str()), (2, "x", "x"));
        m.presences.insert("me".into(), presence("me", "alone"));
        assert!(m.parties().is_empty(), "solo players are not a party");
    }
}
