mod controller;
mod local_files;
mod model;
mod presence;
mod riot;
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

OPTIONS:
    --dump <dir>   Save every raw API response under <dir> (for checking field names)
    -h, --help     Show this help

KEYS:
    q        quit

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' \"Legal Jibber Jabber\" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project.";

struct Options {
    dump: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut options = Options { dump: None };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            // A flag after --dump is a flag, not a directory to fill with live responses.
            "--dump" => options.dump = Some(args.next_if(|a| !a.starts_with('-')).ok_or("--dump needs a directory")?.into()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Some(options))
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = match parse_args() {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{HELP}");
            return Ok(());
        }
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    let shared = Arc::new(Mutex::new(Model::new()));
    tokio::spawn(controller::run(shared.clone(), options.dump));
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
