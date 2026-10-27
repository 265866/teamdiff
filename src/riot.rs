//! HTTP access to the Riot Client's local API and Riot's game servers.
//!
//! Every call here is a read the game client itself makes: the lobby and Riot IDs. The only PUT is
//! the name lookup, which takes a list of player ids and changes nothing.

use crate::local_files::{GameLogInfo, Lockfile, game_log_path, lockfile_path, parse_game_log, parse_lockfile};
use crate::presence::{Presence, parse_presences};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Base64 of the platform JSON the Windows client sends.
fn client_platform() -> String {
    let json = r#"{"platformType":"PC","platformOS":"Windows","platformOSVersion":"10.0.19042.1.256.64bit","platformChipset":"Unknown"}"#;
    base64::engine::general_purpose::STANDARD.encode(json)
}

#[derive(Debug)]
struct Tokens {
    access: String,
    entitlement: String,
}

pub struct Riot {
    local: reqwest::Client,
    remote: reqwest::Client,
    lock: Lockfile,
    pub log: GameLogInfo,
    pub puuid: String,
    tokens: Tokens,
    permits: Semaphore,
}

/// Connects to the running Riot Client. Fails when the client isn't running or isn't signed in.
pub async fn connect() -> Result<Riot> {
    let lock_path = lockfile_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let lock = parse_lockfile(&std::fs::read_to_string(&lock_path).context("Riot Client is not running")?)?;
    let log_path = game_log_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let log = parse_game_log(&std::fs::read_to_string(&log_path).context("VALORANT has never been started on this PC")?)?;
    // The local API serves a self-signed certificate on 127.0.0.1; only that client skips verification.
    let local = reqwest::Client::builder().tls_danger_accept_invalid_certs(true).timeout(Duration::from_secs(5)).build()?;
    let remote = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let (tokens, puuid) = fetch_tokens(&local, &lock).await?;
    Ok(Riot { local, remote, lock, log, puuid, tokens, permits: Semaphore::new(4) })
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

async fn fetch_tokens(client: &reqwest::Client, lock: &Lockfile) -> Result<(Tokens, String)> {
    let (status, v) = local_get(client, lock, "/entitlements/v1/token").await?;
    if !status.is_success() {
        bail!("Riot Client is running but not signed in");
    }
    let s = |k: &str| v[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("token response missing {k}"));
    Ok((Tokens { access: s("accessToken")?, entitlement: s("token")? }, s("subject")?))
}

impl Riot {
    pub fn pd(&self) -> String {
        format!("https://pd.{}.a.pvp.net", self.log.shard)
    }

    pub async fn presences(&self) -> Result<Vec<Presence>> {
        let (status, v) = local_get(&self.local, &self.lock, "/chat/v4/presences").await?;
        if !status.is_success() {
            bail!("presences returned {status}");
        }
        Ok(parse_presences(&v))
    }

    /// A request to the game's player data server. `Ok(None)` means 404.
    pub async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Option<Value>> {
        let _permit = self.permits.acquire().await?;
        let mut req = self
            .remote
            .request(method, format!("{}{path}", self.pd()))
            .bearer_auth(&self.tokens.access)
            .header("X-Riot-Entitlements-JWT", &self.tokens.entitlement)
            .header("X-Riot-ClientVersion", &self.log.client_version)
            .header("X-Riot-ClientPlatform", client_platform());
        if let Some(b) = body {
            req = req.json(b);
        }
        // reqwest's error holds the URL, which can hold player ids; the context names the path.
        let resp = req.send().await.map_err(reqwest::Error::without_url).with_context(|| format!("request failed for {path}"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("{status} from {path}");
        }
        Ok(Some(resp.json().await.unwrap_or(Value::Null)))
    }

    // ---- endpoints ----

    /// Riot ids for the given players, as (puuid, name, tag).
    pub async fn names(&self, puuids: &[String]) -> Result<Vec<(String, String, String)>> {
        let body = Value::from(puuids.to_vec());
        let v = self.send(Method::PUT, "/name-service/v2/players", Some(&body)).await?;
        Ok(v.and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| {
                Some((
                    n["Subject"].as_str()?.to_lowercase(),
                    n["GameName"].as_str()?.to_string(),
                    n["TagLine"].as_str().unwrap_or("").to_string(),
                ))
            })
            .collect())
    }
}
