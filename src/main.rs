const HELP: &str = "teamdiff: a terminal tracker for your VALORANT matches

USAGE:
    teamdiff

OPTIONS:
    -h, --help     Show this help

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' \"Legal Jibber Jabber\" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project.";

fn main() {
    match std::env::args().nth(1).as_deref() {
        None | Some("-h" | "--help") => println!("{HELP}"),
        Some(other) => {
            eprintln!("unknown argument {other:?}\n\n{HELP}");
            std::process::exit(2);
        }
    }
}
