use crate::engine::{
    board::Board,
    movement::Move,
    state::{PieceKind, PlayerSide, Square},
};

pub struct MoveGenerator;

impl MoveGenerator {
    pub fn legal_moves(board: &Board) -> Vec<Move> {
        Self::generate_internal(board, board.to_move(), true, true)
    }

    pub fn legal_moves_for(board: &Board, side: PlayerSide) -> Vec<Move> {
        Self::generate_internal(board, side, true, true)
    }

    pub fn legal_moves_for_options(
        board: &Board,
        side: PlayerSide,
        check_drop_mate: bool,
    ) -> Vec<Move> {
        Self::generate_internal(board, side, true, check_drop_mate)
    }

    pub fn pseudo_legal_moves(board: &Board, side: PlayerSide) -> Vec<Move> {
        Self::generate_internal(board, side, false, false)
    }

    pub fn is_in_check(board: &Board, side: PlayerSide) -> bool {
        if let Some(king_sq) = board.king_square(side) {
            Self::is_square_attacked(board, king_sq, side.opponent())
        } else {
            true
        }
    }

    fn generate_internal(
        board: &Board,
        side: PlayerSide,
        enforce_legality: bool,
        check_drop_mate: bool,
    ) -> Vec<Move> {
        let mut moves = Vec::new();
        Self::generate_piece_moves(board, side, enforce_legality, &mut moves);
        Self::generate_drop_moves(board, side, enforce_legality, check_drop_mate, &mut moves);
        moves
    }

    fn generate_piece_moves(
        board: &Board,
        side: PlayerSide,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
    ) {
        for &kind in &PieceKind::ALL {
            let mut squares = board.bitboards().piece(side, kind).iter_bits();
            let promoted_bb = board.bitboards().promoted(side, kind);
            while let Some(square) = squares.next() {
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

    fn generate_moves_for_piece(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
    ) {
        match kind {
            PieceKind::King => Self::generate_step_moves(
                board,
                side,
                kind,
                promoted,
                from,
                KING_STEPS,
                enforce_legality,
                moves,
            ),
            PieceKind::Gold => Self::generate_step_moves(
                board,
                side,
                kind,
                promoted,
                from,
                GOLD_STEPS,
                enforce_legality,
                moves,
            ),
            PieceKind::Silver => {
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        GOLD_STEPS,
                        enforce_legality,
                        moves,
                    );
                } else {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        SILVER_STEPS,
                        enforce_legality,
                        moves,
                    );
                }
            }
            PieceKind::Knight => {
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        GOLD_STEPS,
                        enforce_legality,
                        moves,
                    );
                } else {
                    Self::generate_knight_moves(board, side, kind, from, enforce_legality, moves);
                }
            }
            PieceKind::Lance => {
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        GOLD_STEPS,
                        enforce_legality,
                        moves,
                    );
                } else {
                    Self::generate_lance_moves(board, side, kind, from, enforce_legality, moves);
                }
            }
            PieceKind::Pawn => {
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        GOLD_STEPS,
                        enforce_legality,
                        moves,
                    );
                } else {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        PAWN_STEPS,
                        enforce_legality,
                        moves,
                    );
                }
            }
            PieceKind::Rook => {
                Self::generate_slide_moves(
                    board,
                    side,
                    kind,
                    promoted,
                    from,
                    ROOK_SLIDES,
                    enforce_legality,
                    moves,
                );
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        BISHOP_STEPS,
                        enforce_legality,
                        moves,
                    );
                }
            }
            PieceKind::Bishop => {
                Self::generate_slide_moves(
                    board,
                    side,
                    kind,
                    promoted,
                    from,
                    BISHOP_SLIDES,
                    enforce_legality,
                    moves,
                );
                if promoted {
                    Self::generate_step_moves(
                        board,
                        side,
                        kind,
                        promoted,
                        from,
                        ROOK_STEPS,
                        enforce_legality,
                        moves,
                    );
                }
            }
        }
    }

    fn generate_step_moves(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        deltas: &[(i8, i8)],
        enforce_legality: bool,
        moves: &mut Vec<Move>,
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

    fn generate_knight_moves(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        from: Square,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
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
                    board,
                    side,
                    kind,
                    false,
                    from,
                    to,
                    capture,
                    enforce_legality,
                    moves,
                );
            }
        }
    }

    fn generate_lance_moves(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        from: Square,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
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

    fn generate_slide_moves(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        directions: &[(i8, i8)],
        enforce_legality: bool,
        moves: &mut Vec<Move>,
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
                Self::emit_move(
                    board,
                    side,
                    kind,
                    promoted,
                    from,
                    next,
                    capture,
                    enforce_legality,
                    moves,
                );
                if capture.is_some() {
                    break;
                }
                current = next;
            }
        }
    }

    fn generate_drop_moves(
        board: &Board,
        side: PlayerSide,
        enforce_legality: bool,
        check_drop_mate: bool,
        moves: &mut Vec<Move>,
    ) {
        let hand = board.hand(side);
        let occupied = board.bitboards().occupancy_all();
        for &kind in &PieceKind::ALL {
            if kind == PieceKind::King {
                continue;
            }
            let count = hand.count(kind);
            if count == 0 {
                continue;
            }
            for idx in 0..81 {
                if let Some(square) = Square::from_index(idx as u8) {
                    if occupied.is_set(square) {
                        continue;
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

    fn emit_move(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        promoted: bool,
        from: Square,
        to: Square,
        capture: Option<PieceKind>,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
    ) {
        Self::for_each_promotion_choice(kind, promoted, side, from, to, |promote| {
            let mv = Move::normal(side, from, to, kind, capture, promote);
            Self::push_move(board, side, mv, enforce_legality, moves);
        });
    }

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
            f(true);
        } else {
            f(false);
            f(true);
        }
    }

    fn must_promote(kind: PieceKind, side: PlayerSide, to: Square) -> bool {
        match (side, kind) {
            (PlayerSide::Sente, PieceKind::Pawn | PieceKind::Lance) => to.rank() == 0,
            (PlayerSide::Gote, PieceKind::Pawn | PieceKind::Lance) => to.rank() == 8,
            (PlayerSide::Sente, PieceKind::Knight) => to.rank() <= 1,
            (PlayerSide::Gote, PieceKind::Knight) => to.rank() >= 7,
            _ => false,
        }
    }

    fn is_drop_legal(
        board: &Board,
        side: PlayerSide,
        kind: PieceKind,
        square: Square,
        check_drop_mate: bool,
    ) -> bool {
        match kind {
            PieceKind::Pawn => {
                if Self::is_last_rank(side, square.rank()) {
                    return false;
                }
                if Self::has_pawn_on_file(board, side, square.file()) {
                    return false;
                }
                if check_drop_mate && Self::is_pawn_drop_mate(board, side, square) {
                    return false;
                }
                true
            }
            PieceKind::Lance => !Self::is_last_rank(side, square.rank()),
            PieceKind::Knight => !Self::is_knight_dead_rank(side, square.rank()),
            PieceKind::King => false,
            _ => true,
        }
    }

    fn is_pawn_drop_mate(board: &Board, side: PlayerSide, square: Square) -> bool {
        let mut clone = board.clone();
        let drop_move = Move::drop(side, PieceKind::Pawn, square);
        clone.apply_move(drop_move);
        if !Self::is_in_check(&clone, side.opponent()) {
            return false;
        }
        let replies = Self::legal_moves_for(&clone, side.opponent());
        replies.is_empty()
    }

    fn is_last_rank(side: PlayerSide, rank: u8) -> bool {
        match side {
            PlayerSide::Sente => rank == 0,
            PlayerSide::Gote => rank == 8,
        }
    }

    fn is_knight_dead_rank(side: PlayerSide, rank: u8) -> bool {
        match side {
            PlayerSide::Sente => rank <= 1,
            PlayerSide::Gote => rank >= 7,
        }
    }

    fn has_pawn_on_file(board: &Board, side: PlayerSide, file: u8) -> bool {
        let pawns = board.bitboards().piece(side, PieceKind::Pawn);
        let promoted = board.bitboards().promoted(side, PieceKind::Pawn);
        let mut bits = pawns.iter_bits();
        while let Some(square) = bits.next() {
            if square.file() == file && !promoted.is_set(square) {
                return true;
            }
        }
        false
    }

    fn push_move(
        board: &Board,
        side: PlayerSide,
        mv: Move,
        enforce_legality: bool,
        moves: &mut Vec<Move>,
    ) {
        if enforce_legality {
            let mut next = board.clone();
            next.apply_move(mv);
            if !Self::is_in_check(&next, side) {
                moves.push(mv);
            }
        } else {
            moves.push(mv);
        }
    }

    fn piece_kind_at(board: &Board, side: PlayerSide, square: Square) -> Option<PieceKind> {
        for &kind in &PieceKind::ALL {
            if board.bitboards().piece(side, kind).is_set(square) {
                return Some(kind);
            }
        }
        None
    }

    fn is_square_attacked(board: &Board, square: Square, attacker: PlayerSide) -> bool {
        let moves = Self::pseudo_legal_moves(board, attacker);
        moves
            .into_iter()
            .any(|mv| mv.to == square && mv.capture.is_some())
    }
}

const KING_STEPS: &[(i8, i8)] = &[
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

const GOLD_STEPS: &[(i8, i8)] = &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (0, 1)];

const SILVER_STEPS: &[(i8, i8)] = &[(-1, -1), (0, -1), (1, -1), (-1, 1), (1, 1)];

const KNIGHT_JUMPS: &[(i8, i8)] = &[(-1, -2), (1, -2)];
const PAWN_STEPS: &[(i8, i8)] = &[(0, -1)];
const LANCE_DIRECTIONS: &[(i8, i8)] = &[(0, -1)];
const ROOK_SLIDES: &[(i8, i8)] = &[(0, -1), (0, 1), (-1, 0), (1, 0)];
const BISHOP_SLIDES: &[(i8, i8)] = &[(1, -1), (-1, -1), (1, 1), (-1, 1)];
const BISHOP_STEPS: &[(i8, i8)] = BISHOP_SLIDES;
const ROOK_STEPS: &[(i8, i8)] = ROOK_SLIDES;
