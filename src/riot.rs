//! HTTP access to the Riot Client's local API.

use crate::local_files::{Lockfile, lockfile_path, parse_lockfile};
use crate::presence::{Presence, parse_presences};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde_json::Value;
use std::time::Duration;

pub struct Riot {
    local: reqwest::Client,
    lock: Lockfile,
    pub puuid: String,
}

/// Connects to the running Riot Client. Fails when the client isn't running or isn't signed in.
pub async fn connect() -> Result<Riot> {
    let lock_path = lockfile_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let lock = parse_lockfile(&std::fs::read_to_string(&lock_path).context("Riot Client is not running")?)?;
    // The local API serves a self-signed certificate on 127.0.0.1; only that client skips verification.
    let local = reqwest::Client::builder().tls_danger_accept_invalid_certs(true).timeout(Duration::from_secs(5)).build()?;
    let puuid = fetch_puuid(&local, &lock).await?;
    Ok(Riot { local, lock, puuid })
}

async fn local_get(client: &reqwest::Client, lock: &Lockfile, path: &str) -> Result<(StatusCode, Value)> {
    let resp = client
        .get(format!("https://127.0.0.1:{}{}", lock.port, path))
        .basic_auth("riot", Some(&lock.password))
        .send()
        .await
        .context("Riot Client is not responding")?;
    let status = resp.status();
    Ok((status, resp.json().await.unwrap_or(Value::Null)))
}

/// Your player id, from the signed-in client's entitlement token.
async fn fetch_puuid(client: &reqwest::Client, lock: &Lockfile) -> Result<String> {
    let (status, v) = local_get(client, lock, "/entitlements/v1/token").await?;
    if !status.is_success() {
        bail!("Riot Client is running but not signed in");
    }
    v["subject"].as_str().map(str::to_string).ok_or_else(|| anyhow!("token response missing subject"))
}

impl Riot {
    pub async fn presences(&self) -> Result<Vec<Presence>> {
        let (status, v) = local_get(&self.local, &self.lock, "/chat/v4/presences").await?;
        if !status.is_success() {
            bail!("presences returned {status}");
        }
        Ok(parse_presences(&v))
    }
}
