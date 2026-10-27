//! Static game content (agent and map names) from valorant-api.com.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Content {
    agents: HashMap<String, String>,
    /// Keyed by lowercase map UUID and by map URL (`/Game/Maps/...`).
    maps: HashMap<String, String>,
}

impl Content {
    pub fn from_responses(agents: &Value, maps: &Value) -> Self {
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
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Content {
        Content::from_responses(
            &json!({ "data": [{ "uuid": "ADD6443A-41BD-E414-F6AD-E58D267F4E95", "displayName": "Jett" }] }),
            &json!({ "data": [{ "uuid": "7EAECC1B-4337-BBF6-6AB9-04B8F06B3319", "displayName": "Ascent", "mapUrl": "/Game/Maps/Ascent/Ascent" }] }),
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
}
