//! The demo's screens, rendered as `teamdiff --demo` shows them.

use ratatui_image::picker::{Picker, ProtocolType};
use teamdiff_core::demo::{self, SCREENS};
use teamdiff_core::goldstars::Accolade;
use teamdiff_core::model::{Load, RiotId};

use crate::draw::{UiState, View, WIDTH};
use crate::icons::Icons;
use crate::rendering;

fn screen(name: &str) -> Vec<String> {
    rendering::render(&demo::model(name), &UiState::default(), WIDTH, 50)
}

fn details(id: &str) -> UiState {
    UiState {
        view: View::Detail(id.into()),
        ..Default::default()
    }
}

#[test]
fn every_demo_screen_fits_the_width() {
    for name in SCREENS {
        let lines = screen(name);
        assert!(
            !lines.iter().any(|l| l.contains("Terminal too small")),
            "{name} doesn't fit 112x50"
        );
        assert!(
            lines
                .iter()
                .all(|l| l.chars().count() <= usize::from(WIDTH)),
            "{name} overflows"
        );
    }
}

#[test]
fn you_see_your_own_name_when_you_play_incognito() {
    let mut m = demo::model("in-game");
    m.lobby
        .as_mut()
        .unwrap()
        .slots
        .iter_mut()
        .find(|s| s.puuid == "me")
        .unwrap()
        .incognito = true;
    let s = rendering::render(&m, &UiState::default(), WIDTH, 50).join("\n");
    assert!(s.contains("Alpha"), "{s}");
}

#[test]
fn agent_select_hides_the_enemy_team_and_incognito_names() {
    let s = screen("agent-select").join("\n");
    assert!(s.contains("AGENT SELECT") && s.contains("YOUR TEAM"));
    assert!(!s.contains("ENEMY TEAM"));
    assert!(!s.contains("Echo"));
    assert!(
        s.contains("Player"),
        "incognito teammate who hasn't picked shows as Player"
    );
    assert!(!s.contains("hidden"), "incognito names are never drawn");
    assert!(s.contains("LAST 5") && s.contains("WIN% ON ASCENT"));
    assert!(s.contains("55% 34-28"));
    assert!(s.contains("49% 65-68 prev"));
    let sage = s.lines().find(|l| l.contains("Delta")).unwrap();
    assert!(
        !sage.contains("14-13"),
        "a record older than last act isn't shown as this act's: {sage}"
    );
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
    assert!(
        killjoy.contains(" — "),
        "hidden level shows a dash: {killjoy}"
    );
}

#[test]
fn private_tracker_profiles_say_so() {
    let mut m = demo::model("in-game");
    let neon = m.players.get_mut("neon").unwrap();
    (neon.profile, neon.maps, neon.recent) = (Load::Private, Load::Private, Load::Private);
    let s = rendering::render(&m, &UiState::default(), WIDTH, 50);
    let row = s.iter().find(|l| l.contains("Hotel")).unwrap();
    assert!(
        row.contains("private") && !row.contains('%') && !row.contains("···"),
        "{row}"
    );
}

#[test]
fn incognito_players_have_no_stats() {
    let s = screen("in-game");
    let cypher = s.iter().find(|l| l.contains("Cypher")).unwrap();
    assert!(
        !cypher.contains('%') && !cypher.contains("Diamond"),
        "incognito players are never looked up: {cypher}"
    );
    let detail =
        rendering::render(&demo::model("in-game"), &details("cypher"), WIDTH, 50).join("\n");
    assert!(
        !detail.contains("Diamond") && !detail.contains("V26"),
        "{detail}"
    );
}

#[test]
fn loading_marks_pending_enemies() {
    let s = screen("loading");
    let neon = s.iter().find(|l| l.contains("Hotel")).unwrap();
    assert!(neon.contains("···"));
}

#[test]
fn postgame_orders_by_performance_and_labels_mvps() {
    let s = screen("victory");
    let omen = s
        .iter()
        .position(|l| l.contains("Omen") && l.contains("431"))
        .unwrap();
    let me = s.iter().position(|l| l.contains("Alpha")).unwrap();
    assert!(omen < me, "sorted by Performance Score");
    assert!(s[omen].contains("MATCH MVP"));
    assert!(
        s.iter()
            .any(|l| l.contains("Echo") && l.contains("TEAM MVP"))
    );
    assert!(
        s.iter()
            .any(|l| l.contains("New act best") && l.contains("5 first bloods"))
    );
    assert!(
        s[me].contains("TFKLFBDM"),
        "text placeholders for accolades in fixed order: {}",
        s[me]
    );
    assert!(s.iter().any(|l| l.contains("+21")));
}

#[test]
fn defeat_banner_and_rr() {
    let s = screen("defeat").join("\n");
    assert!(s.contains("-16"));
    assert!(s.contains("9  –  13"));
}

#[test]
fn postgame_details_list_the_same_parties_as_the_scoreboard() {
    let mut m = demo::model("victory");
    m.players.get_mut("cypher").unwrap().name = Some(RiotId {
        name: "leaked".into(),
        tag: "1".into(),
    });
    let detail = |id: &str| rendering::render(&m, &details(id), WIDTH, 50).join("\n");
    assert!(
        detail("raze").contains("partied with Foxtrot#DEMO"),
        "{}",
        detail("raze")
    );
    let clove = detail("clove");
    assert!(
        !clove.contains("partied with") && !clove.contains("leaked"),
        "an incognito mate is never shown: {clove}"
    );
    assert!(!detail("killjoy").contains("partied with"));
}

#[test]
fn profile_shows_accolades_only_for_you() {
    let mut m = demo::model("in-game");
    let mine = rendering::render(&m, &details("me"), WIDTH, 50).join("\n");
    assert!(mine.contains("ACCOLADES") && mine.contains("Masterful"));
    assert!(mine.contains("5 new"));
    let theirs = rendering::render(&m, &details("raze"), WIDTH, 50).join("\n");
    assert!(!theirs.contains("ACCOLADES"));
    assert!(
        theirs.contains("ACT HISTORY")
            && theirs.contains("MAPS THIS ACT")
            && theirs.contains("RECENT MATCHES")
    );
    m.players.get_mut("cypher").unwrap().name = Some(RiotId {
        name: "leaked".into(),
        tag: "1".into(),
    });
    let incognito = rendering::render(&m, &details("cypher"), WIDTH, 50).join("\n");
    assert!(
        !incognito.contains("leaked"),
        "an incognito player's name never appears, even if known"
    );
}

#[test]
fn accolades_draw_as_sixel_images_in_their_slots() {
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    let icons = Icons::new(Some(&picker));
    let buf = rendering::render_buffer(
        &demo::model("victory"),
        &UiState::default(),
        &icons,
        WIDTH,
        50,
    );
    let text = rendering::render(&demo::model("victory"), &UiState::default(), WIDTH, 50);
    let mut images = Vec::new();
    for y in 0..50u16 {
        for x in 0..WIDTH {
            if buf[(x, y)].symbol().contains("\u{1b}P") {
                images.push((x, y));
            }
        }
    }
    assert_eq!(
        images.len(),
        15,
        "Omen 4, me 4, Sova 2, Killjoy 1, Raze 2, Breach 1, Clove 1: {images:?}"
    );
    let header = text.iter().find(|l| l.contains("ACCOLADES")).unwrap();
    let header_x =
        u16::try_from(header[..header.find("ACCOLADES").unwrap()].chars().count()).unwrap();
    let me_y = u16::try_from(text.iter().position(|l| l.contains("Alpha")).unwrap()).unwrap();
    let mine: Vec<u16> = images
        .iter()
        .filter(|(_, y)| *y == me_y)
        .map(|(x, _)| *x)
        .collect();
    assert_eq!(
        mine,
        [header_x, header_x + 2, header_x + 4, header_x + 6],
        "my four icons sit in order under the header"
    );
}

#[test]
fn legend_lists_every_icon() {
    let ui = UiState {
        legend: true,
        ..Default::default()
    };
    let s = rendering::render(&demo::model("victory"), &ui, WIDTH, 50).join("\n");
    for k in Accolade::ICONS {
        assert!(s.contains(k.name()), "{k:?}");
    }
}
