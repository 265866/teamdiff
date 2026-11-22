//! Writes the third-party license notices for `teamdiff --licenses` to `OUT_DIR`. When
//! `TEAMDIFF_LICENSES` holds the absolute path of the HTML notices `just notices` writes, as the
//! release build sets it, this turns them into the plain text earlier releases printed. Other
//! builds get a note instead.

use std::path::PathBuf;
use std::{env, fs, io};

use anyhow::bail;
use teamdiff_core::notices;

const NOT_GENERATED: &str = "This build of teamdiff didn't include its dependencies' license notices. To write them, install
cargo-about, run `just notices` in teamdiff's repository, and build with TEAMDIFF_LICENSES set to
the absolute path of target/THIRD-PARTY-LICENSES.html.
";

fn main() -> anyhow::Result<()> {
    use std::io::Write as _;

    use anyhow::Context as _;

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "cargo::rerun-if-env-changed=TEAMDIFF_LICENSES")?;
    let out = PathBuf::from(env::var_os("OUT_DIR").context("cargo sets OUT_DIR")?)
        .join("THIRD-PARTY-LICENSES.txt");
    let Some(named) = env::var_os("TEAMDIFF_LICENSES") else {
        fs::write(&out, NOT_GENERATED).with_context(|| format!("writing {}", out.display()))?;
        return Ok(());
    };
    if named.is_empty() {
        bail!("TEAMDIFF_LICENSES is set but empty; unset it, or name the HTML license notices");
    }
    let path = PathBuf::from(named);
    writeln!(stdout, "cargo::rerun-if-changed={}", path.display())?;
    let html = fs::read_to_string(&path)
        .with_context(|| format!("reading TEAMDIFF_LICENSES={}", path.display()))?;
    let text = notices::text_from_html(&html)
        .with_context(|| format!("converting TEAMDIFF_LICENSES={}", path.display()))?;
    fs::write(&out, text).with_context(|| format!("writing {}", out.display()))
}
