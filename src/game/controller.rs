use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::engine::{
    board::{Board, PositionSignature},
    movegen::MoveGenerator,
    movelist::MoveList,
    movement::Move,
    search::{AlphaBetaSearcher, SearchConfig, SearchOutcome},
    state::PlayerSide,
};

use super::{
    config::GameConfig,
    timer::{TimeManager, TimeStatus},
};

// `GameController` is the central coordinator between the engine (pure board
// logic) and any front-end (CLI, USI).  It owns:
//   - The `Board` (current position)
//   - Two optional `AlphaBetaSearcher` instances (one per CPU side)
//   - The `TimeManager` (two player clocks)
//   - A position history map for sennichite (fourfold repetition) detection
//   - The `GameStatus` state machine
//
// Front-ends interact with the controller through:
//   bootstrap()           — transition from Ready to AwaitingMove
//   legal_moves()         — list of legal moves for the side to move
//   apply_move(mv)        — advance the position; returns Ok or a MoveError
//   request_move()        — ask the CPU searcher for its best move
//   resign(side)          — immediately end the game by resignation
//   ensure_clock_started()— lazily start the clock when a turn begins

/// The game is in one of three states at any point.
#[derive(Clone, Debug)]
pub enum GameStatus {
    /// Initial state before `bootstrap()` is called.
    Ready,
    /// Waiting for the given side to submit a move.
    AwaitingMove { side: PlayerSide },
    /// The game has ended with the given result.
    Completed(GameResult),
}

/// How the game ended.
#[derive(Clone, Debug)]
pub enum GameResult {
    Resignation { winner: PlayerSide },
    Checkmate { winner: PlayerSide },
    /// Sennichite: the same position (including side to move and hands) was
    /// reached four times.
    Repetition,
    Timeout { winner: PlayerSide },
}

/// Errors that `apply_move` can return.
#[derive(Clone, Debug)]
pub enum MoveError {
    GameAlreadyFinished,
    IllegalMove,
    /// The moving player's clock flagged during this move; the winner is included.
    Timeout { winner: PlayerSide },
}

/// Result of a successful `apply_move` call.
#[derive(Clone, Debug)]
pub enum AdvanceState {
    Ongoing,
    Completed(GameResult),
}

pub struct GameController {
    config: GameConfig,
    board: Board,
    time_manager: TimeManager,
    /// One searcher per side index.  `None` for human players.
    searchers: [Option<AlphaBetaSearcher>; 2],
    status: GameStatus,
    /// Counts how many times each position has been reached (for sennichite).
    history: HashMap<PositionSignature, u32>,
    /// Ordered list of moves played in this game (for post-game display).
    move_log: Vec<Move>,
    /// Tracks which side's clock is currently running, to avoid double-starting.
    clock_running: Option<PlayerSide>,
}

impl GameController {
    pub fn new(config: GameConfig, debug_log: Option<Arc<Mutex<File>>>) -> Self {
        // Create a searcher for each CPU side; human sides get None.
        let mut searchers: [Option<AlphaBetaSearcher>; 2] = [None, None];
        for &side in &PlayerSide::ALL {
            if let Some(kind) = config.player(side).kind.strength() {
                searchers[side.index()] = Some(AlphaBetaSearcher::new(
                    SearchConfig {
                        strength: kind,
                        ..SearchConfig::default()
                    },
                    debug_log.clone(),
                ));
            }
        }
        let board = Board::new_standard();
        let mut history = HashMap::new();
        // Record the starting position once so a fourfold repetition requires
        // returning to it three more times.
        history.insert(board.signature(), 1);
        let time_manager = TimeManager::new(config.time_control);
        Self {
            config,
            board,
            time_manager,
            searchers,
            status: GameStatus::Ready,
            history,
            move_log: Vec::new(),
            clock_running: None,
        }
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    pub fn time_manager(&self) -> &TimeManager {
        &self.time_manager
    }

    pub fn status(&self) -> &GameStatus {
        &self.status
    }

    pub fn move_log(&self) -> &[Move] {
        &self.move_log
    }

    /// Transitions from `Ready` to `AwaitingMove` for the side to move on the
    /// current board.  Also resets `clock_running` so the clock will be
    /// started lazily by `ensure_clock_started`.
    pub fn bootstrap(&mut self) {
        let side = self.board.to_move();
        self.clock_running = None;
        self.status = GameStatus::AwaitingMove { side };
    }

    /// Returns all legal moves for the current side to move.
    pub fn legal_moves(&mut self) -> MoveList {
        let side = self.board.to_move();
        MoveGenerator::legal_moves_for(&mut self.board, side)
    }

    /// Asks the CPU searcher for this side to choose a move and returns
    /// `(SearchOutcome, time_limit_used)`.  Returns None if this side has no
    /// searcher (i.e. it is a human side — caller should not call this then).
    pub fn request_move(&mut self) -> Option<(SearchOutcome, Duration)> {
        let side = self.board.to_move();
        let idx = side.index();
        let limit = self.think_time_for(side);
        match self.searchers[idx].as_mut() {
            Some(searcher) => {
                let outcome = searcher.search(&self.board, limit);
                Some((outcome, limit))
            }
            None => None,
        }
    }

    /// Applies `mv` to the board after validating it is legal and the clock
    /// has not flagged.  Returns `Ok(AdvanceState)` on success or a
    /// `MoveError` variant on failure.
    pub fn apply_move(&mut self, mv: Move) -> Result<AdvanceState, MoveError> {
        let side = self.board.to_move();
        // Guard: don't accept moves when the game is not in an AwaitingMove state.
        match self.status {
            GameStatus::Completed(_) => return Err(MoveError::GameAlreadyFinished),
            GameStatus::Ready => return Err(MoveError::GameAlreadyFinished),
            GameStatus::AwaitingMove { side: expected } if expected != side => {
                return Err(MoveError::GameAlreadyFinished);
            }
            _ => {}
        }

        // Legality check: the move must appear in the legal move list.
        let legal_moves = self.legal_moves();
        if !legal_moves.contains(&mv) {
            return Err(MoveError::IllegalMove);
        }

        // Stop the clock; if the player used more than main+byoyomi, they flag.
        if let TimeStatus::Flagged = self.time_manager.stop_turn(side) {
            let result = GameResult::Timeout {
                winner: side.opponent(),
            };
            self.status = GameStatus::Completed(result.clone());
            return Err(MoveError::Timeout {
                winner: side.opponent(),
            });
        }

        // Apply the move and record it.
        self.board.apply_move(mv);
        self.move_log.push(mv);

        // Check for game-ending conditions in the new position.
        if let Some(result) = self.evaluate_position() {
            self.clock_running = None;
            self.status = GameStatus::Completed(result.clone());
            Ok(AdvanceState::Completed(result))
        } else {
            let next = self.board.to_move();
            self.clock_running = None;
            self.status = GameStatus::AwaitingMove { side: next };
            Ok(AdvanceState::Ongoing)
        }
    }

    /// Immediately ends the game; the opponent of `loser` wins by resignation.
    pub fn resign(&mut self, loser: PlayerSide) {
        let _ = self.time_manager.stop_turn(loser);
        self.status = GameStatus::Completed(GameResult::Resignation {
            winner: loser.opponent(),
        });
        self.clock_running = None;
    }

    /// Checks for game-ending conditions after each move:
    ///   1. Sennichite (fourfold repetition) → draw.
    ///   2. No legal moves for the side to move → checkmate (that side loses).
    fn evaluate_position(&mut self) -> Option<GameResult> {
        // Update the repetition counter for the new position.
        let sig = self.board.signature();
        let count = self.history.entry(sig).or_insert(0);
        *count += 1;
        if *count >= 4 {
            return Some(GameResult::Repetition);
        }

        // If the side to move has no legal moves, they are checkmated.
        let side = self.board.to_move();
        let legal = MoveGenerator::legal_moves_for(&mut self.board, side);
        if legal.is_empty() {
            return Some(GameResult::Checkmate {
                winner: side.opponent(),
            });
        }
        None
    }

    /// Calculates how much time the CPU should use for the current move.
    ///
    /// - If `config.think_time` is non-zero, use it directly — but cap it at
    ///   whatever time is actually on the clock (main time while main > 0,
    ///   byoyomi once main is exhausted) so we never ask the search to think
    ///   longer than the clock would allow.
    /// - Otherwise, use a fraction of remaining time:
    ///     - Main time: ~1/20 of remaining, clamped to [300 ms, 30 s].
    ///     - Byoyomi only: most of the byoyomi period with a 200 ms margin.
    ///
    /// In byoyomi mode the final returned budget is shortened by
    /// `BYOYOMI_SAFETY_MARGIN` so the search returns before the clock flags.
    /// Search cancellation, move application, and stdout flushing all cost a
    /// few milliseconds each and must fit inside the remaining byoyomi; a hard
    /// deadline equal to byoyomi would cause a time-loss even on a search that
    /// appeared to return on time.
    fn think_time_for(&self, side: PlayerSide) -> Duration {
        /// Leave this much headroom at the end of byoyomi for post-search
        /// overhead (stopping the searcher, applying the move, stdout I/O,
        /// scheduler jitter).  200 ms is comfortable on desktop hardware.
        const BYOYOMI_SAFETY_MARGIN: Duration = Duration::from_millis(200);
        /// Absolute floor for a thinking slice in byoyomi so we never schedule
        /// zero time — even a tiny amount lets iterative deepening reach
        /// depth 1 and return a legal move.
        const MIN_THINK_TIME: Duration = Duration::from_millis(50);

        let (main, byoyomi) = self.time_manager.remaining(side);

        if !self.config.think_time.is_zero() {
            let available = if main.is_zero() { byoyomi } else { main };
            let mut slice = self.config.think_time.min(available);
            if main.is_zero() && !byoyomi.is_zero() {
                slice = slice
                    .saturating_sub(BYOYOMI_SAFETY_MARGIN)
                    .max(MIN_THINK_TIME);
            }
            return slice;
        }
        if !main.is_zero() {
            // Use ~1/20 of remaining main time, clamped to [300 ms, 30 s]
            // and never exceeding what's on the clock.
            (main / 20)
                .max(Duration::from_millis(300))
                .min(Duration::from_secs(30))
                .min(main)
        } else {
            // Byoyomi only: use a fraction of the period with a safety margin
            // so the search returns before the clock flags.
            let slice = (byoyomi / 5)
                .max(Duration::from_millis(200))
                .min(Duration::from_secs(5))
                .min(byoyomi);
            slice
                .saturating_sub(BYOYOMI_SAFETY_MARGIN)
                .max(MIN_THINK_TIME)
        }
    }

    // -----------------------------------------------------------------------
    // USI-style integration points
    // -----------------------------------------------------------------------
    // The methods below allow a front-end that manages its own time (e.g. the
    // USI protocol handler, where the GUI dictates per-move time via `go`) to
    // drive the controller without engaging the TimeManager or the internal
    // `GameStatus` state machine.  They are also used by USI to borrow a
    // searcher, run it on a background thread, and return it afterwards so
    // that the searcher's transposition table / killer / history state is
    // preserved across moves.

    /// Temporarily removes the searcher for `side`, leaving `None` in its
    /// place.  The caller is expected to return it via `install_searcher`
    /// once it is done running the search.
    pub fn take_searcher(&mut self, side: PlayerSide) -> Option<AlphaBetaSearcher> {
        self.searchers[side.index()].take()
    }

    /// Propagates a new thread count to every active searcher.  Called by
    /// the USI `setoption name Threads ...` handler.
    pub fn set_threads(&mut self, threads: usize) {
        for slot in self.searchers.iter_mut() {
            if let Some(searcher) = slot.as_mut() {
                searcher.set_threads(threads);
            }
        }
    }

    /// Re-installs a previously taken searcher into its side slot.
    pub fn install_searcher(&mut self, side: PlayerSide, searcher: AlphaBetaSearcher) {
        self.searchers[side.index()] = Some(searcher);
    }

    /// Resets the board to the standard starting position and clears the
    /// move log and repetition history.  Searchers are **not** touched — any
    /// accumulated TT / killer / history entries remain valid because they
    /// are keyed on position signatures, not on the game that produced them.
    pub fn reset_to_startpos(&mut self) {
        self.board = Board::new_standard();
        self.history.clear();
        self.history.insert(self.board.signature(), 1);
        self.move_log.clear();
        self.clock_running = None;
        self.status = GameStatus::AwaitingMove {
            side: self.board.to_move(),
        };
    }

    /// Applies `mv` to the board without touching the TimeManager or
    /// transitioning the status into `Completed`.  Intended for replaying a
    /// move list that was dictated by an external source (e.g. a USI
    /// `position startpos moves ...` command) where time is managed by the
    /// GUI and the game-end detection is performed by the GUI as well.
    ///
    /// The caller is responsible for having verified that `mv` is legal
    /// for the current board; this method does not re-check legality.
    pub fn apply_move_raw(&mut self, mv: Move) {
        self.board.apply_move(mv);
        let sig = self.board.signature();
        *self.history.entry(sig).or_insert(0) += 1;
        self.move_log.push(mv);
        self.status = GameStatus::AwaitingMove {
            side: self.board.to_move(),
        };
    }

    /// Starts the clock for the current side to move, but only if it is not
    /// already running.  Called at the top of each render loop iteration so
    /// the clock begins as soon as the position is displayed to the player.
    pub fn ensure_clock_started(&mut self) {
        if let GameStatus::AwaitingMove { side } = self.status
            && self.clock_running != Some(side) {
                self.time_manager.start_turn(side);
                self.clock_running = Some(side);
            }
    }
}
