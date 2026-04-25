use crate::engine::{
    board::Board,
    state::{Piece, PieceKind, PlayerSide, Square},
};

// Board display conventions:
//   - Files run right-to-left in the printed grid (file 9 on the left,
//     file 1 on the right) following standard shogi diagram orientation.
//   - Ranks run top-to-bottom (rank 1 at the top = Gote's side).
//   - Sente pieces are shown in UPPERCASE, Gote pieces in lowercase.
//   - Promoted pieces are prefixed with '+' (e.g. "+R" for Dragon).
//
// USI (Universal Shogi Interface) notation:
//   - Board rows are separated by '/' from rank 1 (top) to rank 9 (bottom).
//   - Empty consecutive squares are represented by a digit count (like FEN).
//   - Hands: uppercase letters for Sente, lowercase for Gote; count prefixed
//     if > 1 (e.g. "2P" means two pawns).  '-' if both hands are empty.
//   - Side to move: 'b' for Sente (black), 'w' for Gote (white).

pub struct BoardRenderer;

impl BoardRenderer {
    /// Renders the board as a human-readable ASCII grid with hand summaries.
    pub fn render(board: &Board) -> String {
        let mut lines = Vec::new();
        // Column headers (file 9 to file 1, matching printed column order)
        lines.push("      9   8   7   6   5   4   3   2   1".to_string());
        lines.push("    +---+---+---+---+---+---+---+---+---+".to_string());
        for rank in 0..9 {
            let row_label = rank + 1;
            let mut row = format!(" {}  |", row_label);
            // Iterate files from 0 to 8 (index), which maps to display columns 9 to 1
            for file in 0..9 {
                let square = Square::from_coords(file as u8, rank as u8).expect("valid square");
                let cell = match board.piece_at(square) {
                    Some(piece) => Self::piece_symbol(piece),
                    None => " .".to_string(),
                };
                row.push_str(&cell);
                row.push_str(" |");
            }
            lines.push(row);
            lines.push("    +---+---+---+---+---+---+---+---+---+".to_string());
        }
        // Show each player's captured pieces below the board
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

    /// Prints `render(board)` directly to stdout.
    pub fn print(board: &Board) {
        println!("{}", Self::render(board));
    }

    /// Renders the board in USI SFEN format:
    /// `<ranks> <side> <hands> <move_number>`
    ///
    /// Example: `lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1`
    pub fn render_usi(board: &Board) -> String {
        let mut ranks = Vec::new();
        for rank in 0..9 {
            let mut line = String::new();
            let mut empty = 0;
            for file in 0..9 {
                let square = Square::from_coords(file as u8, rank as u8).expect("valid square");
                if let Some(piece) = board.piece_at(square) {
                    if empty > 0 {
                        line.push_str(&empty.to_string());
                        empty = 0;
                    }
                    line.push_str(&Self::usi_piece_symbol(piece));
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                line.push_str(&empty.to_string());
            }
            ranks.push(line);
        }
        let board_part = ranks.join("/");
        // USI uses 'b' for Sente (black/先手) and 'w' for Gote (white/後手)
        let side_part = if board.to_move() == PlayerSide::Sente {
            'b'
        } else {
            'w'
        };
        let hand_part = Self::usi_hands(board);
        // USI move numbers count full moves (both sides), not half-moves
        let move_number = 1 + (board.ply() / 2);
        format!("{} {} {} {}", board_part, side_part, hand_part, move_number)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Returns a two-character string for a piece in the board grid display.
    /// Format: `[promotion_flag][letter]`, where the flag is '+' or ' ',
    /// and the letter is uppercase for Sente, lowercase for Gote.
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

    /// Returns a compact summary of hand pieces, e.g. "R1 P3" or "(none)".
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

    /// Returns the USI symbol for a piece (promoted pieces get a '+' prefix;
    /// Sente pieces are uppercase, Gote pieces lowercase).
    fn usi_piece_symbol(piece: Piece) -> String {
        let mut text = String::new();
        if piece.is_promoted() {
            text.push('+');
        }
        let letter = piece.kind.short_name().chars().next().unwrap_or(' ');
        let adjusted = match piece.owner {
            PlayerSide::Sente => letter,
            PlayerSide::Gote => letter.to_ascii_lowercase(),
        };
        text.push(adjusted);
        text
    }

    /// Builds the hand portion of a USI SFEN string.
    /// Sente pieces are uppercase, Gote pieces lowercase.
    /// Count is prepended when > 1 (e.g. "2p" for two Gote pawns in hand).
    /// Returns "-" when both hands are empty.
    fn usi_hands(board: &Board) -> String {
        let sente = Self::usi_hand_for(board, PlayerSide::Sente, true);
        let gote = Self::usi_hand_for(board, PlayerSide::Gote, false);
        if sente.is_empty() && gote.is_empty() {
            "-".to_string()
        } else {
            format!("{}{}", sente, gote)
        }
    }

    fn usi_hand_for(board: &Board, side: PlayerSide, uppercase: bool) -> String {
        let hand = board.hand(side);
        let mut parts = Vec::new();
        for &kind in &PieceKind::ALL {
            let count = hand.count(kind);
            if count == 0 {
                continue;
            }
            let letter = kind.short_name().chars().next().unwrap_or('P');
            let adjusted = if uppercase {
                letter
            } else {
                letter.to_ascii_lowercase()
            };
            if count == 1 {
                parts.push(adjusted.to_string());
            } else {
                parts.push(format!("{}{}", count, adjusted));
            }
        }
        parts.join("")
    }
}
