use std::time::Duration;

use crate::engine::{search::SearchStrength, state::PlayerSide};

use super::{
    player::{PlayerDescriptor, PlayerKind},
    timer::TimeControl,
};

/// High-level game mode selected at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    /// Sente is a human, Gote is a CPU.
    PlayerVsCpu,
    /// Both sides are CPU engines (useful for self-play / testing).
    CpuVsCpu,
}

/// All configuration needed to start a game.  Created once at startup and
/// passed to `GameController::new`; treated as immutable during the game.
#[derive(Clone, Debug)]
pub struct GameConfig {
    pub mode: GameMode,
    /// Descriptor for the Sente (first) player.
    pub sente: PlayerDescriptor,
    /// Descriptor for the Gote (second) player.
    pub gote: PlayerDescriptor,
    /// Time control shared by both players (main time + byoyomi).
    pub time_control: TimeControl,
    /// Fixed think time per move for CPU players.  When non-zero, this
    /// overrides the time-management calculation in `GameController::think_time_for`.
    pub think_time: Duration,
    pub debug_mode: bool,
}

impl GameConfig {
    pub fn new(
        mode: GameMode,
        sente: PlayerDescriptor,
        gote: PlayerDescriptor,
        time_control: TimeControl,
        think_time: Duration,
        debug_mode: bool,
    ) -> Self {
        Self {
            mode,
            sente,
            gote,
            time_control,
            think_time,
            debug_mode,
        }
    }

    /// Returns the descriptor for `side`.
    pub fn player(&self, side: PlayerSide) -> &PlayerDescriptor {
        match side {
            PlayerSide::Sente => &self.sente,
            PlayerSide::Gote => &self.gote,
        }
    }

    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }
}

/// Default: Player vs CPU (Normal strength), 10-minute + 10-second byoyomi,
/// 5-second CPU think time, debug off.
impl Default for GameConfig {
    fn default() -> Self {
        let time_control = TimeControl::default();
        let sente = PlayerDescriptor::new(PlayerSide::Sente, PlayerKind::Human);
        let gote = PlayerDescriptor::new(
            PlayerSide::Gote,
            PlayerKind::Cpu {
                strength: SearchStrength::Normal,
            },
        );
        Self {
            mode: GameMode::PlayerVsCpu,
            sente,
            gote,
            time_control,
            think_time: Duration::from_secs(5),
            debug_mode: false,
        }
    }
}
