//! Embeds the dependencies' license notices, which `teamdiff --licenses` prints. The release build writes them with cargo-about
//! first and names the file in TEAMDIFF_LICENSES (see .github/workflows/build-release.yml); other builds embed a note instead.

use std::path::PathBuf;

const NOT_GENERATED: &str = "This build of teamdiff didn't include its dependencies' license notices. To write them, install
cargo-about and run `cargo about generate about.hbs` in teamdiff's repository.
";

fn main() {
    println!("cargo::rerun-if-env-changed=TEAMDIFF_LICENSES");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("THIRD-PARTY-LICENSES.txt");
    match std::env::var_os("TEAMDIFF_LICENSES") {
        Some(notices) => {
            println!("cargo::rerun-if-changed={}", PathBuf::from(&notices).display());
            std::fs::copy(&notices, &out).expect("TEAMDIFF_LICENSES names a readable file");
        }
        None => std::fs::write(&out, NOT_GENERATED).expect("OUT_DIR is writable"),
    }
}
