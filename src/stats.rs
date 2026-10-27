//! Records and per-match stats.

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
    pub agent_id: String,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub score: u32,
    pub rounds: u32,
    pub damage: u32,
    pub headshots: u32,
    pub bodyshots: u32,
    pub legshots: u32,
    pub performance: Option<f64>,
}

impl PlayerMatch {
    pub fn acs(&self) -> u32 {
        if self.rounds == 0 { 0 } else { (self.score as f64 / self.rounds as f64).round() as u32 }
    }
    pub fn shots(&self) -> u32 {
        self.headshots + self.bodyshots + self.legshots
    }
    pub fn hs_pct(&self) -> Option<u32> {
        (self.shots() > 0).then(|| (self.headshots as f64 * 100.0 / self.shots() as f64).round() as u32)
    }
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
