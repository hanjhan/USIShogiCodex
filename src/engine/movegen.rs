use crate::engine::{
    bitboard::Bitboard,
    board::Board,
    movelist::MoveList,
    movement::Move,
    state::{PieceKind, PlayerSide, Square},
};

// Move generation overview
// ========================
// `MoveGenerator` produces legal moves from a given board position.  The
// process is split into two stages:
//
//  1. Generate pseudo-legal moves — moves that obey piece movement patterns
//     but may leave the moving side's king in check.
//  2. Enforce legality — for each pseudo-legal move, apply it and verify the
//     moving side's king is not in check in the resulting position.
//
// The same generation logic is used for both stages; the `enforce_legality`
// flag in the internal helpers controls whether the king-check filter is applied.
//
// Piece movement patterns are encoded as static delta tables (e.g. GOLD_STEPS)
// expressed from Sente's perspective.  `Square::offset_from_perspective` flips
// the deltas for Gote so that "forward" always means toward the enemy back rank.
//
// Special rules implemented:
//   - Mandatory promotion for Pawn/Lance reaching the last rank, Knight reaching
//     the last two ranks.
//   - Optional promotion for pieces entering/leaving the promotion zone.
//   - Nifu: a pawn cannot be dropped on a file that already contains an
//     unpromoted friendly pawn.
//   - Uchi-fu-zume: a pawn drop that immediately checkmates the opponent is illegal.
//   - Drop restrictions: pawns/lances cannot be dropped on the last rank;
//     knights cannot be dropped on the last two ranks.

pub struct MoveGenerator;

#[allow(clippy::too_many_arguments)]
impl MoveGenerator {
    /// Returns all legal moves for the side to move in `board`.
    pub fn legal_moves(board: &mut Board) -> MoveList {
        Self::generate_internal(board, board.to_move(), true, true)
    }

    /// Returns all legal moves for `side` regardless of whose turn it is.
    pub fn legal_moves_for(board: &mut Board, side: PlayerSide) -> MoveList {
        Self::generate_internal(board, side, true, true)
    }

    /// Like `legal_moves_for` but optionally skips the uchi-fu-zume check.
    pub fn legal_moves_for_options(
        board: &mut Board,
        side: PlayerSide,
        check_drop_mate: bool,
    ) -> MoveList {
        Self::generate_internal(board, side, true, check_drop_mate)
    }

    /// Returns pseudo-legal moves (no king-in-check filter).
    pub fn pseudo_legal_moves(board: &Board, side: PlayerSide) -> MoveList {
        let mut board = board.clone();
        Self::generate_internal(&mut board, side, false, false)
    }

    /// Returns only legal captures and promotions — the "loud" moves needed
    /// by quiescence search.  Skips quiet moves and drops entirely, roughly
    /// halving the work compared to full generation + filter.
    pub fn loud_moves(board: &mut Board, side: PlayerSide) -> MoveList {
        let in_check = Self::is_in_check(board, side);
        let pinned = if in_check {
            Bitboard::empty()
        } else {
            Self::compute_pinned(board, side)
        };
        board.set_legality_cache(pinned, in_check);
        let mut moves = MoveList::new();
        Self::generate_loud_piece_moves(board, side, &mut moves);
        moves
    }

    /// Returns true if `side`'s king is currently in check on `board`.
    pub fn is_in_check(board: &Board, side: PlayerSide) -> bool {
        if let Some(king_sq) = board.king_square(side) {
            Self::is_square_attacked(board, king_sq, side.opponent())
        } else {
            // No king found — treat as in check so the position is immediately lost.
            true
        }
    }

    // -----------------------------------------------------------------------
    // Internal generation entry point
    // -----------------------------------------------------------------------

    fn generate_internal(
        board: &mut Board,
        side: PlayerSide,
        enforce_legality: bool,
        check_drop_mate: bool,
    ) -> MoveList {
        let mut moves = MoveList::new();
        if enforce_legality {
            let in_check = Self::is_in_check(board, side);
            let pinned = if in_check {
                Bitboard::empty()
            } else {
                Self::compute_pinned(board, side)
            };
            board.set_legality_cache(pinned, in_check);
        }
        Self::generate_piece_moves(board, side, enforce_legality, &mut moves);
        Self::generate_drop_moves(board, side, enforce_legality, check_drop_mate, &mut moves);
        moves
    }

    // -----------------------------------------------------------------------
    // Board piece moves
    // -----------------------------------------------------------------------

    fn generate_piece_moves(
        board: &mut Board,
        side: PlayerSide,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        for &kind in &PieceKind::ALL {
            let squares = board.bitboards().piece(side, kind).iter_bits();
            let promoted_bb = board.bitboards().promoted(side, kind);
            for square in squares {
                let is_promoted = promoted_bb.is_set(square);
                Self::generate_moves_for_piece(
                    board,
                    side,
                    kind,
                    is_promoted,
                    square,
                    enforce_legality,
                    moves,
                );
            }
        }
    }

    /// Like `generate_piece_moves` but only emits captures and promotions.
    fn generate_loud_piece_moves(
        board: &mut Board,
        side: PlayerSide,
        moves: &mut MoveList,
    ) {
        for &kind in &PieceKind::ALL {
            let squares = board.bitboards().piece(side, kind).iter_bits();
            let promoted_bb = board.bitboards().promoted(side, kind);
            for square in squares {
                let is_promoted = promoted_bb.is_set(square);
                Self::generate_loud_moves_for_piece(
                    board, side, kind, is_promoted, square, moves,
                );
            }
        }
    }

    fn generate_loud_moves_for_piece(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        moves: &mut MoveList,
    ) {
        match kind {
            PieceKind::King => Self::generate_loud_step_moves(
                board, side, kind, promoted, from, KING_STEPS, moves,
            ),
            PieceKind::Gold => Self::generate_loud_step_moves(
                board, side, kind, promoted, from, GOLD_STEPS, moves,
            ),
            PieceKind::Silver => {
                let steps = if promoted { GOLD_STEPS } else { SILVER_STEPS };
                Self::generate_loud_step_moves(board, side, kind, promoted, from, steps, moves);
            }
            PieceKind::Knight => {
                if promoted {
                    Self::generate_loud_step_moves(
                        board, side, kind, promoted, from, GOLD_STEPS, moves,
                    );
                } else {
                    Self::generate_loud_knight_moves(board, side, kind, from, moves);
                }
            }
            PieceKind::Lance => {
                if promoted {
                    Self::generate_loud_step_moves(
                        board, side, kind, promoted, from, GOLD_STEPS, moves,
                    );
                } else {
                    Self::generate_loud_slide_moves(
                        board, side, kind, false, from, LANCE_DIRECTIONS, moves,
                    );
                }
            }
            PieceKind::Pawn => {
                let steps = if promoted { GOLD_STEPS } else { PAWN_STEPS };
                Self::generate_loud_step_moves(board, side, kind, promoted, from, steps, moves);
            }
            PieceKind::Rook => {
                Self::generate_loud_slide_moves(
                    board, side, kind, promoted, from, ROOK_SLIDES, moves,
                );
                if promoted {
                    Self::generate_loud_step_moves(
                        board, side, kind, promoted, from, BISHOP_STEPS, moves,
                    );
                }
            }
            PieceKind::Bishop => {
                Self::generate_loud_slide_moves(
                    board, side, kind, promoted, from, BISHOP_SLIDES, moves,
                );
                if promoted {
                    Self::generate_loud_step_moves(
                        board, side, kind, promoted, from, ROOK_STEPS, moves,
                    );
                }
            }
        }
    }

    /// Dispatches to the correct movement pattern for each piece type, taking
    /// promotion state into account.  Promoted Silver/Knight/Lance/Pawn all
    /// move like Gold; promoted Rook gains diagonal step moves; promoted Bishop
    /// gains orthogonal step moves.
    fn generate_moves_for_piece(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        match kind {
            PieceKind::King => Self::generate_step_moves(
                board, side, kind, promoted, from, KING_STEPS, enforce_legality, moves,
            ),
            PieceKind::Gold => Self::generate_step_moves(
                board, side, kind, promoted, from, GOLD_STEPS, enforce_legality, moves,
            ),
            PieceKind::Silver => {
                // Promoted silver moves like gold
                let steps = if promoted { GOLD_STEPS } else { SILVER_STEPS };
                Self::generate_step_moves(
                    board, side, kind, promoted, from, steps, enforce_legality, moves,
                );
            }
            PieceKind::Knight => {
                if promoted {
                    Self::generate_step_moves(
                        board, side, kind, promoted, from, GOLD_STEPS, enforce_legality, moves,
                    );
                } else {
                    // Unpromoted knight uses special jump generation (not offset-based)
                    Self::generate_knight_moves(board, side, kind, from, enforce_legality, moves);
                }
            }
            PieceKind::Lance => {
                if promoted {
                    Self::generate_step_moves(
                        board, side, kind, promoted, from, GOLD_STEPS, enforce_legality, moves,
                    );
                } else {
                    Self::generate_lance_moves(board, side, kind, from, enforce_legality, moves);
                }
            }
            PieceKind::Pawn => {
                let steps = if promoted { GOLD_STEPS } else { PAWN_STEPS };
                Self::generate_step_moves(
                    board, side, kind, promoted, from, steps, enforce_legality, moves,
                );
            }
            PieceKind::Rook => {
                // Rook slides orthogonally; promoted rook also steps diagonally.
                Self::generate_slide_moves(
                    board, side, kind, promoted, from, ROOK_SLIDES, enforce_legality, moves,
                );
                if promoted {
                    Self::generate_step_moves(
                        board, side, kind, promoted, from, BISHOP_STEPS, enforce_legality, moves,
                    );
                }
            }
            PieceKind::Bishop => {
                // Bishop slides diagonally; promoted bishop also steps orthogonally.
                Self::generate_slide_moves(
                    board, side, kind, promoted, from, BISHOP_SLIDES, enforce_legality, moves,
                );
                if promoted {
                    Self::generate_step_moves(
                        board, side, kind, promoted, from, ROOK_STEPS, enforce_legality, moves,
                    );
                }
            }
        }
    }

    /// Generates all moves for a piece that moves one step in each direction
    /// in `deltas` (King, Gold, Silver, Pawn, and promoted variants).
    fn generate_step_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        deltas: &[(i8, i8)],
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in deltas {
            if let Some(to) = from.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(to) {
                    continue; // Can't move to a square occupied by own piece
                }
                let capture = if opp_occ.is_set(to) {
                    Self::piece_kind_at(board, opp, to)
                } else {
                    None
                };
                Self::emit_move(
                    board,
                    side,
                    kind,
                    promoted,
                    from,
                    to,
                    capture,
                    enforce_legality,
                    moves,
                );
            }
        }
    }

    /// Generates knight moves.  Knights jump in an L-shape (+/-1 file, -2 ranks
    /// from the mover's perspective) and can jump over other pieces.
    fn generate_knight_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        from: Square,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in KNIGHT_JUMPS {
            if let Some(to) = from.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(to) {
                    continue;
                }
                let capture = if opp_occ.is_set(to) {
                    Self::piece_kind_at(board, opp, to)
                } else {
                    None
                };
                Self::emit_move(
                    board, side, kind, false, from, to, capture, enforce_legality, moves,
                );
            }
        }
    }

    /// Lance moves: slides forward-only (one direction, unlimited distance).
    fn generate_lance_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        from: Square,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        Self::generate_slide_moves(
            board,
            side,
            kind,
            false,
            from,
            LANCE_DIRECTIONS,
            enforce_legality,
            moves,
        );
    }

    /// Generates sliding moves (Rook, Bishop, Lance) by extending one step at
    /// a time in each direction until hitting a piece or the board edge.
    /// Stops after capturing (cannot slide through a piece).
    fn generate_slide_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        directions: &[(i8, i8)],
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in directions {
            let mut current = from;
            while let Some(next) = current.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(next) {
                    break; // Blocked by own piece
                }
                let capture = if opp_occ.is_set(next) {
                    Self::piece_kind_at(board, opp, next)
                } else {
                    None
                };
                Self::emit_move(
                    board, side, kind, promoted, from, next, capture, enforce_legality, moves,
                );
                if capture.is_some() {
                    break; // Cannot slide past a captured piece
                }
                current = next;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Loud-only generators (captures + promotions, with legality check)
    // -----------------------------------------------------------------------

    fn generate_loud_step_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        deltas: &[(i8, i8)],
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in deltas {
            if let Some(to) = from.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(to) {
                    continue;
                }
                let capture = if opp_occ.is_set(to) {
                    Self::piece_kind_at(board, opp, to)
                } else {
                    None
                };
                Self::emit_loud_move(
                    board, side, kind, promoted, from, to, capture, moves,
                );
            }
        }
    }

    fn generate_loud_knight_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        from: Square,
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in KNIGHT_JUMPS {
            if let Some(to) = from.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(to) {
                    continue;
                }
                let capture = if opp_occ.is_set(to) {
                    Self::piece_kind_at(board, opp, to)
                } else {
                    None
                };
                Self::emit_loud_move(
                    board, side, kind, false, from, to, capture, moves,
                );
            }
        }
    }

    fn generate_loud_slide_moves(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        directions: &[(i8, i8)],
        moves: &mut MoveList,
    ) {
        let our_occ = board.bitboards().occupancy(side);
        let opp = side.opponent();
        let opp_occ = board.bitboards().occupancy(opp);
        for &(df, dr) in directions {
            let mut current = from;
            while let Some(next) = current.offset_from_perspective(side, df, dr) {
                if our_occ.is_set(next) {
                    break;
                }
                let capture = if opp_occ.is_set(next) {
                    Self::piece_kind_at(board, opp, next)
                } else {
                    None
                };
                Self::emit_loud_move(
                    board, side, kind, promoted, from, next, capture, moves,
                );
                if capture.is_some() {
                    break;
                }
                current = next;
            }
        }
    }

    /// Emits only captures and promotions with legality check.
    fn emit_loud_move(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        to: Square,
        capture: Option<PieceKind>,
        moves: &mut MoveList,
    ) {
        Self::for_each_promotion_choice(kind, promoted, side, from, to, |promote| {
            if capture.is_some() || promote {
                let mv = Move::normal(side, from, to, kind, capture, promote);
                Self::push_move(board, side, mv, true, moves);
            }
        });
    }

    // -----------------------------------------------------------------------
    // Drop moves
    // -----------------------------------------------------------------------

    /// Generates all legal drop moves for `side`.  For each piece type in
    /// hand, try every empty square on the board subject to drop restrictions.
    fn generate_drop_moves(
        board: &mut Board,
        side: PlayerSide,
        enforce_legality: bool,
        check_drop_mate: bool,
        moves: &mut MoveList,
    ) {
        let hand = board.hand(side);
        let occupied = board.bitboards().occupancy_all();
        for &kind in &PieceKind::ALL {
            if kind == PieceKind::King {
                continue; // Kings can never be in hand
            }
            let count = hand.count(kind);
            if count == 0 {
                continue;
            }
            for idx in 0..81 {
                if let Some(square) = Square::from_index(idx as u8) {
                    if occupied.is_set(square) {
                        continue; // Cannot drop onto an occupied square
                    }
                    if !Self::is_drop_legal(board, side, kind, square, check_drop_mate) {
                        continue;
                    }
                    let mv = Move::drop(side, kind, square);
                    Self::push_move(board, side, mv, enforce_legality, moves);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Promotion and move emission
    // -----------------------------------------------------------------------

    /// Emits one or two moves for a (from, to) pair depending on whether
    /// promotion is mandatory, optional, or impossible.
    fn emit_move(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        to: Square,
        capture: Option<PieceKind>,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        Self::for_each_promotion_choice(kind, promoted, side, from, to, |promote| {
            let mv = Move::normal(side, from, to, kind, capture, promote);
            Self::push_move(board, side, mv, enforce_legality, moves);
        });
    }

    /// Calls `f` once for each valid promotion choice for this (piece, from, to)
    /// combination:
    ///   - Already promoted or non-promotable piece: always unpromoted.
    ///   - Mandatory promotion (pawn/lance on last rank, knight on last two): only promoted.
    ///   - Optional promotion (entering or leaving the promotion zone): both choices.
    fn for_each_promotion_choice<F: FnMut(bool)>(
        kind: PieceKind,
        is_promoted: bool,
        side: PlayerSide,
        from: Square,
        to: Square,
        mut f: F,
    ) {
        if is_promoted || !kind.promotable() {
            f(false);
            return;
        }
        let can_promote = from.in_promotion_zone(side) || to.in_promotion_zone(side);
        if !can_promote {
            f(false);
            return;
        }
        if Self::must_promote(kind, side, to) {
            // Only the promoted version is legal (piece would be immobile otherwise)
            f(true);
        } else {
            // Both promoted and unpromoted versions are legal
            f(false);
            f(true);
        }
    }

    /// Returns true if a piece of `kind` for `side` moving to `to` is forced
    /// to promote (it would have no legal moves if it remained unpromoted).
    fn must_promote(kind: PieceKind, side: PlayerSide, to: Square) -> bool {
        match (side, kind) {
            // Pawn and Lance cannot exist on the last rank unpromoted
            (PlayerSide::Sente, PieceKind::Pawn | PieceKind::Lance) => to.rank() == 0,
            (PlayerSide::Gote, PieceKind::Pawn | PieceKind::Lance) => to.rank() == 8,
            // Knight cannot exist on the last two ranks unpromoted
            (PlayerSide::Sente, PieceKind::Knight) => to.rank() <= 1,
            (PlayerSide::Gote, PieceKind::Knight) => to.rank() >= 7,
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Drop legality helpers
    // -----------------------------------------------------------------------

    fn is_drop_legal(
        board: &mut Board,
        side: PlayerSide,
        kind: PieceKind,
        square: Square,
        check_drop_mate: bool,
    ) -> bool {
        match kind {
            PieceKind::Pawn => {
                // Cannot drop on the last rank (piece would be immobile)
                if Self::is_last_rank(side, square.rank()) {
                    return false;
                }
                // Nifu: cannot drop a pawn on a file that already has an
                // unpromoted friendly pawn (promoted pawns don't count)
                if Self::has_pawn_on_file(board, side, square.file()) {
                    return false;
                }
                // Uchi-fu-zume: pawn drop that immediately mates is illegal
                if check_drop_mate && Self::is_pawn_drop_mate(board, side, square) {
                    return false;
                }
                true
            }
            PieceKind::Lance => !Self::is_last_rank(side, square.rank()),
            PieceKind::Knight => !Self::is_knight_dead_rank(side, square.rank()),
            PieceKind::King => false, // King can never be dropped
            _ => true,
        }
    }

    /// Checks if dropping a pawn at `square` for `side` would constitute
    /// uchi-fu-zume (pawn drop checkmate), which is illegal in shogi.
    fn is_pawn_drop_mate(board: &mut Board, side: PlayerSide, square: Square) -> bool {
        let drop_move = Move::drop(side, PieceKind::Pawn, square);
        let undo = board.make_move(drop_move);
        if !Self::is_in_check(board, side.opponent()) {
            board.undo_move(drop_move, undo);
            return false;
        }
        let replies = Self::legal_moves_for(board, side.opponent());
        let is_mate = replies.is_empty();
        board.undo_move(drop_move, undo);
        is_mate
    }

    fn is_last_rank(side: PlayerSide, rank: u8) -> bool {
        match side {
            PlayerSide::Sente => rank == 0,
            PlayerSide::Gote => rank == 8,
        }
    }

    /// Returns true if the rank is in the last two rows for the knight
    /// (where an unpromoted knight would have no forward jumps).
    fn is_knight_dead_rank(side: PlayerSide, rank: u8) -> bool {
        match side {
            PlayerSide::Sente => rank <= 1,
            PlayerSide::Gote => rank >= 7,
        }
    }

    /// Returns true if `side` already has an unpromoted pawn on `file`.
    /// Used to enforce the nifu rule.
    fn has_pawn_on_file(board: &Board, side: PlayerSide, file: u8) -> bool {
        let pawns = board.bitboards().piece(side, PieceKind::Pawn);
        let promoted = board.bitboards().promoted(side, PieceKind::Pawn);
        let bits = pawns.iter_bits();
        for square in bits {
            if square.file() == file && !promoted.is_set(square) {
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Pin detection — which pieces would expose the king if they moved?
    // -----------------------------------------------------------------------

    /// Returns a Bitboard of `side`'s pieces that are pinned to the king.
    /// A pinned piece is the only friendly piece between the king and an
    /// enemy slider — moving it off that line would expose the king.
    ///
    /// By knowing which pieces are pinned, push_move can skip the expensive
    /// is_in_check call for ~80-90% of moves (non-pinned, non-king moves
    /// are always legal).
    fn compute_pinned(board: &Board, side: PlayerSide) -> Bitboard {
        let king_sq = match board.king_square(side) {
            Some(sq) => sq,
            None => return Bitboard::empty(),
        };
        let opp = side.opponent();
        let our_occ = board.bitboards().occupancy(side);
        let all_occ = board.bitboards().occupancy_all();
        let bbs = board.bitboards();
        let mut pinned = Bitboard::empty();

        // Orthogonal pins: Rook, Dragon, or Lance can pin along rank/file.
        let rook_bb = bbs.piece(opp, PieceKind::Rook); // includes Dragon
        for &(df, dr) in ROOK_SLIDES {
            let mut friendly_sq: Option<Square> = None;
            let mut cur = king_sq;
            while let Some(s) = cur.offset(df, dr) {
                if all_occ.is_set(s) {
                    if our_occ.is_set(s) {
                        if friendly_sq.is_some() {
                            break; // Two friendly pieces — no pin
                        }
                        friendly_sq = Some(s);
                    } else {
                        if let Some(fsq) = friendly_sq
                            && rook_bb.is_set(s) {
                                pinned.set(fsq);
                            }
                        break;
                    }
                }
                cur = s;
            }
        }

        // Orthogonal pin by unpromoted Lance (forward-only).
        // A lance on the opponent's side can only pin along its forward
        // direction.  From the king's perspective, the lance must be
        // "behind" in the opponent's forward direction.
        // Opponent's forward = toward our back rank.
        // If opp is Sente, forward is (0, -1) → from king that's (0, +1).
        // If opp is Gote, forward is (0, +1) → from king that's (0, -1).
        let lance_dir: (i8, i8) = match opp {
            PlayerSide::Sente => (0, 1),  // Sente lance moves toward rank 0; from king: +rank
            PlayerSide::Gote => (0, -1),  // Gote lance moves toward rank 8; from king: -rank
        };
        {
            let lance_bb = bbs.piece(opp, PieceKind::Lance);
            let lance_promoted = bbs.promoted(opp, PieceKind::Lance);
            let mut friendly_sq: Option<Square> = None;
            let mut cur = king_sq;
            while let Some(s) = cur.offset(lance_dir.0, lance_dir.1) {
                if all_occ.is_set(s) {
                    if our_occ.is_set(s) {
                        if friendly_sq.is_some() {
                            break;
                        }
                        friendly_sq = Some(s);
                    } else {
                        if let Some(fsq) = friendly_sq
                            && lance_bb.is_set(s) && !lance_promoted.is_set(s) {
                                pinned.set(fsq);
                            }
                        break;
                    }
                }
                cur = s;
            }
        }

        // Diagonal pins: Bishop or Horse can pin along diagonals.
        let bishop_bb = bbs.piece(opp, PieceKind::Bishop); // includes Horse
        for &(df, dr) in BISHOP_SLIDES {
            let mut friendly_sq: Option<Square> = None;
            let mut cur = king_sq;
            while let Some(s) = cur.offset(df, dr) {
                if all_occ.is_set(s) {
                    if our_occ.is_set(s) {
                        if friendly_sq.is_some() {
                            break;
                        }
                        friendly_sq = Some(s);
                    } else {
                        if let Some(fsq) = friendly_sq
                            && bishop_bb.is_set(s) {
                                pinned.set(fsq);
                            }
                        break;
                    }
                }
                cur = s;
            }
        }

        pinned
    }

    // -----------------------------------------------------------------------
    // Legality filter
    // -----------------------------------------------------------------------

    /// Pushes `mv` onto `moves` if legal.
    ///
    /// When `pinned` is Some, uses the pre-computed pin bitboard to skip the
    /// expensive is_in_check call for non-pinned, non-king moves (~80-90% of
    /// all pseudo-legal moves).  When `pinned` is None and `enforce_legality`
    /// is false, skips legality entirely (pseudo-legal generation).
    fn push_move(
        board: &mut Board,
        side: PlayerSide,
        mv: Move,
        enforce_legality: bool,
        moves: &mut MoveList,
    ) {
        if !enforce_legality {
            moves.push(mv);
            return;
        }
        // When NOT in check, a move can only be illegal if it causes a
        // discovered attack on our own king.  This can only happen if:
        //   (a) the moving piece is the king itself, or
        //   (b) the moving piece was pinned (blocking an enemy slider line).
        // Drops can never cause discovered check (they don't remove a piece
        // from the board).  This fast path skips is_in_check for ~80-90%
        // of moves.
        //
        // When IN check, every move must resolve the check, so the fast
        // path is disabled and all moves go through full legality.
        if !board.in_check_cached() && mv.piece != PieceKind::King {
            if let Some(from) = mv.from {
                if !board.is_pinned(from) {
                    moves.push(mv);
                    return;
                }
            } else {
                // Drop — can't cause discovered check.
                moves.push(mv);
                return;
            }
        }
        // Full legality check for: king moves, pinned-piece moves,
        // and all moves when the king is in check.
        let undo = board.make_move(mv);
        let legal = !Self::is_in_check(board, side);
        board.undo_move(mv, undo);
        if legal {
            moves.push(mv);
        }
    }



    // -----------------------------------------------------------------------
    // Utility helpers
    // -----------------------------------------------------------------------

    /// Returns the piece kind belonging to `side` at `square`, or None.
    fn piece_kind_at(board: &Board, side: PlayerSide, square: Square) -> Option<PieceKind> {
        PieceKind::ALL.iter().find(|&&kind| board.bitboards().piece(side, kind).is_set(square)).copied()
    }

    /// Returns true if any piece of `attacker` threatens `square`.
    ///
    /// Implemented by **reverse attack lookup**: for each piece kind, compute
    /// the squares from which such a piece could reach `square` and test if
    /// an attacker piece of that kind is actually there.  This avoids the
    /// cost of generating the entire pseudo-legal move list whenever we only
    /// want to know whether a single square is under attack.
    ///
    /// The encoding uses `offset_from_perspective(attacker, -df, -dr)` — if an
    /// attacker piece's forward step is `(df, dr)` (from the attacker's
    /// perspective), then the square it could be *coming from* relative to
    /// the target is the target plus the negated step in the same perspective.
    ///
    /// Promoted Silver / Knight / Lance / Pawn all move like Gold, so they
    /// are handled together in the gold-movement block.  Unpromoted Lance and
    /// Pawn are handled separately because their forward-only patterns differ.
    /// Rook and Bishop sliding rays also catch their promoted (Dragon / Horse)
    /// counterparts, since promoted bitboards are subsets of the piece bitboards.
    fn is_square_attacked(board: &Board, square: Square, attacker: PlayerSide) -> bool {
        let bbs = board.bitboards();
        let occ = bbs.occupancy_all();

        // --- Unpromoted Pawn ---
        // Pawn at S attacks S + perspective(attacker, 0, -1);
        // so a pawn attacking `square` must be at square + perspective(attacker, 0, 1).
        if let Some(s) = square.offset_from_perspective(attacker, 0, 1) {
            let pawns = bbs.piece(attacker, PieceKind::Pawn);
            let promoted = bbs.promoted(attacker, PieceKind::Pawn);
            if pawns.is_set(s) && !promoted.is_set(s) {
                return true;
            }
        }

        // --- Unpromoted Knight ---
        for &(df, dr) in KNIGHT_JUMPS {
            if let Some(s) = square.offset_from_perspective(attacker, -df, -dr) {
                let knights = bbs.piece(attacker, PieceKind::Knight);
                let promoted = bbs.promoted(attacker, PieceKind::Knight);
                if knights.is_set(s) && !promoted.is_set(s) {
                    return true;
                }
            }
        }

        // --- Unpromoted Silver ---
        for &(df, dr) in SILVER_STEPS {
            if let Some(s) = square.offset_from_perspective(attacker, -df, -dr) {
                let silvers = bbs.piece(attacker, PieceKind::Silver);
                let promoted = bbs.promoted(attacker, PieceKind::Silver);
                if silvers.is_set(s) && !promoted.is_set(s) {
                    return true;
                }
            }
        }

        // --- Gold-movement pieces: Gold + promoted Silver / Knight / Lance / Pawn ---
        let gold_bb = bbs.piece(attacker, PieceKind::Gold)
            | bbs.promoted(attacker, PieceKind::Silver)
            | bbs.promoted(attacker, PieceKind::Knight)
            | bbs.promoted(attacker, PieceKind::Lance)
            | bbs.promoted(attacker, PieceKind::Pawn);
        for &(df, dr) in GOLD_STEPS {
            if let Some(s) = square.offset_from_perspective(attacker, -df, -dr)
                && gold_bb.is_set(s) {
                    return true;
                }
        }

        // --- King ---
        // KING_STEPS is symmetric so perspective is irrelevant; kept for uniformity.
        for &(df, dr) in KING_STEPS {
            if let Some(s) = square.offset_from_perspective(attacker, -df, -dr)
                && bbs.piece(attacker, PieceKind::King).is_set(s) {
                    return true;
                }
        }

        // --- Rook (and Dragon) orthogonal slides ---
        // Ray-cast from `square` until we hit a piece.  If the blocker is any
        // attacker-owned rook (promoted or not), it attacks us.  `piece(Rook)`
        // is a superset of `promoted(Rook)`, so a single test catches both.
        for &(df, dr) in ROOK_SLIDES {
            let mut cur = square;
            while let Some(s) = cur.offset(df, dr) {
                if occ.is_set(s) {
                    if bbs.piece(attacker, PieceKind::Rook).is_set(s) {
                        return true;
                    }
                    break;
                }
                cur = s;
            }
        }

        // --- Bishop (and Horse) diagonal slides ---
        for &(df, dr) in BISHOP_SLIDES {
            let mut cur = square;
            while let Some(s) = cur.offset(df, dr) {
                if occ.is_set(s) {
                    if bbs.piece(attacker, PieceKind::Bishop).is_set(s) {
                        return true;
                    }
                    break;
                }
                cur = s;
            }
        }

        // --- Unpromoted Lance (forward-only slide from attacker's perspective) ---
        // Scan from `square` toward the attacker's home side; the first piece
        // encountered is a threat iff it is the attacker's unpromoted lance.
        // Promoted lances (gold-movers) are handled above.
        let mut cur = square;
        while let Some(s) = cur.offset_from_perspective(attacker, 0, 1) {
            if occ.is_set(s) {
                let lances = bbs.piece(attacker, PieceKind::Lance);
                let promoted = bbs.promoted(attacker, PieceKind::Lance);
                if lances.is_set(s) && !promoted.is_set(s) {
                    return true;
                }
                break;
            }
            cur = s;
        }

        // --- Dragon (promoted Rook): extra one-step diagonal attacks ---
        for &(df, dr) in BISHOP_SLIDES {
            if let Some(s) = square.offset(df, dr)
                && bbs.promoted(attacker, PieceKind::Rook).is_set(s) {
                    return true;
                }
        }

        // --- Horse (promoted Bishop): extra one-step orthogonal attacks ---
        for &(df, dr) in ROOK_SLIDES {
            if let Some(s) = square.offset(df, dr)
                && bbs.promoted(attacker, PieceKind::Bishop).is_set(s) {
                    return true;
                }
        }

        false
    }
}

// ---------------------------------------------------------------------------
// Movement delta tables
// All deltas are expressed from Sente's perspective: positive rank = backward,
// negative rank = forward (toward Gote's back rank).
// `offset_from_perspective` negates both deltas for Gote.
// ---------------------------------------------------------------------------

/// King: one step in all 8 directions.
const KING_STEPS: &[(i8, i8)] = &[
    (-1, -1), (0, -1), (1, -1),
    (-1,  0),          (1,  0),
    (-1,  1), (0,  1), (1,  1),
];

/// Gold (and promoted Silver/Knight/Lance/Pawn): forward three, sides two,
/// no diagonal backward.
const GOLD_STEPS: &[(i8, i8)] = &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (0, 1)];

/// Silver: diagonals and straight forward, no sides or straight backward.
const SILVER_STEPS: &[(i8, i8)] = &[(-1, -1), (0, -1), (1, -1), (-1, 1), (1, 1)];

/// Knight: jumps two forward, one to the side (can jump over pieces).
const KNIGHT_JUMPS: &[(i8, i8)] = &[(-1, -2), (1, -2)];

/// Pawn: one step straight forward only.
const PAWN_STEPS: &[(i8, i8)] = &[(0, -1)];

/// Lance: slides straight forward only.
const LANCE_DIRECTIONS: &[(i8, i8)] = &[(0, -1)];

/// Rook: slides orthogonally in all four directions.
const ROOK_SLIDES: &[(i8, i8)] = &[(0, -1), (0, 1), (-1, 0), (1, 0)];

/// Bishop: slides diagonally in all four directions.
const BISHOP_SLIDES: &[(i8, i8)] = &[(1, -1), (-1, -1), (1, 1), (-1, 1)];

/// Promoted bishop gains one-step orthogonal moves (same deltas as rook slides).
const BISHOP_STEPS: &[(i8, i8)] = BISHOP_SLIDES;
/// Promoted rook gains one-step diagonal moves (same deltas as bishop slides).
const ROOK_STEPS: &[(i8, i8)] = ROOK_SLIDES;

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementation: the original slow definition that generates
    /// every pseudo-legal move for `attacker` and scans for one that reaches
    /// `square` as a capture.  Used only in tests as an oracle.
    fn slow_is_square_attacked(board: &Board, square: Square, attacker: PlayerSide) -> bool {
        let moves = MoveGenerator::pseudo_legal_moves(board, attacker);
        moves
            .into_iter()
            .any(|mv| mv.to == square && mv.capture.is_some())
    }

    /// Plays a long sequence of legal moves (deterministically) and at every
    /// ply verifies that the fast `is_square_attacked` agrees with the slow
    /// oracle on every square that is **not occupied by the attacker**.
    ///
    /// The two definitions differ when the target square holds an attacker-
    /// owned piece: the slow oracle only sees a *capture* move (which cannot
    /// target one's own piece and so returns false), while the fast path
    /// reports "the square is in the attacker's threat set" regardless of
    /// what stands on it.  For the only real caller (`is_in_check`), the
    /// target is always the defender's king square, where both definitions
    /// must agree — so restricting the test to non-attacker-owned squares
    /// exactly covers the production use case.
    #[test]
    fn fast_is_square_attacked_matches_slow() {
        let mut board = Board::new_standard();
        for ply in 0..80 {
            for idx in 0..81 {
                let sq = Square::from_index(idx).unwrap();
                for &atk in &PlayerSide::ALL {
                    // The two definitions only necessarily agree on squares
                    // occupied by the defender (where both report "can be
                    // captured by the attacker").  Empty squares and
                    // attacker-owned squares are excluded.
                    if !board.bitboards().occupancy(atk.opponent()).is_set(sq) {
                        continue;
                    }
                    let fast = MoveGenerator::is_square_attacked(&board, sq, atk);
                    let slow = slow_is_square_attacked(&board, sq, atk);
                    assert_eq!(
                        fast, slow,
                        "mismatch at ply {}, square {:?}, attacker {:?}: fast={} slow={}",
                        ply, sq, atk, fast, slow
                    );
                }
            }
            let moves = MoveGenerator::legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            board.apply_move(moves[ply % moves.len()]);
        }
    }
}
