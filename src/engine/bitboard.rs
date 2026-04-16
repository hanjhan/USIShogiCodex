use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

use super::state::Square;

// A shogi board has 9x9 = 81 squares, which fits comfortably inside a u128 (128 bits).
// Each bit position corresponds to Square::index(), i.e. rank*9 + file.
// Using a bitmask for each (side, piece_kind) pair makes move generation and
// occupancy queries very fast via bitwise operations.

/// A 128-bit bitboard representing a set of squares on the 9×9 shogi board.
/// Bit N is set if the square with index N is in the set.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Bitboard(u128);

/// Only the lowest 81 bits are valid board positions.  All operations that
/// could produce bits outside this range must mask with BOARD_MASK.
const BOARD_MASK: u128 = (1u128 << 81) - 1;

impl Bitboard {
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns a bitboard with all 81 squares set.
    pub const fn full() -> Self {
        Self(BOARD_MASK)
    }

    /// Returns the raw 128-bit integer (only bits 0-80 are meaningful).
    pub fn bits(self) -> u128 {
        self.0
    }

    /// Creates a bitboard with only the given square set.
    pub fn from_square(square: Square) -> Self {
        Self(1u128 << square.index())
    }

    /// Sets the bit for `square`.
    pub fn set(&mut self, square: Square) {
        self.0 |= 1u128 << square.index();
    }

    /// Clears the bit for `square`.
    pub fn clear(&mut self, square: Square) {
        self.0 &= !(1u128 << square.index());
    }

    /// Returns true if the bit for `square` is set.
    pub fn is_set(self, square: Square) -> bool {
        (self.0 >> square.index()) & 1 == 1
    }

    /// Returns the number of set bits (popcount).
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns an iterator that yields each set square in LSB-first order.
    pub fn iter_bits(self) -> BitIter {
        BitIter(self.0)
    }
}

// Standard bitwise operators; NOT is masked to stay within 81 bits.

impl BitOr for Bitboard {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Bitboard {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for Bitboard {
    type Output = Self;

    fn not(self) -> Self::Output {
        // Mask to prevent bits 81-127 from being set after inversion.
        Self((!self.0) & BOARD_MASK)
    }
}

impl fmt::Debug for Bitboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Bitboard")
            .field(&format!("{:0128b}", self.0))
            .finish()
    }
}

/// Iterator over the set squares of a `Bitboard`, yielding them in LSB-first
/// (low-index square first) order.  Uses the standard "clear lowest set bit"
/// trick: `x & (x - 1)` removes the LSB in O(1).
pub struct BitIter(u128);

impl Iterator for BitIter {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        // trailing_zeros() gives the index of the lowest set bit.
        let lsb = self.0.trailing_zeros() as u8;
        // Clear the lowest set bit so the next call advances to the next square.
        self.0 &= self.0 - 1;
        Square::from_index(lsb)
    }
}
