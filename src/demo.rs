//! `--demo`: a sample lobby for trying the UI without a live match. Tab cycles screens.

use crate::content::Content;
use crate::model::{Lobby, Model, Phase, Pick, PlayerData, Side, Slot};
use crate::presence::{LoopState, Presence};
use chrono::{Duration as Days, Utc};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ASCENT: &str = "/Game/Maps/Ascent/Ascent";

pub const SCREENS: [&str; 4] = ["waiting", "menus", "agent-select", "in-game"];

fn content() -> Content {
    let agents: Vec<Value> = ["Jett", "Sova", "Killjoy", "Omen", "Sage", "Raze", "Breach", "Clove", "Cypher", "Neon"]
        .iter()
        .map(|a| json!({ "uuid": a.to_lowercase(), "displayName": a }))
        .collect();
    let maps: Vec<Value> = ["Ascent", "Bind", "Haven", "Lotus", "Sunset", "Abyss", "Pearl"]
        .iter()
        .map(|m| json!({ "uuid": m.to_lowercase(), "displayName": m, "mapUrl": format!("/Game/Maps/{m}/{m}") }))
        .collect();
    Content::from_responses(&json!({ "data": agents }), &json!({ "data": maps }))
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
    party: Option<&'static str>,
}

#[rustfmt::skip]
fn roster() -> Vec<P> {
    use Pick::{Hover, Locked};
    use Side::*;
    vec![
        P { id: "me", name: "Alpha", tag: "DEMO", agent: "jett", side: Ally, pick: Locked, incognito: false, level: Some(214), party: Some("x") },
        P { id: "sova", name: "Bravo", tag: "DEMO", agent: "sova", side: Ally, pick: Hover, incognito: false, level: Some(156), party: Some("x") },
        P { id: "killjoy", name: "Charlie", tag: "DEMO", agent: "killjoy", side: Ally, pick: Pick::None, incognito: false, level: None, party: None },
        P { id: "omen", name: "hidden", tag: "", agent: "omen", side: Ally, pick: Pick::None, incognito: true, level: Some(87), party: None },
        P { id: "sage", name: "Delta", tag: "DEMO", agent: "sage", side: Ally, pick: Hover, incognito: false, level: Some(31), party: None },
        P { id: "raze", name: "Echo", tag: "DEMO", agent: "raze", side: Enemy, pick: Locked, incognito: false, level: Some(302), party: Some("y") },
        P { id: "breach", name: "Foxtrot", tag: "DEMO", agent: "breach", side: Enemy, pick: Locked, incognito: false, level: Some(190), party: Some("y") },
        P { id: "clove", name: "Golf", tag: "DEMO", agent: "clove", side: Enemy, pick: Locked, incognito: false, level: Some(121), party: Some("z") },
        P { id: "cypher", name: "hidden", tag: "", agent: "cypher", side: Enemy, pick: Locked, incognito: true, level: None, party: Some("z") },
        P { id: "neon", name: "Hotel", tag: "DEMO", agent: "neon", side: Enemy, pick: Locked, incognito: false, level: Some(245), party: None },
    ]
}

fn player_data(p: &P) -> PlayerData {
    PlayerData { name: (!p.incognito).then(|| (p.name.to_string(), p.tag.to_string())) }
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
        m.players.insert(p.id.into(), player_data(p));
        if let (Some(party), Side::Ally) = (p.party, p.side) {
            let state = match screen {
                "agent-select" => LoopState::Pregame,
                "in-game" => LoopState::Ingame,
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
}
