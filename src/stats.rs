//! Records and per-match stats.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
    pub wins: u32,
    pub losses: u32,
}

impl Record {
    pub fn games(&self) -> u32 {
        self.wins + self.losses
    }
    pub fn pct(&self) -> Option<u32> {
        (self.games() > 0).then(|| (self.wins as f64 * 100.0 / self.games() as f64).round() as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordSource {
    ThisAct,
    PreviousAct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActRecord {
    pub record: Record,
    pub source: RecordSource,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerMatch {
    pub puuid: String,
    pub team: String,
    pub agent_id: String,
    pub party_id: String,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub score: u32,
    pub rounds: u32,
    pub damage: u32,
    pub headshots: u32,
    pub bodyshots: u32,
    pub legshots: u32,
    pub first_kills: u32,
    pub performance: Option<f64>,
}

impl PlayerMatch {
    pub fn acs(&self) -> u32 {
        if self.rounds == 0 { 0 } else { (self.score as f64 / self.rounds as f64).round() as u32 }
    }
    pub fn adr(&self) -> u32 {
        if self.rounds == 0 { 0 } else { (self.damage as f64 / self.rounds as f64).round() as u32 }
    }
    pub fn shots(&self) -> u32 {
        self.headshots + self.bodyshots + self.legshots
    }
    pub fn hs_pct(&self) -> Option<u32> {
        (self.shots() > 0).then(|| (self.headshots as f64 * 100.0 / self.shots() as f64).round() as u32)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchSummary {
    pub match_id: String,
    pub map_url: String,
    pub queue: String,
    pub start_ms: i64,
    pub length_ms: i64,
    /// team id -> (rounds won, won the match)
    pub teams: HashMap<String, (u32, bool)>,
    pub players: Vec<PlayerMatch>,
}

impl MatchSummary {
    pub fn player(&self, puuid: &str) -> Option<&PlayerMatch> {
        self.players.iter().find(|p| p.puuid == puuid)
    }

    /// puuid -> party id for every player who queued with someone else in this match.
    pub fn parties(&self) -> HashMap<String, String> {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for p in &self.players {
            *count.entry(&p.party_id).or_default() += 1;
        }
        self.players
            .iter()
            .filter(|p| !p.party_id.is_empty() && count[p.party_id.as_str()] > 1)
            .map(|p| (p.puuid.clone(), p.party_id.clone()))
            .collect()
    }

    /// (own rounds, other rounds, won) for a team.
    pub fn score_for(&self, team: &str) -> Option<(u32, u32, bool)> {
        let (mine, won) = *self.teams.get(team)?;
        let theirs = self.teams.iter().filter(|(t, _)| t.as_str() != team).map(|(_, (r, _))| *r).max().unwrap_or(0);
        Some((mine, theirs, won))
    }

    /// Performance Score, with players Riot gave none (such as one who left) below everyone who has
    /// one; ACS for everyone when nobody has one, so the two scales are never compared.
    pub fn rating(&self, p: &PlayerMatch) -> f64 {
        if self.players.iter().any(|x| x.performance.is_some()) { p.performance.unwrap_or(-1.0) } else { p.acs() as f64 }
    }
}

/// Parses `/match-details/v1/matches/{id}`.
pub fn parse_match(v: &Value) -> Option<MatchSummary> {
    let info = &v["matchInfo"];
    let mut m = MatchSummary {
        match_id: info["matchId"].as_str()?.to_string(),
        map_url: info["mapId"].as_str().unwrap_or("").to_string(),
        queue: info["queueID"].as_str().or(info["queueId"].as_str()).unwrap_or("").to_string(),
        start_ms: info["gameStartMillis"].as_i64().unwrap_or(0),
        length_ms: info["gameLengthMillis"].as_i64().unwrap_or(0),
        ..Default::default()
    };
    for t in v["teams"].as_array().into_iter().flatten() {
        if let Some(id) = t["teamId"].as_str() {
            m.teams.insert(id.to_string(), (t["roundsWon"].as_u64().unwrap_or(0) as u32, t["won"].as_bool().unwrap_or(false)));
        }
    }
    let mut by_id: HashMap<String, PlayerMatch> = HashMap::new();
    for p in v["players"].as_array().into_iter().flatten() {
        let Some(id) = p["subject"].as_str() else { continue };
        let s = &p["stats"];
        let n = |k: &str| s[k].as_u64().unwrap_or(0) as u32;
        by_id.insert(
            id.to_string(),
            PlayerMatch {
                puuid: id.to_string(),
                team: p["teamId"].as_str().unwrap_or("").to_string(),
                agent_id: p["characterId"].as_str().unwrap_or("").to_string(),
                party_id: p["partyId"].as_str().unwrap_or("").to_string(),
                kills: n("kills"),
                deaths: n("deaths"),
                assists: n("assists"),
                score: n("score"),
                rounds: n("roundsPlayed"),
                performance: performance_score(p),
                ..Default::default()
            },
        );
    }
    for round in v["roundResults"].as_array().into_iter().flatten() {
        let mut first: Option<(i64, String)> = None;
        for ps in round["playerStats"].as_array().into_iter().flatten() {
            let Some(id) = ps["subject"].as_str() else { continue };
            for d in ps["damage"].as_array().into_iter().flatten() {
                if let Some(pm) = by_id.get_mut(id) {
                    pm.damage += d["damage"].as_u64().unwrap_or(0) as u32;
                    pm.headshots += d["headshots"].as_u64().unwrap_or(0) as u32;
                    pm.bodyshots += d["bodyshots"].as_u64().unwrap_or(0) as u32;
                    pm.legshots += d["legshots"].as_u64().unwrap_or(0) as u32;
                }
            }
            for k in ps["kills"].as_array().into_iter().flatten() {
                let t = k["roundTime"].as_i64().or(k["timeSinceRoundStartMillis"].as_i64()).unwrap_or(i64::MAX);
                let killer = k["killer"].as_str().unwrap_or(id).to_string();
                if first.as_ref().is_none_or(|(ft, _)| t < *ft) {
                    first = Some((t, killer));
                }
            }
        }
        if let Some((_, killer)) = first
            && let Some(pm) = by_id.get_mut(&killer)
        {
            pm.first_kills += 1;
        }
    }
    let order: Vec<String> =
        v["players"].as_array().into_iter().flatten().filter_map(|p| p["subject"].as_str().map(str::to_string)).collect();
    m.players = order.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    Some(m)
}

/// Performance Score (0-500, added in patch 13.06). Riot's match details keep it under an obfuscated
/// key, `scores.TempValueF`, which matched the score the game showed and the one tracker.gg reports.
fn performance_score(player: &Value) -> Option<f64> {
    player["scores"]["TempValueF"].as_f64()
}

/// K/D and headshot % pooled over a player's matches: total kills over total deaths, and total
/// headshots over total shots, so bigger games count for more.
pub fn pooled_kd_hs<'a>(matches: impl IntoIterator<Item = &'a PlayerMatch>) -> (Option<f64>, Option<u32>) {
    let (mut k, mut d, mut hs, mut shots, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
    for p in matches {
        k += p.kills;
        d += p.deaths;
        hs += p.headshots;
        shots += p.shots();
        n += 1;
    }
    let kd = (n > 0).then(|| if d == 0 { k as f64 } else { k as f64 / d as f64 });
    let hs_pct = (shots > 0).then(|| (hs as f64 * 100.0 / shots as f64).round() as u32);
    (kd, hs_pct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn match_json() -> Value {
        json!({
            "matchInfo": { "matchId": "m1", "mapId": "/Game/Maps/Ascent/Ascent", "queueID": "competitive", "gameStartMillis": 1000, "gameLengthMillis": 2000 },
            "teams": [{ "teamId": "Blue", "roundsWon": 13, "won": true }, { "teamId": "Red", "roundsWon": 9, "won": false }],
            "players": [
                { "subject": "a", "teamId": "Blue", "characterId": "jett", "partyId": "p1",
                  "scores": { "TempValueA": 287.0, "TempValueF": 338.0 },
                  "stats": { "score": 6314, "roundsPlayed": 22, "kills": 24, "deaths": 15, "assists": 6, "TempValueA": 2900 } },
                { "subject": "b", "teamId": "Red", "characterId": "raze", "partyId": "p2",
                  "scores": { "TempValueF": 331.5 },
                  "stats": { "score": 5830, "roundsPlayed": 22, "kills": 22, "deaths": 17, "assists": 4 } }
            ],
            "roundResults": [
                { "playerStats": [
                    { "subject": "a", "kills": [{ "roundTime": 9000, "killer": "a", "victim": "b" }],
                      "damage": [{ "receiver": "b", "damage": 150, "headshots": 1, "bodyshots": 2, "legshots": 0 }] },
                    { "subject": "b", "kills": [{ "roundTime": 4000, "killer": "b", "victim": "x" }],
                      "damage": [{ "receiver": "a", "damage": 140, "headshots": 0, "bodyshots": 3, "legshots": 1 }] }
                ]},
                { "playerStats": [
                    { "subject": "a", "kills": [{ "roundTime": 2000, "killer": "a", "victim": "b" }], "damage": [{ "damage": 100, "headshots": 2, "bodyshots": 0, "legshots": 0 }] }
                ]}
            ]
        })
    }

    #[test]
    fn parses_match_stats() {
        let m = parse_match(&match_json()).unwrap();
        let a = m.player("a").unwrap();
        assert_eq!((a.kills, a.deaths, a.assists), (24, 15, 6));
        assert_eq!(a.acs(), 287);
        assert_eq!(a.damage, 250);
        assert_eq!(a.hs_pct(), Some(60));
        assert_eq!(a.first_kills, 1, "b got the first kill in round 1, a in round 2");
        assert_eq!(m.player("b").unwrap().first_kills, 1);
        assert_eq!(a.performance, Some(338.0));
        assert_eq!(m.player("b").unwrap().performance, Some(331.5));
        assert_eq!(m.score_for("Blue"), Some((13, 9, true)));
        assert_eq!(m.score_for("Red"), Some((9, 13, false)));
    }

    /// A player who leaves gets no Performance Score. They rank below everyone who has one, rather
    /// than switching the whole lobby to ACS.
    #[test]
    fn a_player_without_a_performance_score_ranks_last_instead_of_switching_to_acs() {
        let mut v = match_json();
        v["players"][1]["scores"]["TempValueF"] = json!(400);
        let leaver = json!({ "subject": "c", "teamId": "Red", "characterId": "sage", "partyId": "p3",
                              "stats": { "score": 9000, "roundsPlayed": 22, "kills": 30, "deaths": 1, "assists": 0 } });
        v["players"].as_array_mut().unwrap().push(leaver);
        let m = parse_match(&v).unwrap();
        let rating = |id: &str| m.rating(m.player(id).unwrap());
        assert!(rating("b") > rating("a"), "b's 400 beats a's 338");
        assert!(rating("c") < rating("a"), "c's high ACS doesn't count");
    }

    #[test]
    fn ratings_fall_back_to_acs_without_performance() {
        let mut v = match_json();
        v["players"][0]["scores"].as_object_mut().unwrap().remove("TempValueF");
        v["players"][1].as_object_mut().unwrap().remove("scores");
        let m = parse_match(&v).unwrap();
        assert_eq!(m.rating(m.player("a").unwrap()), 287.0, "a's ACS");
        assert_eq!(m.rating(m.player("b").unwrap()), 265.0, "b's ACS");
    }

    #[test]
    fn match_without_id_is_rejected() {
        assert!(parse_match(&json!({ "matchInfo": {} })).is_none());
    }

    #[test]
    fn pooled_stats_weight_bigger_games() {
        let g = |k, d, hs, body| PlayerMatch { kills: k, deaths: d, headshots: hs, bodyshots: body, ..Default::default() };
        let games = [g(20, 10, 10, 30), g(5, 10, 0, 10)];
        let (kd, hs) = pooled_kd_hs(&games);
        assert_eq!(kd, Some(1.25), "25 kills / 20 deaths, not the mean of 2.0 and 0.5");
        assert_eq!(hs, Some(20), "10 / 50 shots");
        assert_eq!(pooled_kd_hs(&[]), (None, None));
        assert_eq!(pooled_kd_hs(&[g(7, 0, 0, 0)]), (Some(7.0), None));
    }
}
