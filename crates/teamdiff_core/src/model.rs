//! Application state shared between the network task and the UI.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use semver::Version;
use serde_json::Value;

use crate::cast;
use crate::content::Content;
use crate::goldstars::{Definitions, MatchAccolades, PlayerGoldstars};
use crate::net_stats::NetStats;
use crate::presence::Presence;
use crate::stats::{CompUpdate, MatchSummary};
use crate::tracker::{ActStats, MapRecord, MatchSide, Profile, RecentMatch};

/// Where the game is, and so which screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "teamdiff_ui draws a different screen for each phase by matching every one, so a \
              new phase has to fail its build until it has a screen"
)]
pub enum Phase {
    /// Not connected; the reason is shown under "Waiting for VALORANT…".
    Waiting(String),
    /// In the menus.
    Menus,
    /// In agent select.
    Pregame,
    /// In a match.
    Ingame,
    /// After a match, showing its results.
    Postgame,
}

/// Which team a player is on, from your side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Side {
    /// Your team.
    Ally,
    /// The other team.
    Enemy,
}

/// A player's agent pick in agent select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pick {
    /// No agent picked.
    None,
    /// An agent picked but not locked in.
    Hover,
    /// An agent locked in.
    Locked,
}

/// One player in a lobby.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Slot {
    /// The player's PUUID, in lowercase.
    pub puuid: String,
    /// The player's team.
    pub side: Side,
    /// The agent's id, in lowercase.
    pub agent_id: Option<String>,
    /// How far the player's pick is.
    pub pick: Pick,
    /// Whether the player hides their name.
    pub incognito: bool,
    /// Whether the player hides their account level.
    pub hide_level: bool,
    /// The player's account level.
    pub level: Option<u32>,
}

/// The players of agent select or of a running match.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Lobby {
    /// The match. Agent select and the match it starts can share an id.
    pub match_id: String,
    /// The map's URL, such as `/Game/Maps/Ascent/Ascent`.
    pub map_url: String,
    /// The queue, such as "competitive".
    pub queue: String,
    /// The players, your team first.
    pub slots: Vec<Slot>,
    /// When agent select ends.
    pub phase_ends: Option<Instant>,
}

impl Lobby {
    /// Whether the lobby is a competitive match.
    #[must_use]
    pub fn is_competitive(&self) -> bool {
        self.queue == "competitive"
    }
}

/// One player of a pregame or core-game response. In-game responses have no selection state;
/// everyone with an agent there is locked in.
fn slot(p: &Value, side: Side) -> Option<Slot> {
    let id = p["PlayerIdentity"].clone();
    let agent = p["CharacterID"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let pick = match p["CharacterSelectionState"].as_str() {
        Some("locked") => Pick::Locked,
        Some("selected") => Pick::Hover,
        None if agent.is_some() => Pick::Locked,
        Some(_) | None => Pick::None,
    };
    Some(Slot {
        puuid: p["Subject"].as_str()?.to_lowercase(),
        side,
        agent_id: agent,
        pick,
        incognito: id["Incognito"].as_bool().unwrap_or(false),
        hide_level: id["HideAccountLevel"].as_bool().unwrap_or(false),
        level: id["AccountLevel"].as_u64().map(cast::low_u32),
    })
}

/// Agent select: only your own team is read. `EnemyTeam` and the enemy lock count are ignored on purpose.
#[must_use]
pub fn parse_pregame(v: &Value, now: Instant) -> Option<Lobby> {
    let slots: Vec<Slot> = v["AllyTeam"]["Players"]
        .as_array()?
        .iter()
        .filter_map(|p| slot(p, Side::Ally))
        .collect();
    let remaining_ns = v["PhaseTimeRemainingNS"].as_u64().unwrap_or(0);
    Some(Lobby {
        match_id: v["ID"].as_str()?.to_string(),
        map_url: v["MapID"].as_str().unwrap_or("").to_string(),
        queue: v["QueueID"].as_str().unwrap_or("").to_string(),
        slots,
        phase_ends: (remaining_ns > 0).then(|| now + Duration::from_nanos(remaining_ns)),
    })
}

/// A running match, with teams from the side of `me`, a PUUID in any case. Coaches are left out,
/// and a match without `me` reads as `None`.
#[must_use]
pub fn parse_coregame(v: &Value, me: &str) -> Option<Lobby> {
    let players = v["Players"].as_array()?;
    let my_team = players.iter().find(|p| {
        p["Subject"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(me))
    })?["TeamID"]
        .as_str()?
        .to_string();
    let mut slots: Vec<Slot> = players
        .iter()
        .filter(|p| !p["IsCoach"].as_bool().unwrap_or(false))
        .filter_map(|p| {
            slot(
                p,
                if p["TeamID"].as_str() == Some(&my_team) {
                    Side::Ally
                } else {
                    Side::Enemy
                },
            )
        })
        .collect();
    slots.sort_by_key(|s| s.side == Side::Enemy);
    Some(Lobby {
        match_id: v["MatchID"].as_str()?.to_string(),
        map_url: v["MapID"].as_str().unwrap_or("").to_string(),
        queue: v["MatchmakingData"]["QueueID"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        slots,
        phase_ends: None,
    })
}

/// Data fetched from tracker.gg for one player.
#[derive(Debug, Clone, Default, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "teamdiff_ui shows each state differently by matching every one, so a new state has \
              to fail its build until it has a look"
)]
pub enum Load<T> {
    /// Not loaded yet.
    #[default]
    Loading,
    /// No such player on tracker.gg.
    Missing,
    /// The player hasn't made their tracker.gg profile public.
    Private,
    /// Loaded.
    Ready(T),
}

impl<T> Load<T> {
    /// The data, once loaded.
    #[must_use]
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }

    /// The data, once loaded, passed through `f`.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Load<U> {
        match self {
            Load::Loading => Load::Loading,
            Load::Missing => Load::Missing,
            Load::Private => Load::Private,
            Load::Ready(t) => Load::Ready(f(t)),
        }
    }
}

/// A player's Riot ID, `name#tag`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff_live's controller builds one from each name Riot's name service returns"
)]
pub struct RiotId {
    /// The game name, before the `#`.
    pub name: String,
    /// The tag line, after the `#`.
    pub tag: String,
}

/// What teamdiff knows about one player, mostly from tracker.gg.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff_ui builds one with every load missing for an incognito player, whom \
              teamdiff never looks up"
)]
pub struct PlayerData {
    /// The player's Riot ID.
    pub name: Option<RiotId>,
    /// The player's rank.
    pub profile: Load<Profile>,
    /// Wins and losses per map this act, by map name.
    pub maps: Load<Vec<MapRecord>>,
    /// The player's recent competitive matches.
    pub recent: Load<Vec<RecentMatch>>,
    /// Every match in the player's tracker.gg history, for spotting who they queue with.
    pub sides: Vec<MatchSide>,
    /// Their newest match on tracker.gg, whose parties are in `Model::match_parties`.
    pub last_match: Option<String>,
    /// Older acts, loaded when the player's details are opened.
    pub history: Vec<ActStats>,
}

/// The results of the match that just ended, filled in as Riot has them.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff_live's controller starts one with a struct literal when a match ends and \
              fills in the rest as Riot's results arrive"
)]
pub struct Postgame {
    /// The match.
    pub match_id: String,
    /// The match's stats.
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "teamdiff_live's postgame task keeps reading the summary after handing it \
                      to the model, without holding the model's lock"
        )
    )]
    pub summary: Option<Arc<MatchSummary>>,
    /// Your RR change.
    pub rr: Option<CompUpdate>,
    /// The match's accolades.
    pub accolades: Option<MatchAccolades>,
    /// The match's lobby, kept for incognito flags and parties.
    pub lobby: Option<Lobby>,
    /// When the banner first appeared, for the reveal animation.
    pub shown_at: Option<Instant>,
}

/// A request failure, shown briefly in the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff_live's controller builds one from each request that fails"
)]
pub struct LastError {
    /// What failed.
    pub message: String,
    /// When it failed.
    pub at: Instant,
}

/// This version, when it replaced an older one since teamdiff last ran; shown briefly in the
/// footer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff's startup builds one when the updater finds that an update ran"
)]
pub struct Updated {
    /// The version teamdiff was updated to.
    pub version: Version,
    /// When teamdiff started.
    pub started: Instant,
}

/// Everything the UI draws.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Model {
    /// Where the game is.
    pub phase: Phase,
    /// Your PUUID.
    pub me: String,
    /// Your region, such as "EU".
    pub region: String,
    /// The names of agents, maps and ranks, and the acts.
    pub content: Content,
    /// The accolade definitions Riot returned.
    pub defs: Definitions,
    /// The lobby of agent select or of the running match.
    pub lobby: Option<Lobby>,
    /// What is known about each player, by PUUID.
    pub players: HashMap<String, PlayerData>,
    /// The player whose details are open, so their older acts get loaded.
    pub viewing: Option<String>,
    /// Latest presence per player.
    pub presences: HashMap<String, Presence>,
    /// The results of the match that just ended.
    pub postgame: Option<Postgame>,
    /// Your own accolades.
    pub my_goldstars: Option<PlayerGoldstars>,
    /// Parties in past matches by match id, as lowercase Riot IDs.
    pub match_parties: HashMap<String, Vec<Vec<String>>>,
    /// The latest request failure; shown briefly in the footer.
    pub last_error: Option<LastError>,
    /// The request counters.
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the Riot and tracker.gg clients in teamdiff_live count their requests into \
                      the counters the footer reads from here"
        )
    )]
    pub net: Option<Arc<NetStats>>,
    /// A newer version that replaced this exe and runs the next time teamdiff starts.
    pub update_ready: Option<Version>,
    /// This version, when it replaced an older one since teamdiff last ran.
    pub updated: Option<Updated>,
}

impl Model {
    /// A model waiting for the Riot Client.
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            phase: Phase::Waiting("Looking for the Riot Client".into()),
            me: String::new(),
            region: String::new(),
            content,
            defs: Definitions::default(),
            lobby: None,
            players: HashMap::new(),
            viewing: None,
            presences: HashMap::new(),
            postgame: None,
            my_goldstars: None,
            match_parties: HashMap::new(),
            last_error: None,
            net: None,
            update_ready: None,
            updated: None,
        }
    }

    /// The lobby, if it was loaded in game for this match. Agent select and the match can share an
    /// id, so an agent-select lobby (allies only) never counts.
    #[must_use]
    pub fn ingame_lobby(&self, match_id: &str) -> Option<&Lobby> {
        self.lobby
            .as_ref()
            .filter(|l| self.phase == Phase::Ingame && l.match_id == match_id)
    }

    /// Your own presence.
    #[must_use]
    pub fn my_presence(&self) -> Option<&Presence> {
        self.presences.get(&self.me)
    }

    /// Riot never had the match's results: back to the menus rather than waiting for them forever.
    pub fn give_up_postgame(&mut self, match_id: &str) {
        if self.phase == Phase::Postgame
            && self
                .postgame
                .as_ref()
                .is_some_and(|p| p.match_id == match_id)
        {
            self.postgame = None;
            self.phase = Phase::Menus;
        }
    }

    /// Party id per player, for parties of two or more. Before the match starts only your own party counts.
    /// Once it is running every party does: from presences, which Riot sends only for you and your
    /// friends, and for everyone else from their tracker.gg history (see `likely_parties`). Post-game
    /// takes every party from the match details. Incognito players other than you are never tied to
    /// a party.
    #[must_use]
    pub fn parties(&self) -> HashMap<String, String> {
        let lobby = if self.phase == Phase::Postgame {
            self.postgame.as_ref().and_then(|p| p.lobby.as_ref())
        } else {
            self.lobby.as_ref()
        };
        let incognito = |p: &str| {
            p != self.me
                && lobby.is_some_and(|l| l.slots.iter().any(|s| s.puuid == p && s.incognito))
        };
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
                by_party
                    .entry(id.clone())
                    .or_default()
                    .push(p.puuid.clone());
            }
        }
        let in_lobby = |puuid: &String| {
            self.lobby
                .as_ref()
                .is_none_or(|l| l.slots.iter().any(|s| &s.puuid == puuid))
        };
        let mut parties: HashMap<String, String> = by_party
            .into_iter()
            .filter(|(id, _)| self.phase == Phase::Ingame || Some(id) == mine.as_ref())
            .flat_map(|(id, members)| {
                let members: Vec<String> = members.into_iter().filter(|m| in_lobby(m)).collect();
                let keep = members.len() > 1;
                members
                    .into_iter()
                    .filter(move |_| keep)
                    .map(move |m| (m, id.clone()))
            })
            .collect();
        if self.phase == Phase::Ingame {
            self.add_likely_parties(&mut parties);
        }
        parties
    }

    /// Adds the parties guessed from tracker.gg histories to `parties`. Presences name each
    /// friend's party and its size. A guess that reaches one such party, such as a friend's with
    /// a stranger in it, joins it only if everyone guessed into that party fits. A guess that
    /// reaches two parties can't tell which, so it places nobody.
    fn add_likely_parties(&self, parties: &mut HashMap<String, String>) {
        let mut joining: HashMap<&String, Vec<String>> = HashMap::new();
        for group in self.likely_parties() {
            let known: HashSet<&String> = group
                .iter()
                .filter_map(|p| self.presences.get(p).and_then(|pr| pr.party_id.as_ref()))
                .collect();
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
            let members: Vec<&Presence> = self
                .presences
                .values()
                .filter(|p| p.party_id.as_ref() == Some(id))
                .collect();
            let newcomers = group
                .iter()
                .filter(|p| self.presences.get(*p).and_then(|pr| pr.party_id.as_ref()) != Some(id))
                .count();
            if members
                .iter()
                .find_map(|p| p.party_size)
                .is_some_and(|size| members.len() + newcomers <= size as usize)
            {
                for p in group {
                    parties.entry(p).or_insert_with(|| id.clone());
                }
            }
        }
    }

    /// Parties guessed from tracker.gg histories. Teammates in this match almost certainly queued
    /// together when they were also teammates in at least `LIKELY_PARTY_MATCHES` of their recent
    /// matches, or partied in one player's last match; that needs only that player's profile public.
    /// Incognito players are never matched: their names are never asked for.
    fn likely_parties(&self) -> Vec<Vec<String>> {
        let Some(lobby) = &self.lobby else {
            return Vec::new();
        };
        let empty = PlayerData::default();
        let slots: Vec<(&Slot, &PlayerData, HashSet<&MatchSide>)> = lobby
            .slots
            .iter()
            .filter(|s| !s.incognito)
            .map(|s| {
                let d = self.players.get(&s.puuid).unwrap_or(&empty);
                (s, d, d.sides.iter().collect())
            })
            .collect();
        let riot_id = |d: &PlayerData| {
            d.name
                .as_ref()
                .map(|RiotId { name, tag }| format!("{name}#{tag}").to_lowercase())
        };
        let partied_in_last_match_of_first = |a: usize, b: usize| {
            let (Some(last), Some(me), Some(them)) = (
                &slots[a].1.last_match,
                riot_id(slots[a].1),
                riot_id(slots[b].1),
            ) else {
                return false;
            };
            self.match_parties
                .get(last)
                .is_some_and(|ps| ps.iter().any(|p| p.contains(&me) && p.contains(&them)))
        };
        let mut group: Vec<usize> = (0..slots.len()).collect();
        for a in 0..slots.len() {
            for b in a + 1..slots.len() {
                let ((sa, _, ha), (sb, _, hb)) = (&slots[a], &slots[b]);
                let together = ha.intersection(hb).count() >= LIKELY_PARTY_MATCHES
                    || partied_in_last_match_of_first(a, b)
                    || partied_in_last_match_of_first(b, a);
                if sa.side == sb.side && together {
                    let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                    group[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        let mut members: HashMap<usize, Vec<String>> = HashMap::new();
        for (i, (slot, ..)) in slots.iter().enumerate() {
            members
                .entry(root(&mut group, i))
                .or_default()
                .push(slot.puuid.clone());
        }
        members.into_values().filter(|m| m.len() > 1).collect()
    }
}

/// The root of `i` in the union-find forest `group`, halving the path on the way.
fn root(group: &mut [usize], i: usize) -> usize {
    let mut i = i;
    while group[i] != i {
        group[i] = group[group[i]];
        i = group[i];
    }
    i
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

    fn player(id: &str, team: &str, agent: &str, state: Option<&str>) -> Value {
        let mut p = json!({ "Subject": id, "TeamID": team, "CharacterID": agent,
            "PlayerIdentity": { "Incognito": false, "HideAccountLevel": false, "AccountLevel": 100 } });
        if let Some(s) = state {
            p["CharacterSelectionState"] = json!(s);
        }
        p
    }

    fn incognito_player(mut player: Value) -> Value {
        player["PlayerIdentity"]["Incognito"] = json!(true);
        player
    }

    #[test]
    fn pregame_reads_allies_only() {
        let v = json!({
            "ID": "pg1", "MapID": "/Game/Maps/Ascent/Ascent", "QueueID": "competitive", "PhaseTimeRemainingNS": 42_000_000_000u64,
            "AllyTeam": { "Players": [player("ME", "Blue", "JETT-ID", Some("locked")), incognito_player(player("b", "Blue", "", Some(""))), player("c", "Blue", "sova", Some("selected"))] },
            "EnemyTeam": { "Players": [player("enemy", "Red", "raze", Some("locked"))] },
            "EnemyTeamLockCount": 3
        });
        let now = Instant::now();
        let l = parse_pregame(&v, now).unwrap();
        assert_eq!(l.slots.len(), 3);
        assert!(l.slots.iter().all(|s| s.side == Side::Ally));
        assert_eq!(l.slots[0].puuid, "me");
        assert_eq!(l.slots[0].agent_id.as_deref(), Some("jett-id"));
        assert_eq!(l.slots[0].pick, Pick::Locked);
        assert_eq!(
            (
                l.slots[1].pick,
                l.slots[1].agent_id.clone(),
                l.slots[1].incognito
            ),
            (Pick::None, None, true)
        );
        assert_eq!(l.slots[2].pick, Pick::Hover);
        assert_eq!(l.phase_ends, Some(now + Duration::from_secs(42)));
        assert!(l.is_competitive());
    }

    #[test]
    fn coregame_splits_teams_relative_to_me() {
        let v = json!({
            "MatchID": "m1", "MapID": "/Game/Maps/Bind/Bind", "MatchmakingData": { "QueueID": "unrated" },
            "Players": [player("e1", "Blue", "raze", None), player("me", "Red", "jett", None),
                        player("a1", "Red", "sova", None), { "Subject": "coach", "TeamID": "Red", "IsCoach": true }]
        });
        let l = parse_coregame(&v, "ME").unwrap();
        let sides: Vec<_> = l
            .slots
            .iter()
            .map(|s| (s.puuid.as_str(), s.side, s.pick))
            .collect();
        assert_eq!(
            sides,
            [
                ("me", Side::Ally, Pick::Locked),
                ("a1", Side::Ally, Pick::Locked),
                ("e1", Side::Enemy, Pick::Locked)
            ]
        );
        assert!(!l.is_competitive());
    }

    #[test]
    fn coregame_without_me_is_rejected() {
        let v = json!({ "MatchID": "m1", "Players": [player("x", "Blue", "raze", None)] });
        assert!(parse_coregame(&v, "me").is_none());
    }

    fn lobby(slots: Vec<Slot>) -> Lobby {
        Lobby {
            match_id: "now".into(),
            map_url: String::new(),
            queue: "competitive".into(),
            slots,
            phase_ends: None,
        }
    }

    #[test]
    fn agent_select_lobby_is_not_the_ingame_lobby() {
        let mut m = Model::new(Content::default());
        m.lobby = Some(Lobby {
            match_id: "x".into(),
            ..lobby(Vec::new())
        });
        m.phase = Phase::Pregame;
        assert!(
            m.ingame_lobby("x").is_none(),
            "same id, but loaded in agent select"
        );
        m.phase = Phase::Ingame;
        assert!(m.ingame_lobby("x").is_some());
        assert!(m.ingame_lobby("y").is_none());
    }

    fn side(id: &str, won: u32, lost: u32) -> MatchSide {
        MatchSide {
            match_id: id.into(),
            rounds_won: won,
            rounds_lost: lost,
        }
    }

    fn slot(puuid: &str, side: Side) -> Slot {
        Slot {
            puuid: puuid.into(),
            side,
            agent_id: None,
            pick: Pick::Locked,
            incognito: false,
            hide_level: false,
            level: None,
        }
    }

    fn incognito(slot: Slot) -> Slot {
        Slot {
            incognito: true,
            ..slot
        }
    }

    fn presence(puuid: &str, party: &str) -> Presence {
        Presence {
            puuid: puuid.into(),
            party_id: Some(party.into()),
            ..Default::default()
        }
    }

    fn in_party_of(puuid: &str, party: &str, size: u32) -> Presence {
        Presence {
            party_size: Some(size),
            ..presence(puuid, party)
        }
    }

    /// The lobby of [`teammates_in_two_recent_matches_count_as_a_likely_party_in_game`], with
    /// presences and tracker.gg histories:
    ///
    /// - your party is you, your mate, and a stranger Riot shares no presence for;
    /// - "friend" is in a party with "duo", whom you don't know;
    /// - "lone" is a friend who queued alone, with a teammate from their recent matches;
    /// - "pal" is a friend in a duo, linked to two players, who can't both be the other half;
    /// - "f1" and "f2" are friends in a party of three, each linked to a different player, so only
    ///   one can be the third;
    /// - "g1" is a friend in a duo, linked to a stranger and to a friend whose presence names no
    ///   party;
    /// - "d" shares nothing with "a", but two matches with "b", so all three are one party;
    /// - "c" shares one match with "a" and "b", which is chance, not a party;
    /// - "x", "y" and the incognito "z" were in the same matches on the other team, so opponents
    ///   back then.
    fn likely_party_lobby() -> Model {
        let mut m = Model::new(Content::default());
        m.me = "me".into();
        let allies = [
            "me", "mate", "stranger", "friend", "duo", "lone", "tagalong", "pal", "x1", "x2", "f1",
            "f2", "s1", "s2", "g1", "g2", "g3", "a", "b", "c", "d",
        ];
        let mut slots: Vec<Slot> = allies.iter().map(|p| slot(p, Side::Ally)).collect();
        slots.extend([
            slot("x", Side::Enemy),
            slot("y", Side::Enemy),
            incognito(slot("z", Side::Enemy)),
        ]);
        m.lobby = Some(lobby(slots));
        for p in ["me", "mate"] {
            m.presences.insert(p.into(), in_party_of(p, "mine", 3));
        }
        m.presences
            .insert("friend".into(), in_party_of("friend", "theirs", 2));
        m.presences
            .insert("lone".into(), in_party_of("lone", "alone", 1));
        m.presences
            .insert("pal".into(), in_party_of("pal", "pals", 2));
        for p in ["f1", "f2"] {
            m.presences.insert(p.into(), in_party_of(p, "trio", 3));
        }
        m.presences
            .insert("g1".into(), in_party_of("g1", "gduo", 2));
        m.presences.insert(
            "g2".into(),
            Presence {
                party_id: None,
                ..presence("g2", "")
            },
        );
        let two = vec![side("m1", 13, 7), side("m2", 9, 13)];
        let history: [(&str, Vec<MatchSide>); 24] = [
            ("me", two.clone()),
            ("mate", two.clone()),
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
            (
                "a",
                vec![side("m5", 13, 2), side("m6", 13, 11), side("m7", 4, 13)],
            ),
            (
                "b",
                vec![
                    side("m5", 13, 2),
                    side("m6", 13, 11),
                    side("m9", 13, 5),
                    side("m10", 13, 8),
                ],
            ),
            ("d", vec![side("m9", 13, 5), side("m10", 13, 8)]),
            ("c", vec![side("m5", 13, 2), side("m8", 13, 0)]),
            ("x", vec![side("m5", 2, 13), side("m6", 11, 13)]),
            ("y", vec![side("m5", 2, 13), side("m6", 11, 13)]),
            ("z", vec![side("m5", 2, 13), side("m6", 11, 13)]),
        ];
        for (p, sides) in history {
            m.players.insert(
                p.into(),
                PlayerData {
                    sides,
                    ..Default::default()
                },
            );
        }
        m
    }

    #[test]
    fn teammates_in_two_recent_matches_count_as_a_likely_party_in_game() {
        let mut m = likely_party_lobby();
        m.phase = Phase::Ingame;
        let parties = m.parties();
        assert_eq!(
            parties["me"], "mine",
            "a known party isn't replaced by a guess"
        );
        assert_eq!(
            parties["stranger"], "mine",
            "a guess that reaches a known party joins it"
        );
        assert_eq!(
            (parties["friend"].as_str(), parties["duo"].as_str()),
            ("theirs", "theirs"),
            "a friend's party with a stranger in it"
        );
        assert!(
            !parties.contains_key("lone") && !parties.contains_key("tagalong"),
            "Riot says the friend is alone"
        );
        assert!(
            ["pal", "x1", "x2"]
                .iter()
                .all(|p| !parties.contains_key(*p)),
            "a guess never grows a party past its size"
        );
        assert_eq!(
            (parties["f1"].as_str(), parties["f2"].as_str()),
            ("trio", "trio")
        );
        assert!(
            !parties.contains_key("s1") && !parties.contains_key("s2"),
            "two guesses can't overfill a party together"
        );
        assert!(
            ["g1", "g2", "g3"].iter().all(|p| !parties.contains_key(*p)),
            "a presence without a party still counts toward the size"
        );
        assert_eq!(parties["a"], parties["b"]);
        assert_eq!(
            parties["d"], parties["b"],
            "a chain through b is one party of three"
        );
        assert!(parties["a"].starts_with(LIKELY_PARTY));
        assert!(!parties.contains_key("c"), "one shared match");
        assert_eq!(parties["x"], parties["y"], "enemies who were teammates");
        assert_ne!(parties["x"], parties["a"]);
        assert!(
            !parties.contains_key("z"),
            "incognito players are never matched"
        );

        m.phase = Phase::Pregame;
        let own = m.parties();
        assert_eq!(
            (own.len(), own["me"].as_str(), own["mate"].as_str()),
            (2, "mine", "mine"),
            "agent select shows only your own party"
        );
    }

    #[test]
    fn a_party_in_one_players_last_match_is_a_likely_party_even_with_a_private_partner() {
        let mut m = Model::new(Content::default());
        m.me = "me".into();
        m.phase = Phase::Ingame;
        let slots = vec![
            slot("me", Side::Ally),
            slot("pub", Side::Ally),
            slot("priv", Side::Ally),
            slot("other", Side::Enemy),
            incognito(slot("hidden", Side::Ally)),
        ];
        m.lobby = Some(lobby(slots));
        let named = |n: &str| PlayerData {
            name: Some(RiotId {
                name: n.into(),
                tag: "TAG".into(),
            }),
            ..Default::default()
        };
        m.players.insert(
            "pub".into(),
            PlayerData {
                last_match: Some("last".into()),
                ..named("Pub")
            },
        );
        let private_on_tracker_gg_with_no_history = named("Priv");
        m.players
            .insert("priv".into(), private_on_tracker_gg_with_no_history);
        m.players.insert("other".into(), named("Other"));
        let named_from_an_earlier_match_but_incognito_now = named("Hidden");
        m.players.insert(
            "hidden".into(),
            named_from_an_earlier_match_but_incognito_now,
        );
        m.match_parties.insert(
            "last".into(),
            vec![vec![
                "pub#tag".into(),
                "priv#tag".into(),
                "other#tag".into(),
                "hidden#tag".into(),
            ]],
        );
        let parties = m.parties();
        assert_eq!(parties.get("pub"), parties.get("priv"));
        assert!(parties["pub"].starts_with(LIKELY_PARTY));
        assert!(!parties.contains_key("other"), "on the other team now");
        assert!(
            !parties.contains_key("hidden"),
            "incognito now, so never matched"
        );
        m.match_parties.clear();
        assert!(m.parties().is_empty(), "nothing until the match loads");
    }

    #[test]
    fn parties_only_include_your_own_outside_a_match() {
        let mut m = Model::new(Content::default());
        m.me = "me".into();
        for (p, party) in [
            ("me", "x"),
            ("mate", "x"),
            ("a", "y"),
            ("b", "y"),
            ("solo", "z"),
        ] {
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
            assert!(
                own.contains_key("mate") && !own.contains_key("a"),
                "{phase:?}"
            );
        }
    }
}
