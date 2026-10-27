//! `--demo`: a sample lobby for trying the UI without a live match. Tab cycles screens.

use crate::content::Content;
use crate::model::{Load, Lobby, Model, Phase, Pick, PlayerData, Side, Slot};
use crate::presence::{LoopState, Presence};
use crate::tracker::Profile;
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
    /// The latest act played; unranked until a game this act.
    this_act: bool,
    party: Option<&'static str>,
}

#[rustfmt::skip]
fn roster() -> Vec<P> {
    use Pick::{Hover, Locked};
    use Side::*;
    vec![
        P { id: "me", name: "Alpha", tag: "DEMO", agent: "jett", side: Ally, pick: Locked, incognito: false, level: Some(214), tier: 19, rr: 47, peak: 21, this_act: true, party: Some("x") },
        P { id: "sova", name: "Bravo", tag: "DEMO", agent: "sova", side: Ally, pick: Hover, incognito: false, level: Some(156), tier: 18, rr: 12, peak: 20, this_act: true, party: Some("x") },
        P { id: "killjoy", name: "Charlie", tag: "DEMO", agent: "killjoy", side: Ally, pick: Pick::None, incognito: false, level: None, tier: 17, rr: 88, peak: 18, this_act: false, party: None },
        P { id: "omen", name: "hidden", tag: "", agent: "omen", side: Ally, pick: Pick::None, incognito: true, level: Some(87), tier: 19, rr: 3, peak: 19, this_act: true, party: None },
        P { id: "sage", name: "Delta", tag: "DEMO", agent: "sage", side: Ally, pick: Hover, incognito: false, level: Some(31), tier: 0, rr: 0, peak: 14, this_act: false, party: None },
        P { id: "raze", name: "Echo", tag: "DEMO", agent: "raze", side: Enemy, pick: Locked, incognito: false, level: Some(302), tier: 21, rr: 34, peak: 22, this_act: true, party: Some("y") },
        P { id: "breach", name: "Foxtrot", tag: "DEMO", agent: "breach", side: Enemy, pick: Locked, incognito: false, level: Some(190), tier: 20, rr: 70, peak: 21, this_act: true, party: Some("y") },
        P { id: "clove", name: "Golf", tag: "DEMO", agent: "clove", side: Enemy, pick: Locked, incognito: false, level: Some(121), tier: 19, rr: 55, peak: 20, this_act: true, party: Some("z") },
        P { id: "cypher", name: "hidden", tag: "", agent: "cypher", side: Enemy, pick: Locked, incognito: true, level: None, tier: 18, rr: 90, peak: 19, this_act: true, party: Some("z") },
        P { id: "neon", name: "Hotel", tag: "DEMO", agent: "neon", side: Enemy, pick: Locked, incognito: false, level: Some(245), tier: 16, rr: 21, peak: 19, this_act: false, party: None },
    ]
}

fn player_data(p: &P) -> PlayerData {
    let this_act = p.this_act;
    PlayerData {
        name: (!p.incognito).then(|| (p.name.to_string(), p.tag.to_string())),
        profile: Load::Ready(Profile { tier: if this_act { p.tier } else { 0 }, rr: this_act.then_some(p.rr), peak_tier: p.peak }),
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
    use crate::ui::WIDTH;
    use crate::ui::tests::render;

    fn screen(name: &str) -> Vec<String> {
        render(&model(name), WIDTH, 50)
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
        let s = render(&m, WIDTH, 50).join("\n");
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
        let sage = s.lines().find(|l| l.contains("Delta")).unwrap();
        assert!(sage.contains("Unranked"), "no game this act yet: {sage}");
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
        neon.profile = Load::Private;
        let s = render(&m, WIDTH, 50);
        let row = s.iter().find(|l| l.contains("Hotel")).unwrap();
        assert!(row.contains("private") && !row.contains('%') && !row.contains("···"), "{row}");
    }

    #[test]
    fn incognito_players_have_no_stats() {
        let s = screen("in-game");
        let cypher = s.iter().find(|l| l.contains("Cypher")).unwrap();
        assert!(!cypher.contains('%') && !cypher.contains("Diamond"), "incognito players are never looked up: {cypher}");
    }

    #[test]
    fn loading_marks_pending_enemies() {
        let s = screen("loading");
        let neon = s.iter().find(|l| l.contains("Hotel")).unwrap();
        assert!(neon.contains("···"));
    }
}
