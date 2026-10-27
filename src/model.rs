//! Application state shared between the network task and the UI.

use crate::content::Content;
use crate::goldstars::{Definitions, MatchAccolades, PlayerGoldstars};
use crate::presence::Presence;
use crate::riot::NetStats;
use crate::stats::{CompUpdate, MatchSummary, Record};
use crate::tracker::{ActStats, MatchSide, Profile, RecentMatch};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Not connected; the reason is shown under "Waiting for VALORANT…".
    Waiting(String),
    Menus,
    Pregame,
    Ingame,
    Postgame,
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
    /// Every match in the player's tracker.gg history, for spotting who they queue with.
    pub sides: Vec<MatchSide>,
    /// Older acts, loaded when the player's details are opened.
    pub history: Vec<ActStats>,
}

#[derive(Debug, Clone, Default)]
pub struct Postgame {
    pub match_id: String,
    pub summary: Option<Arc<MatchSummary>>,
    pub rr: Option<CompUpdate>,
    pub accolades: Option<MatchAccolades>,
    /// The match's lobby, kept for incognito flags and parties.
    pub lobby: Option<Lobby>,
    /// When the banner first appeared, for the reveal animation.
    pub shown_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub phase: Phase,
    pub me: String,
    pub region: String,
    pub content: Arc<Content>,
    pub defs: Arc<Definitions>,
    pub lobby: Option<Lobby>,
    pub players: HashMap<String, PlayerData>,
    /// The player whose details are open, so their older acts get loaded.
    pub viewing: Option<String>,
    /// Latest presence per player.
    pub presences: HashMap<String, Presence>,
    pub postgame: Option<Postgame>,
    pub my_goldstars: Option<PlayerGoldstars>,
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
            defs: Arc::default(),
            lobby: None,
            players: HashMap::new(),
            viewing: None,
            presences: HashMap::new(),
            postgame: None,
            my_goldstars: None,
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

    /// Riot never had the match's results: back to the menus rather than waiting for them forever.
    pub fn give_up_postgame(&mut self, match_id: &str) {
        if self.phase == Phase::Postgame && self.postgame.as_ref().is_some_and(|p| p.match_id == match_id) {
            self.postgame = None;
            self.phase = Phase::Menus;
        }
    }

    /// Party id per player, for parties of two or more. Before the match starts only your own party counts.
    /// Once it is running every party does: from presences, which Riot sends only for you and your
    /// friends, and for everyone else from their tracker.gg history (see `likely_parties`). Post-game
    /// takes every party from the match details.
    pub fn parties(&self) -> HashMap<String, String> {
        // Incognito players other than you are never tied to a party.
        let lobby = if self.phase == Phase::Postgame { self.postgame.as_ref().and_then(|p| p.lobby.as_ref()) } else { self.lobby.as_ref() };
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
        if self.phase == Phase::Postgame
            && let Some(summary) = self.postgame.as_ref().and_then(|p| p.summary.as_ref())
        {
            return summary.parties();
        }
        let mine = self.my_presence().and_then(|p| p.party_id.clone());
        let mut by_party: HashMap<String, Vec<String>> = HashMap::new();
        for p in self.presences.values() {
            if let Some(id) = &p.party_id {
                by_party.entry(id.clone()).or_default().push(p.puuid.clone());
            }
        }
        let in_lobby = |puuid: &String| self.lobby.as_ref().is_none_or(|l| l.slots.iter().any(|s| &s.puuid == puuid));
        let mut parties: HashMap<String, String> = by_party
            .into_iter()
            .filter(|(id, _)| self.phase == Phase::Ingame || Some(id) == mine.as_ref())
            .flat_map(|(id, members)| {
                let members: Vec<String> = members.into_iter().filter(|m| in_lobby(m)).collect();
                let keep = members.len() > 1;
                members.into_iter().filter(move |_| keep).map(move |m| (m, id.clone()))
            })
            .collect();
        if self.phase == Phase::Ingame {
            // Presences name each friend's party and its size. A guess that reaches one such party,
            // such as a friend's with a stranger in it, joins it only if everyone guessed into that
            // party fits. A guess that reaches two parties can't tell which, so it places nobody.
            let mut joining: HashMap<&String, Vec<String>> = HashMap::new();
            for group in self.likely_parties() {
                let known: HashSet<&String> =
                    group.iter().filter_map(|p| self.presences.get(p).and_then(|pr| pr.party_id.as_ref())).collect();
                match known.into_iter().collect::<Vec<_>>()[..] {
                    [] => {
                        let id = format!("{LIKELY_PARTY}{}", group[0]);
                        for p in group {
                            parties.entry(p).or_insert_with(|| id.clone());
                        }
                    }
                    [id] => joining.entry(id).or_default().extend(group),
                    _ => {}
                }
            }
            for (id, group) in joining {
                let members: Vec<&Presence> = self.presences.values().filter(|p| p.party_id.as_ref() == Some(id)).collect();
                let newcomers = group.iter().filter(|p| self.presences.get(*p).and_then(|pr| pr.party_id.as_ref()) != Some(id)).count();
                if members.iter().find_map(|p| p.party_size).is_some_and(|size| members.len() + newcomers <= size as usize) {
                    for p in group {
                        parties.entry(p).or_insert_with(|| id.clone());
                    }
                }
            }
        }
        parties
    }

    /// Parties guessed from tracker.gg histories. Teammates in this match almost certainly queued
    /// together when they were also teammates in at least `LIKELY_PARTY_MATCHES` of their recent
    /// matches. Incognito players are never matched: their names are never asked for.
    fn likely_parties(&self) -> Vec<Vec<String>> {
        let Some(lobby) = &self.lobby else { return Vec::new() };
        let empty = PlayerData::default();
        let slots: Vec<(&Slot, HashSet<&MatchSide>)> = lobby
            .slots
            .iter()
            .filter(|s| !s.incognito)
            .map(|s| {
                let d = self.players.get(&s.puuid).unwrap_or(&empty);
                (s, d.sides.iter().collect())
            })
            .collect();
        // Union-find over players, by index into `slots`.
        let mut group: Vec<usize> = (0..slots.len()).collect();
        fn root(group: &mut [usize], i: usize) -> usize {
            let mut i = i;
            while group[i] != i {
                group[i] = group[group[i]];
                i = group[i];
            }
            i
        }
        for a in 0..slots.len() {
            for b in a + 1..slots.len() {
                let ((sa, ha), (sb, hb)) = (&slots[a], &slots[b]);
                if sa.side == sb.side && ha.intersection(hb).count() >= LIKELY_PARTY_MATCHES {
                    let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                    group[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        let mut members: HashMap<usize, Vec<String>> = HashMap::new();
        for (i, (slot, ..)) in slots.iter().enumerate() {
            members.entry(root(&mut group, i)).or_default().push(slot.puuid.clone());
        }
        members.into_values().filter(|m| m.len() > 1).collect()
    }
}

/// Party ids guessed from match history start with this, so they never clash with Riot's.
const LIKELY_PARTY: &str = "likely:";
/// Recent matches two players must have shared as teammates to count as a party. Two strangers rarely
/// end up teammates twice within each other's last 20 matches.
const LIKELY_PARTY_MATCHES: usize = 2;

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

    fn side(id: &str, won: u32, lost: u32) -> MatchSide {
        MatchSide { match_id: id.into(), rounds_won: won, rounds_lost: lost }
    }

    #[test]
    fn teammates_in_two_recent_matches_count_as_a_likely_party_in_game() {
        let mut m = Model::new(Arc::default());
        m.me = "me".into();
        let slot = |puuid: &str, side: Side, incognito: bool| Slot {
            puuid: puuid.into(),
            side,
            agent_id: None,
            pick: Pick::Locked,
            incognito,
            hide_level: false,
            level: None,
        };
        let slots = vec![
            slot("me", Side::Ally, false),
            slot("mate", Side::Ally, false),
            slot("stranger", Side::Ally, false),
            slot("friend", Side::Ally, false),
            slot("duo", Side::Ally, false),
            slot("lone", Side::Ally, false),
            slot("tagalong", Side::Ally, false),
            slot("pal", Side::Ally, false),
            slot("x1", Side::Ally, false),
            slot("x2", Side::Ally, false),
            slot("f1", Side::Ally, false),
            slot("f2", Side::Ally, false),
            slot("s1", Side::Ally, false),
            slot("s2", Side::Ally, false),
            slot("g1", Side::Ally, false),
            slot("g2", Side::Ally, false),
            slot("g3", Side::Ally, false),
            slot("a", Side::Ally, false),
            slot("b", Side::Ally, false),
            slot("c", Side::Ally, false),
            slot("d", Side::Ally, false),
            slot("x", Side::Enemy, false),
            slot("y", Side::Enemy, false),
            slot("z", Side::Enemy, true),
        ];
        m.lobby = Some(Lobby { match_id: "now".into(), map_url: String::new(), queue: "competitive".into(), slots, phase_ends: None });
        // Your party is you, your mate, and a stranger Riot shares no presence for.
        for p in ["me", "mate"] {
            m.presences.insert(p.into(), Presence { party_size: Some(3), ..presence(p, "mine") });
        }
        // A friend whose party is them and someone you don't know.
        m.presences.insert("friend".into(), Presence { party_size: Some(2), ..presence("friend", "theirs") });
        // A friend who queued alone, with a teammate from their recent matches.
        m.presences.insert("lone".into(), Presence { party_size: Some(1), ..presence("lone", "alone") });
        // A friend in a duo, linked to two players: they can't both be the other half.
        m.presences.insert("pal".into(), Presence { party_size: Some(2), ..presence("pal", "pals") });
        // Two friends in a party of three, each linked to a different player: only one can be the third.
        for p in ["f1", "f2"] {
            m.presences.insert(p.into(), Presence { party_size: Some(3), ..presence(p, "trio") });
        }
        // A friend in a duo, linked to a stranger and to a friend whose presence names no party.
        m.presences.insert("g1".into(), Presence { party_size: Some(2), ..presence("g1", "gduo") });
        m.presences.insert("g2".into(), Presence { party_id: None, ..presence("g2", "") });
        let two = vec![side("m1", 13, 7), side("m2", 9, 13)];
        let history: [(&str, Vec<MatchSide>); 24] = [
            ("me", two.clone()),
            ("mate", two.clone()),
            // A stranger in your party: Riot shares no presence for them.
            ("stranger", two.clone()),
            ("friend", vec![side("m3", 13, 4), side("m4", 13, 6)]),
            ("duo", vec![side("m3", 13, 4), side("m4", 13, 6)]),
            ("lone", vec![side("m11", 13, 4), side("m12", 13, 6)]),
            ("tagalong", vec![side("m11", 13, 4), side("m12", 13, 6)]),
            ("pal", vec![side("m13", 13, 4), side("m14", 13, 6)]),
            ("x1", vec![side("m13", 13, 4), side("m14", 13, 6)]),
            ("x2", vec![side("m13", 13, 4), side("m14", 13, 6)]),
            ("f1", vec![side("m15", 13, 4), side("m16", 13, 6)]),
            ("s1", vec![side("m15", 13, 4), side("m16", 13, 6)]),
            ("f2", vec![side("m17", 13, 4), side("m18", 13, 6)]),
            ("s2", vec![side("m17", 13, 4), side("m18", 13, 6)]),
            ("g1", vec![side("m19", 13, 4), side("m20", 13, 6)]),
            ("g2", vec![side("m19", 13, 4), side("m20", 13, 6)]),
            ("g3", vec![side("m19", 13, 4), side("m20", 13, 6)]),
            ("a", vec![side("m5", 13, 2), side("m6", 13, 11), side("m7", 4, 13)]),
            ("b", vec![side("m5", 13, 2), side("m6", 13, 11), side("m9", 13, 5), side("m10", 13, 8)]),
            // Shares nothing with a, but two matches with b: the same party.
            ("d", vec![side("m9", 13, 5), side("m10", 13, 8)]),
            // One shared match is chance, not a party.
            ("c", vec![side("m5", 13, 2), side("m8", 13, 0)]),
            // Same matches, other team: opponents back then.
            ("x", vec![side("m5", 2, 13), side("m6", 11, 13)]),
            ("y", vec![side("m5", 2, 13), side("m6", 11, 13)]),
            ("z", vec![side("m5", 2, 13), side("m6", 11, 13)]),
        ];
        for (p, sides) in history {
            m.players.insert(p.into(), PlayerData { sides, ..Default::default() });
        }

        m.phase = Phase::Ingame;
        let parties = m.parties();
        assert_eq!(parties["me"], "mine", "a known party isn't replaced by a guess");
        assert_eq!(parties["stranger"], "mine", "a guess that reaches a known party joins it");
        assert_eq!((parties["friend"].as_str(), parties["duo"].as_str()), ("theirs", "theirs"), "a friend's party with a stranger in it");
        assert!(!parties.contains_key("lone") && !parties.contains_key("tagalong"), "Riot says the friend is alone");
        assert!(["pal", "x1", "x2"].iter().all(|p| !parties.contains_key(*p)), "a guess never grows a party past its size");
        assert_eq!((parties["f1"].as_str(), parties["f2"].as_str()), ("trio", "trio"));
        assert!(!parties.contains_key("s1") && !parties.contains_key("s2"), "two guesses can't overfill a party together");
        assert!(["g1", "g2", "g3"].iter().all(|p| !parties.contains_key(*p)), "a presence without a party still counts toward the size");
        assert_eq!(parties["a"], parties["b"]);
        assert_eq!(parties["d"], parties["b"], "a chain through b is one party of three");
        assert!(parties["a"].starts_with(LIKELY_PARTY));
        assert!(!parties.contains_key("c"), "one shared match");
        assert_eq!(parties["x"], parties["y"], "enemies who were teammates");
        assert_ne!(parties["x"], parties["a"]);
        assert!(!parties.contains_key("z"), "incognito players are never matched");

        m.phase = Phase::Pregame;
        let own = m.parties();
        assert_eq!((own.len(), own["me"].as_str(), own["mate"].as_str()), (2, "mine", "mine"), "agent select shows only your own party");
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

        for phase in [Phase::Pregame, Phase::Menus, Phase::Postgame] {
            m.phase = phase.clone();
            let own = m.parties();
            assert_eq!(own.len(), 2, "{phase:?}");
            assert!(own.contains_key("mate") && !own.contains_key("a"), "{phase:?}");
        }
    }
}
