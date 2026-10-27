mod controller;
mod local_files;
mod model;
mod presence;
mod riot;
mod ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use model::Model;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HELP: &str = "teamdiff: a terminal tracker for your VALORANT matches

USAGE:
    teamdiff

OPTIONS:
    -h, --help     Show this help

KEYS:
    q        quit

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' \"Legal Jibber Jabber\" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project.";

/// `Ok(false)` when the help was asked for.
fn parse_args() -> Result<bool, String> {
    match std::env::args().nth(1).as_deref() {
        None => Ok(true),
        Some("-h" | "--help") => Ok(false),
        Some(other) => Err(format!("unknown argument {other:?}")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match parse_args() {
        Ok(true) => {}
        Ok(false) => {
            println!("{HELP}");
            return Ok(());
        }
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    }
    let shared = Arc::new(Mutex::new(Model::new()));
    tokio::spawn(controller::run(shared.clone()));
    run_ui(shared)
}

fn run_ui(shared: controller::Shared) -> Result<()> {
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
                _ => {}
            }
        }
    })();
    ratatui::restore();
    result
}
