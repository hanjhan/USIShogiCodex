use std::fmt;

use super::state::{PieceKind, PlayerSide, Square};

// A `Move` encodes everything needed to apply or display a single half-move
// (ply) without consulting the board again.  The redundant fields (`capture`,
// `kind`) allow move ordering and display to work without an extra board
// lookup.

/// Classifies a move for fast dispatch in move generation and ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Quiet,   // Normal move to an empty square
    Capture, // Normal move that captures an opponent piece
    Drop,    // Piece dropped from hand onto an empty square
}

/// A fully-specified shogi half-move (ply).
///
/// All fields are populated by the move generator so that:
/// - `apply_move` on `Board` can execute the move without extra lookups.
/// - The evaluator / move orderer can inspect captures without touching the board.
/// - USI / display formatting is straightforward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub player: PlayerSide,
    /// Source square. `None` for drops (the piece comes from hand, not the board).
    pub from: Option<Square>,
    /// Destination square.
    pub to: Square,
    /// The type of piece being moved (or dropped).
    pub piece: PieceKind,
    /// The piece type that was on `to` before the move (populated only for captures).
    pub capture: Option<PieceKind>,
    /// Whether the moving piece promotes on this move.
    pub promote: bool,
    pub kind: MoveKind,
}

impl Move {
    /// Creates a normal board move (quiet or capture, determined automatically).
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

    /// Creates a drop move (piece taken from hand and placed on an empty square).
    pub fn drop(player: PlayerSide, piece: PieceKind, to: Square) -> Self {
        Self {
            player,
            from: None, // Drops have no source square
            to,
            piece,
            capture: None, // Drops cannot capture
            promote: false, // Dropped pieces are always unpromoted
            kind: MoveKind::Drop,
        }
    }
}

/// Human-readable display, e.g. "Sente R(77) -> 76" or "Gote drops P to 55".
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
                        .unwrap_or_default(),
                    self.to
                )
            }
        }
    }
}
