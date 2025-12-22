use super::super::{
    board::Board,
    hand::Hand,
    movegen::MoveGenerator,
    state::{PieceKind, PlayerSide, Square},
};

#[derive(Clone)]
pub struct MaterialEvaluator {
    piece_values: [i32; PieceKind::ALL.len()],
    promotion_bonus: [i32; PieceKind::ALL.len()],
    hand_values: [i32; PieceKind::ALL.len()],
    pawn_advance_bonus: i32,
    mobility_weight: i32,
}

impl MaterialEvaluator {
    pub fn new() -> Self {
        // Rough material scale based on common shogi heuristics.
        let piece_values = [
            0,   // King
            900, // Rook
            850, // Bishop
            600, // Gold
            500, // Silver
            350, // Knight
            300, // Lance
            100, // Pawn
        ];
        let promotion_bonus = [0, 200, 200, 0, 150, 150, 150, 100];
        let hand_values = [0, 900, 850, 600, 500, 350, 300, 100];
        Self {
            piece_values,
            promotion_bonus,
            hand_values,
            pawn_advance_bonus: 8,
            mobility_weight: 2,
        }
    }

    pub fn evaluate(&self, board: &Board, perspective: PlayerSide) -> i32 {
        let mut score = 0;
        for &side in &PlayerSide::ALL {
            let sign = if side == perspective { 1 } else { -1 };
            for &kind in &PieceKind::ALL {
                let promoted_bb = board.bitboards().promoted(side, kind);
                for square in board.bitboards().piece(side, kind).iter_bits() {
                    let promoted = promoted_bb.is_set(square);
                    let mut piece_score = self.value_piece(kind, promoted);
                    if kind == PieceKind::Pawn && !promoted {
                        piece_score += self.pawn_advancement_value(side, square);
                    }
                    score += sign * piece_score;
                }
            }
            score += sign * self.value_hand(board.hand(side));
        }
        let our_mobility = MoveGenerator::pseudo_legal_moves(board, perspective).len() as i32;
        let opp_mobility =
            MoveGenerator::pseudo_legal_moves(board, perspective.opponent()).len() as i32;
        score += self.mobility_weight * (our_mobility - opp_mobility);
        score
    }

    fn value_piece(&self, kind: PieceKind, promoted: bool) -> i32 {
        let base = self.piece_values[kind.index()];
        if promoted {
            base + self.promotion_bonus[kind.index()]
        } else {
            base
        }
    }

    fn pawn_advancement_value(&self, side: PlayerSide, square: Square) -> i32 {
        let rank = square.rank() as i32;
        let advancement = match side {
            PlayerSide::Sente => 8 - rank,
            PlayerSide::Gote => rank,
        };
        advancement * self.pawn_advance_bonus
    }

    fn value_hand(&self, hand: Hand) -> i32 {
        let mut total = 0;
        for &kind in &PieceKind::ALL {
            let count = hand.count(kind) as i32;
            total += count * self.hand_values[kind.index()];
        }
        total
    }
}

impl Default for MaterialEvaluator {
    fn default() -> Self {
        Self::new()
    }
}
