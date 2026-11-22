//! `--demo`: a sample lobby for trying the UI without a live match. Tab cycles screens.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::content::Content;
use crate::goldstars::{ALL_TIME, Accolade, Earned, MatchAccolades, PlayerGoldstars, Summary};
use crate::model::{Load, Lobby, Model, Phase, Pick, PlayerData, Postgame, RiotId, Side, Slot};
use crate::presence::{LoopState, MatchScore, Presence};
use crate::stats::{CompUpdate, MatchSummary, PlayerMatch, Record, TeamResult};
use crate::tracker::{ActStats, MapRecord, MatchSide, Profile, RecentMatch};

const ASCENT: &str = "/Game/Maps/Ascent/Ascent";

/// The demo's screens, in the order Tab cycles them.
pub const SCREENS: [&str; 7] = [
    "waiting",
    "menus",
    "agent-select",
    "loading",
    "in-game",
    "victory",
    "defeat",
];

fn days(n: i64) -> SignedDuration {
    SignedDuration::from_secs(86_400 * n)
}

fn content() -> Content {
    let agents: Vec<Value> = [
        "Jett", "Sova", "Killjoy", "Omen", "Sage", "Raze", "Breach", "Clove", "Cypher", "Neon",
    ]
    .iter()
    .map(|a| json!({ "uuid": a.to_lowercase(), "displayName": a }))
    .collect();
    let maps: Vec<Value> = ["Ascent", "Bind", "Haven", "Lotus", "Sunset", "Abyss", "Pearl"]
        .iter()
        .map(|m| json!({ "uuid": m.to_lowercase(), "displayName": m, "mapUrl": format!("/Game/Maps/{m}/{m}") }))
        .collect();
    let mut tiers = vec![json!({ "tier": 0, "tierName": "UNRANKED" })];
    let names = [
        "IRON",
        "BRONZE",
        "SILVER",
        "GOLD",
        "PLATINUM",
        "DIAMOND",
        "ASCENDANT",
        "IMMORTAL",
    ];
    for (i, n) in names.iter().enumerate() {
        for d in 1..=3 {
            tiers.push(json!({ "tier": 3 + i * 3 + d - 1, "tierName": format!("{n} {d}") }));
        }
    }
    tiers.push(json!({ "tier": 27, "tierName": "RADIANT" }));
    let now = Timestamp::now();
    let seasons: Vec<Value> = (1_i64..)
        .zip(["I", "II", "III", "IV", "V", "VI"])
        .map(|(n, numeral)| {
            let start = now - days(3 + 56 * (6 - n));
            json!({ "uuid": format!("a{n}"), "title": format!("V26 // ACT {numeral}"),
                    "type": "EAresSeasonType::Act", "startTime": start.to_string(), "endTime": (start + days(56)).to_string() })
        })
        .collect();
    Content::from_responses(
        &json!({ "data": agents }),
        &json!({ "data": maps }),
        &json!({ "data": [{ "tiers": tiers }] }),
        &json!({ "data": seasons }),
    )
}

/// One player of the demo lobby.
struct P {
    id: &'static str,
    name: &'static str,
    tag: &'static str,
    agent: &'static str,
    side: Side,
    pick: Pick,
    incognito: bool,
    level: Option<u32>,
    tier: u32,
    rr: u32,
    peak: u32,
    /// The record in `act`, the latest act the player has played.
    act_wins: u32,
    act_losses: u32,
    act: &'static str,
    /// The record on Ascent this act.
    ascent_wins: u32,
    ascent_losses: u32,
    /// Kills and deaths over the five recent matches.
    recent_kills: u32,
    recent_deaths: u32,
    /// Hits on the head and on the body in each recent match.
    headshots: u32,
    bodyshots: u32,
    party: Option<&'static str>,
}

#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        rustfmt_skip_justification,
        reason = "manual alignment: one line per player keeps the roster a table to read down"
    )
)]
#[rustfmt::skip]
fn roster() -> Vec<P> {
    use Pick::{Hover, Locked};
    use Side::*;
    vec![
        P { id: "me", name: "Alpha", tag: "DEMO", agent: "jett", side: Ally, pick: Locked, incognito: false, level: Some(214), tier: 19, rr: 47, peak: 21, act_wins: 34, act_losses: 28, act: "a6", ascent_wins: 7, ascent_losses: 5, recent_kills: 121, recent_deaths: 100, headshots: 24, bodyshots: 76, party: Some("x") },
        P { id: "sova", name: "Bravo", tag: "DEMO", agent: "sova", side: Ally, pick: Hover, incognito: false, level: Some(156), tier: 18, rr: 12, peak: 20, act_wins: 36, act_losses: 34, act: "a6", ascent_wins: 7, ascent_losses: 8, recent_kills: 98, recent_deaths: 100, headshots: 19, bodyshots: 81, party: Some("x") },
        P { id: "killjoy", name: "Charlie", tag: "DEMO", agent: "killjoy", side: Ally, pick: Pick::None, incognito: false, level: None, tier: 17, rr: 88, peak: 18, act_wins: 65, act_losses: 68, act: "a5", ascent_wins: 2, ascent_losses: 2, recent_kills: 105, recent_deaths: 100, headshots: 21, bodyshots: 79, party: None },
        P { id: "omen", name: "hidden", tag: "", agent: "omen", side: Ally, pick: Pick::None, incognito: true, level: Some(87), tier: 19, rr: 3, peak: 19, act_wins: 15, act_losses: 10, act: "a6", ascent_wins: 4, ascent_losses: 2, recent_kills: 134, recent_deaths: 100, headshots: 28, bodyshots: 72, party: None },
        P { id: "sage", name: "Delta", tag: "DEMO", agent: "sage", side: Ally, pick: Hover, incognito: false, level: Some(31), tier: 0, rr: 0, peak: 14, act_wins: 14, act_losses: 13, act: "a1", ascent_wins: 0, ascent_losses: 0, recent_kills: 88, recent_deaths: 100, headshots: 15, bodyshots: 85, party: None },
        P { id: "raze", name: "Echo", tag: "DEMO", agent: "raze", side: Enemy, pick: Locked, incognito: false, level: Some(302), tier: 21, rr: 34, peak: 22, act_wins: 49, act_losses: 39, act: "a6", ascent_wins: 11, ascent_losses: 7, recent_kills: 142, recent_deaths: 100, headshots: 26, bodyshots: 74, party: Some("y") },
        P { id: "breach", name: "Foxtrot", tag: "DEMO", agent: "breach", side: Enemy, pick: Locked, incognito: false, level: Some(190), tier: 20, rr: 70, peak: 21, act_wins: 52, act_losses: 52, act: "a6", ascent_wins: 4, ascent_losses: 5, recent_kills: 102, recent_deaths: 100, headshots: 18, bodyshots: 82, party: Some("y") },
        P { id: "clove", name: "Golf", tag: "DEMO", agent: "clove", side: Enemy, pick: Locked, incognito: false, level: Some(121), tier: 19, rr: 55, peak: 20, act_wins: 20, act_losses: 21, act: "a6", ascent_wins: 3, ascent_losses: 5, recent_kills: 110, recent_deaths: 100, headshots: 22, bodyshots: 78, party: Some("z") },
        P { id: "cypher", name: "hidden", tag: "", agent: "cypher", side: Enemy, pick: Locked, incognito: true, level: None, tier: 18, rr: 90, peak: 19, act_wins: 15, act_losses: 18, act: "a6", ascent_wins: 3, ascent_losses: 3, recent_kills: 91, recent_deaths: 100, headshots: 20, bodyshots: 80, party: Some("z") },
        P { id: "neon", name: "Hotel", tag: "DEMO", agent: "neon", side: Enemy, pick: Locked, incognito: false, level: Some(245), tier: 16, rr: 21, peak: 19, act_wins: 51, act_losses: 46, act: "a5", ascent_wins: 6, ascent_losses: 5, recent_kills: 118, recent_deaths: 100, headshots: 23, bodyshots: 77, party: None },
    ]
}

fn summary(
    id: &str,
    map: &str,
    players: Vec<PlayerMatch>,
    blue_rounds: u32,
    red_rounds: u32,
    start_ms: i64,
) -> MatchSummary {
    MatchSummary {
        match_id: id.into(),
        map_url: format!("/Game/Maps/{map}/{map}"),
        queue: "competitive".into(),
        start_ms,
        length_ms: 34 * 60_000 + 12_000,
        teams: HashMap::from([
            (
                "Blue".into(),
                TeamResult {
                    rounds_won: blue_rounds,
                    won: blue_rounds > red_rounds,
                },
            ),
            (
                "Red".into(),
                TeamResult {
                    rounds_won: red_rounds,
                    won: red_rounds > blue_rounds,
                },
            ),
        ]),
        players,
        ..Default::default()
    }
}

/// What tracker.gg would report for `p`: the latest act played, and unranked until a game this act.
fn player_data(p: &P) -> PlayerData {
    let acts = vec![ActStats {
        act_id: p.act.into(),
        tier: p.tier,
        record: Record {
            wins: p.act_wins,
            losses: p.act_losses,
        },
    }];
    let this_act = p.act == "a6";
    let history = vec![
        ActStats {
            act_id: "a4".into(),
            tier: p.peak.saturating_sub(1),
            record: Record {
                wins: 30,
                losses: 27,
            },
        },
        ActStats {
            act_id: "a3".into(),
            tier: p.tier,
            record: Record {
                wins: 40,
                losses: 38,
            },
        },
    ];
    let maps: Vec<MapRecord> = [
        ("Ascent", p.ascent_wins, p.ascent_losses),
        ("Bind", 1, 0),
        ("Haven", 0, 1),
        ("Lotus", 1, 0),
    ]
    .into_iter()
    .filter(|(_, w, l)| w + l > 0)
    .map(|(name, wins, losses)| MapRecord {
        name: name.to_string(),
        record: Record { wins, losses },
    })
    .collect();
    let recent: Vec<RecentMatch> = (0_u32..)
        .zip(["Ascent", "Lotus", "Bind", "Haven", "Sunset"])
        .map(|(i, map)| {
            let won = i % 2 == 0;
            RecentMatch {
                map_id: map.to_lowercase(),
                won: Some(won),
                rounds_won: if won { 13 } else { 10 },
                rounds_lost: if won { 9 } else { 13 },
                player: PlayerMatch {
                    agent_id: p.agent.into(),
                    kills: p.recent_kills / 5,
                    deaths: p.recent_deaths / 5,
                    assists: 5,
                    score: 5800 + i * 300,
                    rounds: 22,
                    headshots: p.headshots,
                    bodyshots: p.bodyshots,
                    performance: Some(250.0 + 20.0 * f64::from(i)),
                    ..Default::default()
                },
            }
        })
        .collect();
    PlayerData {
        name: (!p.incognito).then(|| RiotId {
            name: p.name.to_string(),
            tag: p.tag.to_string(),
        }),
        profile: Load::Ready(Profile {
            tier: if this_act { p.tier } else { 0 },
            rr: this_act.then_some(p.rr),
            peak_tier: p.peak,
            acts,
        }),
        maps: Load::Ready(maps),
        recent: Load::Ready(recent),
        history,
        sides: Vec::new(),
        last_match: None,
    }
}

/// One player's line on the demo match's scoreboard.
struct StatLine {
    id: &'static str,
    team: &'static str,
    performance: f64,
    score: u32,
    kills: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    headshot_pct: u32,
    first_kills: u32,
}

#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        rustfmt_skip_justification,
        reason = "manual alignment: one line per player keeps the scoreboard a table to read down"
    )
)]
#[rustfmt::skip]
const SCOREBOARD: [StatLine; 10] = [
    StatLine { id: "omen", team: "Blue", performance: 431.0, score: 5764, kills: 20, deaths: 11, assists: 8, damage: 3630, headshot_pct: 30, first_kills: 3 },
    StatLine { id: "me", team: "Blue", performance: 338.0, score: 6314, kills: 24, deaths: 15, assists: 6, damage: 3916, headshot_pct: 27, first_kills: 5 },
    StatLine { id: "sova", team: "Blue", performance: 281.0, score: 4422, kills: 15, deaths: 14, assists: 11, damage: 3102, headshot_pct: 19, first_kills: 2 },
    StatLine { id: "killjoy", team: "Blue", performance: 247.0, score: 4136, kills: 14, deaths: 13, assists: 5, damage: 2838, headshot_pct: 22, first_kills: 1 },
    StatLine { id: "sage", team: "Blue", performance: 213.0, score: 2640, kills: 8, deaths: 16, assists: 12, damage: 1870, headshot_pct: 14, first_kills: 0 },
    StatLine { id: "raze", team: "Red", performance: 331.0, score: 5830, kills: 22, deaths: 17, assists: 4, damage: 3696, headshot_pct: 26, first_kills: 4 },
    StatLine { id: "breach", team: "Red", performance: 268.0, score: 3476, kills: 11, deaths: 17, assists: 13, damage: 2420, headshot_pct: 18, first_kills: 1 },
    StatLine { id: "clove", team: "Red", performance: 254.0, score: 4180, kills: 15, deaths: 16, assists: 7, damage: 2860, headshot_pct: 22, first_kills: 2 },
    StatLine { id: "neon", team: "Red", performance: 229.0, score: 3784, kills: 14, deaths: 18, assists: 3, damage: 2662, headshot_pct: 23, first_kills: 2 },
    StatLine { id: "cypher", team: "Red", performance: 188.0, score: 3080, kills: 10, deaths: 16, assists: 6, damage: 2112, headshot_pct: 20, first_kills: 0 },
];

/// The demo match's stats, which you won or lost.
fn match_summary(win: bool) -> MatchSummary {
    let parties = HashMap::from([
        ("me", "x"),
        ("sova", "x"),
        ("raze", "y"),
        ("breach", "y"),
        ("clove", "z"),
        ("cypher", "z"),
    ]);
    let agent = |id: &str| {
        roster()
            .into_iter()
            .find(|p| p.id == id)
            .map_or("jett", |p| p.agent)
    };
    let players: Vec<PlayerMatch> = SCOREBOARD
        .iter()
        .map(|line| PlayerMatch {
            puuid: line.id.into(),
            team: line.team.into(),
            agent_id: agent(line.id).into(),
            party_id: parties.get(line.id).map_or_default(ToString::to_string),
            kills: line.kills,
            deaths: line.deaths,
            assists: line.assists,
            score: line.score,
            rounds: 22,
            damage: line.damage,
            headshots: line.headshot_pct,
            bodyshots: 100 - line.headshot_pct,
            first_kills: line.first_kills,
            performance: Some(line.performance),
            ..Default::default()
        })
        .collect();
    let (blue_rounds, red_rounds) = if win { (13, 9) } else { (9, 13) };
    let mut s = summary(
        "demo-match",
        "Ascent",
        players,
        blue_rounds,
        red_rounds,
        1_000_000,
    );
    if !win {
        for p in &mut s.players {
            p.team = if p.team == "Blue" {
                "Red".into()
            } else {
                "Blue".into()
            };
        }
        s.teams = HashMap::from([
            (
                "Red".into(),
                TeamResult {
                    rounds_won: 9,
                    won: false,
                },
            ),
            (
                "Blue".into(),
                TeamResult {
                    rounds_won: 13,
                    won: true,
                },
            ),
        ]);
    }
    s
}

/// The accolades each player earned in the demo match.
fn match_accolades() -> HashMap<String, HashMap<Accolade, Earned>> {
    use Accolade::*;

    let earned: [(&str, &[Accolade]); 7] = [
        ("omen", &[Mvp, Masterful, Headshots, Clutches, Trades]),
        ("me", &[TopFrag, Kills, FirstBlood, Damage]),
        ("sova", &[Assists, Trades]),
        ("killjoy", &[Plants]),
        ("raze", &[Aces, Kills]),
        ("breach", &[Trades]),
        ("clove", &[Clutches]),
    ];
    earned
        .iter()
        .map(|(id, ks)| {
            (
                id.to_string(),
                ks.iter()
                    .map(|k| {
                        (
                            *k,
                            Earned {
                                value: 5.0,
                                act_record: *id == "me" && *k == FirstBlood,
                            },
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

fn postgame(win: bool, lobby: Lobby) -> Postgame {
    let s = match_summary(win);
    let players = match_accolades();
    let rr = if win { 21 } else { -16 };
    Postgame {
        match_id: "demo-match".into(),
        summary: Some(Arc::new(s)),
        rr: Some(CompUpdate {
            match_id: "demo-match".into(),
            map_url: ASCENT.into(),
            act_id: "a6".into(),
            rr_earned: rr,
            tier_after: 19,
            rr_after: 47_u32.saturating_add_signed(rr),
        }),
        accolades: Some(MatchAccolades {
            start_ms: 1_000_000,
            players,
        }),
        lobby: Some(lobby),
        shown_at: Some(Instant::now()),
    }
}

fn goldstars() -> PlayerGoldstars {
    use Accolade::*;

    let rows: [(Accolade, u32, f64, u32, f64); 12] = [
        (Mvp, 6, 461.0, 6, 461.0),
        (Masterful, 9, 461.0, 9, 461.0),
        (TopFrag, 11, 31.0, 11, 31.0),
        (Kills, 14, 31.0, 16, 31.0),
        (FirstBlood, 8, 5.0, 10, 7.0),
        (Damage, 7, 211.0, 9, 211.0),
        (Headshots, 5, 41.0, 7, 44.0),
        (Assists, 3, 14.0, 4, 16.0),
        (Plants, 4, 5.0, 5, 6.0),
        (Aces, 1, 1.0, 1, 1.0),
        (Clutches, 4, 2.0, 5, 2.0),
        (Trades, 6, 9.0, 7, 9.0),
    ];
    let act: HashMap<Accolade, Summary> = rows
        .iter()
        .map(|&(kind, count, best, ..)| (kind, Summary { count, best }))
        .collect();
    let all: HashMap<Accolade, Summary> = rows
        .iter()
        .map(|&(kind, _, _, count, best)| (kind, Summary { count, best }))
        .collect();
    let last = MatchAccolades {
        start_ms: 1,
        players: HashMap::from([(
            "me".into(),
            HashMap::from([(
                FirstBlood,
                Earned {
                    value: 5.0,
                    act_record: true,
                },
            )]),
        )]),
    };
    PlayerGoldstars {
        acts: HashMap::from([("a6".into(), act), (ALL_TIME.into(), all)]),
        matches: vec![last],
    }
}

/// The demo model for one of [`SCREENS`]. Riot shares only your friends' parties, so the enemies'
/// parties come from their tracker.gg histories, as in a live match.
#[must_use]
pub fn model(screen: &str) -> Model {
    let mut m = Model::new(content());
    m.me = "me".into();
    m.region = "EU".into();
    let roster = roster();
    let pregame = screen == "agent-select";
    let slots: Vec<Slot> = roster
        .iter()
        .filter(|p| !pregame || p.side == Side::Ally)
        .map(|p| Slot {
            puuid: p.id.into(),
            side: p.side,
            agent_id: (!pregame || p.pick != Pick::None).then(|| p.agent.to_string()),
            pick: if pregame { p.pick } else { Pick::Locked },
            incognito: p.incognito,
            hide_level: p.level.is_none(),
            level: p.level,
        })
        .collect();
    let lobby = Lobby {
        match_id: "demo-match".into(),
        map_url: ASCENT.into(),
        queue: "competitive".into(),
        slots,
        phase_ends: pregame.then(|| Instant::now() + Duration::from_secs(42)),
    };
    for p in &roster {
        let mut d = player_data(p);
        if let (Some(party), Side::Enemy) = (p.party, p.side) {
            d.sides = (0..3)
                .map(|i| MatchSide {
                    match_id: format!("{party}{i}"),
                    rounds_won: 13,
                    rounds_lost: 9,
                })
                .collect();
        }
        if screen == "loading" && p.side == Side::Enemy && p.id != "raze" {
            d = PlayerData {
                name: d.name,
                ..Default::default()
            };
        }
        m.players.insert(p.id.into(), d);
        if let (Some(party), Side::Ally) = (p.party, p.side) {
            let state = match screen {
                "agent-select" => LoopState::Pregame,
                "loading" | "in-game" => LoopState::Ingame,
                _ => LoopState::Menus,
            };
            m.presences.insert(
                p.id.into(),
                Presence {
                    puuid: p.id.into(),
                    state: Some(state),
                    party_id: Some(party.into()),
                    party_size: None,
                    queue_id: Some("competitive".into()),
                    score: Some(MatchScore { ally: 5, enemy: 3 }),
                    queue_entry_time: Some(
                        (Timestamp::now() - SignedDuration::from_secs(72))
                            .strftime("%Y.%m.%d-%H.%M.%S")
                            .to_string(),
                    ),
                    party_state: Some("MATCHMAKING".into()),
                    account_level: Some(214),
                },
            );
        }
    }
    m.my_goldstars = Some(goldstars());
    m.phase = match screen {
        "waiting" => Phase::Waiting("Riot Client is not running".into()),
        "menus" => Phase::Menus,
        "agent-select" => Phase::Pregame,
        "loading" | "in-game" => Phase::Ingame,
        _ => Phase::Postgame,
    };
    match screen {
        "victory" | "defeat" => m.postgame = Some(postgame(screen == "victory", lobby)),
        "menus" | "waiting" => {}
        _ => m.lobby = Some(lobby),
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_shows_its_phase() {
        let phases: Vec<Phase> = SCREENS.iter().map(|s| model(s).phase).collect();
        assert_eq!(
            phases,
            [
                Phase::Waiting("Riot Client is not running".into()),
                Phase::Menus,
                Phase::Pregame,
                Phase::Ingame,
                Phase::Ingame,
                Phase::Postgame,
                Phase::Postgame,
            ]
        );
    }

    #[test]
    fn incognito_players_are_never_tied_to_a_party() {
        let mut m = model("in-game");
        let mut friend = m.presences["sova"].clone();
        friend.puuid = "omen".into();
        m.presences.insert("omen".into(), friend);
        let parties = m.parties();
        assert!(parties.contains_key("sova"), "{parties:?}");
        assert!(
            !parties.contains_key("omen"),
            "an incognito friend in your party isn't marked: {parties:?}"
        );
    }

    #[test]
    fn a_postgame_whose_results_never_come_goes_back_to_the_menus() {
        let mut m = model("victory");
        m.give_up_postgame("another match");
        assert_eq!(m.phase, Phase::Postgame, "only the match it waited for");
        let id = m.postgame.as_ref().unwrap().match_id.clone();
        m.give_up_postgame(&id);
        assert_eq!(m.phase, Phase::Menus);
        assert!(m.postgame.is_none());
    }
}
