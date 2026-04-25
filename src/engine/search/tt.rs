// Concurrent Transposition Table
// ===============================
//
// Fixed-size lock-free TT using Hyatt's XOR verification trick:
//   * each bucket stores two u64s — `key_xor_data` and `data`;
//   * on store, `key_xor_data = zobrist ^ data`;
//   * on probe, a caller computes `stored_key ^ data` and compares against
//     the zobrist it is looking for.  A torn read (two halves written by
//     different stores interleaving) simply fails the check and presents
//     as a miss — the TT is a hint, never a source of truth, so this is
//     safe for correctness.
//
// All buckets are allocated once at construction.  The bucket count is a
// power of two, so index = `zobrist & (buckets - 1)`.  Replacement is
// always-replace — simple, and within 5-10 Elo of more complex schemes
// (depth-preferred, two-deep, etc.) at our TT size.
//
// The empty-bucket sentinel is `data == 0`.  The caller never stores an
// entry with `depth == 0` (the search routes depth-0 calls to quiescence
// before the TT-store site), so legitimate stores always have the low byte
// non-zero and are distinguishable from the all-zero initial state.
//
// Thread-safety: the `Vec<Bucket>` is never resized after construction;
// each bucket is a pair of `AtomicU64`s.  `ConcurrentTT: Send + Sync` is
// therefore automatic and the type can be shared between search threads
// behind an `Arc`.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::{
    movement::{Move, MoveKind},
    state::{PieceKind, PlayerSide, Square},
};

/// Classification of a stored score relative to the search window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TTFlag {
    Exact,
    LowerBound,
    UpperBound,
}

/// Decoded view of a TT bucket.
#[derive(Clone, Copy, Debug)]
pub struct TtEntry {
    pub depth: u8,
    pub score: i32,
    pub flag: TTFlag,
    pub best_move: Option<Move>,
}

struct Bucket {
    key_xor_data: AtomicU64,
    data: AtomicU64,
}

pub struct ConcurrentTT {
    buckets: Vec<Bucket>,
    mask: u64,
    capacity: usize,
}

impl ConcurrentTT {
    /// Allocates a TT with at least `min_entries` slots, rounded up to the
    /// next power of two.
    pub fn new(min_entries: usize) -> Self {
        let size = min_entries.next_power_of_two().max(2);
        let mut buckets = Vec::with_capacity(size);
        for _ in 0..size {
            buckets.push(Bucket {
                key_xor_data: AtomicU64::new(0),
                data: AtomicU64::new(0),
            });
        }
        Self {
            buckets,
            mask: (size - 1) as u64,
            capacity: size,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Erases every bucket.  Cheap enough between searches when needed,
    /// though the engine normally preserves the TT across moves to carry
    /// learned knowledge forward.
    pub fn clear(&self) {
        for b in &self.buckets {
            b.data.store(0, Ordering::Relaxed);
            b.key_xor_data.store(0, Ordering::Relaxed);
        }
    }

    /// USI-style hashfull estimate (per-mille): samples the first 1000
    /// buckets and reports how many carry a non-empty entry.
    pub fn hashfull(&self) -> u64 {
        let sample = 1000.min(self.buckets.len());
        if sample == 0 {
            return 0;
        }
        let mut filled = 0u64;
        for i in 0..sample {
            if self.buckets[i].data.load(Ordering::Relaxed) != 0 {
                filled += 1;
            }
        }
        filled * 1000 / sample as u64
    }

    pub fn probe(&self, zobrist: u64) -> Option<TtEntry> {
        let idx = (zobrist & self.mask) as usize;
        let b = &self.buckets[idx];
        // Read `key_xor_data` first, then `data`: the store writes them in
        // the reverse order, so this ordering means that a concurrent store
        // can either (a) be fully visible, (b) be entirely invisible, or
        // (c) tear such that `stored ^ data != zobrist` and we miss.
        let stored = b.key_xor_data.load(Ordering::Relaxed);
        let data = b.data.load(Ordering::Relaxed);
        if data == 0 {
            return None;
        }
        if stored ^ data != zobrist {
            return None;
        }
        decode(data)
    }

    pub fn store(&self, zobrist: u64, entry: TtEntry) {
        let idx = (zobrist & self.mask) as usize;
        let b = &self.buckets[idx];
        let data = encode(&entry);
        // Write `data` first, then `key_xor_data`.  This pairs with the
        // probe's load ordering so torn reads fail the XOR check.
        b.data.store(data, Ordering::Relaxed);
        b.key_xor_data.store(zobrist ^ data, Ordering::Relaxed);
    }
}

// --- Packed encoding -------------------------------------------------------
//
// u64 layout (LSB first):
//   0..8    depth              u8
//   8..24   score              i16 (cast via u16)
//   24..26  flag               0=Exact, 1=Lower, 2=Upper
//   26      has_move           1 when best_move is Some
//   27..29  move kind          0=Quiet, 1=Capture, 2=Drop
//   29      move player        0=Sente, 1=Gote
//   30      move promote
//   31..34  move piece         PieceKind index (0-7)
//   34..38  move capture       PieceKind index, 0xF when None
//   38..46  move from          Square index (0-80), 0xFF when None (drop)
//   46..53  move to            Square index (0-80)
//   53..64  reserved (zero)
//
// Score is stored as i16 because the search never uses scores outside
// ±MATE_SCORE (30000), which comfortably fits.

fn encode(entry: &TtEntry) -> u64 {
    let mut data: u64 = 0;
    data |= entry.depth as u64;
    data |= (entry.score as i16 as u16 as u64) << 8;
    let flag_bits: u64 = match entry.flag {
        TTFlag::Exact => 0,
        TTFlag::LowerBound => 1,
        TTFlag::UpperBound => 2,
    };
    data |= flag_bits << 24;
    if let Some(mv) = entry.best_move {
        data |= 1u64 << 26;
        let kind_bits: u64 = match mv.kind {
            MoveKind::Quiet => 0,
            MoveKind::Capture => 1,
            MoveKind::Drop => 2,
        };
        data |= kind_bits << 27;
        data |= (mv.player as u64) << 29;
        data |= (mv.promote as u64) << 30;
        data |= (mv.piece.index() as u64) << 31;
        let cap = mv.capture.map(|c| c.index() as u64).unwrap_or(0xF);
        data |= cap << 34;
        let from = mv.from.map(|s| s.index() as u64).unwrap_or(0xFF);
        data |= from << 38;
        data |= (mv.to.index() as u64) << 46;
    }
    data
}

fn decode(data: u64) -> Option<TtEntry> {
    let depth = (data & 0xFF) as u8;
    if depth == 0 {
        return None;
    }
    let score = ((data >> 8) & 0xFFFF) as u16 as i16 as i32;
    let flag = match (data >> 24) & 0x3 {
        0 => TTFlag::Exact,
        1 => TTFlag::LowerBound,
        2 => TTFlag::UpperBound,
        _ => return None,
    };
    let best_move = if (data >> 26) & 1 == 1 {
        let kind = match (data >> 27) & 0x3 {
            0 => MoveKind::Quiet,
            1 => MoveKind::Capture,
            2 => MoveKind::Drop,
            _ => return None,
        };
        let player = if (data >> 29) & 1 == 0 {
            PlayerSide::Sente
        } else {
            PlayerSide::Gote
        };
        let promote = (data >> 30) & 1 == 1;
        let piece_idx = ((data >> 31) & 0x7) as usize;
        let piece = *PieceKind::ALL.get(piece_idx)?;
        let cap_raw = (data >> 34) & 0xF;
        let capture = if cap_raw == 0xF {
            None
        } else {
            Some(*PieceKind::ALL.get(cap_raw as usize)?)
        };
        let from_raw = (data >> 38) & 0xFF;
        let from = if from_raw == 0xFF {
            None
        } else {
            Square::from_index(from_raw as u8)
        };
        let to_raw = (data >> 46) & 0x7F;
        let to = Square::from_index(to_raw as u8)?;
        Some(Move {
            player,
            from,
            to,
            piece,
            capture,
            promote,
            kind,
        })
    } else {
        None
    };
    Some(TtEntry {
        depth,
        score,
        flag,
        best_move,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_without_move() {
        let tt = ConcurrentTT::new(256);
        let entry = TtEntry {
            depth: 7,
            score: -1234,
            flag: TTFlag::UpperBound,
            best_move: None,
        };
        tt.store(0xdead_beef_cafe_f00d, entry);
        let got = tt.probe(0xdead_beef_cafe_f00d).unwrap();
        assert_eq!(got.depth, 7);
        assert_eq!(got.score, -1234);
        assert_eq!(got.flag, TTFlag::UpperBound);
        assert!(got.best_move.is_none());
    }

    #[test]
    fn round_trip_with_drop_move() {
        let tt = ConcurrentTT::new(256);
        let mv = Move::drop(
            PlayerSide::Gote,
            PieceKind::Pawn,
            Square::from_file_rank(5, 5).unwrap(),
        );
        tt.store(
            0x1234,
            TtEntry {
                depth: 12,
                score: 250,
                flag: TTFlag::Exact,
                best_move: Some(mv),
            },
        );
        let got = tt.probe(0x1234).unwrap();
        assert_eq!(got.best_move, Some(mv));
    }

    #[test]
    fn round_trip_with_capture_move() {
        let tt = ConcurrentTT::new(256);
        let mv = Move::normal(
            PlayerSide::Sente,
            Square::from_file_rank(7, 7).unwrap(),
            Square::from_file_rank(7, 6).unwrap(),
            PieceKind::Pawn,
            Some(PieceKind::Rook),
            true,
        );
        tt.store(
            0x9876_5432,
            TtEntry {
                depth: 20,
                score: 25_000, // near mate
                flag: TTFlag::LowerBound,
                best_move: Some(mv),
            },
        );
        let got = tt.probe(0x9876_5432).unwrap();
        assert_eq!(got.best_move, Some(mv));
        assert_eq!(got.score, 25_000);
    }

    #[test]
    fn miss_on_empty() {
        let tt = ConcurrentTT::new(256);
        assert!(tt.probe(0x1234).is_none());
    }

    #[test]
    fn miss_on_collision() {
        let tt = ConcurrentTT::new(256);
        tt.store(
            0xAAAA,
            TtEntry {
                depth: 1,
                score: 0,
                flag: TTFlag::Exact,
                best_move: None,
            },
        );
        // Pick a key that maps to the same bucket (same low bits, different high).
        let other = 0xAAAA | (1u64 << 40);
        assert!(tt.probe(other).is_none());
    }
}
