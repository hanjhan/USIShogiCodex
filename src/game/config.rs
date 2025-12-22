use crate::engine::{search::SearchStrength, state::PlayerSide};

use super::{
    player::{PlayerDescriptor, PlayerKind},
    timer::TimeControl,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    PlayerVsCpu,
    CpuVsCpu,
}

#[derive(Clone, Debug)]
pub struct GameConfig {
    pub mode: GameMode,
    pub sente: PlayerDescriptor,
    pub gote: PlayerDescriptor,
    pub time_control: TimeControl,
    pub debug_mode: bool,
}

impl GameConfig {
    pub fn new(
        mode: GameMode,
        sente: PlayerDescriptor,
        gote: PlayerDescriptor,
        time_control: TimeControl,
        debug_mode: bool,
    ) -> Self {
        Self {
            mode,
            sente,
            gote,
            time_control,
            debug_mode,
        }
    }

    pub fn player(&self, side: PlayerSide) -> &PlayerDescriptor {
        match side {
            PlayerSide::Sente => &self.sente,
            PlayerSide::Gote => &self.gote,
        }
    }
}

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
            debug_mode: false,
        }
    }
}

impl GameConfig {
    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }
}
