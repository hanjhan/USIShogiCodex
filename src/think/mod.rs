// Thinking Mode — interactive analysis front-end.
//
// The user picks a starting position (standard startpos or a custom SFEN
// file) and then enters a loop:
//   1. The current board is printed.
//   2. A background search starts and streams `info` progress lines.
//   3. The user types a move (CLI or USI notation), `undo`, or `quit`.
//   4. The search is stopped, the command is applied to the position, and
//      the loop repeats from step 1.
//
// The session keeps a single persistent `AlphaBetaSearcher`, so each restart
// benefits from the transposition-table and killer/history tables populated
// by the previous search — this is essentially free and substantially
// improves depth-to-wall-clock ratio.
//
// The code is split into:
//   * `sfen`    — parses `<board> <side> <hand> <move_no>` into a `Board`.
//   * `command` — parses user input lines into typed `Command`s.
//   * `session` — owns the board, move history, and searcher; runs the
//                  background search as an abortable thread.
//
// The planned tree-based position navigation (storing played moves as a
// tree so the user can jump to any previously-seen branch) can be layered
// on top of `Session` without touching this file: replace the flat
// `Vec<Move>` with a tree of nodes and add commands to move between them.

pub mod command;
pub mod session;
pub mod sfen;

use std::fs;
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::cli::board_render::BoardRenderer;
use crate::engine::{
    board::Board,
    movement::{Move, MoveKind},
};
use command::{Command, resolve_move};
use session::Session;

pub fn run() {
    print_banner();

    // Spawn a dedicated stdin reader so the main thread can interleave
    // reading user input with managing the background search.
    let stdin_rx = spawn_stdin_reader();

    let board = match prompt_starting_board(&stdin_rx) {
        Some(b) => b,
        None => {
            println!("No input — exiting.");
            return;
        }
    };
    let mut session = Session::new(board);
    run_session_loop(&mut session, &stdin_rx);
    println!("Goodbye.");
}

fn print_banner() {
    println!("============== Shogi Thinking Mode ==============");
    println!("Type 'help' at any prompt to see available commands.");
    println!();
}

fn print_help_short() {
    println!();
    println!("Commands:");
    println!("  <move>       apply a move (e.g. '7776', '7g7f+', 'P*5e')");
    println!("  undo (u)     take back the last move");
    println!("  moves        list legal moves in this position");
    println!("  help (?)     show this help");
    println!("  quit / exit  end the session");
    println!();
}

/// Spawns a thread that reads lines from stdin and forwards them on a
/// channel.  The reader exits (closing the channel) on EOF.
fn spawn_stdin_reader() -> Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(text) => {
                    if tx.send(text).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

fn read_prompt(prompt: &str, rx: &Receiver<String>) -> Option<String> {
    print!("{}", prompt);
    io::stdout().flush().ok();
    rx.recv().ok()
}

/// Repeatedly asks the user whether to start from the standard opening or
/// load a custom SFEN file, until they provide a valid choice.  Returns
/// `None` only when stdin closes before a selection is made.
fn prompt_starting_board(rx: &Receiver<String>) -> Option<Board> {
    loop {
        println!("Choose a starting position:");
        println!("  [1] Standard opening (startpos)");
        println!("  [2] Load a custom board from an SFEN file");
        let choice = read_prompt("Selection (default 1): ", rx)?;
        let choice = choice.trim();
        match choice {
            "" | "1" => return Some(Board::new_standard()),
            "2" => {
                if let Some(board) = prompt_sfen_file(rx) {
                    return Some(board);
                }
                // Invalid file → loop back to the position picker.
            }
            other => println!("Unknown selection '{}'. Please pick 1 or 2.\n", other),
        }
    }
}

/// Prompts for a file path, reads and parses it as SFEN.  Returns the
/// resulting board on success; on any error, prints the error and returns
/// `None` so the caller loops back to the position menu.
fn prompt_sfen_file(rx: &Receiver<String>) -> Option<Board> {
    let path = read_prompt("Path to SFEN file: ", rx)?;
    let path = path.trim();
    if path.is_empty() {
        println!("No path given.\n");
        return None;
    }
    let contents = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            println!("Could not read '{}': {}\n", path, e);
            return None;
        }
    };
    // Use the first non-blank, non-comment line as the SFEN string.  Comment
    // lines start with '#' to match common conventions.
    let sfen = contents
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'));
    let sfen = match sfen {
        Some(s) => s,
        None => {
            println!("File '{}' has no SFEN line.\n", path);
            return None;
        }
    };
    match sfen::parse_sfen(sfen) {
        Ok(board) => Some(board),
        Err(e) => {
            println!("Invalid SFEN in '{}': {}\n", path, e);
            None
        }
    }
}

/// Main interactive loop: analyse → read input → apply → repeat.
fn run_session_loop(session: &mut Session, rx: &Receiver<String>) {
    loop {
        println!("\n{}", BoardRenderer::render(session.board()));
        println!("SFEN: {}", BoardRenderer::render_usi(session.board()));
        let side = session.board().to_move();
        println!("{} to move. (moves played: {})", side.label(), session.move_count());

        // If no legal moves, the position is mate or stalemate — the engine
        // can't analyse further, so just wait for undo/quit.
        let legal = session.legal_moves();
        if legal.is_empty() {
            let in_check = crate::engine::movegen::MoveGenerator::is_in_check(
                session.board_mut(),
                side,
            );
            if in_check {
                println!("Position is checkmate — {} has no legal moves.", side.label());
            } else {
                println!("Position has no legal moves (stalemate).");
            }
            println!("Type 'undo' to take back a move, or 'quit' to exit.");
            if !await_non_search_command(session, rx) {
                return;
            }
            continue;
        }

        println!("Engine is thinking — press Enter with any command to interrupt.\n");
        let task = session.start_search();

        let input = match rx.recv() {
            Ok(line) => line,
            Err(_) => {
                // stdin closed — shut down cleanly.
                session.stop_search(task);
                return;
            }
        };
        let outcome = session.stop_search(task);

        // A short divider so engine output and the user's prompt don't run
        // together after the search stops.
        println!();

        if let Some(outcome) = outcome
            && let Some(best) = outcome.best_move
        {
            println!(
                "Search stopped at depth {} — best {} (eval from {}'s view).",
                outcome.depth,
                format_move(best),
                side.label(),
            );
        }

        match command::parse(&input) {
            Command::Quit => return,
            Command::Empty => {}
            Command::Help => print_help_short(),
            Command::ListMoves => print_legal_moves(session),
            Command::Undo => match session.undo() {
                Ok(mv) => println!("Undid {}.", format_move(mv)),
                Err(msg) => println!("Error: {}", msg),
            },
            Command::Move(spec) => {
                let legal = session.legal_moves();
                match resolve_move(&spec, &legal) {
                    Some(mv) => {
                        session.play_move(mv);
                        println!("Played {}.", format_move(mv));
                    }
                    None => println!("That move is not legal in this position."),
                }
            }
            Command::Unknown(text) => {
                println!("Unrecognised command: '{}'. Type 'help' for a list.", text);
            }
        }
    }
}

/// Used when the position is terminal (no legal moves): the engine has
/// nothing to analyse, so we just block on stdin for an undo/quit/help.
/// Returns `false` when the caller should terminate (quit or stdin EOF).
fn await_non_search_command(session: &mut Session, rx: &Receiver<String>) -> bool {
    loop {
        let input = match read_prompt("> ", rx) {
            Some(line) => line,
            None => return false,
        };
        match command::parse(&input) {
            Command::Quit => return false,
            Command::Empty => continue,
            Command::Help => {
                print_help_short();
                continue;
            }
            Command::ListMoves => {
                print_legal_moves(session);
                continue;
            }
            Command::Undo => match session.undo() {
                Ok(mv) => {
                    println!("Undid {}.", format_move(mv));
                    return true;
                }
                Err(msg) => {
                    println!("Error: {}", msg);
                    continue;
                }
            },
            Command::Move(_) => {
                println!("No legal moves are available here. Use 'undo' instead.");
                continue;
            }
            Command::Unknown(text) => {
                println!("Unrecognised command: '{}'. Type 'help' for a list.", text);
                continue;
            }
        }
    }
}

fn print_legal_moves(session: &mut Session) {
    let legal = session.legal_moves();
    let mut texts: Vec<String> = legal.iter().map(|m| format_move(*m)).collect();
    texts.sort();
    println!("Legal moves ({}):", texts.len());
    for chunk in texts.chunks(8) {
        println!("  {}", chunk.join("  "));
    }
}

fn format_move(mv: Move) -> String {
    match mv.kind {
        MoveKind::Drop => format!("{}*{}", mv.piece.short_name(), mv.to),
        _ => {
            let mut text = format!("{}{}", mv.from.expect("non-drop has from"), mv.to);
            if mv.promote {
                text.push('+');
            }
            text
        }
    }
}

