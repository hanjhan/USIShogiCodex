// Parses a single line of user input in thinking mode into a structured
// command.  Accepts both CLI-style (`7776`, `2888+`, `P*57`) and USI-style
// (`7g7f`, `2b8h+`, `P*5e`) move notations because `Square::from_text`
// already handles both rank encodings (digit 1–9 or letter a–i).

use crate::engine::{
    movelist::MoveList,
    movement::{Move, MoveKind},
    state::{PieceKind, Square},
};

/// Top-level interpretation of a line the user typed.
#[derive(Clone, Debug)]
pub enum Command {
    /// Leave the program.
    Quit,
    /// Undo the most recently applied move.
    Undo,
    /// Apply a move specified by the user.  The spec is resolved against the
    /// current position's legal moves later via `resolve_move`.
    Move(MoveSpec),
    /// Show the in-app help text.
    Help,
    /// Print the list of legal moves for the current position.
    ListMoves,
    /// Input was empty (whitespace only).
    Empty,
    /// Input was non-empty but could not be interpreted.
    Unknown(String),
}

/// Loose description of a move parsed from user input, before it is matched
/// against the legal move list.  `promote = None` means "no preference"
/// (used when the user typed a bare `7776` that happens to be a promotable
/// pawn push — we'll default to non-promote).
#[derive(Clone, Debug)]
pub struct MoveSpec {
    pub from: Option<Square>,
    pub to: Square,
    pub drop: Option<PieceKind>,
    pub promote: Option<bool>,
}

pub fn parse(line: &str) -> Command {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Command::Empty;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "quit" | "exit" | ":q" => return Command::Quit,
        "undo" | "u" => return Command::Undo,
        "help" | "?" => return Command::Help,
        "moves" => return Command::ListMoves,
        _ => {}
    }
    if let Some(spec) = parse_move_spec(trimmed) {
        return Command::Move(spec);
    }
    Command::Unknown(trimmed.to_string())
}

/// Attempts to decode a move notation.  Returns `None` if the text does not
/// look like a move at all.
fn parse_move_spec(token: &str) -> Option<MoveSpec> {
    // Drop: <piece>*<square>, e.g. "P*57" or "p*5e".
    if let Some((piece_part, rest)) = token.split_once('*') {
        let piece_char = piece_part.chars().next()?;
        let kind = PieceKind::from_char(piece_char.to_ascii_uppercase())?;
        let to = Square::from_text(rest.trim())?;
        return Some(MoveSpec {
            from: None,
            to,
            drop: Some(kind),
            promote: Some(false),
        });
    }

    // Normal move: <from><to>[+|=], where each coordinate is 2 chars.
    let mut text = token;
    let mut promote = None;
    if let Some(stripped) = text.strip_suffix('+') {
        text = stripped;
        promote = Some(true);
    } else if let Some(stripped) = text.strip_suffix('=') {
        text = stripped;
        promote = Some(false);
    }
    if text.len() != 4 {
        return None;
    }
    let from = Square::from_text(&text[0..2])?;
    let to = Square::from_text(&text[2..4])?;
    Some(MoveSpec {
        from: Some(from),
        to,
        drop: None,
        promote,
    })
}

/// Resolves a `MoveSpec` against the current legal-move list.  Returns the
/// matched `Move`, or `None` if no legal move matches.  When the user did
/// not specify `+` or `=` and both promote and non-promote variants exist,
/// we prefer the non-promoting form (conservative default; users can always
/// re-enter with `+` to promote).
pub fn resolve_move(spec: &MoveSpec, legal: &MoveList) -> Option<Move> {
    let mut candidates: Vec<Move> = legal
        .iter()
        .copied()
        .filter(|mv| match spec.drop {
            Some(kind) => mv.kind == MoveKind::Drop && mv.piece == kind && mv.to == spec.to,
            None => mv.from == spec.from && mv.to == spec.to && mv.kind != MoveKind::Drop,
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if let Some(choice) = spec.promote {
        candidates.retain(|mv| mv.promote == choice);
        if candidates.is_empty() {
            return None;
        }
    }
    if candidates.len() > 1
        && let Some(non_promo) = candidates.iter().find(|mv| !mv.promote)
    {
        return Some(*non_promo);
    }
    candidates.into_iter().next()
}
