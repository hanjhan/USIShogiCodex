use crate::engine::{
    movegen::MoveGenerator,
    movement::{Move, MoveKind},
    search::{AlphaBetaSearcher, SearchOutcome, SearchStrength},
    state::{PieceKind, PlayerSide, Square},
};
use crate::game::{
    GameConfig, GameController,
    config::GameMode,
    player::{PlayerDescriptor, PlayerKind},
    timer::TimeControl,
};
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// USI (Universal Shogi Interface) engine implementation
// ======================================================
// USI is the standard protocol for shogi GUIs to communicate with engines,
// analogous to UCI in chess.  Communication is via stdin/stdout text lines.
//
// Supported commands:
//   usi          → respond with engine name/author + "usiok"
//   isready      → stop any running search, respond "readyok"
//   usinewgame   → reset board to starting position
//   position startpos [moves <move_list>]
//              → rebuild board state from startpos and a move list
//   go [btime X] [wtime X] [byoyomi X] [binc X] [winc X] [movetime X]
//              → start a search on a background thread; respond with
//                "bestmove <move>" when done or stopped
//   stop         → abort the running search and emit "bestmove"
//   quit         → stop search and exit
//
// Move format (USI): "<from><to>[+]" for normal moves (e.g. "7g7f", "2b2a+")
//                    "<piece>*<to>"   for drops (e.g. "P*5e")
// Square format: file digit (1–9) + rank letter ('a'–'i')
//
// Architecture
// ------------
// `UsiEngine` delegates position management to a `GameController`, the same
// coordinator used by the CLI front-end.  This means both front-ends share
// identical core logic for:
//   - board state and move application
//   - legal-move generation (including uchi-fu-zume filtering)
//   - sennichite (fourfold-repetition) history tracking
//   - persistent `AlphaBetaSearcher` instances (one per side)
//
// Crucially, the searcher is **persistent across `go` commands**: when `go`
// arrives, the engine borrows the searcher out of the controller, runs it on
// a background thread, and returns it to the controller afterwards.  This
// preserves the transposition table, killer-move table, and history heuristic
// between moves — a significant strength boost compared to spawning a fresh
// searcher on every `go`.
//
// Time management differs from the CLI: the USI GUI dictates per-move time
// limits via the `go` command (`btime`/`wtime`/`byoyomi`/`movetime`/etc.), so
// `UsiEngine` parses those parameters directly rather than consulting the
// controller's `TimeManager`.  Game-end detection (checkmate / repetition /
// flag-fall) is also left to the GUI; the engine only reports `bestmove`.

pub fn run() {
    let mut engine = UsiEngine::new();
    engine.run();
}

/// Result payload sent from the search thread back to the main loop.
/// Carries the finished `SearchOutcome` **and** returns the borrowed searcher
/// so it can be re-installed on the controller for the next move.
struct SearchTaskResult {
    searcher: AlphaBetaSearcher,
    outcome: SearchOutcome,
}

/// Represents a running background search.
struct SearchTask {
    /// Setting this to true signals the searcher to stop at the next time check.
    abort: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    /// Channel on which the background thread sends the finished search.
    rx: Receiver<SearchTaskResult>,
    /// Which side's searcher is currently out on loan (so we can reinstall).
    side: PlayerSide,
}

struct UsiEngine {
    /// Unified game state shared with the CLI front-end (board, searchers,
    /// repetition history, move log).
    controller: GameController,
    /// Currently-running background search, if any.
    search_task: Option<SearchTask>,
    /// Debug log file, created at startup.  Every USI command and every
    /// bestmove output is appended.
    log_file: Option<BufWriter<File>>,
}

impl UsiEngine {
    fn new() -> Self {
        // Configure both sides as Strong CPUs so the controller allocates a
        // persistent searcher per side.  The GUI will select whichever colour
        // this engine plays on a given move.
        let config = GameConfig::new(
            GameMode::CpuVsCpu,
            PlayerDescriptor::new(
                PlayerSide::Sente,
                PlayerKind::Cpu {
                    strength: SearchStrength::Strong,
                },
            ),
            PlayerDescriptor::new(
                PlayerSide::Gote,
                PlayerKind::Cpu {
                    strength: SearchStrength::Strong,
                },
            ),
            TimeControl::default(),
            Duration::ZERO, // USI manages think time externally (per `go`)
            false,
        );
        let mut controller = GameController::new(config, None);
        controller.bootstrap();
        Self {
            controller,
            search_task: None,
            log_file: Self::create_log_file(),
        }
    }

    /// Creates a timestamped log file named "usi-log-<pid>-<nanos>.log".
    fn create_log_file() -> Option<BufWriter<File>> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let pid = std::process::id();
        let filename = format!("usi-log-{}-{}.log", pid, timestamp);
        File::create(filename).ok().map(BufWriter::new)
    }

    /// Main event loop.  Spawns a stdin reader thread and polls for commands
    /// every 50 ms, interleaved with checking whether the background search
    /// has finished.
    fn run(&mut self) {
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

        loop {
            // Collect a finished search result (if any) without blocking.
            self.poll_search();
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    let trimmed = line.trim();
                    if !self.handle_command(trimmed) {
                        break; // "quit"
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// Dispatches a single USI command line.  Returns false only for "quit".
    fn handle_command(&mut self, line: &str) -> bool {
        let mut parts = line.split_whitespace();
        let cmd = match parts.next() {
            Some(c) => c,
            None => return true,
        };
        match cmd {
            "usi" => {
                println!("id name ShogiCodexEngine");
                println!("id author Codex");
                println!("usiok");
            }
            "isready" => {
                self.stop_search();
                println!("readyok");
            }
            "usinewgame" => {
                self.stop_search();
                self.controller.reset_to_startpos();
            }
            "position" => {
                let rest = line["position".len()..].trim();
                self.log_line(line);
                self.handle_position(rest);
            }
            "go" => {
                let rest = line["go".len()..].trim();
                self.log_line(line);
                let time_limit = self.parse_go_time(rest);
                self.log_line(&format!("computed time_limit={:?}", time_limit));
                self.start_search(time_limit);
            }
            "stop" => {
                self.stop_search();
            }
            "quit" => {
                self.stop_search();
                return false;
            }
            _ => {}
        }
        io::stdout().flush().ok();
        true
    }

    /// Handles "position startpos [moves <move_list>]".
    ///
    /// Resets the controller to the starting position, then parses and
    /// applies each USI move token via `apply_move_raw` (no clock / no
    /// game-end transitions).  Each move is validated against the legal
    /// move list at that intermediate board state before being applied.
    fn handle_position(&mut self, args: &str) {
        self.controller.reset_to_startpos();
        let mut tokens = args.split_whitespace();
        if tokens.next() != Some("startpos") {
            return;
        }
        // The "moves" keyword is optional: a bare "position startpos" is valid.
        match tokens.next() {
            Some("moves") => {}
            Some(_) | None => return,
        }
        for mv_text in tokens {
            let legal = MoveGenerator::legal_moves(self.controller.board_mut());
            match Self::select_usi_move(legal.as_slice(), mv_text) {
                Some(mv) => self.controller.apply_move_raw(mv),
                None => {
                    eprintln!("Failed to parse move: {}", mv_text);
                    self.log_line(&format!("ERROR: failed to parse move {}", mv_text));
                    break;
                }
            }
        }
    }

    /// Parses the arguments of a "go" command and returns the time limit
    /// the engine should use for this move.
    ///
    /// Time budget strategy:
    ///  1. `movetime <ms>` — use exactly that duration.
    ///  2. `btime`/`wtime` with optional `byoyomi`/`binc`/`winc`:
    ///     - Estimate ~40 moves remaining in the game.
    ///     - Budget = remaining_time / moves_left + increment + byoyomi.
    ///     - Clamp to [500ms, remaining_time / 3] to avoid both
    ///       spending too little (shallow search) and too much (flagging).
    ///     - Subtract 200ms safety margin for overhead.
    ///  3. Byoyomi only (main time = 0) — use byoyomi minus 200ms margin.
    ///  4. No time info — return Duration::ZERO (searcher uses its default).
    fn parse_go_time(&self, args: &str) -> Duration {
        const SAFETY_MARGIN: u64 = 200;
        const EXPECTED_MOVES_LEFT: u64 = 40;

        let mut tokens = args.split_whitespace();
        let mut btime: Option<u64> = None;
        let mut wtime: Option<u64> = None;
        let mut byoyomi: Option<u64> = None;
        let mut binc: Option<u64> = None;
        let mut winc: Option<u64> = None;
        let mut movetime: Option<u64> = None;

        while let Some(key) = tokens.next() {
            let val: Option<u64> = tokens.next().and_then(|v| v.parse().ok());
            match key {
                "btime" => btime = val,
                "wtime" => wtime = val,
                "byoyomi" => byoyomi = val,
                "binc" => binc = val,
                "winc" => winc = val,
                "movetime" => movetime = val,
                _ => {}
            }
        }

        if let Some(ms) = movetime {
            return Duration::from_millis(ms);
        }

        let side = self.controller.board().to_move();
        let (remaining_ms, inc_ms) = match side {
            PlayerSide::Sente => (btime, binc),
            PlayerSide::Gote => (wtime, winc),
        };

        let byo_ms = byoyomi.unwrap_or(0);
        let inc_ms = inc_ms.unwrap_or(0);

        if let Some(rem) = remaining_ms
            && rem > 0 {
                // Budget: spread remaining time over expected moves, plus
                // increment and byoyomi as bonus time per move.
                let base = rem / EXPECTED_MOVES_LEFT + inc_ms + byo_ms;
                // Never use more than 1/3 of remaining time on a single move,
                // and never less than 500ms.
                let clamped = base.max(500).min(rem / 3);
                let safe = clamped.saturating_sub(SAFETY_MARGIN).max(100);
                return Duration::from_millis(safe);
            }

        // Byoyomi only (main time exhausted).
        if byo_ms > 0 {
            return Duration::from_millis(byo_ms.saturating_sub(SAFETY_MARGIN).max(100));
        }

        Duration::ZERO
    }

    /// Spawns a background thread to run the search.
    ///
    /// Borrows the searcher for the side to move out of the controller,
    /// clones the board for cross-thread use, and ships both to a worker
    /// thread.  The worker runs `searcher.search(&board, time_limit)` and
    /// sends the finished searcher + outcome back via the channel; the main
    /// loop will pick that result up in `poll_search` or `stop_search` and
    /// re-install the searcher on the controller.
    fn start_search(&mut self, time_limit: Duration) {
        if self.search_task.is_some() {
            return;
        }
        let side = self.controller.board().to_move();
        let mut searcher = match self.controller.take_searcher(side) {
            Some(s) => s,
            None => {
                // No searcher for this side — shouldn't happen given our
                // config, but degrade gracefully.
                println!("bestmove resign");
                io::stdout().flush().ok();
                return;
            }
        };
        let board_clone = self.controller.board().clone();
        let abort = Arc::new(AtomicBool::new(false));
        searcher.set_abort_flag(Some(abort.clone()));
        searcher.set_usi_output(true);
        let (tx, rx) = mpsc::channel::<SearchTaskResult>();
        let handle = thread::spawn(move || {
            let outcome = searcher.search(&board_clone, time_limit);
            let _ = tx.send(SearchTaskResult { searcher, outcome });
        });
        self.search_task = Some(SearchTask {
            abort,
            handle: Some(handle),
            rx,
            side,
        });
    }

    /// Blocking stop: signals the search to abort, waits for the result, and
    /// emits "bestmove".  The borrowed searcher is returned to the controller.
    fn stop_search(&mut self) {
        let Some(mut task) = self.search_task.take() else {
            return;
        };
        task.abort.store(true, Ordering::SeqCst);
        match task.rx.recv() {
            Ok(result) => {
                if let Some(handle) = task.handle.take() {
                    let _ = handle.join();
                }
                self.controller.install_searcher(task.side, result.searcher);
                self.output_bestmove(result.outcome);
            }
            Err(_) => {
                // Thread died without sending — output resign as a fallback.
                self.output_bestmove(SearchOutcome::default());
            }
        }
    }

    /// Non-blocking poll: if the background search has produced a result,
    /// reinstall the searcher and emit "bestmove".  Otherwise returns
    /// quickly and leaves the task in place.
    fn poll_search(&mut self) {
        let Some(mut task) = self.search_task.take() else {
            return;
        };
        match task.rx.try_recv() {
            Ok(result) => {
                if let Some(handle) = task.handle.take() {
                    let _ = handle.join();
                }
                self.controller.install_searcher(task.side, result.searcher);
                self.output_bestmove(result.outcome);
            }
            Err(TryRecvError::Empty) => {
                // Still running — put the task back.
                self.search_task = Some(task);
            }
            Err(TryRecvError::Disconnected) => {
                // Thread died.
                self.output_bestmove(SearchOutcome::default());
            }
        }
    }

    /// Prints "bestmove <move>" (or "bestmove resign" if no move was found)
    /// to stdout and logs it.
    fn output_bestmove(&mut self, outcome: SearchOutcome) {
        let text = if let Some(mv) = outcome.best_move {
            format!("bestmove {}", Self::format_usi_move(mv))
        } else {
            "bestmove resign".to_string()
        };
        self.log_line(&text);
        println!("{}", text);
        io::stdout().flush().ok();
    }

    fn log_line(&mut self, text: &str) {
        if let Some(writer) = self.log_file.as_mut() {
            let _ = writeln!(writer, "{}", text);
            let _ = writer.flush();
        }
    }

    /// Finds the legal move that matches a USI move string.
    /// Drop format: `<PIECE>*<square>` (e.g. "P*5e")
    /// Normal format: `<from><to>[+]` (e.g. "7g7f", "2b3a+")
    fn select_usi_move(legal: &[Move], text: &str) -> Option<Move> {
        if let Some(idx) = text.find('*') {
            let piece_char = text[..idx].chars().next()?;
            let to = Self::parse_square(&text[idx + 1..idx + 3])?;
            let kind = Self::piece_from_char(piece_char)?;
            legal
                .iter()
                .copied()
                .find(|mv| mv.kind == MoveKind::Drop && mv.piece == kind && mv.to == to)
        } else {
            let promote = text.ends_with('+');
            let core = if promote {
                &text[..text.len() - 1]
            } else {
                text
            };
            if core.len() < 4 {
                return None;
            }
            let from = Self::parse_square(&core[0..2])?;
            let to = Self::parse_square(&core[2..4])?;
            let mut candidates: Vec<Move> = legal
                .iter()
                .copied()
                .filter(|mv| mv.from == Some(from) && mv.to == to)
                .collect();
            if promote {
                candidates.retain(|mv| mv.promote);
            } else {
                candidates.retain(|mv| !mv.promote);
            }
            candidates.into_iter().next()
        }
    }

    /// Parses a two-character USI square string (e.g. "7f") into a `Square`.
    fn parse_square(text: &str) -> Option<Square> {
        if text.len() != 2 {
            return None;
        }
        Square::from_text(text)
    }

    /// Formats a `Move` in USI notation.
    /// Drops: `<PIECE-lowercase-for-gote>*<square>` (e.g. "P*5e")
    /// Normal: `<from><to>[+]` (e.g. "7g7f", "2b3a+")
    fn format_usi_move(mv: Move) -> String {
        match mv.kind {
            MoveKind::Drop => {
                let piece = mv.piece.short_name().chars().next().unwrap_or('P')
                    .to_ascii_lowercase();
                format!("{}*{}", piece, Self::square_to_usi(mv.to))
            }
            _ => {
                let mut text = format!(
                    "{}{}",
                    Self::square_to_usi(mv.from.expect("from square")),
                    Self::square_to_usi(mv.to)
                );
                if mv.promote {
                    text.push('+');
                }
                text
            }
        }
    }

    /// Converts an internal `Square` to USI square notation: file digit (1–9)
    /// followed by rank letter ('a'–'i').  File 0 (internal) → '9' (USI).
    fn square_to_usi(square: Square) -> String {
        let file = 9 - square.file();
        let rank = (b'a' + square.rank()) as char;
        format!("{}{}", file, rank)
    }

    fn piece_from_char(ch: char) -> Option<PieceKind> {
        PieceKind::from_char(ch)
    }
}
