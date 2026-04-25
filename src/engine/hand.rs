use super::state::PieceKind;

// In shogi, captured pieces go to the capturing player's "hand" and can later
// be dropped back onto the board.  A player can hold multiple copies of the
// same piece type (e.g. several pawns captured over many turns).
//
// We pack the count for each piece type into a single u32 using 4 bits per
// piece.  With 8 piece types this takes only 32 bits total, so the whole hand
// fits in a register and can be hashed cheaply.
//
// Bit layout (4 bits each, from LSB):
//   [3:0]   King   (index 0) — never actually held; reserved for uniform indexing
//   [7:4]   Rook   (index 1)
//   [11:8]  Bishop (index 2)
//   [15:12] Gold   (index 3)
//   [19:16] Silver (index 4)
//   [23:20] Knight (index 5)
//   [27:24] Lance  (index 6)
//   [31:28] Pawn   (index 7)

/// Captured pieces in hand, stored as packed 4-bit counters in a single u32.
/// Maximum count per piece type is 15 (2^4 - 1), which far exceeds the 18
/// pawns on a standard shogi board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hand(u32);

/// Number of bits used to store the count of each individual piece type.
const BITS_PER_PIECE: u32 = 4;
/// Bitmask for a single 4-bit field (value 0–15).
const MAX_COUNT: u32 = (1 << BITS_PER_PIECE) - 1;

impl Hand {
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns the raw packed u32 (used for hashing / equality).
    pub fn raw_bits(self) -> u32 {
        self.0
    }

    /// Returns how many pieces of `kind` are currently in this hand.
    pub fn count(self, kind: PieceKind) -> u8 {
        let offset = kind.index() as u32 * BITS_PER_PIECE;
        ((self.0 >> offset) & MAX_COUNT) as u8
    }

    /// Adds one piece of `kind` to the hand (capped at 15 to prevent overflow).
    pub fn add(&mut self, kind: PieceKind) {
        let offset = kind.index() as u32 * BITS_PER_PIECE;
        let current = (self.0 >> offset) & MAX_COUNT;
        let next = (current + 1).min(MAX_COUNT);
        // Zero out the 4-bit field, then write the new value.
        self.0 &= !(MAX_COUNT << offset);
        self.0 |= next << offset;
    }

    /// Removes one piece of `kind` from the hand.  Returns false if the hand
    /// has none of that piece (should not happen in a legal game).
    pub fn remove(&mut self, kind: PieceKind) -> bool {
        let offset = kind.index() as u32 * BITS_PER_PIECE;
        let current = (self.0 >> offset) & MAX_COUNT;
        if current == 0 {
            return false;
        }
        let next = current - 1;
        self.0 &= !(MAX_COUNT << offset);
        self.0 |= next << offset;
        true
    }
}

impl Default for Hand {
    fn default() -> Self {
        Hand::empty()
    }
}
