//! Application state shared between the network task and the UI.

use crate::content::Content;
use crate::presence::Presence;
use crate::riot::NetStats;
use crate::stats::Record;
use crate::tracker::{Profile, RecentMatch};
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
    Ingame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ally,
    Enemy,
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
    pub side: Side,
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

fn slot(p: &Value, side: Side) -> Option<Slot> {
    let id = p["PlayerIdentity"].clone();
    let agent = p["CharacterID"].as_str().filter(|s| !s.is_empty()).map(str::to_lowercase);
    let pick = match p["CharacterSelectionState"].as_str() {
        Some("locked") => Pick::Locked,
        Some("selected") => Pick::Hover,
        Some(_) => Pick::None,
        // In-game responses have no selection state; everyone is locked in.
        None if agent.is_some() => Pick::Locked,
        None => Pick::None,
    };
    Some(Slot {
        puuid: p["Subject"].as_str()?.to_lowercase(),
        side,
        agent_id: agent,
        pick,
        incognito: id["Incognito"].as_bool().unwrap_or(false),
        hide_level: id["HideAccountLevel"].as_bool().unwrap_or(false),
        level: id["AccountLevel"].as_u64().map(|n| n as u32),
    })
}

/// Agent select: only your own team is read. `EnemyTeam` and the enemy lock count are ignored on purpose.
pub fn parse_pregame(v: &Value, now: Instant) -> Option<Lobby> {
    let slots = v["AllyTeam"]["Players"].as_array()?.iter().filter_map(|p| slot(p, Side::Ally)).collect();
    let remaining_ns = v["PhaseTimeRemainingNS"].as_u64().unwrap_or(0);
    Some(Lobby {
        match_id: v["ID"].as_str()?.to_string(),
        map_url: v["MapID"].as_str().unwrap_or("").to_string(),
        queue: v["QueueID"].as_str().unwrap_or("").to_string(),
        slots,
        phase_ends: (remaining_ns > 0).then(|| now + Duration::from_nanos(remaining_ns)),
    })
}

pub fn parse_coregame(v: &Value, me: &str) -> Option<Lobby> {
    let players = v["Players"].as_array()?;
    let my_team =
        players.iter().find(|p| p["Subject"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(me)))?["TeamID"].as_str()?.to_string();
    let mut slots: Vec<Slot> = players
        .iter()
        .filter(|p| !p["IsCoach"].as_bool().unwrap_or(false))
        .filter_map(|p| slot(p, if p["TeamID"].as_str() == Some(&my_team) { Side::Ally } else { Side::Enemy }))
        .collect();
    slots.sort_by_key(|s| s.side == Side::Enemy);
    Some(Lobby {
        match_id: v["MatchID"].as_str()?.to_string(),
        map_url: v["MapID"].as_str().unwrap_or("").to_string(),
        queue: v["MatchmakingData"]["QueueID"].as_str().unwrap_or("").to_string(),
        slots,
        phase_ends: None,
    })
}

/// Data fetched from tracker.gg for one player.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load<T> {
    #[default]
    Loading,
    /// tracker.gg has no such player.
    Missing,
    /// The player hasn't made their tracker.gg profile public.
    Private,
    Ready(T),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Load<U> {
        match self {
            Load::Loading => Load::Loading,
            Load::Missing => Load::Missing,
            Load::Private => Load::Private,
            Load::Ready(t) => Load::Ready(f(t)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlayerData {
    pub name: Option<(String, String)>,
    pub profile: Load<Profile>,
    /// Wins and losses per map this act, by map name.
    pub maps: Load<Vec<(String, Record)>>,
    pub recent: Load<Vec<RecentMatch>>,
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

    /// The lobby, if it was loaded in game for this match. Agent select and the match can share an
    /// id, so an agent-select lobby (allies only) never counts.
    pub fn ingame_lobby(&self, match_id: &str) -> Option<&Lobby> {
        self.lobby.as_ref().filter(|l| self.phase == Phase::Ingame && l.match_id == match_id)
    }

    pub fn my_presence(&self) -> Option<&Presence> {
        self.presences.get(&self.me)
    }

    /// Party id per player, for parties of two or more. Before the match starts only your own party counts.
    /// Once it is running every party in the presences does, which Riot sends only for you and your
    /// friends.
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
            .filter(|(id, _)| self.phase == Phase::Ingame || Some(id) == mine.as_ref())
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
        assert!(l.slots.iter().all(|s| s.side == Side::Ally));
        assert_eq!(l.slots[0].puuid, "me");
        assert_eq!(l.slots[0].agent_id.as_deref(), Some("jett-id"));
        assert_eq!(l.slots[0].pick, Pick::Locked);
        assert_eq!((l.slots[1].pick, l.slots[1].agent_id.clone(), l.slots[1].incognito), (Pick::None, None, true));
        assert_eq!(l.slots[2].pick, Pick::Hover);
        assert_eq!(l.phase_ends, Some(now + Duration::from_secs(42)));
        assert!(l.is_competitive());
    }

    #[test]
    fn coregame_splits_teams_relative_to_me() {
        let v = json!({
            "MatchID": "m1", "MapID": "/Game/Maps/Bind/Bind", "MatchmakingData": { "QueueID": "unrated" },
            "Players": [player("e1", "Blue", "raze", None, false), player("me", "Red", "jett", None, false),
                        player("a1", "Red", "sova", None, false), { "Subject": "coach", "TeamID": "Red", "IsCoach": true }]
        });
        let l = parse_coregame(&v, "ME").unwrap();
        let sides: Vec<_> = l.slots.iter().map(|s| (s.puuid.as_str(), s.side, s.pick)).collect();
        assert_eq!(sides, [("me", Side::Ally, Pick::Locked), ("a1", Side::Ally, Pick::Locked), ("e1", Side::Enemy, Pick::Locked)]);
        assert!(!l.is_competitive());
    }

    #[test]
    fn coregame_without_me_is_rejected() {
        let v = json!({ "MatchID": "m1", "Players": [player("x", "Blue", "raze", None, false)] });
        assert!(parse_coregame(&v, "me").is_none());
    }

    #[test]
    fn agent_select_lobby_is_not_the_ingame_lobby() {
        let mut m = Model::new(Arc::default());
        m.lobby =
            Some(Lobby { match_id: "x".into(), map_url: String::new(), queue: "competitive".into(), slots: Vec::new(), phase_ends: None });
        m.phase = Phase::Pregame;
        assert!(m.ingame_lobby("x").is_none(), "same id, but loaded in agent select");
        m.phase = Phase::Ingame;
        assert!(m.ingame_lobby("x").is_some());
        assert!(m.ingame_lobby("y").is_none());
    }

    fn presence(puuid: &str, party: &str) -> Presence {
        Presence { puuid: puuid.into(), party_id: Some(party.into()), ..Default::default() }
    }

    #[test]
    fn parties_only_include_your_own_outside_a_match() {
        let mut m = Model::new(Arc::default());
        m.me = "me".into();
        for (p, party) in [("me", "x"), ("mate", "x"), ("a", "y"), ("b", "y"), ("solo", "z")] {
            m.presences.insert(p.into(), presence(p, party));
        }
        m.phase = Phase::Ingame;
        let all = m.parties();
        assert_eq!(all.len(), 4, "solo players are not a party");
        assert_eq!(all["a"], "y");

        for phase in [Phase::Pregame, Phase::Menus] {
            m.phase = phase.clone();
            let own = m.parties();
            assert_eq!(own.len(), 2, "{phase:?}");
            assert!(own.contains_key("mate") && !own.contains_key("a"), "{phase:?}");
        }
    }
}
