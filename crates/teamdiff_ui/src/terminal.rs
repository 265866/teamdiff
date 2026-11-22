//! The terminal loop: draws the model, and reads keys until you quit.

use std::time::Duration;
use std::{env, io};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui_image::picker::Picker;
use teamdiff_core::demo;
use teamdiff_core::model::{Model, Phase};
use teamdiff_core::shared::{self, Shared};

use crate::draw::{self, UiState, View};
use crate::icons::Icons;

/// Takes over the terminal and draws the shared model until you press `q` or Ctrl+C, then gives
/// the terminal back.
///
/// The arrow keys (or `k` and `j`) move the cursor over the players, Enter opens the selected
/// player's details, `?` shows or hides the accolade legend, and Esc closes the legend, else the
/// details. With `demo_screen`, the index of a screen in [`demo::SCREENS`], Tab replaces the model
/// with the next demo screen's. A change of phase closes the details and moves the cursor back to
/// the top. The screen redraws every 50 ms while it animates, and every 250 ms or on a key
/// otherwise.
///
/// # Errors
///
/// Fails when drawing to the terminal or reading its events fails.
///
/// # Panics
///
/// When the terminal can't be set up, which [`ratatui::init()`] panics on, and when a thread
/// panicked while it held the model's lock, which [`shared::lock`] panics on.
pub fn run(shared: &Shared, demo_screen: Option<usize>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let picker = asks_for_images()
        .then(|| Picker::from_query_stdio().ok())
        .flatten();
    let icons = Icons::new(picker.as_ref());
    let result = read_keys(&mut terminal, shared, demo_screen, &icons);
    ratatui::restore();
    result
}

/// Whether the terminal is one that answers the graphics query. One that doesn't would stall
/// startup for two seconds and swallow the next keypress, so every other terminal keeps the text
/// labels.
fn asks_for_images() -> bool {
    env::var_os("WT_SESSION").is_some()
        || env::var("TERM_PROGRAM").is_ok_and(|t| t == "WezTerm" || t == "vscode")
}

/// Closes a details view from the last phase and moves the cursor back to the top when the phase
/// has changed: a new phase shows other players.
fn reset_on_new_phase(ui: &mut UiState, m: &mut Model, last_phase: &mut Option<Phase>) {
    if last_phase.as_ref() != Some(&m.phase) {
        ui.cursor = 0;
        ui.view = View::Main;
        m.viewing = None;
        *last_phase = Some(m.phase.clone());
    }
}

/// Whether the terminal loop goes on after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// Applies one key event to the UI state, the shared model and the demo screen. Only presses
/// count, not releases or repeats.
fn handle_key(
    key: KeyEvent,
    ui: &mut UiState,
    shared: &Shared,
    demo_screen: &mut Option<usize>,
) -> Flow {
    if key.kind != KeyEventKind::Press {
        return Flow::Continue;
    }
    let players = draw::selectable(&shared::lock(shared));
    match key.code {
        KeyCode::Char('q') => return Flow::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Flow::Quit,
        KeyCode::Char('?') => ui.legend = !ui.legend,
        KeyCode::Esc if ui.legend => ui.legend = false,
        KeyCode::Esc => {
            ui.view = View::Main;
            shared::lock(shared).viewing = None;
        }
        KeyCode::Up | KeyCode::Char('k') => ui.cursor = ui.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            ui.cursor = (ui.cursor + 1).min(players.len().saturating_sub(1));
        }
        KeyCode::Enter => {
            if let Some(p) = players.get(ui.cursor.min(players.len().saturating_sub(1))) {
                ui.view = View::Detail(p.clone());
                shared::lock(shared).viewing = Some(p.clone());
            }
        }
        KeyCode::Tab if demo_screen.is_some() => {
            let next = (demo_screen.unwrap_or(0) + 1) % demo::SCREENS.len();
            *demo_screen = Some(next);
            *shared::lock(shared) = demo::model(demo::SCREENS[next]);
            *ui = UiState::default();
        }
        _ => {}
    }
    Flow::Continue
}

fn read_keys(
    terminal: &mut DefaultTerminal,
    shared: &Shared,
    mut demo_screen: Option<usize>,
    icons: &Icons,
) -> io::Result<()> {
    let mut ui = UiState::default();
    let mut last_phase = None;
    loop {
        let animating = {
            let mut m = shared::lock(shared);
            reset_on_new_phase(&mut ui, &mut m, &mut last_phase);
            terminal.draw(|f| draw::draw(f, &m, &ui, icons))?;
            draw::animating(&m)
        };
        let wait = Duration::from_millis(if animating { 50 } else { 250 });
        if !event::poll(wait)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if handle_key(key, &mut ui, shared, &mut demo_screen) == Flow::Quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shared_demo(screen: &str) -> Shared {
        Arc::new(Mutex::new(demo::model(screen)))
    }

    #[test]
    fn q_and_ctrl_c_quit_and_nothing_else_does() {
        let shared = shared_demo("in-game");
        let mut ui = UiState::default();
        let mut screen = None;
        let mut flow = |key| handle_key(key, &mut ui, &shared, &mut screen);
        assert_eq!(flow(press(KeyCode::Char('q'))), Flow::Quit);
        assert_eq!(
            flow(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Flow::Quit
        );
        assert_eq!(flow(press(KeyCode::Char('c'))), Flow::Continue);
        assert_eq!(
            flow(KeyEvent::new_with_kind(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            Flow::Continue,
            "only presses count"
        );
    }

    #[test]
    fn the_cursor_stays_on_the_players() {
        let shared = shared_demo("in-game");
        let players = draw::selectable(&shared::lock(&shared)).len();
        let mut ui = UiState::default();
        let mut screen = None;
        handle_key(press(KeyCode::Up), &mut ui, &shared, &mut screen);
        assert_eq!(ui.cursor, 0, "no row above the first");
        for _ in 0..players + 3 {
            handle_key(press(KeyCode::Char('j')), &mut ui, &shared, &mut screen);
        }
        assert_eq!(ui.cursor, players - 1, "no row below the last");
        handle_key(press(KeyCode::Char('k')), &mut ui, &shared, &mut screen);
        handle_key(press(KeyCode::Down), &mut ui, &shared, &mut screen);
        handle_key(press(KeyCode::Up), &mut ui, &shared, &mut screen);
        assert_eq!(ui.cursor, players - 2);
    }

    #[test]
    fn enter_opens_the_selected_players_details_and_esc_closes_the_legend_first() {
        let shared = shared_demo("in-game");
        let mut ui = UiState::default();
        let mut screen = None;
        handle_key(press(KeyCode::Down), &mut ui, &shared, &mut screen);
        handle_key(press(KeyCode::Enter), &mut ui, &shared, &mut screen);
        let second = draw::selectable(&shared::lock(&shared))[1].clone();
        assert_eq!(ui.view, View::Detail(second.clone()));
        assert_eq!(
            shared::lock(&shared).viewing,
            Some(second),
            "the controller loads older acts for the player whose details are open"
        );
        handle_key(press(KeyCode::Char('?')), &mut ui, &shared, &mut screen);
        assert!(ui.legend);
        handle_key(press(KeyCode::Esc), &mut ui, &shared, &mut screen);
        assert!(!ui.legend);
        assert_ne!(ui.view, View::Main, "the first Esc only closes the legend");
        handle_key(press(KeyCode::Esc), &mut ui, &shared, &mut screen);
        assert_eq!(ui.view, View::Main);
        assert_eq!(shared::lock(&shared).viewing, None);
        handle_key(press(KeyCode::Char('?')), &mut ui, &shared, &mut screen);
        handle_key(press(KeyCode::Char('?')), &mut ui, &shared, &mut screen);
        assert!(!ui.legend, "? toggles the legend");
    }

    #[test]
    fn enter_without_players_opens_nothing() {
        let shared = shared_demo("waiting");
        let mut ui = UiState::default();
        let mut screen = None;
        handle_key(press(KeyCode::Enter), &mut ui, &shared, &mut screen);
        assert_eq!(ui.view, View::Main);
        assert_eq!(shared::lock(&shared).viewing, None);
    }

    #[test]
    fn tab_cycles_the_demo_screens_only_in_the_demo() {
        let shared = shared_demo("in-game");
        let mut ui = UiState::default();
        let mut screen = None;
        handle_key(press(KeyCode::Tab), &mut ui, &shared, &mut screen);
        assert_eq!(screen, None);
        assert_eq!(
            shared::lock(&shared).phase,
            Phase::Ingame,
            "no demo, no Tab"
        );

        let last = demo::SCREENS.len() - 1;
        let mut screen = Some(last - 1);
        ui.cursor = 3;
        ui.legend = true;
        handle_key(press(KeyCode::Tab), &mut ui, &shared, &mut screen);
        assert_eq!(screen, Some(last));
        assert_eq!(
            shared::lock(&shared).phase,
            demo::model(demo::SCREENS[last]).phase
        );
        assert_eq!(
            (ui.cursor, ui.legend, &ui.view),
            (0, false, &View::Main),
            "a new screen starts afresh"
        );
        handle_key(press(KeyCode::Tab), &mut ui, &shared, &mut screen);
        assert_eq!(screen, Some(0), "after the last screen comes the first");
        assert_eq!(
            shared::lock(&shared).phase,
            demo::model(demo::SCREENS[0]).phase
        );
    }

    #[test]
    fn a_new_phase_closes_the_details_and_moves_the_cursor_to_the_top() {
        let mut m = demo::model("in-game");
        let mut ui = UiState {
            view: View::Detail("raze".into()),
            cursor: 4,
            legend: true,
        };
        let mut last_phase = Some(Phase::Ingame);
        m.viewing = Some("raze".into());
        reset_on_new_phase(&mut ui, &mut m, &mut last_phase);
        assert_eq!(
            (ui.cursor, &ui.view),
            (4, &View::Detail("raze".into())),
            "same phase"
        );
        m.phase = Phase::Postgame;
        reset_on_new_phase(&mut ui, &mut m, &mut last_phase);
        assert_eq!((ui.cursor, &ui.view, &m.viewing), (0, &View::Main, &None));
        assert!(ui.legend, "the legend stays open");
        assert_eq!(last_phase, Some(Phase::Postgame));
    }
}
