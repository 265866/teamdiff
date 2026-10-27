//! Application state shared between the network task and the UI.

use crate::presence::Presence;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Not connected; the reason is shown under "Waiting for VALORANT…".
    Waiting(String),
    Menus,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub phase: Phase,
    pub me: String,
    /// Latest presence per player.
    pub presences: HashMap<String, Presence>,
}

impl Model {
    pub fn new() -> Self {
        Self { phase: Phase::Waiting("Looking for the Riot Client".into()), me: String::new(), presences: HashMap::new() }
    }

    pub fn my_presence(&self) -> Option<&Presence> {
        self.presences.get(&self.me)
    }
}
