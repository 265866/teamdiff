mod browser;
mod content;
mod controller;
mod demo;
mod local_files;
mod model;
mod presence;
mod riot;
mod tracker;
mod ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use model::Model;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HELP: &str = "teamdiff: a terminal tracker for your VALORANT matches

USAGE:
    teamdiff [--dump <dir>]
    teamdiff --demo [screen]

OPTIONS:
    --dump <dir>   Save raw API responses under <dir> (for checking field names)
    --demo         Show sample data instead of connecting; Tab cycles screens
                   (waiting, menus, agent-select, loading, in-game)
    -h, --help     Show this help

KEYS:
    q        quit

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' \"Legal Jibber Jabber\" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project.";

enum Mode {
    Live { dump: Option<PathBuf> },
    Demo { screen: usize },
}

fn parse_args() -> Result<Option<Mode>, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut mode = Mode::Live { dump: None };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--dump" => {
                // A flag after --dump is a flag, not a directory to fill with live responses.
                let dir = args.next_if(|a| !a.starts_with('-')).ok_or("--dump needs a directory")?;
                if matches!(mode, Mode::Demo { .. }) {
                    return Err("--demo makes no requests, so --dump has nothing to save".into());
                }
                mode = Mode::Live { dump: Some(dir.into()) };
            }
            "--demo" => {
                if matches!(mode, Mode::Live { dump: Some(_) }) {
                    return Err("--demo makes no requests, so --dump has nothing to save".into());
                }
                // The screen is optional, so a flag after --demo is a flag, not a screen.
                let screen = match args.next_if(|a| !a.starts_with('-')) {
                    Some(name) => demo::SCREENS.iter().position(|s| *s == name).ok_or_else(|| format!("unknown demo screen {name:?}"))?,
                    None => 0,
                };
                mode = Mode::Demo { screen };
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Some(mode))
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = match parse_args() {
        Ok(Some(m)) => m,
        Ok(None) => {
            println!("{HELP}");
            return Ok(());
        }
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    let shared = Arc::new(Mutex::new(match &mode {
        Mode::Demo { screen } => demo::model(demo::SCREENS[*screen]),
        Mode::Live { .. } => Model::new(Arc::default()),
    }));
    if let Mode::Live { dump } = &mode {
        tokio::spawn(controller::run(shared.clone(), dump.clone()));
    }
    let demo_screen = match mode {
        Mode::Demo { screen } => Some(screen),
        Mode::Live { .. } => None,
    };
    run_ui(shared, demo_screen)
}

fn run_ui(shared: controller::Shared, mut demo_screen: Option<usize>) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| ui::draw(f, &controller::lock(&shared)))?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let Event::Key(key) = event::read()? else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                KeyCode::Tab if demo_screen.is_some() => {
                    let next = (demo_screen.unwrap_or(0) + 1) % demo::SCREENS.len();
                    demo_screen = Some(next);
                    *controller::lock(&shared) = demo::model(demo::SCREENS[next]);
                }
                _ => {}
            }
        }
    })();
    ratatui::restore();
    result
}
