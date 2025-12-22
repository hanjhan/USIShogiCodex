use std::fmt;

use super::state::{PieceKind, PlayerSide, Square};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Quiet,
    Capture,
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub player: PlayerSide,
    pub from: Option<Square>,
    pub to: Square,
    pub piece: PieceKind,
    pub capture: Option<PieceKind>,
    pub promote: bool,
    pub kind: MoveKind,
}

impl Move {
    pub fn normal(
        player: PlayerSide,
        from: Square,
        to: Square,
        piece: PieceKind,
        capture: Option<PieceKind>,
        promote: bool,
    ) -> Self {
        let kind = if capture.is_some() {
            MoveKind::Capture
        } else {
            MoveKind::Quiet
        };
        Self {
            player,
            from: Some(from),
            to,
            piece,
            capture,
            promote,
            kind,
        }
    }

    pub fn drop(player: PlayerSide, piece: PieceKind, to: Square) -> Self {
        Self {
            player,
            from: None,
            to,
            piece,
            capture: None,
            promote: false,
            kind: MoveKind::Drop,
        }
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            MoveKind::Drop => write!(
                f,
                "{} drops {} to {}",
                self.player.label(),
                self.piece.short_name(),
                self.to
            ),
            _ => {
                let promotion_flag = if self.promote { "+" } else { "" };
                write!(
                    f,
                    "{} {}{}{} -> {}",
                    self.player.label(),
                    self.piece.short_name(),
                    promotion_flag,
                    self.from
                        .map(|sq| format!("({})", sq))
                        .unwrap_or_else(|| "".to_string()),
                    self.to
                )
            }
        }
    }
}
