//! `--demo`: a sample lobby for trying the UI without a live match. Tab cycles screens.

use crate::content::Content;
use crate::model::{Load, Lobby, Model, Phase, Pick, PlayerData, Side, Slot};
use crate::presence::{LoopState, Presence};
use crate::stats::{PlayerMatch, Record};
use crate::tracker::{ActStats, Profile, RecentMatch};
use chrono::{Duration as Days, Utc};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ASCENT: &str = "/Game/Maps/Ascent/Ascent";

pub const SCREENS: [&str; 5] = ["waiting", "menus", "agent-select", "loading", "in-game"];

fn content() -> Content {
    let agents: Vec<Value> = ["Jett", "Sova", "Killjoy", "Omen", "Sage", "Raze", "Breach", "Clove", "Cypher", "Neon"]
        .iter()
        .map(|a| json!({ "uuid": a.to_lowercase(), "displayName": a }))
        .collect();
    let maps: Vec<Value> = ["Ascent", "Bind", "Haven", "Lotus", "Sunset", "Abyss", "Pearl"]
        .iter()
        .map(|m| json!({ "uuid": m.to_lowercase(), "displayName": m, "mapUrl": format!("/Game/Maps/{m}/{m}") }))
        .collect();
    let mut tiers = vec![json!({ "tier": 0, "tierName": "UNRANKED" })];
    let names = ["IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "DIAMOND", "ASCENDANT", "IMMORTAL"];
    for (i, n) in names.iter().enumerate() {
        for d in 1..=3 {
            tiers.push(json!({ "tier": 3 + i * 3 + d - 1, "tierName": format!("{n} {d}") }));
        }
    }
    tiers.push(json!({ "tier": 27, "tierName": "RADIANT" }));
    let now = Utc::now();
    let seasons: Vec<Value> = (1..=6)
        .map(|n| {
            let start = now - Days::days(3 + 56 * (6 - n as i64));
            json!({ "uuid": format!("a{n}"), "title": format!("V26 // ACT {}", ["I", "II", "III", "IV", "V", "VI"][n - 1]),
                    "type": "EAresSeasonType::Act", "startTime": start.to_rfc3339(), "endTime": (start + Days::days(56)).to_rfc3339() })
        })
        .collect();
    Content::from_responses(
        &json!({ "data": agents }),
        &json!({ "data": maps }),
        &json!({ "data": [{ "tiers": tiers }] }),
        &json!({ "data": seasons }),
    )
}

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
    act: (u32, u32, &'static str),
    map: (u32, u32),
    kd: (u32, u32),
    hs: (u32, u32),
    party: Option<&'static str>,
}

#[rustfmt::skip]
fn roster() -> Vec<P> {
    use Pick::{Hover, Locked};
    use Side::*;
    vec![
        P { id: "me", name: "Alpha", tag: "DEMO", agent: "jett", side: Ally, pick: Locked, incognito: false, level: Some(214), tier: 19, rr: 47, peak: 21, act: (34, 28, "a6"), map: (7, 5), kd: (121, 100), hs: (24, 76), party: Some("x") },
        P { id: "sova", name: "Bravo", tag: "DEMO", agent: "sova", side: Ally, pick: Hover, incognito: false, level: Some(156), tier: 18, rr: 12, peak: 20, act: (36, 34, "a6"), map: (7, 8), kd: (98, 100), hs: (19, 81), party: Some("x") },
        P { id: "killjoy", name: "Charlie", tag: "DEMO", agent: "killjoy", side: Ally, pick: Pick::None, incognito: false, level: None, tier: 17, rr: 88, peak: 18, act: (65, 68, "a5"), map: (2, 2), kd: (105, 100), hs: (21, 79), party: None },
        P { id: "omen", name: "hidden", tag: "", agent: "omen", side: Ally, pick: Pick::None, incognito: true, level: Some(87), tier: 19, rr: 3, peak: 19, act: (15, 10, "a6"), map: (4, 2), kd: (134, 100), hs: (28, 72), party: None },
        P { id: "sage", name: "Delta", tag: "DEMO", agent: "sage", side: Ally, pick: Hover, incognito: false, level: Some(31), tier: 0, rr: 0, peak: 14, act: (14, 13, "a1"), map: (0, 0), kd: (88, 100), hs: (15, 85), party: None },
        P { id: "raze", name: "Echo", tag: "DEMO", agent: "raze", side: Enemy, pick: Locked, incognito: false, level: Some(302), tier: 21, rr: 34, peak: 22, act: (49, 39, "a6"), map: (11, 7), kd: (142, 100), hs: (26, 74), party: Some("y") },
        P { id: "breach", name: "Foxtrot", tag: "DEMO", agent: "breach", side: Enemy, pick: Locked, incognito: false, level: Some(190), tier: 20, rr: 70, peak: 21, act: (52, 52, "a6"), map: (4, 5), kd: (102, 100), hs: (18, 82), party: Some("y") },
        P { id: "clove", name: "Golf", tag: "DEMO", agent: "clove", side: Enemy, pick: Locked, incognito: false, level: Some(121), tier: 19, rr: 55, peak: 20, act: (20, 21, "a6"), map: (3, 5), kd: (110, 100), hs: (22, 78), party: Some("z") },
        P { id: "cypher", name: "hidden", tag: "", agent: "cypher", side: Enemy, pick: Locked, incognito: true, level: None, tier: 18, rr: 90, peak: 19, act: (15, 18, "a6"), map: (3, 3), kd: (91, 100), hs: (20, 80), party: Some("z") },
        P { id: "neon", name: "Hotel", tag: "DEMO", agent: "neon", side: Enemy, pick: Locked, incognito: false, level: Some(245), tier: 16, rr: 21, peak: 19, act: (51, 46, "a5"), map: (6, 5), kd: (118, 100), hs: (23, 77), party: None },
    ]
}

fn player_data(p: &P) -> PlayerData {
    // Like tracker.gg: the latest act played, and unranked until a game this act.
    let acts = vec![ActStats { act_id: p.act.2.into(), tier: p.tier, record: Record { wins: p.act.0, losses: p.act.1 } }];
    let this_act = p.act.2 == "a6";
    let maps = [("Ascent", p.map.0, p.map.1), ("Bind", 1, 0), ("Haven", 0, 1), ("Lotus", 1, 0)]
        .into_iter()
        .filter(|(_, w, l)| w + l > 0)
        .map(|(name, wins, losses)| (name.to_string(), Record { wins, losses }))
        .collect();
    let recent = ["Ascent", "Lotus", "Bind", "Haven", "Sunset"]
        .iter()
        .enumerate()
        .map(|(i, map)| {
            let won = i % 2 == 0;
            RecentMatch {
                map_id: map.to_lowercase(),
                won: Some(won),
                rounds_won: if won { 13 } else { 10 },
                rounds_lost: if won { 9 } else { 13 },
                player: PlayerMatch {
                    agent_id: p.agent.into(),
                    kills: p.kd.0 / 5,
                    deaths: p.kd.1 / 5,
                    assists: 5,
                    score: 5800 + i as u32 * 300,
                    rounds: 22,
                    headshots: p.hs.0,
                    bodyshots: p.hs.1,
                    performance: Some(250.0 + 20.0 * i as f64),
                    ..Default::default()
                },
            }
        })
        .collect();
    PlayerData {
        name: (!p.incognito).then(|| (p.name.to_string(), p.tag.to_string())),
        profile: Load::Ready(Profile { tier: if this_act { p.tier } else { 0 }, rr: this_act.then_some(p.rr), peak_tier: p.peak, acts }),
        maps: Load::Ready(maps),
        recent: Load::Ready(recent),
    }
}

/// The demo model for one of [`SCREENS`].
pub fn model(screen: &str) -> Model {
    let mut m = Model::new(Arc::new(content()));
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
        if screen == "loading" && p.side == Side::Enemy && p.id != "raze" {
            d = PlayerData { name: d.name, ..Default::default() };
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
                    queue_id: Some("competitive".into()),
                    score: Some((5, 3)),
                    queue_entry_time: Some((Utc::now() - Days::seconds(72)).format("%Y.%m.%d-%H.%M.%S").to_string()),
                    party_state: Some("MATCHMAKING".into()),
                    account_level: Some(214),
                },
            );
        }
    }
    m.phase = match screen {
        "waiting" => Phase::Waiting("Riot Client is not running".into()),
        "menus" => Phase::Menus,
        "agent-select" => Phase::Pregame,
        _ => Phase::Ingame,
    };
    if !matches!(screen, "menus" | "waiting") {
        m.lobby = Some(lobby);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::render;
    use crate::ui::{UiState, View, WIDTH};

    fn screen(name: &str) -> Vec<String> {
        render(&model(name), &UiState::default(), WIDTH, 50)
    }

    #[test]
    fn every_demo_screen_fits_the_width() {
        for name in SCREENS {
            let lines = screen(name);
            assert!(!lines.iter().any(|l| l.contains("Terminal too small")), "{name} doesn't fit 112x50");
            assert!(lines.iter().all(|l| l.chars().count() <= WIDTH as usize), "{name} overflows");
        }
    }

    #[test]
    fn incognito_players_are_never_tied_to_a_party() {
        let mut m = model("in-game");
        let mut friend = m.presences["sova"].clone();
        friend.puuid = "omen".into();
        m.presences.insert("omen".into(), friend);
        let parties = m.parties();
        assert!(parties.contains_key("sova"), "{parties:?}");
        assert!(!parties.contains_key("omen"), "an incognito friend in your party isn't marked: {parties:?}");
    }

    #[test]
    fn you_see_your_own_name_when_you_play_incognito() {
        let mut m = model("in-game");
        m.lobby.as_mut().unwrap().slots.iter_mut().find(|s| s.puuid == "me").unwrap().incognito = true;
        let s = render(&m, &UiState::default(), WIDTH, 50).join("\n");
        assert!(s.contains("Alpha"), "{s}");
    }

    #[test]
    fn agent_select_hides_the_enemy_team_and_incognito_names() {
        let s = screen("agent-select").join("\n");
        assert!(s.contains("AGENT SELECT") && s.contains("YOUR TEAM"));
        assert!(!s.contains("ENEMY TEAM"));
        assert!(!s.contains("Echo"));
        assert!(s.contains("Player"), "incognito teammate who hasn't picked shows as Player");
        assert!(!s.contains("hidden"), "incognito names are never drawn");
        assert!(s.contains("LAST 5") && s.contains("WIN% ON ASCENT"));
        assert!(s.contains("55% 34-28"));
        assert!(s.contains("49% 65-68 prev"));
        let sage = s.lines().find(|l| l.contains("Delta")).unwrap();
        assert!(!sage.contains("14-13"), "a record older than last act isn't shown as this act's: {sage}");
    }

    #[test]
    fn in_game_shows_both_teams_without_pick_markers() {
        let s = screen("in-game").join("\n");
        assert!(s.contains("ENEMY TEAM") && s.contains("Echo#DEMO"));
        assert!(!s.contains('◆') && !s.contains('◇'));
        assert!(s.contains("Cypher"), "incognito enemy shows as their agent");
        assert!(s.contains("5 – 3"));
    }

    #[test]
    fn hidden_level_shows_a_dash() {
        let s = screen("in-game");
        let killjoy = s.iter().find(|l| l.contains("Charlie")).unwrap();
        assert!(killjoy.contains(" — "), "hidden level shows a dash: {killjoy}");
    }

    #[test]
    fn private_tracker_profiles_say_so() {
        let mut m = model("in-game");
        let neon = m.players.get_mut("neon").unwrap();
        (neon.profile, neon.maps, neon.recent) = (Load::Private, Load::Private, Load::Private);
        let s = render(&m, &UiState::default(), WIDTH, 50);
        let row = s.iter().find(|l| l.contains("Hotel")).unwrap();
        assert!(row.contains("private") && !row.contains('%') && !row.contains("···"), "{row}");
    }

    #[test]
    fn incognito_players_have_no_stats() {
        let s = screen("in-game");
        let cypher = s.iter().find(|l| l.contains("Cypher")).unwrap();
        assert!(!cypher.contains('%') && !cypher.contains("Diamond"), "incognito players are never looked up: {cypher}");
        let detail =
            render(&model("in-game"), &UiState { view: View::Detail("cypher".into()), ..Default::default() }, WIDTH, 50).join("\n");
        assert!(!detail.contains("Diamond") && !detail.contains("V26"), "{detail}");
    }

    #[test]
    fn loading_marks_pending_enemies() {
        let s = screen("loading");
        let neon = s.iter().find(|l| l.contains("Hotel")).unwrap();
        assert!(neon.contains("···"));
    }

    #[test]
    fn details_show_act_history_maps_and_recent_matches() {
        let mut m = model("in-game");
        let ui = UiState { view: View::Detail("raze".into()), ..Default::default() };
        let theirs = render(&m, &ui, WIDTH, 50).join("\n");
        assert!(theirs.contains("ACT HISTORY") && theirs.contains("MAPS THIS ACT") && theirs.contains("RECENT MATCHES"));
        m.players.get_mut("cypher").unwrap().name = Some(("leaked".into(), "1".into()));
        let ui = UiState { view: View::Detail("cypher".into()), ..Default::default() };
        let incog = render(&m, &ui, WIDTH, 50).join("\n");
        assert!(!incog.contains("leaked"), "an incognito player's name never appears, even if known");
    }
}
