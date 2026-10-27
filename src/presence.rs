//! Decoding of chat presences from the local API.
//!
//! VALORANT puts its state in a base64 JSON `private` field. Riot has switched that JSON between a flat
//! layout (`sessionLoopState`, `partyId`, ...) and a nested one (`matchPresenceData.sessionLoopState`,
//! `partyPresenceData.partyId`, ...), so every field is looked up by key at any depth.

use base64::Engine;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopState {
    Menus,
    Pregame,
    Ingame,
    Other(String),
}

impl LoopState {
    fn parse(s: &str) -> Self {
        match s {
            "MENUS" => Self::Menus,
            "PREGAME" => Self::Pregame,
            "INGAME" => Self::Ingame,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presence {
    pub puuid: String,
    pub state: Option<LoopState>,
    pub party_id: Option<String>,
    pub queue_id: Option<String>,
    /// (ally, enemy) rounds won, from the party owner's point of view.
    pub score: Option<(u32, u32)>,
    pub queue_entry_time: Option<String>,
    /// e.g. "MATCHMAKING" while queueing.
    pub party_state: Option<String>,
    pub account_level: Option<u32>,
}

/// Parses the `/chat/v4/presences` body, keeping only VALORANT presences with a readable private field.
pub fn parse_presences(body: &Value) -> Vec<Presence> {
    body["presences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["product"].as_str() == Some("valorant"))
        .filter_map(|p| decode(p["puuid"].as_str()?, p["private"].as_str()?))
        .collect()
}

fn decode(puuid: &str, private: &str) -> Option<Presence> {
    if private.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(private).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    if find(&v, "isValid").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let str_of = |k| find(&v, k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let num_of = |k| find(&v, k).and_then(Value::as_u64).map(|n| n as u32);
    let score = match (num_of("partyOwnerMatchScoreAllyTeam"), num_of("partyOwnerMatchScoreEnemyTeam")) {
        (Some(a), Some(e)) => Some((a, e)),
        _ => None,
    };
    Some(Presence {
        puuid: puuid.to_string(),
        state: str_of("sessionLoopState").map(|s| LoopState::parse(&s)),
        party_id: str_of("partyId"),
        queue_id: str_of("queueId"),
        score,
        queue_entry_time: str_of("queueEntryTime"),
        party_state: str_of("partyState"),
        account_level: num_of("accountLevel"),
    })
}

/// Depth-first lookup of the first value stored under `key`.
fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|c| find(c, key))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wrap(puuid: &str, private: &Value) -> Value {
        let enc = base64::engine::general_purpose::STANDARD.encode(private.to_string());
        json!({ "puuid": puuid, "product": "valorant", "private": enc })
    }

    #[test]
    fn reads_flat_layout() {
        let body = json!({ "presences": [wrap("me", &json!({
            "isValid": true, "sessionLoopState": "INGAME", "partyId": "p1", "partySize": 2, "accountLevel": 214, "partyState": "DEFAULT",
            "queueId": "competitive", "partyOwnerMatchScoreAllyTeam": 5, "partyOwnerMatchScoreEnemyTeam": 3
        }))]});
        let p = &parse_presences(&body)[0];
        assert_eq!(p.state, Some(LoopState::Ingame));
        assert_eq!(p.party_id.as_deref(), Some("p1"));
        assert_eq!(p.queue_id.as_deref(), Some("competitive"));
        assert_eq!(p.score, Some((5, 3)));
        assert_eq!(p.account_level, Some(214));
        assert_eq!(p.party_state.as_deref(), Some("DEFAULT"));
    }

    #[test]
    fn reads_nested_layout() {
        let body = json!({ "presences": [wrap("me", &json!({
            "isValid": true,
            "matchPresenceData": { "sessionLoopState": "PREGAME", "queueId": "competitive" },
            "partyPresenceData": { "partyId": "p9", "partySize": 3 }
        }))]});
        let p = &parse_presences(&body)[0];
        assert_eq!(p.state, Some(LoopState::Pregame));
        assert_eq!(p.party_id.as_deref(), Some("p9"));
        assert_eq!(p.score, None);
    }

    #[test]
    fn skips_other_products_and_empty_private() {
        let body = json!({ "presences": [
            { "puuid": "a", "product": "league_of_legends", "private": "e30=" },
            { "puuid": "b", "product": "valorant", "private": "" },
            { "puuid": "c", "product": "valorant", "private": null },
            { "puuid": "d", "product": "valorant", "private": "not base64!" },
            wrap("e", &json!({ "isValid": false, "sessionLoopState": "MENUS" })),
        ]});
        assert!(parse_presences(&body).is_empty());
    }

    #[test]
    fn unknown_loop_state_is_kept() {
        let body = json!({ "presences": [wrap("me", &json!({ "sessionLoopState": "REPLAY" }))]});
        assert_eq!(parse_presences(&body)[0].state, Some(LoopState::Other("REPLAY".into())));
    }

    #[test]
    fn malformed_body_yields_nothing() {
        assert!(parse_presences(&json!({ "nope": 1 })).is_empty());
    }
}
