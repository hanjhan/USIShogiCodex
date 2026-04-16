use super::{
    bitboard::Bitboard,
    eval_tables::eval_table,
    hand::Hand,
    movement::{Move, MoveKind},
    state::{Piece, PieceKind, PlayerSide, Square},
    zobrist::{MAX_HAND_SLOTS, zobrist},
};

// The board representation uses a set of bitboards (one per (side, piece_kind)
// pair) plus a separate "promoted" bitboard for each (side, piece_kind) that
// indicates which squares hold a promoted version.  This means a single piece
// on the board is tracked in two places:
//   1. `pieces[side][kind]`    — which square it occupies
//   2. `promoted[side][kind]`  — whether it is in promoted form
//
// `occupancy[side]` is a union of all piece bitboards for that side; it is
// maintained redundantly for fast "is this square occupied?" checks.
//
// `PositionSignature` is a lightweight snapshot of the full position (pieces,
// promoted flags, hands, side to move) that can be hashed and compared for:
//   - Repetition detection (sennichite) in `GameController`
//   - Transposition table keys in `AlphaBetaSearcher`

/// State saved by `make_move` that is needed to reverse the move in `undo_move`.
#[derive(Clone, Copy, Debug)]
pub struct UndoInfo {
    pub captured_was_promoted: bool,
    pub from_was_promoted: bool,
}

/// A hashable snapshot of the board position used for repetition detection
/// and as a transposition table key.  It captures everything that defines a
/// unique position: piece placement, promotion flags, hands, and side to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PositionSignature {
    pub side_to_move: PlayerSide,
    pieces: [[Bitboard; PieceKind::ALL.len()]; 2],
    promoted: [[Bitboard; PieceKind::ALL.len()]; 2],
    hands: [Hand; 2],
}

/// All bitboards for both sides: occupancy, piece placement, and promotion flags.
#[derive(Clone)]
pub struct PieceBitboards {
    /// Union of all piece bitboards per side; used for fast occupancy queries.
    occupancy: [Bitboard; 2],
    /// `pieces[side][kind]` — squares occupied by unpromoted or promoted pieces
    /// of this (side, kind) pair.
    pieces: [[Bitboard; PieceKind::ALL.len()]; 2],
    /// `promoted[side][kind]` — subset of `pieces[side][kind]` that are promoted.
    promoted: [[Bitboard; PieceKind::ALL.len()]; 2],
}

impl PieceBitboards {
    pub fn new() -> Self {
        Self {
            occupancy: [Bitboard::empty(), Bitboard::empty()],
            pieces: [[Bitboard::empty(); PieceKind::ALL.len()]; 2],
            promoted: [[Bitboard::empty(); PieceKind::ALL.len()]; 2],
        }
    }

    /// Places a piece of `(side, kind)` on `square`, setting its promoted flag.
    pub fn place(&mut self, side: PlayerSide, kind: PieceKind, square: Square, promoted: bool) {
        let idx = side.index();
        self.occupancy[idx].set(square);
        self.pieces[idx][kind.index()].set(square);
        if promoted {
            self.promoted[idx][kind.index()].set(square);
        } else {
            // Explicitly clear the promoted bit in case a promoted piece is
            // being replaced with an unpromoted one (e.g. after an undo).
            self.promoted[idx][kind.index()].clear(square);
        }
    }

    /// Removes the piece of `(side, kind)` from `square`.  The caller is
    /// responsible for ensuring the piece is actually there.
    pub fn remove(&mut self, side: PlayerSide, kind: PieceKind, square: Square) {
        let idx = side.index();
        self.occupancy[idx].clear(square);
        self.pieces[idx][kind.index()].clear(square);
        self.promoted[idx][kind.index()].clear(square);
    }

    /// Returns the bitboard of all squares occupied by `(side, kind)` pieces.
    pub fn piece(&self, side: PlayerSide, kind: PieceKind) -> Bitboard {
        self.pieces[side.index()][kind.index()]
    }

    /// Returns the bitboard of promoted `(side, kind)` pieces (subset of `piece()`).
    pub fn promoted(&self, side: PlayerSide, kind: PieceKind) -> Bitboard {
        self.promoted[side.index()][kind.index()]
    }

    /// Returns the occupancy bitboard (all pieces) for `side`.
    pub fn occupancy(&self, side: PlayerSide) -> Bitboard {
        self.occupancy[side.index()]
    }

    /// Returns the occupancy bitboard for both sides combined.
    pub fn occupancy_all(&self) -> Bitboard {
        self.occupancy[0] | self.occupancy[1]
    }
}

impl Default for PieceBitboards {
    fn default() -> Self {
        Self::new()
    }
}

/// The complete game state: piece placement, captured pieces in hand, and
/// the side whose turn it is to move.
#[derive(Clone)]
pub struct Board {
    bitboards: PieceBitboards,
    hands: [Hand; 2],
    side_to_move: PlayerSide,
    /// Half-move counter (incremented on every `apply_move` call).
    ply: u32,
    /// Incrementally updated Zobrist hash of the full position.  The zero
    /// value corresponds to an empty board with Sente to move; every piece
    /// placement, hand change, and side-to-move flip XORs the relevant key.
    zobrist: u64,
    /// Incrementally maintained material+PST score per side.  Updated in
    /// make_move/undo_move/apply_move.  Avoids the full board scan in evaluate().
    eval_score: [i32; 2],
    /// Cached pinned-piece bitboard, set by MoveGenerator before legal move
    /// generation.  Used by push_move to skip is_in_check for non-pinned moves.
    pinned: Bitboard,
    /// True when the side to move is in check.  When in check, the pin
    /// shortcut is disabled — all moves must resolve the check.
    in_check_cached: bool,
}

impl Board {
    /// Creates a standard starting position with all pieces in their initial
    /// squares and Sente to move.
    pub fn new_standard() -> Self {
        let mut board = Self {
            bitboards: PieceBitboards::new(),
            hands: [Hand::default(), Hand::default()],
            side_to_move: PlayerSide::Sente,
            ply: 0,
            zobrist: 0,
            eval_score: [0, 0],
            pinned: Bitboard::empty(),
            in_check_cached: false,
        };
        board.setup_standard();
        board
    }

    /// Places all pieces in the standard shogi opening position.
    fn setup_standard(&mut self) {
        // Back rank piece order (left to right from each player's perspective):
        // Lance, Knight, Silver, Gold, King, Gold, Silver, Knight, Lance
        const BACK_RANK: [PieceKind; 9] = [
            PieceKind::Lance,
            PieceKind::Knight,
            PieceKind::Silver,
            PieceKind::Gold,
            PieceKind::King,
            PieceKind::Gold,
            PieceKind::Silver,
            PieceKind::Knight,
            PieceKind::Lance,
        ];
        // Gote's second rank: Rook on file 1 (index 1), Bishop on file 7 (index 7)
        const GOTE_SECOND: [Option<PieceKind>; 9] = [
            None,
            Some(PieceKind::Rook),
            None,
            None,
            None,
            None,
            None,
            Some(PieceKind::Bishop),
            None,
        ];
        // Sente's second rank: Bishop on file 1, Rook on file 7
        const SENTE_SECOND: [Option<PieceKind>; 9] = [
            None,
            Some(PieceKind::Bishop),
            None,
            None,
            None,
            None,
            None,
            Some(PieceKind::Rook),
            None,
        ];

        self.clear();

        // Place back-rank pieces for both sides (rank 0 = Gote, rank 8 = Sente)
        for (file, kind) in BACK_RANK.iter().enumerate() {
            let top = Square::from_coords(file as u8, 0).expect("valid square");
            let bottom = Square::from_coords(file as u8, 8).expect("valid square");
            self.place_piece(PlayerSide::Gote, *kind, top, false);
            self.place_piece(PlayerSide::Sente, *kind, bottom, false);
        }

        // Second-rank pieces (Rook/Bishop positions differ between sides)
        for (file, maybe_kind) in GOTE_SECOND.iter().enumerate() {
            if let Some(kind) = maybe_kind {
                let square = Square::from_coords(file as u8, 1).expect("valid square");
                self.place_piece(PlayerSide::Gote, *kind, square, false);
            }
        }
        for (file, maybe_kind) in SENTE_SECOND.iter().enumerate() {
            if let Some(kind) = maybe_kind {
                let square = Square::from_coords(file as u8, 7).expect("valid square");
                self.place_piece(PlayerSide::Sente, *kind, square, false);
            }
        }

        // Nine pawns per side: Gote on rank 2, Sente on rank 6
        for file in 0..9 {
            let gote_pawn = Square::from_coords(file, 2).expect("valid square");
            let sente_pawn = Square::from_coords(file, 6).expect("valid square");
            self.place_piece(PlayerSide::Gote, PieceKind::Pawn, gote_pawn, false);
            self.place_piece(PlayerSide::Sente, PieceKind::Pawn, sente_pawn, false);
        }

        self.recompute_zobrist();
    }

    /// Resets the board to an empty state (no pieces, empty hands, Sente to move).
    pub fn clear(&mut self) {
        self.bitboards = PieceBitboards::new();
        self.hands = [Hand::default(), Hand::default()];
        self.side_to_move = PlayerSide::Sente;
        self.ply = 0;
        self.zobrist = 0;
        self.eval_score = [0, 0];
        self.pinned = Bitboard::empty();
        self.in_check_cached = false;
    }

    #[inline]
    pub fn zobrist(&self) -> u64 {
        self.zobrist
    }

    #[inline]
    pub fn eval_score(&self, side: PlayerSide) -> i32 {
        self.eval_score[side.index()]
    }

    #[inline]
    pub fn set_legality_cache(&mut self, pinned: Bitboard, in_check: bool) {
        self.pinned = pinned;
        self.in_check_cached = in_check;
    }

    #[inline]
    pub fn is_pinned(&self, square: Square) -> bool {
        self.pinned.is_set(square)
    }

    #[inline]
    pub fn in_check_cached(&self) -> bool {
        self.in_check_cached
    }

    #[inline]
    fn eval_add_piece(&mut self, side: PlayerSide, kind: PieceKind, promoted: bool, square: Square) {
        self.eval_score[side.index()] +=
            eval_table().board_score[side.index()][kind.index()][promoted as usize][square.index() as usize];
    }

    #[inline]
    fn eval_remove_piece(&mut self, side: PlayerSide, kind: PieceKind, promoted: bool, square: Square) {
        self.eval_score[side.index()] -=
            eval_table().board_score[side.index()][kind.index()][promoted as usize][square.index() as usize];
    }

    #[inline]
    fn eval_add_hand(&mut self, side: PlayerSide, kind: PieceKind) {
        self.eval_score[side.index()] += eval_table().hand_value[kind.index()];
    }

    #[inline]
    fn eval_remove_hand(&mut self, side: PlayerSide, kind: PieceKind) {
        self.eval_score[side.index()] -= eval_table().hand_value[kind.index()];
    }

    #[inline]
    fn xor_piece_key(&mut self, side: PlayerSide, kind: PieceKind, promoted: bool, square: Square) {
        let p = promoted as usize;
        self.zobrist ^= zobrist().pieces[side.index()][kind.index()][p][square.index() as usize];
    }

    /// XOR the hand-count key that represents the `slot`-th copy of `kind` in
    /// `side`'s hand (slot is zero-based).  `slot` is the count that changes:
    /// when adding, the new piece occupies slot = old_count; when removing,
    /// we un-XOR slot = new_count (== old_count - 1).
    #[inline]
    fn xor_hand_slot(&mut self, side: PlayerSide, kind: PieceKind, slot: usize) {
        debug_assert!(slot < MAX_HAND_SLOTS);
        self.zobrist ^= zobrist().hands[side.index()][kind.index()][slot];
    }

    #[inline]
    fn xor_side_to_move(&mut self) {
        self.zobrist ^= zobrist().side_to_move;
    }

    /// Recomputes the Zobrist hash from the current bitboards, hands, and
    /// side-to-move.  Call this after bulk mutations (e.g. setting up a new
    /// position); `apply_move` itself updates the hash incrementally and
    /// does not need this.
    pub fn recompute_zobrist(&mut self) {
        let mut h: u64 = 0;
        let ztable = zobrist();
        let etable = eval_table();
        self.eval_score = [0, 0];
        for &side in &PlayerSide::ALL {
            let si = side.index();
            for &kind in &PieceKind::ALL {
                let ki = kind.index();
                let piece_bb = self.bitboards.piece(side, kind);
                let promoted_bb = self.bitboards.promoted(side, kind);
                for square in piece_bb.iter_bits() {
                    let promoted = promoted_bb.is_set(square);
                    let p = promoted as usize;
                    h ^= ztable.pieces[si][ki][p][square.index() as usize];
                    self.eval_score[si] += etable.board_score[si][ki][p][square.index() as usize];
                }
                let count = self.hands[si].count(kind) as usize;
                for slot in 0..count {
                    h ^= ztable.hands[si][ki][slot];
                }
                self.eval_score[si] += count as i32 * etable.hand_value[ki];
            }
        }
        if self.side_to_move == PlayerSide::Gote {
            h ^= ztable.side_to_move;
        }
        self.zobrist = h;
    }

    pub fn bitboards(&self) -> &PieceBitboards {
        &self.bitboards
    }

    /// Returns a `PositionSignature` capturing the full position for hashing.
    pub fn signature(&self) -> PositionSignature {
        PositionSignature {
            side_to_move: self.side_to_move,
            pieces: self.bitboards.pieces,
            promoted: self.bitboards.promoted,
            hands: self.hands,
        }
    }

    /// Returns the square of `side`'s King, or None if absent (should not
    /// happen in a legal game, but guarded for safety).
    pub fn king_square(&self, side: PlayerSide) -> Option<Square> {
        self.bitboards
            .piece(side, PieceKind::King)
            .iter_bits()
            .next()
    }

    pub fn hands(&self) -> &[Hand; 2] {
        &self.hands
    }

    pub fn hand_mut(&mut self, side: PlayerSide) -> &mut Hand {
        &mut self.hands[side.index()]
    }

    pub fn hand(&self, side: PlayerSide) -> Hand {
        self.hands[side.index()]
    }

    /// Returns the side whose turn it is to move.
    pub fn to_move(&self) -> PlayerSide {
        self.side_to_move
    }

    pub fn set_to_move(&mut self, side: PlayerSide) {
        if self.side_to_move != side {
            self.side_to_move = side;
            self.xor_side_to_move();
        }
    }

    /// Returns the half-move counter (number of moves played since the start).
    pub fn ply(&self) -> u32 {
        self.ply
    }

    /// Returns the piece (owner, kind, promoted) at `square`, or None if empty.
    /// O(n) in the number of piece types; use bitboards directly in hot paths.
    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        for &side in &PlayerSide::ALL {
            for &kind in &PieceKind::ALL {
                if self.bitboards.piece(side, kind).is_set(square) {
                    let promoted = self.bitboards.promoted(side, kind).is_set(square);
                    return Some(Piece {
                        owner: side,
                        kind,
                        promoted,
                    });
                }
            }
        }
        None
    }

    pub fn place_piece(
        &mut self,
        side: PlayerSide,
        kind: PieceKind,
        square: Square,
        promoted: bool,
    ) {
        self.bitboards.place(side, kind, square, promoted);
    }

    pub fn remove_piece(&mut self, side: PlayerSide, kind: PieceKind, square: Square) {
        self.bitboards.remove(side, kind, square);
    }

    /// Applies a move unconditionally (no legality check).  The caller is
    /// responsible for passing a legal move.
    ///
    /// For drops:  remove the piece from the player's hand, place it on `to`.
    /// For normal: remove any captured piece (add it to the capturer's hand),
    ///             then move the piece from `from` to `to`, updating the
    ///             promoted flag if `mv.promote` is set.
    pub fn apply_move(&mut self, mv: Move) {
        self.ply += 1;
        match mv.kind {
            MoveKind::Drop => {
                let old_count = self.hand(mv.player).count(mv.piece) as usize;
                if self.hand_mut(mv.player).remove(mv.piece) {
                    self.xor_hand_slot(mv.player, mv.piece, old_count - 1);
                    self.bitboards.place(mv.player, mv.piece, mv.to, false);
                    self.xor_piece_key(mv.player, mv.piece, false, mv.to);
                    self.eval_remove_hand(mv.player, mv.piece);
                    self.eval_add_piece(mv.player, mv.piece, false, mv.to);
                }
            }
            _ => {
                if let Some(captured) = mv.capture {
                    let opponent = mv.player.opponent();
                    let was_promoted =
                        self.bitboards.promoted(opponent, captured).is_set(mv.to);
                    self.xor_piece_key(opponent, captured, was_promoted, mv.to);
                    self.bitboards.remove(opponent, captured, mv.to);
                    let old_count = self.hand(mv.player).count(captured) as usize;
                    self.hand_mut(mv.player).add(captured);
                    self.xor_hand_slot(mv.player, captured, old_count);
                    self.eval_remove_piece(opponent, captured, was_promoted, mv.to);
                    self.eval_add_hand(mv.player, captured);
                }
                if let Some(from) = mv.from {
                    let from_promoted =
                        self.bitboards.promoted(mv.player, mv.piece).is_set(from);
                    let to_promoted = from_promoted || mv.promote;
                    self.xor_piece_key(mv.player, mv.piece, from_promoted, from);
                    self.bitboards.remove(mv.player, mv.piece, from);
                    self.bitboards
                        .place(mv.player, mv.piece, mv.to, to_promoted);
                    self.xor_piece_key(mv.player, mv.piece, to_promoted, mv.to);
                    self.eval_remove_piece(mv.player, mv.piece, from_promoted, from);
                    self.eval_add_piece(mv.player, mv.piece, to_promoted, mv.to);
                }
            }
        }
        self.side_to_move = self.side_to_move.opponent();
        self.xor_side_to_move();
    }

    /// Like `apply_move` but returns an `UndoInfo` token that can be passed to
    /// `undo_move` to restore the board to its pre-move state.  This avoids
    /// cloning the entire Board for each search node.
    pub fn make_move(&mut self, mv: Move) -> UndoInfo {
        let captured_was_promoted;
        let from_was_promoted;

        self.ply += 1;
        match mv.kind {
            MoveKind::Drop => {
                captured_was_promoted = false;
                from_was_promoted = false;
                let old_count = self.hand(mv.player).count(mv.piece) as usize;
                if self.hand_mut(mv.player).remove(mv.piece) {
                    self.xor_hand_slot(mv.player, mv.piece, old_count - 1);
                    self.bitboards.place(mv.player, mv.piece, mv.to, false);
                    self.xor_piece_key(mv.player, mv.piece, false, mv.to);
                    self.eval_remove_hand(mv.player, mv.piece);
                    self.eval_add_piece(mv.player, mv.piece, false, mv.to);
                }
            }
            _ => {
                if let Some(captured) = mv.capture {
                    let opponent = mv.player.opponent();
                    let wp = self.bitboards.promoted(opponent, captured).is_set(mv.to);
                    captured_was_promoted = wp;
                    self.xor_piece_key(opponent, captured, wp, mv.to);
                    self.bitboards.remove(opponent, captured, mv.to);
                    let old_count = self.hand(mv.player).count(captured) as usize;
                    self.hand_mut(mv.player).add(captured);
                    self.xor_hand_slot(mv.player, captured, old_count);
                    self.eval_remove_piece(opponent, captured, wp, mv.to);
                    self.eval_add_hand(mv.player, captured);
                } else {
                    captured_was_promoted = false;
                }
                if let Some(from) = mv.from {
                    let fp = self.bitboards.promoted(mv.player, mv.piece).is_set(from);
                    from_was_promoted = fp;
                    let to_promoted = fp || mv.promote;
                    self.xor_piece_key(mv.player, mv.piece, fp, from);
                    self.bitboards.remove(mv.player, mv.piece, from);
                    self.bitboards
                        .place(mv.player, mv.piece, mv.to, to_promoted);
                    self.xor_piece_key(mv.player, mv.piece, to_promoted, mv.to);
                    self.eval_remove_piece(mv.player, mv.piece, fp, from);
                    self.eval_add_piece(mv.player, mv.piece, to_promoted, mv.to);
                } else {
                    from_was_promoted = false;
                }
            }
        }
        self.side_to_move = self.side_to_move.opponent();
        self.xor_side_to_move();

        UndoInfo {
            captured_was_promoted,
            from_was_promoted,
        }
    }

    /// Reverses a `make_move` call.  `mv` must be the same Move that was passed
    /// to `make_move`, and `undo` must be the UndoInfo it returned.
    pub fn undo_move(&mut self, mv: Move, undo: UndoInfo) {
        self.xor_side_to_move();
        self.side_to_move = self.side_to_move.opponent();
        self.ply -= 1;

        match mv.kind {
            MoveKind::Drop => {
                self.xor_piece_key(mv.player, mv.piece, false, mv.to);
                self.bitboards.remove(mv.player, mv.piece, mv.to);
                let new_count = self.hand(mv.player).count(mv.piece) as usize;
                self.xor_hand_slot(mv.player, mv.piece, new_count);
                self.hand_mut(mv.player).add(mv.piece);
                self.eval_remove_piece(mv.player, mv.piece, false, mv.to);
                self.eval_add_hand(mv.player, mv.piece);
            }
            _ => {
                if let Some(from) = mv.from {
                    let to_promoted = undo.from_was_promoted || mv.promote;
                    self.xor_piece_key(mv.player, mv.piece, to_promoted, mv.to);
                    self.bitboards.remove(mv.player, mv.piece, mv.to);
                    self.bitboards
                        .place(mv.player, mv.piece, from, undo.from_was_promoted);
                    self.xor_piece_key(mv.player, mv.piece, undo.from_was_promoted, from);
                    self.eval_remove_piece(mv.player, mv.piece, to_promoted, mv.to);
                    self.eval_add_piece(mv.player, mv.piece, undo.from_was_promoted, from);
                }
                if let Some(captured) = mv.capture {
                    let opponent = mv.player.opponent();
                    let cur_count = self.hand(mv.player).count(captured) as usize;
                    self.xor_hand_slot(mv.player, captured, cur_count - 1);
                    self.hand_mut(mv.player).remove(captured);
                    self.bitboards
                        .place(opponent, captured, mv.to, undo.captured_was_promoted);
                    self.xor_piece_key(opponent, captured, undo.captured_was_promoted, mv.to);
                    self.eval_remove_hand(mv.player, captured);
                    self.eval_add_piece(opponent, captured, undo.captured_was_promoted, mv.to);
                }
            }
        }
    }

    /// Flips the side to move without touching anything else.  Used for
    /// null-move pruning.  Call `undo_null_move` to reverse.
    #[inline]
    pub fn make_null_move(&mut self) {
        self.side_to_move = self.side_to_move.opponent();
        self.xor_side_to_move();
    }

    /// Reverses a `make_null_move`.
    #[inline]
    pub fn undo_null_move(&mut self) {
        self.xor_side_to_move();
        self.side_to_move = self.side_to_move.opponent();
    }
}

impl Default for Board {
    fn default() -> Self {
        Board::new_standard()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::movegen::MoveGenerator;

    /// Plays a random-ish sequence of legal moves and verifies that the
    /// incrementally-maintained Zobrist hash always matches a from-scratch
    /// recomputation — catches any missed XOR in `apply_move`.
    #[test]
    fn zobrist_matches_recompute() {
        let mut board = Board::new_standard();
        // Verify initial state.
        let mut check = board.clone();
        check.recompute_zobrist();
        assert_eq!(board.zobrist(), check.zobrist());

        for ply in 0..80 {
            let moves = MoveGenerator::legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            // Deterministic pick: use ply as an index into the move list.
            let mv = moves[ply % moves.len()];
            board.apply_move(mv);

            let mut fresh = board.clone();
            fresh.recompute_zobrist();
            assert_eq!(
                board.zobrist(),
                fresh.zobrist(),
                "zobrist mismatch after {} plies (move {:?})",
                ply + 1,
                mv
            );
        }
    }

    #[test]
    fn make_undo_restores_board() {
        let mut board = Board::new_standard();
        for ply in 0..80 {
            let moves = MoveGenerator::legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            let mv = moves[ply % moves.len()];
            let snapshot = board.clone();
            let undo = board.make_move(mv);
            board.undo_move(mv, undo);
            assert_eq!(
                board.zobrist(),
                snapshot.zobrist(),
                "zobrist not restored after make/undo at ply {} (move {:?})",
                ply,
                mv
            );
            assert_eq!(board.to_move(), snapshot.to_move(), "side not restored at ply {}", ply);
            assert_eq!(board.ply(), snapshot.ply(), "ply not restored at ply {}", ply);
            assert_eq!(board.signature(), snapshot.signature(), "signature not restored at ply {}", ply);
            board.apply_move(mv);
        }
    }
}
