//! Static game content (agent, map, rank and act names) from valorant-api.com.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Act {
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct Content {
    agents: HashMap<String, String>,
    /// Keyed by lowercase map UUID and by map URL (`/Game/Maps/...`).
    maps: HashMap<String, String>,
    tiers: HashMap<u32, String>,
    /// Competitive acts, oldest first.
    pub acts: Vec<Act>,
}

impl Content {
    pub fn from_responses(agents: &Value, maps: &Value, tiers: &Value, seasons: &Value) -> Self {
        let data = |v: &Value| v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut c = Content::default();
        for a in data(agents) {
            if let (Some(id), Some(name)) = (a["uuid"].as_str(), a["displayName"].as_str()) {
                c.agents.insert(id.to_lowercase(), name.to_string());
            }
        }
        for m in data(maps) {
            if let Some(name) = m["displayName"].as_str() {
                if let Some(id) = m["uuid"].as_str() {
                    c.maps.insert(id.to_lowercase(), name.to_string());
                }
                if let Some(url) = m["mapUrl"].as_str() {
                    c.maps.insert(url.to_string(), name.to_string());
                }
            }
        }
        if let Some(latest) = data(tiers).last() {
            for t in latest["tiers"].as_array().into_iter().flatten() {
                if let (Some(n), Some(name)) = (t["tier"].as_u64(), t["tierName"].as_str()) {
                    c.tiers.insert(n as u32, title_case(name));
                }
            }
        }
        let mut acts: Vec<Act> = data(seasons)
            .iter()
            .filter(|s| s["type"].as_str() == Some("EAresSeasonType::Act"))
            .filter_map(|s| {
                Some(Act {
                    id: s["uuid"].as_str()?.to_lowercase(),
                    start: s["startTime"].as_str()?.parse().ok()?,
                    end: s["endTime"].as_str()?.parse().ok()?,
                })
            })
            .collect();
        acts.sort_by_key(|a| a.start);
        c.acts = acts;
        c
    }

    pub fn agent(&self, id: &str) -> String {
        self.agents.get(&id.to_lowercase()).cloned().unwrap_or_else(|| "?".into())
    }

    pub fn map(&self, id_or_url: &str) -> String {
        self.maps
            .get(id_or_url)
            .or_else(|| self.maps.get(&id_or_url.to_lowercase()))
            .cloned()
            .unwrap_or_else(|| id_or_url.rsplit('/').next().unwrap_or(id_or_url).to_string())
    }

    pub fn tier_name(&self, tier: u32) -> String {
        self.tiers.get(&tier).cloned().unwrap_or_else(|| if tier == 0 { "Unranked".into() } else { format!("Tier {tier}") })
    }

    pub fn current_act(&self, now: DateTime<Utc>) -> Option<&Act> {
        self.acts.iter().find(|a| a.start <= now && now < a.end)
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Content {
        Content::from_responses(
            &json!({ "data": [{ "uuid": "ADD6443A-41BD-E414-F6AD-E58D267F4E95", "displayName": "Jett" }] }),
            &json!({ "data": [{ "uuid": "7EAECC1B-4337-BBF6-6AB9-04B8F06B3319", "displayName": "Ascent", "mapUrl": "/Game/Maps/Ascent/Ascent" }] }),
            &json!({ "data": [
                { "tiers": [{ "tier": 21, "tierName": "IMMORTAL 1" }] },
                { "tiers": [{ "tier": 0, "tierName": "UNRANKED" }, { "tier": 14, "tierName": "PLATINUM 2" }, { "tier": 21, "tierName": "ASCENDANT 1" }, { "tier": 24, "tierName": "IMMORTAL 1" }] }
            ]}),
            &json!({ "data": [
                { "uuid": "old", "title": "EPISODE 4 // ACT III", "type": "EAresSeasonType::Act", "startTime": "2022-04-27T00:00:00Z", "endTime": "2022-06-22T00:00:00Z" },
                { "uuid": "ep", "title": "V26", "type": "EAresSeasonType::Episode", "startTime": "2026-06-24T00:00:00Z", "endTime": "2027-01-06T00:00:00Z" },
                { "uuid": "a5", "title": "V26 // ACT V", "type": "EAresSeasonType::Act", "startTime": "2026-08-19T00:00:00Z", "endTime": "2026-10-14T00:00:00Z" },
                { "uuid": "a6", "title": "V26 // ACT VI", "type": "EAresSeasonType::Act", "startTime": "2026-10-14T00:00:00Z", "endTime": "2027-01-06T00:00:00Z" }
            ]}),
        )
    }

    #[test]
    fn names_resolve_case_insensitively() {
        let c = sample();
        assert_eq!(c.agent("add6443a-41bd-e414-f6ad-e58d267f4e95"), "Jett");
        assert_eq!(c.map("/Game/Maps/Ascent/Ascent"), "Ascent");
        assert_eq!(c.map("7eaecc1b-4337-bbf6-6ab9-04b8f06b3319"), "Ascent");
        assert_eq!(c.map("/Game/Maps/Unknown/Mystery"), "Mystery");
    }

    #[test]
    fn uses_latest_tier_table() {
        let c = sample();
        assert_eq!(c.tier_name(21), "Ascendant 1");
        assert_eq!(c.tier_name(14), "Platinum 2");
        assert_eq!(c.tier_name(0), "Unranked");
    }

    #[test]
    fn acts_are_ordered() {
        let c = sample();
        let ids: Vec<_> = c.acts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["old", "a5", "a6"]);
        let now: DateTime<Utc> = "2026-10-17T00:00:00Z".parse().unwrap();
        assert_eq!(c.current_act(now).unwrap().id, "a6");
    }
}
