// =============================================================================
// Pre-computed Evaluation Tables
// =============================================================================
//
// This module provides a static lookup table that pre-computes the evaluation
// score for every possible (side, piece_kind, promoted, square) combination.
// The Board uses this table to maintain an incremental score — on every
// make_move/undo_move, it adds/subtracts the table value for the affected
// pieces instead of scanning the entire board.
//
// ## What's included in each table entry
//
// `board_score[side][kind][promoted][square]` combines:
//   1. **Base material value** — Pawn=100, Lance=430, Knight=410, Silver=550,
//      Gold=620, Bishop=910, Rook=1040 (Bonanza-inspired scale).
//   2. **Promotion bonus** — added when promoted: Dragon(Rook)=+600,
//      Horse(Bishop)=+450, Tokin(Pawn)=+500, ProSilver=+70, ProKnight=+210,
//      ProLance=+190.  All promoted minors ≈ Gold value.
//   3. **Piece-square table (PST)** — positional bonus based on piece type
//      and location.  Uses "advancement level" (0=own back rank, 8=enemy
//      back rank) so the same table works for both Sente and Gote.
//   4. **Pawn advancement** — unpromoted pawns get +8 per rank advanced
//      toward the promotion zone.
//
// `hand_value[kind]` is the value of one piece in hand (same as base material).
//
// ## PST Design Philosophy
//
// Each piece type has a positional preference:
//   - **King**:   prefers own back rank (+30) and castle files (+10).
//                 Penalised for advancing too far (-15).
//   - **Rook**:   benefits from advancing (+4/rank unpromoted, +2 promoted).
//   - **Bishop**: rewards central files (diagonal control) and mid-ranks.
//   - **Silver**: forward-moving piece; mild advancement bonus (+3/rank).
//   - **Knight**: must advance (can't retreat); steep bonus at adv 6+ (+5/rank).
//   - **Lance**:  forward-only; strong advancement bonus (+4/rank).
//   - **Gold**:   defensive piece; mild advance bonus (+2/rank).
//   - **Pawn**:   advancement handled separately (not in PST).
//
// =============================================================================

use std::sync::OnceLock;

use super::state::{PieceKind, PlayerSide, Square};

pub struct EvalTable {
    /// `board_score[side][kind][promoted][square]` — total value of a piece
    /// sitting on the board at that square (material + promotion + PST + pawn adv).
    pub board_score: [[[[i32; 81]; 2]; 8]; 2],
    /// `hand_value[kind]` — value of one piece held in hand.
    pub hand_value: [i32; 8],
}

static EVAL_TABLE: OnceLock<EvalTable> = OnceLock::new();

pub fn eval_table() -> &'static EvalTable {
    EVAL_TABLE.get_or_init(EvalTable::new)
}

impl EvalTable {
    fn new() -> Self {
        //                   King  Rook  Bishop Gold  Silver Knight Lance Pawn
        let piece_values:    [i32; 8] = [0, 1040, 910, 620, 550, 410, 430, 100];
        let promotion_bonus: [i32; 8] = [0,  600, 450,   0,  70, 210, 190, 500];
        let hand_value:      [i32; 8] = [0, 1040, 910, 620, 550, 410, 430, 100];
        let pawn_advance_bonus: i32 = 8;

        let mut board_score = [[[[0i32; 81]; 2]; 8]; 2];

        for (side_idx, side_scores) in board_score.iter_mut().enumerate() {
            let side = PlayerSide::ALL[side_idx];
            for (kind_idx, kind_scores) in side_scores.iter_mut().enumerate() {
                let kind = PieceKind::ALL[kind_idx];
                for (promoted_idx, promo_scores) in kind_scores.iter_mut().enumerate() {
                    let promoted = promoted_idx == 1;
                    for sq_idx in 0..81u8 {
                        let square = Square::from_index(sq_idx).unwrap();
                        let mut val = piece_values[kind_idx];
                        if promoted {
                            val += promotion_bonus[kind_idx];
                        }
                        if kind == PieceKind::Pawn && !promoted {
                            val += Self::pawn_advancement(side, square, pawn_advance_bonus);
                        }
                        val += Self::pst_bonus(kind, promoted, side, square);
                        promo_scores[sq_idx as usize] = val;
                    }
                }
            }
        }

        Self {
            board_score,
            hand_value,
        }
    }

    /// Advancement bonus for unpromoted pawns: +bonus per rank advanced.
    fn pawn_advancement(side: PlayerSide, square: Square, bonus: i32) -> i32 {
        let rank = square.rank() as i32;
        let advancement = match side {
            PlayerSide::Sente => 8 - rank, // Sente advances toward rank 0
            PlayerSide::Gote => rank,      // Gote advances toward rank 8
        };
        advancement * bonus
    }

    /// Piece-square table bonus.  Uses "advancement level" (0 = own back rank,
    /// 8 = enemy back rank) so the same logic applies to both sides.
    fn pst_bonus(kind: PieceKind, promoted: bool, side: PlayerSide, square: Square) -> i32 {
        let rank = square.rank() as i32;
        let adv = match side {
            PlayerSide::Sente => 8 - rank,
            PlayerSide::Gote => rank,
        };
        let file = square.file() as i32;

        match kind {
            PieceKind::King => {
                if !promoted {
                    // Strongly prefer own territory; penalise adventurous kings.
                    let rank_bonus = match adv {
                        0 => 30,   // own back rank — ideal castle position
                        1 => 15,   // one step forward — acceptable
                        2 => 5,    // two steps — marginal
                        _ => -15,  // too far forward — exposed
                    };
                    // Corner files (0-1 or 7-8) are typical castle positions.
                    let file_bonus = if file <= 1 || file >= 7 { 10 } else { 0 };
                    rank_bonus + file_bonus
                } else {
                    0
                }
            }
            PieceKind::Rook => {
                // Rooks gain power as they advance into enemy territory.
                if !promoted { adv * 4 } else { adv * 2 }
            }
            PieceKind::Bishop => {
                if !promoted {
                    // Bishops thrive on central diagonals.
                    let center_file_bonus = (4 - (file - 4).abs()) * 3; // 0..12
                    let mid_rank_bonus = if (2..=6).contains(&adv) { 5 } else { 0 };
                    center_file_bonus + mid_rank_bonus
                } else {
                    adv * 2 // Horse — mild advancement preference
                }
            }
            PieceKind::Silver => {
                if !promoted { adv * 3 } else { adv * 2 }
            }
            PieceKind::Knight => {
                if !promoted {
                    // Knights can't retreat — deep advancement is very valuable.
                    if adv >= 6 { adv * 5 } else { adv * 3 }
                } else {
                    adv * 2
                }
            }
            PieceKind::Lance => {
                // Forward-only piece; advancement directly increases its range.
                if !promoted { adv * 4 } else { adv * 2 }
            }
            PieceKind::Gold => adv * 2, // Defensive piece — mild forward preference
            PieceKind::Pawn => 0,       // Handled by pawn_advancement() above
        }
    }
}
