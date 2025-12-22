use std::fmt;

use crate::engine::{search::SearchStrength, state::PlayerSide};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerKind {
    Human,
    Cpu { strength: SearchStrength },
}

impl PlayerKind {
    pub fn describe(self) -> String {
        match self {
            PlayerKind::Human => "Human".to_string(),
            PlayerKind::Cpu { strength } => format!("CPU ({})", strength.describe()),
        }
    }

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

#[derive(Clone, Debug)]
pub struct PlayerDescriptor {
    pub side: PlayerSide,
    pub kind: PlayerKind,
}

impl PlayerDescriptor {
    pub fn new(side: PlayerSide, kind: PlayerKind) -> Self {
        Self { side, kind }
    }

    pub fn label(&self) -> String {
        format!("{} ({})", self.side.label(), self.kind)
    }

    pub fn is_human(&self) -> bool {
        self.kind.is_human()
    }
}
