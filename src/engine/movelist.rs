use std::mem::MaybeUninit;

use super::movement::Move;

/// Maximum number of moves that can be generated from any legal shogi position.
/// The theoretical maximum is ~593; 512 is a safe practical upper bound.
const MAX_MOVES: usize = 512;

/// A stack-allocated list of moves.  Avoids the heap allocation that
/// `Vec<Move>` incurs on every move-generation call — at ~500k+ nodes/sec
/// this eliminates millions of malloc/free pairs per second.
pub struct MoveList {
    data: [MaybeUninit<Move>; MAX_MOVES],
    len: usize,
}

impl MoveList {
    #[inline]
    pub fn new() -> Self {
        Self {
            // SAFETY: MaybeUninit doesn't require initialization.
            data: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < MAX_MOVES);
        self.data[self.len] = MaybeUninit::new(mv);
        self.len += 1;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn first(&self) -> Option<Move> {
        if self.len > 0 {
            Some(unsafe { self.data[0].assume_init() })
        } else {
            None
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        // SAFETY: elements 0..len are all initialized via push().
        unsafe { std::slice::from_raw_parts(self.data.as_ptr().cast(), self.len) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        unsafe { std::slice::from_raw_parts_mut(self.data.as_mut_ptr().cast(), self.len) }
    }

    pub fn contains(&self, mv: &Move) -> bool {
        self.as_slice().contains(mv)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Move> {
        self.as_slice().iter()
    }

    pub fn sort_by<F: FnMut(&Move, &Move) -> std::cmp::Ordering>(&mut self, compare: F) {
        self.as_mut_slice().sort_by(compare);
    }

    pub fn sort_by_key<K: Ord, F: FnMut(&Move) -> K>(&mut self, f: F) {
        self.as_mut_slice().sort_by_key(f);
    }

    /// Removes all elements that don't satisfy the predicate, compacting in place.
    pub fn retain<F: FnMut(&Move) -> bool>(&mut self, mut f: F) {
        let mut write = 0;
        for read in 0..self.len {
            let mv = unsafe { self.data[read].assume_init() };
            if f(&mv) {
                self.data[write] = MaybeUninit::new(mv);
                write += 1;
            }
        }
        self.len = write;
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Index<usize> for MoveList {
    type Output = Move;
    #[inline]
    fn index(&self, idx: usize) -> &Move {
        &self.as_slice()[idx]
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
