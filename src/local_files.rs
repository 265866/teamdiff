//! Files the Riot Client and VALORANT leave on disk: the client lockfile and the game log.

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

/// The version of teamdiff that last ran, so the next one can tell it was updated.
pub fn last_version_path() -> Option<PathBuf> {
    local_app_data().map(|d| d.join("teamdiff").join("version"))
}

/// The tracker.gg browser's own profile, which keeps its cookies between runs.
pub fn browser_profile_dir() -> Option<PathBuf> {
    local_app_data().map(|d| d.join("teamdiff").join("WebView2"))
}

pub fn game_log_path() -> Option<PathBuf> {
    local_app_data().map(|d| d.join("VALORANT").join("Saved").join("Logs").join("ShooterGame.log"))
}

/// Region, shard, and client version, as the game last recorded them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameLogInfo {
    pub region: String,
    pub shard: String,
    pub client_version: String,
}

pub fn parse_game_log(text: &str) -> Result<GameLogInfo> {
    let client_version = text
        .lines()
        .find_map(|l| l.split("CI server version: ").nth(1))
        .map(|v| v.split_whitespace().next().unwrap_or("").to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("no client version in game log"))?;
    // e.g. https://glz-eu-1.eu.a.pvp.net
    let (region, shard) = text
        .split("https://glz-")
        .skip(1)
        .find_map(|rest| {
            let host = rest.split(['/', ' ', ']', '"']).next()?;
            let (region, tail) = host.split_once("-1.")?;
            let shard = tail.strip_suffix(".a.pvp.net")?;
            Some((region.to_string(), shard.to_string()))
        })
        .ok_or_else(|| anyhow!("no glz host in game log"))?;
    Ok(GameLogInfo { region, shard, client_version })
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

    #[test]
    fn parses_game_log() {
        let log = "[2026.01.01-00.00.00:000][  0]LogShooter: Display: CI server version: release-13.08-shipping-14-1234567\n\
                   [2026.01.01-00.00.03:000][ 46]LogPlatformCommon: Platform HTTP Query End. URL [GET https://glz-eu-1.eu.a.pvp.net/pregame/v1/players/x], ok\n";
        let info = parse_game_log(log).unwrap();
        assert_eq!(info.client_version, "release-13.08-shipping-14-1234567");
        assert_eq!(info.region, "eu");
        assert_eq!(info.shard, "eu");
    }

    #[test]
    fn latam_region_uses_na_shard() {
        let log = "CI server version: release-13.08-shipping-14-1\nURL [GET https://glz-latam-1.na.a.pvp.net/session]";
        let info = parse_game_log(log).unwrap();
        assert_eq!((info.region.as_str(), info.shard.as_str()), ("latam", "na"));
    }

    #[test]
    fn game_log_without_version_is_an_error() {
        assert!(parse_game_log("https://glz-eu-1.eu.a.pvp.net/x").is_err());
    }
}
