//! Player stats from tracker.gg: what its responses hold, and how to read them.
//!
//! tracker.gg has no public Valorant API. Its staff allowed this app to call the endpoints its site
//! uses, as a browser would, until they offer one. `teamdiff_live` makes the requests.

use std::collections::HashMap;

use serde_json::Value;

use crate::cast;
use crate::stats::{ActRecord, PlayerMatch, Record, RecordSource};

/// A player's competitive record and rank in one act.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActStats {
    /// The act, in lowercase.
    pub act_id: String,
    /// The rank the act ended with, or has so far; tier 0 is unranked.
    pub tier: u32,
    /// Wins and losses in the act.
    pub record: Record,
}

/// A player's rank from their tracker.gg profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
    /// Current rank; tier 0 is unranked.
    pub tier: u32,
    /// Ranked rating within the tier, when tracker.gg shows it; it often doesn't.
    pub rr: Option<u32>,
    /// The highest rank the player reached.
    pub peak_tier: u32,
    /// The latest act the player has played.
    pub acts: Vec<ActStats>,
}

impl Profile {
    /// This act's record, else last act's.
    #[must_use]
    pub fn act_record(&self, current: &str, previous: Option<&str>) -> Option<ActRecord> {
        let played = |id: &str| {
            self.acts
                .iter()
                .find(|a| a.act_id == id)
                .filter(|a| a.record.games() > 0)
                .map(|a| a.record)
        };
        if let Some(record) = played(current) {
            return Some(ActRecord {
                record,
                source: RecordSource::ThisAct,
            });
        }
        previous.and_then(played).map(|record| ActRecord {
            record,
            source: RecordSource::PreviousAct,
        })
    }
}

/// One of a player's recent competitive matches, from their side.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct RecentMatch {
    /// The map's id, as tracker.gg names it.
    pub map_id: String,
    /// `None` for a draw.
    pub won: Option<bool>,
    /// Rounds the player's team won.
    pub rounds_won: u32,
    /// Rounds the player's team lost.
    pub rounds_lost: u32,
    /// The player's stats; only the agent, the counts and the performance score are set.
    pub player: PlayerMatch,
}

/// A player's wins and losses on one map.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MapRecord {
    /// The map's display name.
    pub name: String,
    /// Wins and losses on it.
    pub record: Record,
}

/// The tier in one of tracker.gg's rank icon URLs (`.../tiersv2/24.png`), which use today's numbering.
fn tier_from_icon(stat: &Value) -> u32 {
    stat["metadata"]["iconUrl"]
        .as_str()
        .and_then(|u| u.rsplit('/').next())
        .and_then(|f| f.strip_suffix(".png"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn stat_u32(stats: &Value, key: &str) -> u32 {
    stats[key]["value"]
        .as_f64()
        .map_or(0, |v| cast::round_u32(v.max(0.0)))
}

fn record(stats: &Value) -> Record {
    Record {
        wins: stat_u32(stats, "matchesWon"),
        losses: stat_u32(stats, "matchesLost"),
    }
}

/// Segments from either response shape: `{ segments: [...] }` (profile) or a bare list (segment endpoints).
fn segments(v: &Value) -> impl Iterator<Item = &Value> {
    let data = &v["data"];
    data.as_array()
        .or_else(|| data["segments"].as_array())
        .into_iter()
        .flatten()
}

/// Rank, peak and latest act from `/profile/riot/{id}`. Its season entry is the latest act the player
/// has played, which isn't the current one until they play a game in it; until then they're unranked.
#[must_use]
pub fn parse_profile(v: &Value, current_act: &str) -> Profile {
    let Some(season) = segments(v).find(|s| s["type"] == "season") else {
        return Profile::default();
    };
    let stats = &season["stats"];
    let act = parse_season(v);
    let current = act
        .as_ref()
        .is_some_and(|a| a.act_id.eq_ignore_ascii_case(current_act));
    Profile {
        tier: if current {
            tier_from_icon(&stats["rank"])
        } else {
            0
        },
        rr: if current {
            stats["rank"]["value"]
                .as_f64()
                .map(|v| cast::round_u32(v.max(0.0)))
        } else {
            None
        },
        peak_tier: tier_from_icon(&stats["peakRank"]),
        acts: act.into_iter().collect(),
    }
}

/// One act's rank and record from a response's season entry (`/segments/season?seasonId=...`).
#[must_use]
pub fn parse_season(v: &Value) -> Option<ActStats> {
    let s = segments(v).find(|s| s["type"] == "season")?;
    Some(ActStats {
        act_id: s["attributes"]["seasonId"].as_str()?.to_lowercase(),
        tier: tier_from_icon(&s["stats"]["rank"]),
        record: record(&s["stats"]),
    })
}

/// Wins and losses per map (by display name) from `/segments/map`, most played first.
#[must_use]
pub fn parse_maps(v: &Value) -> Vec<MapRecord> {
    let mut maps: Vec<MapRecord> = segments(v)
        .filter(|s| s["type"] == "map")
        .filter_map(|s| {
            Some(MapRecord {
                name: s["metadata"]["name"].as_str()?.to_string(),
                record: record(&s["stats"]),
            })
        })
        .filter(|m| m.record.games() > 0)
        .collect();
    maps.sort_by(|a, b| {
        b.record
            .games()
            .cmp(&a.record.games())
            .then_with(|| a.name.cmp(&b.name))
    });
    maps
}

/// A match a player was in, with their team's score: two players with the same entry were teammates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_structs,
    reason = "teamdiff_live's tests build a player's match history from these, as tracker.gg \
              would report it"
)]
pub struct MatchSide {
    /// The match, in lowercase.
    pub match_id: String,
    /// Rounds the player's team won.
    pub rounds_won: u32,
    /// Rounds the player's team lost.
    pub rounds_lost: u32,
}

/// Every decided match in a `/matches` response, as `MatchSide`s. A tie, or a match without a score,
/// gives both teams the same entry, so it can't tell teammates apart and is left out.
#[must_use]
pub fn parse_match_sides(v: &Value) -> Vec<MatchSide> {
    v["data"]["matches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let seg = m["segments"]
                .as_array()?
                .iter()
                .find(|s| s["type"] == "overview")?;
            let (won, lost) = (
                stat_u32(&seg["stats"], "roundsWon"),
                stat_u32(&seg["stats"], "roundsLost"),
            );
            if won == lost {
                return None;
            }
            Some(MatchSide {
                match_id: m["attributes"]["id"].as_str()?.to_lowercase(),
                rounds_won: won,
                rounds_lost: lost,
            })
        })
        .collect()
}

/// The parties of two or more in a `match_players` response, as lowercase Riot IDs (`name#tag`).
#[must_use]
pub fn parse_match_parties(v: &Value) -> Vec<Vec<String>> {
    let mut parties: HashMap<(&str, &str), Vec<String>> = HashMap::new();
    let players = segments(v).filter(|s| s["type"] == "player-summary");
    for s in players {
        let (Some(name), Some(team), Some(party)) = (
            s["attributes"]["platformUserIdentifier"].as_str(),
            s["metadata"]["teamId"].as_str(),
            s["metadata"]["partyId"].as_str(),
        ) else {
            continue;
        };
        parties
            .entry((team, party))
            .or_default()
            .push(name.to_lowercase());
    }
    parties.into_values().filter(|p| p.len() > 1).collect()
}

/// The newest `count` matches from `/matches/riot/{id}`.
#[must_use]
pub fn parse_matches(v: &Value, count: usize) -> Vec<RecentMatch> {
    v["data"]["matches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let seg = m["segments"]
                .as_array()?
                .iter()
                .find(|s| s["type"] == "overview")?;
            let st = &seg["stats"];
            let n = |k: &str| stat_u32(st, k);
            let won = match m["metadata"]["result"].as_str() {
                Some("victory") => Some(true),
                Some("defeat") => Some(false),
                _ => None,
            };
            Some(RecentMatch {
                map_id: m["metadata"]["map"].as_str().unwrap_or("").to_string(),
                won,
                rounds_won: n("roundsWon"),
                rounds_lost: n("roundsLost"),
                player: PlayerMatch {
                    agent_id: seg["metadata"]["agent"].as_str().unwrap_or("").to_string(),
                    kills: n("kills"),
                    deaths: n("deaths"),
                    assists: n("assists"),
                    score: n("score"),
                    rounds: n("roundsPlayed"),
                    damage: n("damage"),
                    headshots: n("dealtHeadshots"),
                    bodyshots: n("dealtBodyshots"),
                    legshots: n("dealtLegshots"),
                    performance: st["performance"]["value"].as_f64(),
                    ..Default::default()
                },
            })
        })
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::*;

    fn stat(v: &Value) -> Value {
        json!({ "value": v, "metadata": {} })
    }

    fn rank(tier: u32, rr: u32) -> Value {
        json!({ "value": rr, "metadata": { "iconUrl": format!("https://trackercdn.com/cdn/tracker.gg/valorant/icons/tiersv2/{tier}.png") } })
    }

    fn season(act: &str, tier: u32, rr: u32, peak: u32, w: u32, l: u32) -> Value {
        json!({ "type": "season", "attributes": { "seasonId": act, "playlist": "competitive" },
                "stats": { "rank": rank(tier, rr), "peakRank": rank(peak, 0), "matchesWon": stat(&json!(w)), "matchesLost": stat(&json!(l)), "matchesTied": stat(&json!(1)) } })
    }

    #[test]
    fn match_parties_group_teammates_by_party() {
        let p = |name: &str, team: &str, party: &str| json!({ "type": "player-summary", "attributes": { "platformUserIdentifier": name }, "metadata": { "teamId": team, "partyId": party } });
        let v = json!({ "data": { "segments": [
            p("A#1", "Red", "1"), p("b#2", "Red", "1"), p("solo#3", "Red", "2"),
            p("x#4", "Blue", "1"), p("y#5", "Blue", "1"), p("z#6", "Blue", "1"),
            { "type": "round-summary" }
        ]}});
        let mut parties = parse_match_parties(&v);
        for p in &mut parties {
            p.sort();
        }
        parties.sort();
        assert_eq!(
            parties,
            [vec!["a#1", "b#2"], vec!["x#4", "y#5", "z#6"]],
            "party ids are only unique within a team"
        );
    }

    #[test]
    fn profile_reads_the_current_act() {
        let v = json!({ "data": { "segments": [
            { "type": "agent", "attributes": { "seasonId": "A6" }, "stats": {} },
            season("A6", 24, 23, 25, 29, 7)
        ]}});
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (24, Some(23), 25));
        let mut hidden = v.clone();
        hidden["data"]["segments"][1]["stats"]["rank"]["value"] = Value::Null;
        assert_eq!(
            parse_profile(&hidden, "a6").rr,
            None,
            "tracker.gg often leaves RR out; it isn't 0"
        );
        assert_eq!(
            p.acts,
            [ActStats {
                act_id: "a6".into(),
                tier: 24,
                record: Record {
                    wins: 29,
                    losses: 7
                }
            }],
            "ties don't count as losses"
        );
        assert_eq!(
            p.act_record("a6", Some("a5")),
            Some(ActRecord {
                record: Record {
                    wins: 29,
                    losses: 7
                },
                source: RecordSource::ThisAct
            })
        );
    }

    #[test]
    fn no_games_this_act_is_unranked_but_keeps_peak_and_last_act() {
        let v = json!({ "data": { "segments": [season("a5", 20, 50, 21, 65, 68)] } });
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (0, None, 21));
        let r = p.act_record("a6", Some("a5")).unwrap();
        assert_eq!(
            (r.record, r.source),
            (
                Record {
                    wins: 65,
                    losses: 68
                },
                RecordSource::PreviousAct
            )
        );
        assert_eq!(
            p.act_record("a6", Some("a4")),
            None,
            "an act older than the last one isn't this act's record"
        );
        assert_eq!(
            parse_profile(&json!({ "data": { "segments": [] } }), "a6"),
            Profile::default()
        );
    }

    #[test]
    fn season_endpoint_reads_its_season_entry() {
        let a = parse_season(
            &json!({ "data": [{ "type": "weapon" }, season("A4", 17, 0, 18, 30, 27)] }),
        )
        .unwrap();
        assert_eq!(
            a,
            ActStats {
                act_id: "a4".into(),
                tier: 17,
                record: Record {
                    wins: 30,
                    losses: 27
                }
            }
        );
        assert_eq!(
            parse_season(&json!({ "data": [{ "type": "weapon" }] })),
            None,
            "an unplayed act has no season entry"
        );
    }

    #[test]
    fn maps_skip_unplayed_and_sort_by_games() {
        let map = |name: &str, w: u32, l: u32| json!({ "type": "map", "metadata": { "name": name }, "stats": { "matchesWon": stat(&json!(w)), "matchesLost": stat(&json!(l)) } });
        let v = json!({ "data": [map("Bind", 5, 2), map("Abyss", 4, 1), { "type": "map-top-agent", "metadata": { "name": "Jett" }, "stats": {} }, map("Pearl", 0, 0), map("Ascent", 3, 4)] });
        let maps = parse_maps(&v);
        let names: Vec<_> = maps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Ascent", "Bind", "Abyss"]);
        assert_eq!(maps[1].record, Record { wins: 5, losses: 2 });
    }

    #[test]
    fn matches_read_the_players_side() {
        let st = |k: &str, v: f64| (k.to_string(), stat(&json!(v)));
        let stats: Map<String, Value> = [
            st("roundsWon", 13.0),
            st("roundsLost", 7.0),
            st("kills", 24.0),
            st("deaths", 13.0),
            st("assists", 7.0),
            st("score", 7037.0),
            st("roundsPlayed", 20.0),
            st("damage", 4300.0),
            st("dealtHeadshots", 4.0),
            st("dealtBodyshots", 40.0),
            st("dealtLegshots", 0.0),
            st("performance", 400.97),
        ]
        .into_iter()
        .collect();
        let m = |id: &str, result: &str| {
            json!({ "attributes": { "id": id }, "metadata": { "map": "92584fbe", "result": result },
                    "segments": [{ "type": "overview", "metadata": { "agent": "add6443a" }, "stats": stats.clone() }] })
        };
        let v = json!({ "data": { "matches": [m("m1", "victory"), m("m2", "defeat"), m("m3", "draw"), { "attributes": { "id": "bad" } }] } });
        let all = parse_matches(&v, 5);
        assert_eq!(
            all.len(),
            3,
            "a match without the player's segment is skipped"
        );
        let sides = parse_match_sides(&v);
        assert_eq!(sides.len(), 3);
        assert_eq!(
            sides[0],
            MatchSide {
                match_id: "m1".into(),
                rounds_won: 13,
                rounds_lost: 7
            }
        );
        let mut tie = v.clone();
        tie["data"]["matches"][0]["segments"][0]["stats"]["roundsLost"] = stat(&json!(13.0));
        assert_eq!(
            parse_match_sides(&tie).len(),
            2,
            "a tie gives both teams the same entry, so it's left out"
        );
        let first = &all[0];
        assert_eq!(
            (
                first.won,
                first.rounds_won,
                first.rounds_lost,
                first.map_id.as_str()
            ),
            (Some(true), 13, 7, "92584fbe")
        );
        assert_eq!(
            (
                first.player.kills,
                first.player.deaths,
                first.player.assists,
                first.player.acs()
            ),
            (24, 13, 7, 352)
        );
        assert_eq!(first.player.hs_pct(), Some(9));
        assert_eq!(first.player.performance, Some(400.97));
        assert_eq!((all[1].won, all[2].won), (Some(false), None));
        assert_eq!(parse_matches(&v, 1).len(), 1);
    }
}
