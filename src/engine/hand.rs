use super::state::PieceKind;

/// Captured pieces in hand represented as packed 4-bit counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hand(u32);

const BITS_PER_PIECE: u32 = 4;
const MAX_COUNT: u32 = (1 << BITS_PER_PIECE) - 1;

impl Hand {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn raw_bits(self) -> u32 {
        self.0
    }

    pub fn count(self, kind: PieceKind) -> u8 {
        let offset = kind.index() as u32 * BITS_PER_PIECE;
        ((self.0 >> offset) & MAX_COUNT) as u8
    }

    pub fn add(&mut self, kind: PieceKind) {
        let offset = kind.index() as u32 * BITS_PER_PIECE;
        let current = (self.0 >> offset) & MAX_COUNT;
        let next = (current + 1).min(MAX_COUNT);
        self.0 &= !(MAX_COUNT << offset);
        self.0 |= next << offset;
    }

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
