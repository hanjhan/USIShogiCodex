use std::fmt;

use crate::engine::{search::SearchStrength, state::PlayerSide};

/// Describes whether a player slot is controlled by a human or a CPU engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerKind {
    Human,
    /// CPU player with a configured search strength.
    Cpu { strength: SearchStrength },
}

impl PlayerKind {
    /// Returns a human-readable description, e.g. "CPU (Normal)".
    pub fn describe(self) -> String {
        match self {
            PlayerKind::Human => "Human".to_string(),
            PlayerKind::Cpu { strength } => format!("CPU ({})", strength.describe()),
        }
    }

    /// Returns the search strength if this is a CPU player, None for humans.
    /// Used by `GameController` to decide whether to create an `AlphaBetaSearcher`.
    pub fn strength(self) -> Option<SearchStrength> {
        match self {
            PlayerKind::Cpu { strength } => Some(strength),
            PlayerKind::Human => None,
        }
    }

    pub fn is_human(self) -> bool {
        matches!(self, PlayerKind::Human)
    }
}

impl fmt::Display for PlayerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// Pairs a `PlayerSide` (Sente or Gote) with a `PlayerKind` (Human or CPU).
/// Used throughout the game layer to route turns to the correct handler.
#[derive(Clone, Debug)]
pub struct PlayerDescriptor {
    pub side: PlayerSide,
    pub kind: PlayerKind,
}

impl PlayerDescriptor {
    pub fn new(side: PlayerSide, kind: PlayerKind) -> Self {
        Self { side, kind }
    }

    /// Returns a label like "Sente (CPU (Strong))" for display.
    pub fn label(&self) -> String {
        format!("{} ({})", self.side.label(), self.kind)
    }

    pub fn is_human(&self) -> bool {
        self.kind.is_human()
    }
}
