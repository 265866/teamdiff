//! Embeds the third-party license notices `teamdiff --licenses` prints. The release build names
//! the HTML notices `just notices` writes in `TEAMDIFF_LICENSES`, and this turns them into the
//! plain text earlier releases printed. Other builds embed a note instead.

use std::path::PathBuf;
use std::{env, fs, io};

use teamdiff_core::notices;

const NOT_GENERATED: &str = "This build of teamdiff didn't include its dependencies' license notices. To write them, install
cargo-about, run `just notices` in teamdiff's repository, and build with TEAMDIFF_LICENSES naming
target/THIRD-PARTY-LICENSES.html.
";

fn main() -> io::Result<()> {
    use std::io::Write as _;

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "cargo::rerun-if-env-changed=TEAMDIFF_LICENSES")?;
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("THIRD-PARTY-LICENSES.txt");
    match env::var_os("TEAMDIFF_LICENSES") {
        Some(named) => {
            let path = PathBuf::from(named);
            writeln!(stdout, "cargo::rerun-if-changed={}", path.display())?;
            let html = fs::read_to_string(&path)?;
            let text = notices::text_from_html(&html)
                .map_err(|e| io::Error::other(format!("{}: {e:#}", path.display())))?;
            fs::write(out, text)
        }
        None => fs::write(out, NOT_GENERATED),
    }
}
