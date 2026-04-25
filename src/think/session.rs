// Thinking-mode session.
//
// Owns the current analysis position, the sequence of moves played from the
// starting position, and a persistent `AlphaBetaSearcher` whose transposition
// table / killer / history state is preserved across restarts.
//
// Forward progress (applying a move) is tracked as a flat `Vec<Move>`; undo
// pops the last move and replays the list from `starting_board`.  Replaying
// is O(n) in move count, which is trivial for any realistic analysis depth.
// This structure was chosen — rather than caching `UndoInfo` per move — so
// the same representation naturally extends to the planned tree-based
// navigation: a tree node just holds `Vec<Move>` to that node.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};
use std::thread;
use std::time::Duration;

use crate::engine::{
    board::Board,
    movegen::MoveGenerator,
    movelist::MoveList,
    movement::Move,
    search::{AlphaBetaSearcher, InfoOutputMode, SearchConfig, SearchOutcome, SearchStrength},
};

/// Container for all session state.
pub struct Session {
    starting_board: Board,
    current_board: Board,
    moves: Vec<Move>,
    searcher: Option<AlphaBetaSearcher>,
}

/// Handle to a background analysis thread.  Set `abort` to true to request
/// termination; the search returns via `rx` with the finished searcher.
pub struct RunningSearch {
    abort: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    rx: Receiver<SearchResult>,
}

struct SearchResult {
    searcher: AlphaBetaSearcher,
    outcome: SearchOutcome,
}

impl Session {
    pub fn new(board: Board) -> Self {
        // Strong strength is chosen so the confidence-based stopping rule
        // rarely fires — in analysis we want the engine to keep thinking
        // until the user stops it.  The 600 s MAX_SEARCH_TIME safety cap
        // inside the searcher still applies.
        //
        // `threads` is inherited from `SearchConfig::default()`, which maps
        // to every available logical core — thinking mode benefits most
        // from Lazy SMP because there is no time pressure.
        let config = SearchConfig {
            strength: SearchStrength::Strong,
            time_per_move: Duration::ZERO, // ZERO → search runs without a deadline
            info_output: InfoOutputMode::Think,
            ..SearchConfig::default()
        };
        let searcher = AlphaBetaSearcher::new(config);
        Self {
            starting_board: board.clone(),
            current_board: board,
            moves: Vec::new(),
            searcher: Some(searcher),
        }
    }

    pub fn board(&self) -> &Board {
        &self.current_board
    }

    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.current_board
    }

    pub fn move_count(&self) -> usize {
        self.moves.len()
    }

    pub fn legal_moves(&mut self) -> MoveList {
        MoveGenerator::legal_moves(&mut self.current_board)
    }

    /// Applies `mv` to the current board and records it in the move stack.
    /// The caller is responsible for ensuring legality.
    pub fn play_move(&mut self, mv: Move) {
        self.current_board.apply_move(mv);
        self.moves.push(mv);
    }

    /// Removes the most recent move from the stack and replays the remainder
    /// from the starting position.  Returns an error if there is nothing to
    /// undo.
    pub fn undo(&mut self) -> Result<Move, &'static str> {
        let mv = self
            .moves
            .pop()
            .ok_or("Cannot undo: already at the starting position.")?;
        self.replay();
        Ok(mv)
    }

    fn replay(&mut self) {
        self.current_board = self.starting_board.clone();
        for mv in &self.moves {
            self.current_board.apply_move(*mv);
        }
    }

    /// Starts a background analysis of the current position.  The search
    /// runs until `RunningSearch::abort` is set or the engine finishes on
    /// its own (mate found, confidence reached, or the searcher's internal
    /// safety cap triggers).
    pub fn start_search(&mut self) -> RunningSearch {
        let mut searcher = self
            .searcher
            .take()
            .expect("searcher must be installed before start_search");
        let abort = Arc::new(AtomicBool::new(false));
        searcher.set_abort_flag(Some(abort.clone()));
        searcher.set_info_output(InfoOutputMode::Think);
        let board_clone = self.current_board.clone();
        let (tx, rx) = mpsc::channel::<SearchResult>();
        let handle = thread::spawn(move || {
            let outcome = searcher.search(&board_clone, Duration::ZERO);
            let _ = tx.send(SearchResult { searcher, outcome });
        });
        RunningSearch {
            abort,
            handle: Some(handle),
            rx,
        }
    }

    /// Signals the running search to stop, waits for it to finish, and
    /// re-installs the searcher so it's ready for the next start_search.
    pub fn stop_search(&mut self, mut task: RunningSearch) -> Option<SearchOutcome> {
        task.abort.store(true, Ordering::SeqCst);
        let result = task.rx.recv().ok();
        if let Some(handle) = task.handle.take() {
            let _ = handle.join();
        }
        result.map(|r| {
            self.searcher = Some(r.searcher);
            r.outcome
        })
    }
}

