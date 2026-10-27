//! Application state shared between the network task and the UI.

use crate::content::Content;
use crate::presence::Presence;
use crate::riot::NetStats;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Not connected; the reason is shown under "Waiting for VALORANT…".
    Waiting(String),
    Menus,
    Pregame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    None,
    Hover,
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub puuid: String,
    pub agent_id: Option<String>,
    pub pick: Pick,
    pub incognito: bool,
    pub hide_level: bool,
    pub level: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lobby {
    pub match_id: String,
    pub map_url: String,
    pub queue: String,
    pub slots: Vec<Slot>,
    pub phase_ends: Option<Instant>,
}

impl Lobby {
    pub fn is_competitive(&self) -> bool {
        self.queue == "competitive"
    }
}

fn slot(p: &Value) -> Option<Slot> {
    let id = p["PlayerIdentity"].clone();
    let agent = p["CharacterID"].as_str().filter(|s| !s.is_empty()).map(str::to_lowercase);
    let pick = match p["CharacterSelectionState"].as_str() {
        Some("locked") => Pick::Locked,
        Some("selected") => Pick::Hover,
        _ => Pick::None,
    };
    Some(Slot {
        puuid: p["Subject"].as_str()?.to_lowercase(),
        agent_id: agent,
        pick,
        incognito: id["Incognito"].as_bool().unwrap_or(false),
        hide_level: id["HideAccountLevel"].as_bool().unwrap_or(false),
        level: id["AccountLevel"].as_u64().map(|n| n as u32),
    })
}

/// Agent select: only your own team is read. `EnemyTeam` and the enemy lock count are ignored on purpose.
pub fn parse_pregame(v: &Value, now: Instant) -> Option<Lobby> {
    let slots = v["AllyTeam"]["Players"].as_array()?.iter().filter_map(slot).collect();
    let remaining_ns = v["PhaseTimeRemainingNS"].as_u64().unwrap_or(0);
    Some(Lobby {
        match_id: v["ID"].as_str()?.to_string(),
        map_url: v["MapID"].as_str().unwrap_or("").to_string(),
        queue: v["QueueID"].as_str().unwrap_or("").to_string(),
        slots,
        phase_ends: (remaining_ns > 0).then(|| now + Duration::from_nanos(remaining_ns)),
    })
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
    pub content: Arc<Content>,
    pub lobby: Option<Lobby>,
    pub players: HashMap<String, PlayerData>,
    /// Latest presence per player.
    pub presences: HashMap<String, Presence>,
    /// The latest request failure and when it happened; shown briefly in the footer.
    pub last_error: Option<(String, Instant)>,
    pub net: Option<Arc<NetStats>>,
}

impl Model {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            phase: Phase::Waiting("Looking for the Riot Client".into()),
            me: String::new(),
            region: String::new(),
            content,
            lobby: None,
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
        // Incognito players other than you are never tied to a party.
        let lobby = self.lobby.as_ref();
        let incognito = |p: &str| p != self.me && lobby.is_some_and(|l| l.slots.iter().any(|s| s.puuid == p && s.incognito));
        let mut parties = self.all_parties();
        parties.retain(|p, _| !incognito(p));
        let mut sizes: HashMap<String, usize> = HashMap::new();
        for id in parties.values() {
            *sizes.entry(id.clone()).or_default() += 1;
        }
        parties.retain(|_, id| sizes[id.as_str()] > 1);
        parties
    }

    fn all_parties(&self) -> HashMap<String, String> {
        let mine = self.my_presence().and_then(|p| p.party_id.clone());
        let mut by_party: HashMap<String, Vec<String>> = HashMap::new();
        for p in self.presences.values() {
            if let Some(id) = &p.party_id {
                by_party.entry(id.clone()).or_default().push(p.puuid.clone());
            }
        }
        let in_lobby = |puuid: &String| self.lobby.as_ref().is_none_or(|l| l.slots.iter().any(|s| &s.puuid == puuid));
        by_party
            .into_iter()
            .filter(|(id, _)| Some(id) == mine.as_ref())
            .flat_map(|(id, members)| {
                let members: Vec<String> = members.into_iter().filter(|m| in_lobby(m)).collect();
                let keep = members.len() > 1;
                members.into_iter().filter(move |_| keep).map(move |m| (m, id.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn player(id: &str, team: &str, agent: &str, state: Option<&str>, incog: bool) -> Value {
        let mut p = json!({ "Subject": id, "TeamID": team, "CharacterID": agent,
            "PlayerIdentity": { "Incognito": incog, "HideAccountLevel": false, "AccountLevel": 100 } });
        if let Some(s) = state {
            p["CharacterSelectionState"] = json!(s);
        }
        p
    }

    #[test]
    fn pregame_reads_allies_only() {
        let v = json!({
            "ID": "pg1", "MapID": "/Game/Maps/Ascent/Ascent", "QueueID": "competitive", "PhaseTimeRemainingNS": 42_000_000_000u64,
            "AllyTeam": { "Players": [player("ME", "Blue", "JETT-ID", Some("locked"), false), player("b", "Blue", "", Some(""), true), player("c", "Blue", "sova", Some("selected"), false)] },
            "EnemyTeam": { "Players": [player("enemy", "Red", "raze", Some("locked"), false)] },
            "EnemyTeamLockCount": 3
        });
        let now = Instant::now();
        let l = parse_pregame(&v, now).unwrap();
        assert_eq!(l.slots.len(), 3);
        assert_eq!(l.slots[0].puuid, "me");
        assert_eq!(l.slots[0].agent_id.as_deref(), Some("jett-id"));
        assert_eq!(l.slots[0].pick, Pick::Locked);
        assert_eq!((l.slots[1].pick, l.slots[1].agent_id.clone(), l.slots[1].incognito), (Pick::None, None, true));
        assert_eq!(l.slots[2].pick, Pick::Hover);
        assert_eq!(l.phase_ends, Some(now + Duration::from_secs(42)));
        assert!(l.is_competitive());
    }

    fn presence(puuid: &str, party: &str) -> Presence {
        Presence { puuid: puuid.into(), party_id: Some(party.into()), ..Default::default() }
    }

    #[test]
    fn parties_only_include_your_own() {
        let mut m = Model::new(Arc::default());
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
