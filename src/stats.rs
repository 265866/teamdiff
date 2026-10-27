//! Win-loss records.

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
