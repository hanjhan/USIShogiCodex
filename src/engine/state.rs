use std::fmt;

// Core types for describing the state of a shogi position:
//   PlayerSide  — which of the two players (Sente / Gote)
//   PieceKind   — the type of a piece (King, Rook, …, Pawn)
//   Piece       — a (owner, kind, promoted) triple
//   Square      — a single board square encoded as rank*9 + file (0–80)
//
// Square coordinate system:
//   file 0 = column 9 (leftmost from Sente's view)  ... file 8 = column 1
//   rank 0 = row 1 (Gote's back rank)               ... rank 8 = row 9 (Sente's back rank)
//
// In standard shogi notation, the square "77" is file=2, rank=6 (0-based).
// Square::from_notation handles the translation from human-readable notation.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayerSide {
    Sente, // First player (moves first, pieces displayed in uppercase)
    Gote,  // Second player (pieces displayed in lowercase)
}

impl PlayerSide {
    pub const ALL: [PlayerSide; 2] = [PlayerSide::Sente, PlayerSide::Gote];

    pub fn opponent(self) -> Self {
        match self {
            PlayerSide::Sente => PlayerSide::Gote,
            PlayerSide::Gote => PlayerSide::Sente,
        }
    }

    /// Returns the 0-based index used to index into two-element arrays.
    pub fn index(self) -> usize {
        match self {
            PlayerSide::Sente => 0,
            PlayerSide::Gote => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PlayerSide::Sente => "Sente",
            PlayerSide::Gote => "Gote",
        }
    }
}

/// The type of a shogi piece, ignoring promotion state.
/// The `index()` method maps each variant to a stable 0-based integer so that
/// piece types can be used as array indices (e.g. in bitboard tables, hand
/// counters, and history heuristic tables).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    King,   // 王将 / 玉将 — cannot be promoted, never captured
    Rook,   // 飛車 — promotes to Dragon (竜王)
    Bishop, // 角行 — promotes to Horse (竜馬)
    Gold,   // 金将 — cannot be promoted
    Silver, // 銀将 — promotes to Promoted Silver (成銀)
    Knight, // 桂馬 — promotes to Promoted Knight (成桂)
    Lance,  // 香車 — promotes to Promoted Lance (成香)
    Pawn,   // 歩兵 — promotes to Tokin (と金)
}

impl PieceKind {
    pub const ALL: [PieceKind; 8] = [
        PieceKind::King,
        PieceKind::Rook,
        PieceKind::Bishop,
        PieceKind::Gold,
        PieceKind::Silver,
        PieceKind::Knight,
        PieceKind::Lance,
        PieceKind::Pawn,
    ];

    /// Returns a stable 0-based index (matches the order in `ALL`).
    /// Used for array indexing throughout the engine.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Single-character abbreviation used in USI notation and board display.
    pub fn short_name(self) -> &'static str {
        match self {
            PieceKind::King => "K",
            PieceKind::Rook => "R",
            PieceKind::Bishop => "B",
            PieceKind::Gold => "G",
            PieceKind::Silver => "S",
            PieceKind::Knight => "N",
            PieceKind::Lance => "L",
            PieceKind::Pawn => "P",
        }
    }

    /// Parses a single-character piece abbreviation (case-insensitive).
    pub fn from_char(ch: char) -> Option<Self> {
        match ch.to_ascii_uppercase() {
            'K' => Some(PieceKind::King),
            'R' => Some(PieceKind::Rook),
            'B' => Some(PieceKind::Bishop),
            'G' => Some(PieceKind::Gold),
            'S' => Some(PieceKind::Silver),
            'N' => Some(PieceKind::Knight),
            'L' => Some(PieceKind::Lance),
            'P' => Some(PieceKind::Pawn),
            _ => None,
        }
    }

    /// Returns true if this piece type can be promoted.
    /// King and Gold can never be promoted in standard shogi rules.
    pub fn promotable(self) -> bool {
        !matches!(self, PieceKind::King | PieceKind::Gold)
    }
}

/// A piece on the board: its owner, type, and whether it is promoted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub owner: PlayerSide,
    pub kind: PieceKind,
    pub promoted: bool,
}

impl Piece {
    pub fn new(owner: PlayerSide, kind: PieceKind) -> Self {
        Self {
            owner,
            kind,
            promoted: false,
        }
    }

    pub fn is_promoted(self) -> bool {
        self.promoted
    }
}

/// A single square on the 9×9 board, encoded as `rank * 9 + file` (0–80).
///
/// Coordinate ranges:
///   file: 0–8 (maps to shogi columns 9–1, so file=0 is the leftmost column)
///   rank: 0–8 (rank=0 is Gote's back rank, rank=8 is Sente's back rank)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square(u8);

impl Square {
    /// Creates a square from 0-based (file, rank) coordinates.
    /// Returns None if either coordinate is out of range.
    pub fn from_coords(file: u8, rank: u8) -> Option<Self> {
        if file < 9 && rank < 9 {
            Some(Self(rank * 9 + file))
        } else {
            None
        }
    }

    /// Creates a square from 1-based (file, rank) coordinates as used in
    /// standard shogi notation (e.g. file=7, rank=7 → square "77").
    pub fn from_file_rank(file: u8, rank: u8) -> Option<Self> {
        if !(1..=9).contains(&file) || !(1..=9).contains(&rank) {
            return None;
        }
        Self::from_coords(file - 1, rank - 1)
    }

    /// Creates a square from its raw index (0–80).
    pub fn from_index(idx: u8) -> Option<Self> {
        if idx < 81 { Some(Self(idx)) } else { None }
    }

    /// Returns the 0-based file (column) of the square.
    pub fn file(self) -> u8 {
        self.0 % 9
    }

    /// Returns the 0-based rank (row) of the square.
    pub fn rank(self) -> u8 {
        self.0 / 9
    }

    pub fn coords(self) -> (u8, u8) {
        (self.file(), self.rank())
    }

    /// Returns the raw index (0–80) used in bitboard bit positions.
    pub fn index(self) -> u8 {
        self.0
    }

    /// Parses a square from a two-character string like "77" or "7g".
    /// The first character is the file digit (1–9), the second is either a
    /// digit (1–9) or a letter ('a'–'i') for the rank.
    pub fn from_notation(file_char: char, rank_char: char) -> Option<Self> {
        if !file_char.is_ascii_digit() {
            return None;
        }
        let file_digit = file_char.to_digit(10)? as u8;
        if !(1..=9).contains(&file_digit) {
            return None;
        }
        // Shogi file notation: "1" is the rightmost column (file index 8),
        // "9" is the leftmost (file index 0).
        let file = 9 - file_digit;
        let rank = if rank_char.is_ascii_digit() {
            let rank_digit = rank_char.to_digit(10)? as u8;
            if !(1..=9).contains(&rank_digit) {
                return None;
            }
            rank_digit - 1
        } else {
            // Letter rank: 'a' = rank 1 (index 0), 'i' = rank 9 (index 8)
            let rank_letter = rank_char.to_ascii_lowercase();
            let rank_byte = rank_letter as u8;
            if !(b'a'..=b'i').contains(&rank_byte) {
                return None;
            }
            rank_byte - b'a'
        };
        Square::from_coords(file, rank)
    }

    /// Parses a square from a two-character text slice (delegates to `from_notation`).
    pub fn from_text(coord: &str) -> Option<Self> {
        if coord.len() != 2 {
            return None;
        }
        let mut chars = coord.chars();
        let file = chars.next()?;
        let rank = chars.next()?;
        Square::from_notation(file, rank)
    }

    /// Returns the square reached by moving (df, dr) steps in absolute
    /// coordinates, or None if the result is off the board.
    pub fn offset(self, df: i8, dr: i8) -> Option<Self> {
        let file = self.file() as i8 + df;
        let rank = self.rank() as i8 + dr;
        if (0..=8).contains(&file) && (0..=8).contains(&rank) {
            Square::from_coords(file as u8, rank as u8)
        } else {
            None
        }
    }

    /// Returns the square reached by moving (df, dr) steps *from the
    /// perspective of `side`*.  For Gote, both deltas are negated so that
    /// "forward" always means toward the opponent's back rank regardless of
    /// which side is moving.
    pub fn offset_from_perspective(self, side: PlayerSide, df: i8, dr: i8) -> Option<Self> {
        match side {
            PlayerSide::Sente => self.offset(df, dr),
            PlayerSide::Gote => self.offset(-df, -dr),
        }
    }

    /// Returns true if this square is inside the promotion zone for `side`.
    /// Sente's promotion zone is ranks 0–2 (rows 1–3, the far side of the board).
    /// Gote's promotion zone is ranks 6–8 (rows 7–9).
    pub fn in_promotion_zone(self, side: PlayerSide) -> bool {
        match side {
            PlayerSide::Sente => self.rank() <= 2,
            PlayerSide::Gote => self.rank() >= 6,
        }
    }
}

/// Displays a square in standard shogi notation: file digit (9 down to 1)
/// followed by rank digit (1 up to 9), e.g. "77" or "19".
impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = 9 - self.file();
        let rank = self.rank() + 1;
        write!(f, "{}{}", file, rank)
    }
}
