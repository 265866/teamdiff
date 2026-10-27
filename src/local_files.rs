//! The lockfile the Riot Client leaves on disk while it runs.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;

/// Connection details for the Riot Client's local HTTPS API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub port: u16,
    pub password: String,
}

/// Parses `name:pid:port:password:protocol`.
pub fn parse_lockfile(text: &str) -> Result<Lockfile> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 5 {
        return Err(anyhow!("lockfile has {} fields, expected 5", parts.len()));
    }
    let port = parts[2].parse().with_context(|| format!("bad lockfile port {:?}", parts[2]))?;
    Ok(Lockfile { port, password: parts[3].to_string() })
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

pub fn lockfile_path() -> Option<PathBuf> {
    local_app_data().map(|d| d.join("Riot Games").join("Riot Client").join("Config").join("lockfile"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lockfile() {
        let l = parse_lockfile("Riot Client:32076:54321:s3cret-pw:https\n").unwrap();
        assert_eq!(l, Lockfile { port: 54321, password: "s3cret-pw".into() });
    }

    #[test]
    fn rejects_malformed_lockfile() {
        assert!(parse_lockfile("Riot Client:32076:https").is_err());
        assert!(parse_lockfile("Riot Client:1:notaport:pw:https").is_err());
    }
}
