use crate::engine::{
    board::Board,
    state::{Piece, PieceKind, PlayerSide, Square},
};

pub struct BoardRenderer;

impl BoardRenderer {
    pub fn render(board: &Board) -> String {
        let mut lines = Vec::new();
        lines.push("      9   8   7   6   5   4   3   2   1".to_string());
        lines.push("    +---+---+---+---+---+---+---+---+---+".to_string());
        for rank in 0..9 {
            let row_label = rank + 1;
            let mut row = format!(" {}  |", row_label);
            for file in 0..9 {
                let square = Square::from_coords(file as u8, rank as u8).expect("valid square");
                let cell = match board.piece_at(square) {
                    Some(piece) => Self::piece_symbol(piece),
                    None => " .".to_string(),
                };
                row.push_str(&cell);
                row.push('|');
            }
            lines.push(row);
            lines.push("    +---+---+---+---+---+---+---+---+---+".to_string());
        }
        lines.push(format!(
            "Sente in hand: {}",
            Self::hand_summary(board, PlayerSide::Sente)
        ));
        lines.push(format!(
            "Gote  in hand: {}",
            Self::hand_summary(board, PlayerSide::Gote)
        ));
        lines.join("\n")
    }

    pub fn print(board: &Board) {
        println!("{}", Self::render(board));
    }

    fn piece_symbol(piece: Piece) -> String {
        let mut text = String::new();
        if piece.is_promoted() {
            text.push('+');
        } else {
            text.push(' ');
        }
        let mut chars = piece.kind.short_name().chars();
        if let Some(letter) = chars.next() {
            let adjusted = match piece.owner {
                PlayerSide::Sente => letter,
                PlayerSide::Gote => letter.to_ascii_lowercase(),
            };
            text.push(adjusted);
        }
        text
    }

    fn hand_summary(board: &Board, side: PlayerSide) -> String {
        let hand = board.hand(side);
        let mut parts = Vec::new();
        for &kind in &PieceKind::ALL {
            let count = hand.count(kind);
            if count > 0 {
                parts.push(format!("{}{}", kind.short_name(), count));
            }
        }
        if parts.is_empty() {
            "(none)".to_string()
        } else {
            parts.join(" ")
        }
    }
}
