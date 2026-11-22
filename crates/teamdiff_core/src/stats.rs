//! Records, RR changes, and per-match stats.

use std::collections::HashMap;

use serde_json::Value;

use crate::cast;

/// Wins and losses. Draws count as neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Record {
    /// Matches won.
    pub wins: u32,
    /// Matches lost.
    pub losses: u32,
}

impl Record {
    /// Wins and losses together.
    #[must_use]
    pub fn games(self) -> u32 {
        self.wins.wrapping_add(self.losses)
    }

    /// The win percentage, rounded, or `None` without games.
    #[must_use]
    pub fn pct(self) -> Option<u32> {
        (self.games() > 0)
            .then(|| cast::round_u32(f64::from(self.wins) * 100.0 / f64::from(self.games())))
    }
}

/// Which act an [`ActRecord`] is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordSource {
    /// The current act.
    ThisAct,
    /// The act before it, for a player without games this act.
    PreviousAct,
}

/// A player's record in one act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActRecord {
    /// The record.
    pub record: Record,
    /// The act it is from.
    pub source: RecordSource,
}

/// One entry of `/mmr/v1/players/{puuid}/competitiveupdates`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CompUpdate {
    /// The match.
    pub match_id: String,
    /// The map's URL, such as `/Game/Maps/Ascent/Ascent`.
    pub map_url: String,
    /// The act, in lowercase.
    pub act_id: String,
    /// The RR the match earned, negative for a loss.
    pub rr_earned: i32,
    /// The rank after the match.
    pub tier_after: u32,
    /// The RR within that rank after the match.
    pub rr_after: u32,
}

/// Reads `/mmr/v1/players/{puuid}/competitiveupdates`, skipping entries without a match id.
#[must_use]
pub fn parse_comp_updates(v: &Value) -> Vec<CompUpdate> {
    v["Matches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(CompUpdate {
                match_id: m["MatchID"].as_str()?.to_string(),
                map_url: m["MapID"].as_str().unwrap_or("").to_string(),
                act_id: m["SeasonID"].as_str().unwrap_or("").to_lowercase(),
                rr_earned: cast::low_i32(m["RankedRatingEarned"].as_i64().unwrap_or(0)),
                tier_after: cast::low_u32(m["TierAfterUpdate"].as_u64().unwrap_or(0)),
                rr_after: cast::low_u32(m["RankedRatingAfterUpdate"].as_u64().unwrap_or(0)),
            })
        })
        .collect()
}

/// One player's stats in one match.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct PlayerMatch {
    /// The player's PUUID.
    pub puuid: String,
    /// The player's team id, such as "Blue".
    pub team: String,
    /// The agent's id.
    pub agent_id: String,
    /// The player's party.
    pub party_id: String,
    /// Kills.
    pub kills: u32,
    /// Deaths.
    pub deaths: u32,
    /// Assists.
    pub assists: u32,
    /// Combat score over the whole match.
    pub score: u32,
    /// Rounds played.
    pub rounds: u32,
    /// Damage dealt.
    pub damage: u32,
    /// Hits on the head.
    pub headshots: u32,
    /// Hits on the body.
    pub bodyshots: u32,
    /// Hits on the legs.
    pub legshots: u32,
    /// Rounds in which the player made the first kill.
    pub first_kills: u32,
    /// Riot's Performance Score (0-500), when the match has one for the player.
    pub performance: Option<f64>,
}

impl PlayerMatch {
    /// Average combat score per round, rounded.
    #[must_use]
    pub fn acs(&self) -> u32 {
        if self.rounds == 0 {
            0
        } else {
            cast::round_u32(f64::from(self.score) / f64::from(self.rounds))
        }
    }

    /// Average damage per round, rounded.
    #[must_use]
    pub fn adr(&self) -> u32 {
        if self.rounds == 0 {
            0
        } else {
            cast::round_u32(f64::from(self.damage) / f64::from(self.rounds))
        }
    }

    /// Every hit.
    fn shots(&self) -> u32 {
        self.headshots
            .wrapping_add(self.bodyshots)
            .wrapping_add(self.legshots)
    }

    /// The share of hits on the head, as a rounded percentage, or `None` without hits.
    #[must_use]
    pub fn hs_pct(&self) -> Option<u32> {
        (self.shots() > 0)
            .then(|| cast::round_u32(f64::from(self.headshots) * 100.0 / f64::from(self.shots())))
    }
}

/// One team's result in a match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TeamResult {
    /// Rounds the team won.
    pub rounds_won: u32,
    /// Whether the team won the match.
    pub won: bool,
}

/// A match's score from one team's side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TeamScore {
    /// Rounds the team won.
    pub own: u32,
    /// Rounds the other team won.
    pub other: u32,
    /// Whether the team won the match.
    pub won: bool,
}

/// A finished match: its players' stats and the result.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct MatchSummary {
    /// The match.
    pub match_id: String,
    /// The map's URL, such as `/Game/Maps/Ascent/Ascent`.
    pub map_url: String,
    /// The queue, such as "competitive".
    pub queue: String,
    /// When the match started, in milliseconds since the Unix epoch.
    pub start_ms: i64,
    /// How long the match took, in milliseconds.
    pub length_ms: i64,
    /// Team id -> the team's result.
    pub teams: HashMap<String, TeamResult>,
    /// The players, in the order Riot lists them.
    pub players: Vec<PlayerMatch>,
    /// The match MVP Riot named.
    pub match_mvp: Option<String>,
    /// The MVP Riot named for each team, by team id.
    pub team_mvps: HashMap<String, String>,
}

impl MatchSummary {
    /// The player with this PUUID.
    #[must_use]
    pub fn player(&self, puuid: &str) -> Option<&PlayerMatch> {
        self.players.iter().find(|p| p.puuid == puuid)
    }

    /// PUUID -> party id for every player who queued with someone else in this match.
    #[must_use]
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

    /// The match's score from `team`'s side, with the most rounds any other team won.
    #[must_use]
    pub fn score_for(&self, team: &str) -> Option<TeamScore> {
        let TeamResult { rounds_won, won } = *self.teams.get(team)?;
        let other = self
            .teams
            .iter()
            .filter(|(t, _)| t.as_str() != team)
            .map(|(_, result)| result.rounds_won)
            .max()
            .unwrap_or(0);
        Some(TeamScore {
            own: rounds_won,
            other,
            won,
        })
    }

    /// The match MVP Riot named, else the highest-rated player.
    #[must_use]
    pub fn mvp(&self) -> Option<&PlayerMatch> {
        self.match_mvp
            .as_deref()
            .and_then(|id| self.player(id))
            .or_else(|| self.best(|_| true))
    }

    /// The match MVP when they're on `team`, so a team never shows two different MVPs; else the team
    /// MVP Riot named, else the team's highest-rated player.
    #[must_use]
    pub fn team_best(&self, team: &str) -> Option<&PlayerMatch> {
        self.mvp()
            .filter(|p| p.team == team)
            .or_else(|| self.team_mvps.get(team).and_then(|id| self.player(id)))
            .or_else(|| self.best(|p| p.team == team))
    }

    fn best(&self, keep: impl Fn(&PlayerMatch) -> bool) -> Option<&PlayerMatch> {
        self.players
            .iter()
            .filter(|p| keep(p))
            .max_by(|a, b| self.rating(a).total_cmp(&self.rating(b)))
    }

    /// Performance Score, with players Riot gave none (such as one who left) below everyone who has
    /// one; ACS for everyone when nobody has one, so the two scales are never compared.
    #[must_use]
    pub fn rating(&self, p: &PlayerMatch) -> f64 {
        if self.players.iter().any(|x| x.performance.is_some()) {
            p.performance.unwrap_or(-1.0)
        } else {
            f64::from(p.acs())
        }
    }
}

/// Parses `/match-details/v1/matches/{id}`.
#[must_use]
pub fn parse_match(v: &Value) -> Option<MatchSummary> {
    let info = &v["matchInfo"];
    let mut m = MatchSummary {
        match_id: info["matchId"].as_str()?.to_string(),
        map_url: info["mapId"].as_str().unwrap_or("").to_string(),
        queue: info["queueID"]
            .as_str()
            .or(info["queueId"].as_str())
            .unwrap_or("")
            .to_string(),
        start_ms: info["gameStartMillis"].as_i64().unwrap_or(0),
        length_ms: info["gameLengthMillis"].as_i64().unwrap_or(0),
        match_mvp: v["matchMvp"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        ..Default::default()
    };
    for t in v["teams"].as_array().into_iter().flatten() {
        if let Some(id) = t["teamId"].as_str() {
            m.teams.insert(
                id.to_string(),
                TeamResult {
                    rounds_won: cast::low_u32(t["roundsWon"].as_u64().unwrap_or(0)),
                    won: t["won"].as_bool().unwrap_or(false),
                },
            );
            if let Some(mvp) = t["mvp"].as_str().filter(|s| !s.is_empty()) {
                m.team_mvps.insert(id.to_string(), mvp.to_string());
            }
        }
    }
    let mut by_id = players_by_id(v);
    for round in v["roundResults"].as_array().into_iter().flatten() {
        add_round(round, &mut by_id);
    }
    let order = v["players"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["subject"].as_str());
    for id in order {
        if let Some(player) = by_id.remove(id) {
            m.players.push(player);
        }
    }
    Some(m)
}

/// The match's players with their totals from `players`, by PUUID. The per-round damage and first
/// kills come from [`add_round`].
fn players_by_id(v: &Value) -> HashMap<String, PlayerMatch> {
    let mut by_id: HashMap<String, PlayerMatch> = HashMap::new();
    for p in v["players"].as_array().into_iter().flatten() {
        let Some(id) = p["subject"].as_str() else {
            continue;
        };
        let s = &p["stats"];
        let n = |k: &str| cast::low_u32(s[k].as_u64().unwrap_or(0));
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
    by_id
}

/// Adds one entry of `roundResults` to the players' damage, hits and first kills.
fn add_round(round: &Value, by_id: &mut HashMap<String, PlayerMatch>) {
    let mut first: Option<(i64, String)> = None;
    for ps in round["playerStats"].as_array().into_iter().flatten() {
        let Some(id) = ps["subject"].as_str() else {
            continue;
        };
        for d in ps["damage"].as_array().into_iter().flatten() {
            if let Some(pm) = by_id.get_mut(id) {
                pm.damage = pm
                    .damage
                    .wrapping_add(cast::low_u32(d["damage"].as_u64().unwrap_or(0)));
                pm.headshots = pm
                    .headshots
                    .wrapping_add(cast::low_u32(d["headshots"].as_u64().unwrap_or(0)));
                pm.bodyshots = pm
                    .bodyshots
                    .wrapping_add(cast::low_u32(d["bodyshots"].as_u64().unwrap_or(0)));
                pm.legshots = pm
                    .legshots
                    .wrapping_add(cast::low_u32(d["legshots"].as_u64().unwrap_or(0)));
            }
        }
        for k in ps["kills"].as_array().into_iter().flatten() {
            let t = k["roundTime"]
                .as_i64()
                .or(k["timeSinceRoundStartMillis"].as_i64())
                .unwrap_or(i64::MAX);
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

/// Performance Score (0-500, added in patch 13.06). Riot's match details keep it under an obfuscated
/// key, `scores.TempValueF`, which matched the score the game showed and the one tracker.gg reports.
fn performance_score(player: &Value) -> Option<f64> {
    player["scores"]["TempValueF"].as_f64()
}

/// K/D and headshot % pooled over a player's matches.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PooledStats {
    /// Total kills over total deaths, or the kills alone without deaths; `None` without matches.
    pub kd: Option<f64>,
    /// Total headshots over total hits, as a rounded percentage; `None` without hits.
    pub hs_pct: Option<u32>,
}

/// K/D and headshot % pooled over a player's matches: total kills over total deaths, and total
/// headshots over total shots, so bigger games count for more.
#[must_use]
pub fn pooled_kd_hs<'a>(matches: impl IntoIterator<Item = &'a PlayerMatch>) -> PooledStats {
    let (mut kills, mut deaths, mut headshots, mut shots, mut games) =
        (0u32, 0u32, 0u32, 0u32, 0u32);
    for p in matches {
        kills = kills.wrapping_add(p.kills);
        deaths = deaths.wrapping_add(p.deaths);
        headshots = headshots.wrapping_add(p.headshots);
        shots = shots.wrapping_add(p.shots());
        games += 1;
    }
    let kd = (games > 0).then(|| {
        if deaths == 0 {
            f64::from(kills)
        } else {
            f64::from(kills) / f64::from(deaths)
        }
    });
    let hs_pct =
        (shots > 0).then(|| cast::round_u32(f64::from(headshots) * 100.0 / f64::from(shots)));
    PooledStats { kd, hs_pct }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn comp_updates_parse_rr_changes() {
        let updates = parse_comp_updates(&json!({ "Matches": [
            { "MatchID": "1", "MapID": "/Game/Maps/Ascent/Ascent", "SeasonID": "A6", "RankedRatingEarned": -16, "TierAfterUpdate": 17, "RankedRatingAfterUpdate": 31 },
            { "MapID": "no id" }
        ]}));
        assert_eq!(updates.len(), 1);
        assert_eq!(
            (
                updates[0].act_id.as_str(),
                updates[0].rr_earned,
                updates[0].tier_after,
                updates[0].rr_after
            ),
            ("a6", -16, 17, 31)
        );
    }

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
        assert_eq!(
            a.first_kills, 1,
            "b got the first kill in round 1, a in round 2"
        );
        assert_eq!(m.player("b").unwrap().first_kills, 1);
        assert_eq!(a.performance, Some(338.0));
        assert_eq!(m.player("b").unwrap().performance, Some(331.5));
        assert_eq!(
            m.score_for("Blue"),
            Some(TeamScore {
                own: 13,
                other: 9,
                won: true
            })
        );
        assert_eq!(
            m.score_for("Red"),
            Some(TeamScore {
                own: 9,
                other: 13,
                won: false
            })
        );
        assert_eq!(m.mvp().unwrap().puuid, "a");
        assert_eq!(m.team_best("Red").unwrap().puuid, "b");
    }

    /// Adds Blue player "d", whose Performance Score tops the lobby.
    fn with_star(v: &mut Value) {
        let star = json!({ "subject": "d", "teamId": "Blue", "characterId": "sova", "partyId": "p4", "scores": { "TempValueF": 450.0 },
                           "stats": { "score": 7000, "roundsPlayed": 22, "kills": 30, "deaths": 10, "assists": 2 } });
        v["players"].as_array_mut().unwrap().push(star);
    }

    #[test]
    fn riot_names_the_mvps_when_it_says() {
        let mut v = match_json();
        with_star(&mut v);
        v["matchMvp"] = json!("b");
        v["teams"][0]["mvp"] = json!("a");
        v["teams"][1]["mvp"] = json!("b");
        let m = parse_match(&v).unwrap();
        assert_eq!(
            m.mvp().unwrap().puuid,
            "b",
            "Riot's pick, though d has the top score"
        );
        assert_eq!(
            m.team_best("Blue").unwrap().puuid,
            "a",
            "Riot's pick, though d has Blue's top score"
        );

        v["teams"][0]["mvp"] = json!("nobody");
        v["matchMvp"] = json!("");
        let m = parse_match(&v).unwrap();
        assert_eq!(m.match_mvp, None, "an empty id isn't a pick");
        assert_eq!(
            m.mvp().unwrap().puuid,
            "d",
            "computed when Riot names no one"
        );
        assert_eq!(
            m.team_best("Blue").unwrap().puuid,
            "d",
            "computed when Riot's pick isn't in the match"
        );
    }

    #[test]
    fn a_team_with_the_match_mvp_shows_them_as_its_mvp() {
        let mut v = match_json();
        with_star(&mut v);
        v["matchMvp"] = json!("a");
        let m = parse_match(&v).unwrap();
        assert_eq!(
            m.team_best("Blue").unwrap().puuid,
            "a",
            "not d, which would put two MVPs on Blue"
        );
        v["teams"][0]["mvp"] = json!("d");
        let m = parse_match(&v).unwrap();
        assert_eq!(
            m.team_best("Blue").unwrap().puuid,
            "a",
            "even when Riot's team pick differs"
        );
    }

    /// A player who leaves gets no Performance Score. They rank below everyone who has one, rather
    /// than switching the whole lobby to ACS and crowning the wrong MVP.
    #[test]
    fn a_player_without_a_performance_score_ranks_last_instead_of_switching_to_acs() {
        let mut v = match_json();
        v["players"][1]["scores"]["TempValueF"] = json!(400);
        let leaver = json!({ "subject": "c", "teamId": "Red", "characterId": "sage", "partyId": "p3",
                              "stats": { "score": 9000, "roundsPlayed": 22, "kills": 30, "deaths": 1, "assists": 0 } });
        v["players"].as_array_mut().unwrap().push(leaver);
        let m = parse_match(&v).unwrap();
        assert_eq!(
            m.mvp().unwrap().puuid,
            "b",
            "b's 400 beats a's 338; c's high ACS doesn't count"
        );
        assert!(m.rating(m.player("c").unwrap()) < m.rating(m.player("a").unwrap()));
    }

    #[test]
    fn mvp_falls_back_to_acs_without_performance() {
        let mut v = match_json();
        v["players"][0]["scores"]
            .as_object_mut()
            .unwrap()
            .remove("TempValueF");
        v["players"][1].as_object_mut().unwrap().remove("scores");
        let m = parse_match(&v).unwrap();
        assert_eq!(m.mvp().unwrap().puuid, "a", "higher ACS wins");
    }

    #[test]
    fn match_without_id_is_rejected() {
        assert!(parse_match(&json!({ "matchInfo": {} })).is_none());
    }

    #[test]
    fn pooled_stats_weight_bigger_games() {
        let g = |kills, deaths, headshots, bodyshots| PlayerMatch {
            kills,
            deaths,
            headshots,
            bodyshots,
            ..Default::default()
        };
        let pooled = |kd, hs_pct| PooledStats { kd, hs_pct };
        let games = [g(20, 10, 10, 30), g(5, 10, 0, 10)];
        let PooledStats { kd, hs_pct } = pooled_kd_hs(&games);
        assert_eq!(
            kd,
            Some(1.25),
            "25 kills / 20 deaths, not the mean of 2.0 and 0.5"
        );
        assert_eq!(hs_pct, Some(20), "10 / 50 shots");
        assert_eq!(pooled_kd_hs(&[]), pooled(None, None));
        assert_eq!(pooled_kd_hs(&[g(7, 0, 0, 0)]), pooled(Some(7.0), None));
    }

    #[test]
    fn malformed_counts_wrap_as_every_earlier_release_did() {
        let mut v = match_json();
        v["roundResults"][1]["playerStats"][0]["damage"] = json!([
            { "damage": u64::from(u32::MAX), "headshots": u64::from(u32::MAX), "bodyshots": 0, "legshots": 0 },
            { "damage": 2, "headshots": 2, "bodyshots": 0, "legshots": 0 }
        ]);
        let m = parse_match(&v).unwrap();
        let a = m.player("a").unwrap();
        assert_eq!(
            a.damage, 151,
            "150 from round 1, then u32::MAX + 2 wraps to 1"
        );
        assert_eq!(
            a.headshots, 2,
            "1 from round 1, then u32::MAX + 2 wraps to 1"
        );
        let pooled = pooled_kd_hs([a, a]);
        assert_eq!(
            pooled.hs_pct,
            a.hs_pct(),
            "pooling the same match twice keeps its share"
        );
        let record = Record {
            wins: u32::MAX,
            losses: 2,
        };
        assert_eq!(
            record.games(),
            1,
            "a malformed record wraps instead of aborting"
        );
    }
}
