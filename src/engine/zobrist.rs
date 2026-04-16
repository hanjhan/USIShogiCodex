use std::sync::OnceLock;

use super::state::PieceKind;

// Zobrist hashing
// ===============
// Each distinct board-state component (a piece at a square, a piece in hand,
// side-to-move) is assigned a fixed random u64.  The hash of a position is
// the XOR of the keys for every component present.  XOR is self-inverse, so
// a move can be applied to the hash by XORing out the keys that leave and
// XORing in the keys that arrive — no need to recompute from scratch.
//
// The table is generated deterministically via splitmix64 so the same binary
// produces the same hashes every run (useful for reproducibility / testing).

const NUM_SIDES: usize = 2;
const NUM_KINDS: usize = PieceKind::ALL.len();
const NUM_SQUARES: usize = 81;
/// Hand counts are stored in 4 bits (max 15).  16 slots are sufficient.
pub const MAX_HAND_SLOTS: usize = 16;

pub struct ZobristTable {
    /// pieces[side][kind][promoted as usize][square] — XOR when a piece of
    /// (side, kind, promoted) enters/leaves `square`.
    pub pieces: [[[[u64; NUM_SQUARES]; 2]; NUM_KINDS]; NUM_SIDES],
    /// hands[side][kind][slot] — XOR when the (slot+1)-th piece of this
    /// (side, kind) is added to hand.  Equivalently, a hand with count c
    /// contributes `XOR over slot=0..c` of these keys.
    pub hands: [[[u64; MAX_HAND_SLOTS]; NUM_KINDS]; NUM_SIDES],
    /// Toggled when it is Gote to move.  (Sente-to-move is the zero state.)
    pub side_to_move: u64,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

static ZOBRIST: OnceLock<ZobristTable> = OnceLock::new();

pub fn zobrist() -> &'static ZobristTable {
    ZOBRIST.get_or_init(|| {
        let mut state: u64 = 0xDEADBEEF_CAFEBABE;
        let mut pieces = [[[[0u64; NUM_SQUARES]; 2]; NUM_KINDS]; NUM_SIDES];
        for side in &mut pieces {
            for kind in side.iter_mut() {
                for promoted in kind.iter_mut() {
                    for sq in promoted.iter_mut() {
                        *sq = splitmix64(&mut state);
                    }
                }
            }
        }
        let mut hands = [[[0u64; MAX_HAND_SLOTS]; NUM_KINDS]; NUM_SIDES];
        for side in &mut hands {
            for kind in side.iter_mut() {
                for slot in kind.iter_mut() {
                    *slot = splitmix64(&mut state);
                }
            }
        }
        let side_to_move = splitmix64(&mut state);
        ZobristTable {
            pieces,
            hands,
            side_to_move,
        }
    })
}
