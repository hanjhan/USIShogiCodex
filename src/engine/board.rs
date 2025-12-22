use super::{
    bitboard::Bitboard,
    hand::Hand,
    movement::{Move, MoveKind},
    state::{Piece, PieceKind, PlayerSide, Square},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PositionSignature {
    pub side_to_move: PlayerSide,
    pieces: [[Bitboard; PieceKind::ALL.len()]; 2],
    promoted: [[Bitboard; PieceKind::ALL.len()]; 2],
    hands: [Hand; 2],
}

#[derive(Clone)]
pub struct PieceBitboards {
    occupancy: [Bitboard; 2],
    pieces: [[Bitboard; PieceKind::ALL.len()]; 2],
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

    pub fn place(&mut self, side: PlayerSide, kind: PieceKind, square: Square, promoted: bool) {
        let idx = side.index();
        self.occupancy[idx].set(square);
        self.pieces[idx][kind.index()].set(square);
        if promoted {
            self.promoted[idx][kind.index()].set(square);
        } else {
            self.promoted[idx][kind.index()].clear(square);
        }
    }

    pub fn remove(&mut self, side: PlayerSide, kind: PieceKind, square: Square) {
        let idx = side.index();
        self.occupancy[idx].clear(square);
        self.pieces[idx][kind.index()].clear(square);
        self.promoted[idx][kind.index()].clear(square);
    }

    pub fn piece(&self, side: PlayerSide, kind: PieceKind) -> Bitboard {
        self.pieces[side.index()][kind.index()]
    }

    pub fn promoted(&self, side: PlayerSide, kind: PieceKind) -> Bitboard {
        self.promoted[side.index()][kind.index()]
    }

    pub fn occupancy(&self, side: PlayerSide) -> Bitboard {
        self.occupancy[side.index()]
    }

    pub fn occupancy_all(&self) -> Bitboard {
        self.occupancy[0] | self.occupancy[1]
    }
}

impl Default for PieceBitboards {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Board {
    bitboards: PieceBitboards,
    hands: [Hand; 2],
    side_to_move: PlayerSide,
    ply: u32,
}

impl Board {
    pub fn new_standard() -> Self {
        let mut board = Self {
            bitboards: PieceBitboards::new(),
            hands: [Hand::default(), Hand::default()],
            side_to_move: PlayerSide::Sente,
            ply: 0,
        };
        board.setup_standard();
        board
    }

    fn setup_standard(&mut self) {
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

        for (file, kind) in BACK_RANK.iter().enumerate() {
            let top = Square::from_coords(file as u8, 0).expect("valid square");
            let bottom = Square::from_coords(file as u8, 8).expect("valid square");
            self.place_piece(PlayerSide::Gote, *kind, top, false);
            self.place_piece(PlayerSide::Sente, *kind, bottom, false);
        }

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

        for file in 0..9 {
            let gote_pawn = Square::from_coords(file, 2).expect("valid square");
            let sente_pawn = Square::from_coords(file, 6).expect("valid square");
            self.place_piece(PlayerSide::Gote, PieceKind::Pawn, gote_pawn, false);
            self.place_piece(PlayerSide::Sente, PieceKind::Pawn, sente_pawn, false);
        }
    }

    pub fn clear(&mut self) {
        self.bitboards = PieceBitboards::new();
        self.hands = [Hand::default(), Hand::default()];
        self.side_to_move = PlayerSide::Sente;
        self.ply = 0;
    }

    pub fn bitboards(&self) -> &PieceBitboards {
        &self.bitboards
    }

    pub fn signature(&self) -> PositionSignature {
        PositionSignature {
            side_to_move: self.side_to_move,
            pieces: self.bitboards.pieces,
            promoted: self.bitboards.promoted,
            hands: self.hands,
        }
    }

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

    pub fn to_move(&self) -> PlayerSide {
        self.side_to_move
    }

    pub fn set_to_move(&mut self, side: PlayerSide) {
        self.side_to_move = side;
    }

    pub fn ply(&self) -> u32 {
        self.ply
    }

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

    pub fn apply_move(&mut self, mv: Move) {
        self.ply += 1;
        match mv.kind {
            MoveKind::Drop => {
                if self.hand_mut(mv.player).remove(mv.piece) {
                    self.bitboards.place(mv.player, mv.piece, mv.to, false);
                }
            }
            _ => {
                if let Some(captured) = mv.capture {
                    let opponent = mv.player.opponent();
                    self.bitboards.remove(opponent, captured, mv.to);
                    self.hand_mut(mv.player).add(captured);
                }
                if let Some(from) = mv.from {
                    let mut promoted_state =
                        self.bitboards.promoted(mv.player, mv.piece).is_set(from);
                    if mv.promote {
                        promoted_state = true;
                    }
                    self.bitboards.remove(mv.player, mv.piece, from);
                    self.bitboards
                        .place(mv.player, mv.piece, mv.to, promoted_state);
                }
            }
        }
        self.side_to_move = self.side_to_move.opponent();
    }
}

impl Default for Board {
    fn default() -> Self {
        Board::new_standard()
    }
}
