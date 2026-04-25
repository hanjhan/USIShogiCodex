// SFEN (Shogi Forsyth-Edwards Notation) parser.
//
// An SFEN string encodes a full shogi position as four whitespace-separated
// fields:
//
//   <board> <side_to_move> <hands> <move_number>
//
// * `<board>` — 9 ranks separated by '/', top rank (Gote's back rank) first.
//   Each rank lists cells left-to-right (left = file 9 in shogi notation).
//   A digit 1–9 encodes that many empty squares in a row.  A letter encodes
//   a piece: uppercase = Sente, lowercase = Gote.  A leading '+' promotes
//   the following piece.
// * `<side_to_move>` — `b` for Sente (black/先手), `w` for Gote (white/後手).
// * `<hands>` — `-` when both hands are empty, otherwise a concatenation of
//   `[count]piece` tokens.  The count is omitted when equal to 1.  Uppercase
//   pieces belong to Sente, lowercase to Gote.
// * `<move_number>` — an integer (accepted but not stored by this parser).
//
// Example (starting position):
//   lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1
//
// The parser is strict: any malformed token produces a descriptive error so
// the caller can show it back to the user.

use crate::engine::{
    board::Board,
    state::{PieceKind, PlayerSide, Square},
};

/// Parses an SFEN string into a fully-initialised `Board`.
///
/// Returns a human-readable error message on invalid input.  The resulting
/// board has its Zobrist hash and incremental evaluation recomputed.
pub fn parse_sfen(sfen: &str) -> Result<Board, String> {
    let parts: Vec<&str> = sfen.split_whitespace().collect();
    if parts.len() != 3 && parts.len() != 4 {
        return Err(format!(
            "SFEN must have 3 or 4 whitespace-separated fields, found {}",
            parts.len()
        ));
    }

    let mut board = Board::new_standard();
    board.clear();

    parse_board(&mut board, parts[0])?;
    parse_side(&mut board, parts[1])?;
    parse_hands(&mut board, parts[2])?;
    // Move number (parts[3]) is accepted but intentionally ignored — the
    // engine's ply counter starts at 0 for any position, which is fine for
    // analysis.  We still validate the field is a non-negative integer.
    if let Some(num) = parts.get(3)
        && num.parse::<u32>().is_err()
    {
        return Err(format!("move number must be a non-negative integer, got '{}'", num));
    }

    board.recompute_zobrist();
    Ok(board)
}

fn parse_board(board: &mut Board, text: &str) -> Result<(), String> {
    let ranks: Vec<&str> = text.split('/').collect();
    if ranks.len() != 9 {
        return Err(format!(
            "board field must have 9 ranks separated by '/', found {}",
            ranks.len()
        ));
    }
    for (rank_idx, rank_str) in ranks.iter().enumerate() {
        let mut file: u8 = 0;
        let mut chars = rank_str.chars().peekable();
        while let Some(ch) = chars.next() {
            if file >= 9 {
                return Err(format!("rank {} has too many squares", rank_idx + 1));
            }
            if ch.is_ascii_digit() {
                let n = ch.to_digit(10).unwrap() as u8;
                if !(1..=9).contains(&n) {
                    return Err(format!("invalid empty-count '{}' in rank {}", n, rank_idx + 1));
                }
                file = file.checked_add(n).ok_or_else(|| {
                    format!("rank {} overflows past file 9", rank_idx + 1)
                })?;
            } else {
                let promoted = ch == '+';
                let piece_char = if promoted {
                    chars.next().ok_or_else(|| {
                        format!("rank {}: '+' not followed by a piece letter", rank_idx + 1)
                    })?
                } else {
                    ch
                };
                let (side, kind) = decode_piece(piece_char).ok_or_else(|| {
                    format!(
                        "rank {}: unknown piece symbol '{}'",
                        rank_idx + 1,
                        piece_char
                    )
                })?;
                if promoted && !kind.promotable() {
                    return Err(format!(
                        "rank {}: piece '{}' cannot be promoted",
                        rank_idx + 1,
                        piece_char
                    ));
                }
                let square = Square::from_coords(file, rank_idx as u8).ok_or_else(|| {
                    format!("rank {}: square out of range at file {}", rank_idx + 1, file)
                })?;
                board.place_piece(side, kind, square, promoted);
                file += 1;
            }
        }
        if file != 9 {
            return Err(format!(
                "rank {} covers {} files, expected 9",
                rank_idx + 1,
                file
            ));
        }
    }
    Ok(())
}

fn parse_side(board: &mut Board, text: &str) -> Result<(), String> {
    let side = match text {
        "b" => PlayerSide::Sente,
        "w" => PlayerSide::Gote,
        other => {
            return Err(format!(
                "side-to-move must be 'b' (Sente) or 'w' (Gote), got '{}'",
                other
            ));
        }
    };
    board.set_to_move(side);
    Ok(())
}

fn parse_hands(board: &mut Board, text: &str) -> Result<(), String> {
    if text == "-" {
        return Ok(());
    }
    let mut chars = text.chars().peekable();
    while let Some(&ch) = chars.peek() {
        let count: u32 = if ch.is_ascii_digit() {
            let mut n: u32 = 0;
            while let Some(&d) = chars.peek() {
                if let Some(digit) = d.to_digit(10) {
                    n = n.saturating_mul(10).saturating_add(digit);
                    chars.next();
                } else {
                    break;
                }
            }
            if n == 0 {
                return Err("hand count cannot be zero".to_string());
            }
            n
        } else {
            1
        };
        let piece_ch = chars
            .next()
            .ok_or_else(|| "hand count without a piece letter".to_string())?;
        let (side, kind) = decode_piece(piece_ch)
            .ok_or_else(|| format!("unknown hand piece '{}'", piece_ch))?;
        if kind == PieceKind::King {
            return Err("King cannot be held in hand".to_string());
        }
        for _ in 0..count {
            board.hand_mut(side).add(kind);
        }
    }
    Ok(())
}

/// Decodes a single piece letter into (side, kind).  Case determines the side:
/// uppercase → Sente, lowercase → Gote.
fn decode_piece(ch: char) -> Option<(PlayerSide, PieceKind)> {
    let side = if ch.is_ascii_uppercase() {
        PlayerSide::Sente
    } else if ch.is_ascii_lowercase() {
        PlayerSide::Gote
    } else {
        return None;
    };
    PieceKind::from_char(ch).map(|kind| (side, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_round_trips() {
        let sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        let board = parse_sfen(sfen).expect("startpos must parse");
        let reference = Board::new_standard();
        assert_eq!(board.zobrist(), reference.zobrist());
        assert_eq!(board.to_move(), PlayerSide::Sente);
    }

    #[test]
    fn rejects_too_few_fields() {
        assert!(parse_sfen("lnsgkgsnl/9/9/9/9/9/9/9/LNSGKGSNL b").is_err());
    }

    #[test]
    fn rejects_invalid_symbol() {
        assert!(parse_sfen("xnsgkgsnl/9/9/9/9/9/9/9/LNSGKGSNL b - 1").is_err());
    }

    #[test]
    fn parses_hands() {
        let sfen = "9/9/9/9/9/9/9/9/9 b R2Pbp 1";
        let board = parse_sfen(sfen).expect("parse");
        assert_eq!(board.hand(PlayerSide::Sente).count(PieceKind::Rook), 1);
        assert_eq!(board.hand(PlayerSide::Sente).count(PieceKind::Pawn), 2);
        assert_eq!(board.hand(PlayerSide::Gote).count(PieceKind::Bishop), 1);
        assert_eq!(board.hand(PlayerSide::Gote).count(PieceKind::Pawn), 1);
    }
}
