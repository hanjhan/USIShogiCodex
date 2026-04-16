// =============================================================================
// Static Evaluator
// =============================================================================
//
// Evaluates a board position and returns a score in centipawn-like units.
// Positive = good for the given perspective, negative = bad.
//
// ## Architecture: Incremental + Dynamic Components
//
// Most of the evaluation is **incremental** — maintained inside the Board
// struct and updated automatically on every make_move() / undo_move():
//   - Material values, promotion bonuses, PST, pawn advancement, hand material
//   - Stored as `board.eval_score[side]` — see `eval_tables.rs`
//
// The evaluator adds **dynamic components** that depend on multi-piece
// relationships and are too complex to maintain incrementally:
//
//   1. **King safety (defense)** — Gold/Silver defenders near own king.
//   2. **King threat (attack)** — our pieces near the opponent's king,
//      weighted by piece type and count.  Multiple attackers compound
//      non-linearly (the "attack scaling table").
//   3. **Drop threat amplifier** — pieces in hand multiply attack potential
//      because they can be dropped as reinforcements anywhere.
//
// =============================================================================

use super::super::{
    board::Board,
    state::{PieceKind, PlayerSide},
};

// Attack weight per piece type when near the opponent's king.
// Higher = more dangerous as an attacker.
const ATTACK_WEIGHT: [i32; 8] = [
    0,   // King    — not counted as attacker
    50,  // Rook    — very dangerous near enemy king
    40,  // Bishop  — strong diagonal pressure
    25,  // Gold    — solid close-range attacker
    20,  // Silver  — good forward attacker
    15,  // Knight  — useful but limited movement
    10,  // Lance   — forward-only, less flexible
    5,   // Pawn    — weak but can promote to gold
];

// Non-linear scaling table: maps total attack weight to an actual bonus.
// Having one piece near the king is worth little; having 3+ pieces
// creates a decisive attack.  The table rewards concentration of force.
//
// Index = total_attack_weight / 10 (capped at table length).
// Values rise steeply after 3-4 attackers accumulate.
const ATTACK_SCALE: [i32; 16] = [
    0, 0, 5, 15, 30, 50, 75, 105, 140, 180, 225, 270, 320, 370, 420, 470,
];

// Bonus per piece in hand when we have attackers near the enemy king.
// Pieces in hand are potent threats because they can be dropped as
// reinforcements right next to the king.
const DROP_THREAT_PER_PIECE: i32 = 8;

#[derive(Clone)]
pub struct MaterialEvaluator {
    king_defender_bonus: [i32; 3],
}

impl MaterialEvaluator {
    pub fn new() -> Self {
        Self {
            king_defender_bonus: [0, 40, 15],
        }
    }

    /// Returns a score from `perspective`'s point of view.
    pub fn evaluate(&self, board: &Board, perspective: PlayerSide) -> i32 {
        let opponent = perspective.opponent();
        let base = board.eval_score(perspective) - board.eval_score(opponent);
        let defense = self.king_defense(board, perspective)
            - self.king_defense(board, opponent);
        let attack = self.king_threat(board, perspective)
            - self.king_threat(board, opponent);
        base + defense + attack
    }

    /// Defense: bonus for Gold/Silver defenders near own king, penalty for
    /// exposed king in enemy territory.
    fn king_defense(&self, board: &Board, side: PlayerSide) -> i32 {
        let king_sq = match board.king_square(side) {
            Some(sq) => sq,
            None => return 0,
        };
        let kf = king_sq.file() as i32;
        let kr = king_sq.rank() as i32;
        let mut bonus = 0;

        for &kind in &[PieceKind::Gold, PieceKind::Silver] {
            for sq in board.bitboards().piece(side, kind).iter_bits() {
                let dist = (sq.file() as i32 - kf).abs().max((sq.rank() as i32 - kr).abs());
                if dist == 1 {
                    bonus += self.king_defender_bonus[1];
                } else if dist == 2 {
                    bonus += self.king_defender_bonus[2];
                }
            }
        }

        let in_enemy_territory = match side {
            PlayerSide::Sente => kr <= 2,
            PlayerSide::Gote => kr >= 6,
        };
        if in_enemy_territory {
            bonus -= 60;
        }

        bonus
    }

    /// Attack: bonus for having our pieces near the opponent's king.
    /// Multiple attackers compound non-linearly via the scaling table.
    /// Pieces in hand amplify the threat (drop reinforcements).
    fn king_threat(&self, board: &Board, attacker: PlayerSide) -> i32 {
        let defender = attacker.opponent();
        let king_sq = match board.king_square(defender) {
            Some(sq) => sq,
            None => return 0,
        };
        let kf = king_sq.file() as i32;
        let kr = king_sq.rank() as i32;

        // Sum up attack weight from all our pieces near the enemy king.
        let mut total_weight: i32 = 0;

        for &kind in &PieceKind::ALL {
            if kind == PieceKind::King {
                continue;
            }
            let weight = ATTACK_WEIGHT[kind.index()];
            if weight == 0 {
                continue;
            }
            for sq in board.bitboards().piece(attacker, kind).iter_bits() {
                let dist = (sq.file() as i32 - kf).abs().max((sq.rank() as i32 - kr).abs());
                // Distance 1: full weight (adjacent to king)
                // Distance 2: half weight (one step away from adjacent)
                // Distance 3+: not counted
                if dist == 1 {
                    total_weight += weight;
                } else if dist == 2 {
                    total_weight += weight / 2;
                }
            }
        }

        // Look up the non-linear scaling table.
        let idx = (total_weight / 10).min(ATTACK_SCALE.len() as i32 - 1) as usize;
        let mut attack_score = ATTACK_SCALE[idx];

        // Pieces in hand amplify attack when we have attackers near the king.
        // The drop threat is only meaningful if we actually have attackers.
        if total_weight >= 20 {
            let hand = board.hand(attacker);
            let mut hand_count: i32 = 0;
            for &kind in &PieceKind::ALL {
                if kind == PieceKind::King {
                    continue;
                }
                hand_count += hand.count(kind) as i32;
            }
            attack_score += hand_count * DROP_THREAT_PER_PIECE;
        }

        attack_score
    }
}

impl Default for MaterialEvaluator {
    fn default() -> Self {
        Self::new()
    }
}
