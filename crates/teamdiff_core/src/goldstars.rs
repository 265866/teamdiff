//! Accolades, which Riot calls "goldstars".
//!
//! `/goldstars/v1/players/{puuid}` uses obfuscated keys that have changed between versions:
//! `tempS` act summaries (`tempA` per accolade with `tempC` count and `tempB` best), and `tempM` recent
//! matches (`tempG` start time, `tempP` per player, `tempA` per accolade with `tempV` value and `tempB`
//! act-record flag). The all-zero act id is the all-time summary.

use std::collections::HashMap;

use serde_json::Value;

use crate::cast;

/// A kind of accolade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[expect(
    clippy::exhaustive_enums,
    reason = "teamdiff_ui matches every kind to its icon, so a new kind has to fail its build \
              until the kind has an icon"
)]
pub enum Accolade {
    /// The match's MVP.
    Mvp,
    /// Riot calls it "Distinction".
    Masterful,
    /// The most kills in the match.
    TopFrag,
    /// Kills.
    Kills,
    /// First bloods.
    FirstBlood,
    /// Damage.
    Damage,
    /// Headshot percentage.
    Headshots,
    /// Assists.
    Assists,
    /// Spike plants.
    Plants,
    /// Aces.
    Aces,
    /// Clutches.
    Clutches,
    /// Trades.
    Trades,
}

impl Accolade {
    /// Display order for icons. MVP is shown as its own label, not an icon.
    pub const ICONS: [Accolade; 11] = [
        Self::Masterful,
        Self::TopFrag,
        Self::Kills,
        Self::FirstBlood,
        Self::Damage,
        Self::Headshots,
        Self::Assists,
        Self::Plants,
        Self::Aces,
        Self::Clutches,
        Self::Trades,
    ];
    /// Every kind, in display order.
    pub const ALL: [Accolade; 12] = [
        Self::Mvp,
        Self::Masterful,
        Self::TopFrag,
        Self::Kills,
        Self::FirstBlood,
        Self::Damage,
        Self::Headshots,
        Self::Assists,
        Self::Plants,
        Self::Aces,
        Self::Clutches,
        Self::Trades,
    ];

    /// The names the game shows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mvp => "MVP",
            Self::Masterful => "Masterful",
            Self::TopFrag => "Top Frag",
            Self::Kills => "Kills",
            Self::FirstBlood => "First Bloods",
            Self::Damage => "Damage",
            Self::Headshots => "Headshots",
            Self::Assists => "Assists",
            Self::Plants => "Spike Plants",
            Self::Aces => "Aces",
            Self::Clutches => "Clutches",
            Self::Trades => "Trades",
        }
    }

    /// Two-letter fallback for terminals without image support.
    #[must_use]
    pub fn abbrev(self) -> &'static str {
        match self {
            Self::Mvp => "MV",
            Self::Masterful => "MF",
            Self::TopFrag => "TF",
            Self::Kills => "KL",
            Self::FirstBlood => "FB",
            Self::Damage => "DM",
            Self::Headshots => "HS",
            Self::Assists => "AS",
            Self::Plants => "PL",
            Self::Aces => "AC",
            Self::Clutches => "CL",
            Self::Trades => "TR",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Some(match id.to_lowercase().as_str() {
            "745b27f0-4bc2-13a4-4ee7-77be323155c2" => Self::Mvp,
            "4815a8a2-4649-9bfe-afd2-38ae9cc22898" => Self::Masterful,
            "1c926cba-48cb-8aeb-c68d-a1ba2d012784" => Self::TopFrag,
            "1b13755f-4d5a-2c9e-6a39-bea7e6c53e7f" => Self::Kills,
            "bfe96c47-44d0-e473-585d-749146d2d05e" => Self::FirstBlood,
            "6dc31cfd-41da-895f-9246-a8b63558cdb8" => Self::Damage,
            "2c8b6129-4384-230d-c7e5-cda12019533c" => Self::Headshots,
            "e43f9acb-448c-4aa1-1591-6db72c0b8dae" => Self::Assists,
            "352a3ac8-4b2c-db6a-1f36-c0a67b428e65" => Self::Plants,
            "244cf4ab-4c27-cc59-3323-c985858d6ddb" => Self::Aces,
            "0497d585-42ad-61a7-42bc-189571f40e2c" => Self::Clutches,
            "3bc0563a-4fe9-a15c-0078-23a3408d64b5" => Self::Trades,
            _ => return None,
        })
    }

    /// Internal names from `/goldstars/v1/goldstars`, for ids that aren't in the table above.
    fn from_dev_name(name: &str) -> Option<Self> {
        Some(match name {
            "MVP" => Self::Mvp,
            "Distinction" => Self::Masterful,
            "TopFrag" => Self::TopFrag,
            "Kills" => Self::Kills,
            "FirstBlood" | "FirstBloods" => Self::FirstBlood,
            "Damage" => Self::Damage,
            "HeadshotPercentage" | "HeadshotRate" => Self::Headshots,
            "Assists" => Self::Assists,
            "Plants" | "SpikePlants" => Self::Plants,
            "Aces" => Self::Aces,
            "Clutches" => Self::Clutches,
            "Trades" => Self::Trades,
            _ => return None,
        })
    }
}

/// Maps accolade ids to kinds, using the known ids plus any extra definitions Riot returns.
#[derive(Debug, Clone, Default)]
pub struct Definitions(HashMap<String, Accolade>);

impl Definitions {
    /// Reads `/goldstars/v1/goldstars`, keeping the definitions whose name is a known kind.
    #[must_use]
    pub fn from_response(v: &Value) -> Self {
        let mut map = HashMap::new();
        for d in v.as_array().into_iter().flatten() {
            let id = d["id"].as_str().unwrap_or("").to_lowercase();
            let name = ["tempT", "devName", "displayName", "name", "title"]
                .iter()
                .find_map(|k| d[*k].as_str());
            if let Some(kind) = name.and_then(Accolade::from_dev_name) {
                map.insert(id, kind);
            }
        }
        Self(map)
    }

    fn kind(&self, id: &str) -> Option<Accolade> {
        Accolade::from_id(id).or_else(|| self.0.get(&id.to_lowercase()).copied())
    }
}

/// How often a player earned one accolade in an act, and their best value for it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct Summary {
    /// How many times it was earned.
    pub count: u32,
    /// The best value it was earned with.
    pub best: f64,
}

/// One accolade earned in one match.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Earned {
    /// The value it was earned with; see [`format_value`].
    pub value: f64,
    /// Whether the value is the player's best this act.
    pub act_record: bool,
}

/// The accolades of one recent match.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct MatchAccolades {
    /// When the match started, in milliseconds since the Unix epoch.
    pub start_ms: i64,
    /// Lowercase PUUID -> accolade -> how it was earned.
    pub players: HashMap<String, HashMap<Accolade, Earned>>,
}

/// One player's accolades: their act summaries and recent matches.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct PlayerGoldstars {
    /// Act id (lowercase) -> accolade -> summary. The all-zero id is all-time.
    pub acts: HashMap<String, HashMap<Accolade, Summary>>,
    /// Recent matches, in the order Riot lists them.
    pub matches: Vec<MatchAccolades>,
}

/// The act id of the all-time summary.
pub const ALL_TIME: &str = "00000000-0000-0000-0000-000000000000";

/// Reads `/goldstars/v1/players/{puuid}`, skipping accolades of unknown kinds. A body of any
/// other shape reads as no accolades.
#[must_use]
pub fn parse_player(v: &Value, defs: &Definitions) -> PlayerGoldstars {
    let mut out = PlayerGoldstars::default();
    for (act, s) in v["tempS"].as_object().into_iter().flatten() {
        let mut m = HashMap::new();
        for (id, a) in s["tempA"].as_object().into_iter().flatten() {
            if let Some(kind) = defs.kind(a["id"].as_str().unwrap_or(id)) {
                m.insert(
                    kind,
                    Summary {
                        count: cast::low_u32(a["tempC"].as_u64().unwrap_or(0)),
                        best: a["tempB"].as_f64().unwrap_or(0.0),
                    },
                );
            }
        }
        out.acts.insert(act.to_lowercase(), m);
    }
    for g in v["tempM"].as_array().into_iter().flatten() {
        let mut players = HashMap::new();
        for (puuid, p) in g["tempP"].as_object().into_iter().flatten() {
            let mut earned = HashMap::new();
            for (id, a) in p["tempA"].as_object().into_iter().flatten() {
                if let Some(kind) = defs.kind(a["id"].as_str().unwrap_or(id)) {
                    earned.insert(
                        kind,
                        Earned {
                            value: a["tempV"].as_f64().unwrap_or(0.0),
                            act_record: a["tempB"].as_bool().unwrap_or(false),
                        },
                    );
                }
            }
            players.insert(puuid.to_lowercase(), earned);
        }
        out.matches.push(MatchAccolades {
            start_ms: g["tempG"].as_i64().unwrap_or(0),
            players,
        });
    }
    out
}

impl PlayerGoldstars {
    /// Recent-match entries carry no match id, so the match is found by start time, within 10 minutes.
    #[must_use]
    pub fn match_near(&self, start_ms: i64) -> Option<&MatchAccolades> {
        const WINDOW_MS: i64 = 10 * 60 * 1000;
        self.matches
            .iter()
            .filter(|m| (m.start_ms - start_ms).abs() <= WINDOW_MS)
            .min_by_key(|m| (m.start_ms - start_ms).abs())
    }
}

/// Accolade values are float32 and sometimes stored per round; display them rounded.
#[must_use]
pub fn format_value(kind: Accolade, v: f64) -> String {
    match kind {
        Accolade::Headshots if v <= 1.0 => format!("{}%", (v * 100.0).round()),
        Accolade::Headshots => format!("{}%", v.round()),
        _ if v.fract().abs() > 0.05 && v < 10.0 => format!("{v:.1}"),
        _ => format!("{}", v.round()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOP_FRAG: &str = "1c926cba-48cb-8aeb-c68d-a1ba2d012784";
    const FIRST_BLOOD: &str = "BFE96C47-44D0-E473-585D-749146D2D05E";

    fn body() -> Value {
        json!({
            "puuid": "me", "version": 32,
            "tempS": {
                "A6": { "tempS": "A6", "tempA": { TOP_FRAG: { "id": TOP_FRAG, "tempC": 11, "tempB": 31.0 } } },
                ALL_TIME: { "tempA": { FIRST_BLOOD: { "id": FIRST_BLOOD, "tempC": 10, "tempB": 7.0 } } }
            },
            "tempM": [
                { "id": "", "tempG": 1_000_000, "tempP": {
                    "ME": { "tempA": { FIRST_BLOOD: { "id": FIRST_BLOOD, "tempV": 5.000_001, "tempB": true } } },
                    "other": { "tempA": { "new-id": { "id": "new-id", "tempV": 3.0, "tempB": false } } }
                }},
                { "id": "", "tempG": 9_000_000, "tempP": {} }
            ]
        })
    }

    #[test]
    fn parses_summaries_and_matches() {
        let defs = Definitions::from_response(&json!([{ "id": "NEW-ID", "tempT": "Clutches" }]));
        let g = parse_player(&body(), &defs);
        assert_eq!(
            g.acts["a6"][&Accolade::TopFrag],
            Summary {
                count: 11,
                best: 31.0
            }
        );
        assert_eq!(g.acts[ALL_TIME][&Accolade::FirstBlood].count, 10);
        let m = g.match_near(1_000_000 + 4 * 60 * 1000).unwrap();
        let mine = &m.players["me"];
        assert!(mine[&Accolade::FirstBlood].act_record);
        assert_eq!(
            format_value(Accolade::FirstBlood, mine[&Accolade::FirstBlood].value),
            "5"
        );
        assert!(
            m.players["other"].contains_key(&Accolade::Clutches),
            "unknown id resolved through definitions"
        );
    }

    #[test]
    fn no_match_outside_window() {
        let g = parse_player(&body(), &Definitions::default());
        assert!(g.match_near(1_000_000 + 11 * 60 * 1000).is_none());
        assert_eq!(g.match_near(8_500_000).unwrap().start_ms, 9_000_000);
    }

    #[test]
    fn empty_and_malformed_bodies() {
        assert_eq!(
            parse_player(
                &json!({ "tempS": {}, "tempM": [] }),
                &Definitions::default()
            ),
            PlayerGoldstars::default()
        );
        assert_eq!(
            parse_player(&json!("nope"), &Definitions::default()),
            PlayerGoldstars::default()
        );
    }

    #[test]
    fn headshot_values_format_as_percent() {
        assert_eq!(format_value(Accolade::Headshots, 0.41), "41%");
        assert_eq!(format_value(Accolade::Headshots, 44.0), "44%");
        assert_eq!(format_value(Accolade::Kills, 1.25), "1.2");
        assert_eq!(format_value(Accolade::Damage, 211.4), "211");
    }

    #[test]
    fn every_kind_has_a_unique_abbreviation() {
        let mut seen: Vec<_> = Accolade::ALL.iter().map(|a| a.abbrev()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), Accolade::ALL.len());
    }
}
