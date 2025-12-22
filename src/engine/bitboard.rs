use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

use super::state::Square;

/// Wrapper around a 128-bit integer to represent a shogi bitboard.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Bitboard(u128);

const BOARD_MASK: u128 = (1u128 << 81) - 1;

impl Bitboard {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn full() -> Self {
        Self(BOARD_MASK)
    }

    pub fn bits(self) -> u128 {
        self.0
    }

    pub fn from_square(square: Square) -> Self {
        Self(1u128 << square.index())
    }

    pub fn set(&mut self, square: Square) {
        self.0 |= 1u128 << square.index();
    }

    pub fn clear(&mut self, square: Square) {
        self.0 &= !(1u128 << square.index());
    }

    pub fn is_set(self, square: Square) -> bool {
        (self.0 >> square.index()) & 1 == 1
    }

    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter_bits(self) -> BitIter {
        BitIter(self.0)
    }
}

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

pub struct BitIter(u128);

impl Iterator for BitIter {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        let lsb = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Square::from_index(lsb)
    }
}
