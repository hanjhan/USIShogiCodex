use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayerSide {
    Sente,
    Gote,
}

impl PlayerSide {
    pub const ALL: [PlayerSide; 2] = [PlayerSide::Sente, PlayerSide::Gote];

    pub fn opponent(self) -> Self {
        match self {
            PlayerSide::Sente => PlayerSide::Gote,
            PlayerSide::Gote => PlayerSide::Sente,
        }
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    King,
    Rook,
    Bishop,
    Gold,
    Silver,
    Knight,
    Lance,
    Pawn,
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

    pub fn index(self) -> usize {
        self as usize
    }

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

    pub fn promotable(self) -> bool {
        !matches!(self, PieceKind::King | PieceKind::Gold)
    }
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square(u8);

impl Square {
    pub fn from_coords(file: u8, rank: u8) -> Option<Self> {
        if file < 9 && rank < 9 {
            Some(Self(rank * 9 + file))
        } else {
            None
        }
    }

    pub fn from_file_rank(file: u8, rank: u8) -> Option<Self> {
        if !(1..=9).contains(&file) || !(1..=9).contains(&rank) {
            return None;
        }
        Self::from_coords(file - 1, rank - 1)
    }

    pub fn from_index(idx: u8) -> Option<Self> {
        if idx < 81 { Some(Self(idx)) } else { None }
    }

    pub fn file(self) -> u8 {
        self.0 % 9
    }

    pub fn rank(self) -> u8 {
        self.0 / 9
    }

    pub fn coords(self) -> (u8, u8) {
        (self.file(), self.rank())
    }

    pub fn index(self) -> u8 {
        self.0
    }

    pub fn from_notation(file_char: char, rank_char: char) -> Option<Self> {
        if !file_char.is_ascii_digit() {
            return None;
        }
        let file_digit = file_char.to_digit(10)? as u8;
        if !(1..=9).contains(&file_digit) {
            return None;
        }
        let file = 9 - file_digit;
        let rank = if rank_char.is_ascii_digit() {
            let rank_digit = rank_char.to_digit(10)? as u8;
            if !(1..=9).contains(&rank_digit) {
                return None;
            }
            rank_digit - 1
        } else {
            let rank_letter = rank_char.to_ascii_lowercase();
            let rank_byte = rank_letter as u8;
            if !(b'a'..=b'i').contains(&rank_byte) {
                return None;
            }
            rank_byte - b'a'
        };
        Square::from_coords(file, rank)
    }

    pub fn from_text(coord: &str) -> Option<Self> {
        if coord.len() != 2 {
            return None;
        }
        let mut chars = coord.chars();
        let file = chars.next()?;
        let rank = chars.next()?;
        Square::from_notation(file, rank)
    }

    pub fn offset(self, df: i8, dr: i8) -> Option<Self> {
        let file = self.file() as i8 + df;
        let rank = self.rank() as i8 + dr;
        if (0..=8).contains(&file) && (0..=8).contains(&rank) {
            Square::from_coords(file as u8, rank as u8)
        } else {
            None
        }
    }

    pub fn offset_from_perspective(self, side: PlayerSide, df: i8, dr: i8) -> Option<Self> {
        match side {
            PlayerSide::Sente => self.offset(df, dr),
            PlayerSide::Gote => self.offset(-df, -dr),
        }
    }

    pub fn in_promotion_zone(self, side: PlayerSide) -> bool {
        match side {
            PlayerSide::Sente => self.rank() <= 2,
            PlayerSide::Gote => self.rank() >= 6,
        }
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = 9 - self.file();
        let rank = self.rank() + 1;
        write!(f, "{}{}", file, rank)
    }
}
