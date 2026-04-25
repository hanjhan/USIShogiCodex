// =============================================================================
// Alpha-Beta Search Engine
// =============================================================================
//
// This file implements the core search algorithm for the shogi engine.
// The engine uses **iterative deepening** with **negamax alpha-beta** search,
// enhanced with several standard techniques to prune the search tree and
// improve move ordering.
//
// ## Search Architecture Overview
//
// The search proceeds in layers:
//
//   search()                    ← entry point: iterative deepening loop
//     └─ search_root_window()   ← root node with aspiration window
//          └─ alpha_beta()      ← recursive negamax with pruning
//               └─ quiescence() ← extends tactical lines beyond depth limit
//
// ## Key Techniques Used
//
// **Tree pruning** (skip parts of the tree that can't affect the result):
//   - Alpha-beta pruning:   the fundamental technique — skip moves that the
//                            opponent would never allow (score ≥ beta).
//   - Null-move pruning:    "what if I pass?" — if the opponent still can't
//                            beat beta, our position is so strong we can skip.
//   - Futility pruning:     at depth 1-2, skip quiet moves that can't
//                            possibly raise alpha even with a large margin.
//   - Delta pruning:        in quiescence, skip captures whose piece value
//                            can't bridge the gap to alpha.
//
// **Search reductions** (search less promising moves to shallower depth):
//   - Late Move Reductions: after the first few well-ordered moves, search
//                            the rest at reduced depth — re-search if they
//                            surprisingly beat alpha.
//
// **Move ordering** (search the best moves first to maximise pruning):
//   - TT move:         the best move from a previous search of this position.
//   - MVV-LVA:         for captures, prefer taking valuable pieces with cheap ones.
//   - Killer moves:    quiet moves that caused beta cutoffs at this ply.
//   - History table:   bonus for (from, to) pairs that frequently cause cutoffs.
//
// **Position caching**:
//   - Transposition table (TT): memoises positions already searched at
//                                sufficient depth. Uses Zobrist hash keys.
//
// **Search extensions**:
//   - Check extension:  when in check, search 1 ply deeper to avoid missing
//                        forced mating sequences near the horizon.
//
// **Time management**:
//   - Iterative deepening: search depth 1, then 2, 3, … until time runs out.
//     The last *completed* depth gives the best move.  Also seeds the TT and
//     move ordering for deeper iterations.
//   - Aspiration windows: at depth ≥ 4, use a narrow window around the
//     previous score.  This prunes aggressively when the score is stable,
//     and widens automatically on fail-high/fail-low.
//
// ## Evaluation Architecture
//
// Evaluation is **incremental**: material values, piece-square table bonuses,
// and pawn advancement are maintained inside the Board struct and updated on
// every make/undo.  The evaluator only adds king safety (defender proximity +
// exposed-king penalty) on top — a fast O(small) computation.
//
// See `eval_tables.rs` for the pre-computed scoring tables and `evaluator.rs`
// for the king-safety logic.
//
// ## Performance
//
// Key optimisations for nodes-per-second:
//   - **MoveList**: stack-allocated move list (no heap alloc per node).
//   - **Make/unmake**: board.make_move() + board.undo_move() instead of clone.
//   - **Incremental eval**: O(1) material score via Board.eval_score().
//   - **Loud-only movegen**: quiescence uses a dedicated capture+promotion
//     generator, skipping all quiet moves and drops.
//   - **Fast is_in_check**: reverse attack lookups instead of full movegen.
//   - **Amortised time checks**: Instant::now() called every 1024 nodes, not
//     every node.
//
// =============================================================================

use std::time::{Duration, Instant};

use super::{
    evaluator::MaterialEvaluator,
    strength::{MAX_DEPTH, SearchStrength},
    tt::{ConcurrentTT, TTFlag, TtEntry},
};
use crate::engine::{
    board::Board,
    movegen::MoveGenerator,
    movelist::MoveList,
    movement::{Move, MoveKind},
    state::{PieceKind, PlayerSide, Square},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// Mate score: large enough to dominate any material score, but small enough
// that (MATE_SCORE - depth) still fits in i32 and can be compared.
const MATE_SCORE: i32 = 30_000;

const MAX_SEARCH_TIME: Duration = Duration::from_secs(600);

// Scores within this range of MATE_SCORE are treated as mate scores and
// excluded from TT reuse.  Without this, a "mate in 5" stored at ply 3
// could be replayed at ply 10, producing a wrong distance-to-mate.
const MATE_THRESHOLD: i32 = MATE_SCORE - 500;

// ~8M positions (~256MB).  Larger tables prevent eviction at deep searches
// where the previous 1M was filling to 100% at depth 14.
const TT_MAX_SIZE: usize = 1 << 23;

// How often to poll the wall clock.  1024 nodes ≈ 0.6 ms at 1.6M nps.
const TIME_CHECK_INTERVAL: u32 = 1024;

// History table dimensions: 81 board squares + 8 drop piece kinds = 89 "from" slots,
// 81 "to" squares.  history[from][to] stores a score for how often this
// (from, to) pair caused a beta cutoff — used for quiet-move ordering.
const HISTORY_FROM_SQUARES: usize = 81;
const HISTORY_FROM_SIZE: usize = HISTORY_FROM_SQUARES + PieceKind::ALL.len(); // 89
const HISTORY_TO_SIZE: usize = 81;

// --- Pruning / reduction tuning constants ---

// Null-move pruning: adaptive reduction R = 3 + depth/6.
// Deeper nodes get bigger reductions (R=4 at depth 12, R=5 at depth 18).
const NMP_MIN_DEPTH: u8 = 3;

// Late Move Reductions: logarithmic formula with aggressive divisor.
//   reduction = ln(depth) * ln(moveIndex) / LMR_DIVISOR
// Divisor 1.4 gives ~2 ply for move 4 at depth 8, ~5 ply for move 20 at depth 16.
const LMR_MIN_DEPTH: u8 = 2;
const LMR_MOVE_THRESHOLD: usize = 2;
const LMR_DIVISOR: f64 = 1.4;

// Late Move Pruning (LMP): at shallow depths, after searching this many
// moves, skip ALL remaining quiet moves entirely.  Very aggressive but
// safe because well-ordered moves (TT, captures, killers) are searched first.
// Index by depth: depth 1 → 5 moves, depth 2 → 8, ..., depth 6 → 24.
const LMP_MOVE_LIMITS: [usize; 7] = [0, 5, 8, 12, 16, 20, 24];

// Futility pruning margins indexed by depth (extended to depth 6).
const FUTILITY_MARGIN: [i32; 7] = [0, 150, 300, 500, 750, 1050, 1400];

// Reverse futility pruning margins (extended to depth 7).
const REVERSE_FUTILITY_MARGIN: [i32; 8] = [0, 150, 300, 500, 750, 1050, 1400, 1800];

// Delta pruning in quiescence: skip a capture if
//   stand_pat + captured_piece_value + DELTA_MARGIN < alpha.
const DELTA_MARGIN: i32 = 200;
const DELTA_PIECE_VALUES: [i32; PieceKind::ALL.len()] = [
    0, 1040, 910, 620, 550, 410, 430, 100,
];

// Aspiration window: initial half-width around the previous iteration's
// score.  Doubles on each fail-high/fail-low until the true score is found.
const ASPIRATION_DELTA: i32 = 50;

// Piece values used for move ordering (MVV-LVA).  These don't need to match
// the evaluation values exactly — they just need to rank captures correctly.
const MOVE_ORDER_VALUES: [i32; PieceKind::ALL.len()] = [
    0,   // King    — never captured (game ends first)
    900, // Rook
    850, // Bishop
    600, // Gold
    500, // Silver
    350, // Knight
    300, // Lance
    100, // Pawn
];

// ---------------------------------------------------------------------------
// Transposition Table (TT)
// ---------------------------------------------------------------------------
// The TT stores results of previously searched positions keyed by Zobrist hash.
// When the same position is reached again (via a different move order), we can
// reuse the result instead of re-searching.
//
// Storage: `ConcurrentTT` (see `tt.rs`) — a lock-free fixed-size array of
// `AtomicU64` pairs using Hyatt's XOR verification trick.  The TT is held
// behind an `Arc` so that Lazy-SMP worker threads can share it.
//
// Each logical entry stores:
//   - depth:     how deeply this position was searched.
//   - score:     the minimax score found.
//   - flag:      how to interpret the score (see TTFlag).
//   - best_move: the best move found (used for move ordering even when the
//                score is not directly usable).
//
// TTFlag meanings:
//   Exact:      the score is the true minimax value (alpha < score < beta).
//   LowerBound: the search cut off (score >= beta), so the true value is
//               *at least* this high.  Useful for tightening alpha.
//   UpperBound: no move beat alpha, so the true value is *at most* this.
//               Useful for tightening beta.

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// Controls how per-iteration progress information is reported during search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoOutputMode {
    /// Do not print anything per iteration.
    None,
    /// USI-protocol `info` lines for GUI consumption.
    Usi,
    /// Human-readable lines tailored for the `think` binary.
    Think,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    pub strength: SearchStrength,
    /// Default think time when no explicit time limit is provided.
    pub time_per_move: Duration,
    /// Format used for per-iteration progress reports (see `InfoOutputMode`).
    pub info_output: InfoOutputMode,
    /// Number of worker threads to run in parallel (Lazy SMP).  Values of
    /// 0 or 1 run the classical single-threaded search.  Each worker keeps
    /// its own killer/history/PV tables but shares the transposition table
    /// and a stop signal with the others.
    pub threads: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        // Default to all available logical cores.  Individual binaries can
        // still override via `set_threads` or by constructing a different
        // `SearchConfig` explicitly (USI `setoption name Threads ...`).
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self {
            strength: SearchStrength::Normal,
            time_per_move: Duration::from_secs(1),
            info_output: InfoOutputMode::None,
            threads,
        }
    }
}

/// Maximum PV length we track through the search tree.
const MAX_PV_PLY: usize = 64;

/// Why the search stopped iterating.
#[derive(Clone, Copy, Debug, Default)]
pub enum StopReason {
    #[default]
    TimeUp,
    Confident,
}

/// The result of a search: the best move found, its score, how many nodes
/// were visited, and the deepest fully completed iteration.
#[derive(Default, Debug)]
pub struct SearchOutcome {
    pub best_move: Option<Move>,
    /// Score in centipawn-like units from the searching side's perspective.
    /// Positive = good, negative = bad, near ±MATE_SCORE = forced mate.
    pub score: i32,
    pub nodes: u64,
    /// Deepest iteration that completed before time ran out.
    pub depth: u8,
    pub stop_reason: StopReason,
}

// ---------------------------------------------------------------------------
// Searcher
// ---------------------------------------------------------------------------

pub struct AlphaBetaSearcher {
    evaluator: MaterialEvaluator,
    config: SearchConfig,
    /// Total nodes visited in the current search (reset per search() call).
    nodes: u64,
    /// Wall-clock deadline; None = no time limit.
    deadline: Option<Instant>,
    /// Sticky flag: once true, every node returns immediately.
    time_up: bool,
    /// External stop signal (e.g. USI "stop" command).
    abort_flag: Option<Arc<AtomicBool>>,
    /// Transposition table — persists across moves so accumulated knowledge
    /// carries over.  Shared behind an `Arc` so Lazy-SMP worker threads can
    /// probe/store concurrently.
    tt: Arc<ConcurrentTT>,
    /// Killer moves: the 2 most recent quiet moves that caused a beta cutoff
    /// at each ply distance from root.  Cheap to probe, excellent for ordering
    /// non-capture moves that are tactically important.
    killers: Vec<[Option<Move>; 2]>,
    /// History heuristic table: history[from_idx][to_idx] accumulates a bonus
    /// every time a (from, to) pair causes a beta cutoff.  Higher scores mean
    /// "this move is usually good" — used to order quiet moves after killers.
    history: Box<[[i32; HISTORY_TO_SIZE]; HISTORY_FROM_SIZE]>,
    /// Countdown to next Instant::now() call; avoids a syscall on every node.
    check_counter: u32,
    /// Triangular PV table: pv_table[ply] holds the best continuation from
    /// that ply.  Updated when a move raises alpha.
    pv_table: Box<[[Option<Move>; MAX_PV_PLY]; MAX_PV_PLY]>,
    pv_length: [usize; MAX_PV_PLY],
    /// Timestamp when the current search started (for `info time` output).
    search_start: Option<Instant>,
}

impl AlphaBetaSearcher {
    pub fn new(config: SearchConfig) -> Self {
        Self::with_shared_tt(config, Arc::new(ConcurrentTT::new(TT_MAX_SIZE)))
    }

    /// Creates a searcher that shares its transposition table with other
    /// searchers (the Lazy-SMP helper-thread path).  The caller is
    /// responsible for ensuring the same `Arc` is handed to every peer.
    pub fn with_shared_tt(config: SearchConfig, tt: Arc<ConcurrentTT>) -> Self {
        Self {
            evaluator: MaterialEvaluator::default(),
            config,
            nodes: 0,
            deadline: None,
            time_up: false,
            abort_flag: None,
            tt,
            killers: Vec::new(),
            history: Box::new([[0; HISTORY_TO_SIZE]; HISTORY_FROM_SIZE]),
            check_counter: TIME_CHECK_INTERVAL,
            pv_table: Box::new([[None; MAX_PV_PLY]; MAX_PV_PLY]),
            pv_length: [0; MAX_PV_PLY],
            search_start: None,
        }
    }

    /// Returns a cloneable handle to the transposition table.  Used to fan
    /// out the same TT across Lazy-SMP helper threads.
    pub fn shared_tt(&self) -> Arc<ConcurrentTT> {
        Arc::clone(&self.tt)
    }

    pub fn set_abort_flag(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.abort_flag = flag;
    }

    pub fn set_usi_output(&mut self, enabled: bool) {
        self.config.info_output = if enabled {
            InfoOutputMode::Usi
        } else {
            InfoOutputMode::None
        };
    }

    pub fn set_info_output(&mut self, mode: InfoOutputMode) {
        self.config.info_output = mode;
    }

    pub fn set_threads(&mut self, threads: usize) {
        self.config.threads = threads;
    }

    /// Main entry point.  Dispatches to the single-threaded search path
    /// or to Lazy-SMP parallel search based on `config.threads`.
    pub fn search(&mut self, board: &Board, time_limit: Duration) -> SearchOutcome {
        let threads = self.config.threads.max(1);
        if threads == 1 {
            self.search_once(board, time_limit)
        } else {
            self.search_parallel(board, time_limit, threads)
        }
    }

    /// Single-threaded iterative-deepening search.  Exposed separately so
    /// Lazy-SMP worker threads can call into the same kernel.
    ///
    /// Runs iterative deepening from depth 1 up to the strength limit.  Each
    /// completed iteration improves move ordering for the next (via TT and
    /// history).  The best move from the last *completed* iteration is
    /// returned — incomplete iterations are discarded.
    ///
    /// A fallback move is seeded before the loop so that even if depth 1
    /// times out mid-search, we still return a legal move instead of None.
    pub fn search_once(&mut self, board: &Board, time_limit: Duration) -> SearchOutcome {
        self.nodes = 0;
        self.time_up = false;
        self.check_counter = TIME_CHECK_INTERVAL;
        self.killers.clear();
        *self.history = [[0; HISTORY_TO_SIZE]; HISTORY_FROM_SIZE];
        self.pv_length = [0; MAX_PV_PLY];
        self.search_start = Some(Instant::now());

        let slice = if time_limit.is_zero() {
            self.config.time_per_move
        } else {
            time_limit
        }
        .min(MAX_SEARCH_TIME);
        self.deadline = if slice.is_zero() {
            None
        } else {
            Some(Instant::now() + slice)
        };

        // Clone once at the top; the recursive search uses make/unmake on
        // this single board instance (no further clones).
        let mut board = board.clone();
        let confidence = self.config.strength.confidence();

        // Fallback: grab the first legal move so we never return None.
        let side_to_move = board.to_move();
        let fallback_moves = MoveGenerator::legal_moves_for(&mut board, side_to_move);
        let mut best_move = fallback_moves.first();
        let mut best_score = self.evaluator.evaluate(&board, board.to_move());
        let mut best_depth = 0;

        // Confidence tracking: how many consecutive iterations returned the
        // same best move and a stable score.
        let mut prev_best_move: Option<Move> = None;
        let mut move_stable_count: u8 = 0;
        let mut stop_reason = StopReason::TimeUp;

        // Only the info-producing thread logs the start-of-search line, so
        // that Lazy-SMP worker threads don't flood stderr.
        if self.config.info_output != InfoOutputMode::None {
            eprintln!("START search limit={:?} side={:?}", slice, board.to_move());
        }

        // --- Iterative deepening loop ---
        // Each iteration searches one ply deeper.  Shallow iterations are
        // very fast and populate the TT + history, making deeper iterations
        // dramatically more efficient (better move ordering → more cutoffs).
        //
        // Instead of a fixed depth cap, we search up to MAX_DEPTH and stop
        // early when the engine is "confident" — the best move and score
        // have been stable for several iterations (controlled by strength).
        for depth in 1..=MAX_DEPTH {
            // At depth >= 4, use aspiration windows: search with a narrow
            // window [score-δ, score+δ] around the previous iteration's
            // score.  If the result falls outside, widen δ and re-search.
            // This prunes far more than a full [-∞, +∞] window when the
            // score is stable between iterations.
            let (cand_move, cand_score) = if depth >= 4 && best_depth > 0 {
                let mut delta = ASPIRATION_DELTA;
                let mut asp_alpha = best_score - delta;
                let mut asp_beta = best_score + delta;
                loop {
                    let (m, s) = self.search_root_window(&mut board, depth, asp_alpha, asp_beta);
                    if self.time_up {
                        break (m, s);
                    }
                    // Mate score found — no point widening the window further.
                    if s.abs() >= MATE_THRESHOLD {
                        break (m, s);
                    }
                    if s <= asp_alpha {
                        asp_alpha = (asp_alpha - delta).max(-MATE_SCORE);
                        delta *= 2;
                    } else if s >= asp_beta {
                        asp_beta = (asp_beta + delta).min(MATE_SCORE);
                        delta *= 2;
                    } else {
                        break (m, s);
                    }
                }
            } else {
                // Depths 1-3: use a full window (no prior score to anchor on).
                self.search_root(&mut board, depth)
            };

            // If time ran out mid-iteration, discard partial results UNLESS
            // we have no completed iteration yet (best_depth == 0) — in that
            // case, take whatever we got so we don't resign with no move.
            if self.time_up {
                if best_depth == 0 && cand_move.is_some() {
                    best_move = cand_move;
                }
                break;
            }

            let prev_score = best_score;
            if cand_move.is_some() {
                best_move = cand_move;
                best_score = cand_score;
                best_depth = depth;
            }

            // --- Per-iteration info output ---
            match self.config.info_output {
                InfoOutputMode::None => {}
                InfoOutputMode::Usi => self.output_info_usi(depth, best_score),
                InfoOutputMode::Think => self.output_info_think(depth, best_score),
            }

            // --- Forced mate: stop immediately ---
            // A mate score means we found a forced checkmate sequence.
            // No deeper search can improve on this — stop now.
            if best_score.abs() >= MATE_THRESHOLD {
                stop_reason = StopReason::Confident;
                break;
            }

            // --- Confidence-based early termination ---
            // Track whether the best move is stable across iterations.
            // If the same move has been best for enough consecutive iterations
            // AND the score isn't swinging wildly, the engine is confident
            // enough to stop (according to its strength level).
            if cand_move == prev_best_move && cand_move.is_some() {
                move_stable_count += 1;
            } else {
                move_stable_count = 1;
            }
            prev_best_move = cand_move;

            let score_stable = (cand_score - prev_score).unsigned_abs() <= confidence.score_threshold as u32;

            if depth >= confidence.min_depth
                && move_stable_count >= confidence.stable_iterations
                && score_stable
            {
                stop_reason = StopReason::Confident;
                break;
            }
        }
        self.deadline = None;

        SearchOutcome {
            best_move,
            score: best_score,
            nodes: self.nodes,
            depth: best_depth,
            stop_reason,
        }
    }

    // -----------------------------------------------------------------------
    // Lazy SMP
    // -----------------------------------------------------------------------
    // The `Lazy SMP` pattern: spawn N worker threads that all run the same
    // iterative-deepening search on the same root position, each with its
    // own killer/history/PV tables but sharing a single transposition table.
    // Threads diverge because of timing jitter (one's TT probe sees the
    // other's earlier store), which makes them explore different parts of
    // the tree.  Each thread's cutoffs end up helping the others via the
    // shared TT — this gives most of the scaling benefit with minimal
    // coordination logic.
    //
    // Stop semantics:
    //   * The main thread honours `self.abort_flag` (set, for example, by
    //     a USI `stop` command or by the CLI/think-mode front-end when the
    //     user types a command).
    //   * Workers poll a dedicated `worker_abort` flag that the main
    //     thread sets to true *after* its own search returns.  This way,
    //     if a worker happens to finish later (it reached a deeper depth
    //     than main, say), its result is still available to be chosen;
    //     but once main returns, workers are promptly stopped to free
    //     CPU time.
    //
    // Result aggregation:
    //   * Across all outcomes with a usable best move, pick the one with
    //     the greatest `depth`; break ties by the reported `score`.
    //   * `nodes` in the aggregated outcome is the sum across all threads
    //     so callers see the true total work.
    //   * `stop_reason` is taken from the main thread (workers' stop
    //     reasons are less informative because they were externally
    //     aborted).

    fn search_parallel(
        &mut self,
        board: &Board,
        time_limit: Duration,
        threads: usize,
    ) -> SearchOutcome {
        // Dedicated abort flag used by the main thread to stop workers once
        // it has finished its own search.  Does **not** touch the
        // externally-provided `self.abort_flag` — that flag belongs to the
        // caller and we must not mutate their state.
        let worker_abort = Arc::new(AtomicBool::new(false));

        // Worker threads always get an empty move log (they shouldn't emit
        // progress info lines) and no debug-log handle (keeps file I/O
        // serial through the main thread).
        let mut worker_config = self.config;
        worker_config.info_output = InfoOutputMode::None;
        // Nested parallelism inside a worker would explode thread count;
        // keep worker searches strictly single-threaded.
        worker_config.threads = 1;

        let mut handles: Vec<std::thread::JoinHandle<SearchOutcome>> =
            Vec::with_capacity(threads.saturating_sub(1));
        for _ in 0..threads.saturating_sub(1) {
            let tt = Arc::clone(&self.tt);
            let abort = Arc::clone(&worker_abort);
            let board_snapshot = board.clone();
            let cfg = worker_config;
            let handle = std::thread::spawn(move || {
                let mut worker = AlphaBetaSearcher::with_shared_tt(cfg, tt);
                worker.set_abort_flag(Some(abort));
                worker.search_once(&board_snapshot, time_limit)
            });
            handles.push(handle);
        }

        // Run the main thread's search locally (reuses this searcher's
        // killer/history/PV state across invocations, same as before).
        let main_outcome = self.search_once(board, time_limit);

        // Tell workers to stop.  Each worker polls `worker_abort` in its
        // time-check routine and will return within a few-thousand nodes.
        worker_abort.store(true, Ordering::SeqCst);

        let mut outcomes: Vec<SearchOutcome> = Vec::with_capacity(handles.len() + 1);
        let main_stop_reason = main_outcome.stop_reason;
        outcomes.push(main_outcome);
        for h in handles {
            if let Ok(outcome) = h.join() {
                outcomes.push(outcome);
            }
        }

        aggregate_outcomes(outcomes, main_stop_reason)
    }

    // -----------------------------------------------------------------------
    // Root search
    // -----------------------------------------------------------------------
    // The root is special: we need to track which *move* is best (not just the
    // score), and we use PVS (Principal Variation Search) — search the first
    // move with a full window, then probe subsequent moves with a zero window
    // (-alpha-1, -alpha).  Only if a probe beats alpha do we re-search with
    // the full window.  This saves ~30-50% of nodes when the first move
    // (usually the TT move) is indeed best.

    fn search_root(&mut self, board: &mut Board, depth: u8) -> (Option<Move>, i32) {
        self.search_root_window(board, depth, i32::MIN / 2, i32::MAX / 2)
    }

    fn search_root_window(
        &mut self,
        board: &mut Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
    ) -> (Option<Move>, i32) {
        let side = board.to_move();
        let tt_move = self.tt.probe(board.zobrist()).and_then(|e| e.best_move);
        let moves = self.ordered_moves(MoveGenerator::legal_moves_for(board, side), tt_move, 0);
        if moves.is_empty() {
            let score = if MoveGenerator::is_in_check(board, side) {
                -Self::mate_score(depth)
            } else {
                0
            };
            return (None, score);
        }

        self.pv_length[0] = 0;
        let mut best_move = moves.first();
        for (i, mv) in moves.iter().enumerate() {
            if self.time_up {
                break;
            }
            let mv = *mv;
            let undo = board.make_move(mv);
            let score = if i == 0 {
                -self.alpha_beta(board, depth - 1, -beta, -alpha, side.opponent(), 1, true)
            } else {
                let s = -self.alpha_beta(
                    board,
                    depth - 1,
                    -alpha - 1,
                    -alpha,
                    side.opponent(),
                    1,
                    true,
                );
                if s > alpha && s < beta && !self.time_up {
                    -self.alpha_beta(board, depth - 1, -beta, -alpha, side.opponent(), 1, true)
                } else {
                    s
                }
            };
            board.undo_move(mv, undo);
            if score > alpha {
                alpha = score;
                best_move = Some(mv);
                self.update_pv(0, mv);
            }
        }

        (best_move, alpha)
    }

    // -----------------------------------------------------------------------
    // Alpha-beta (negamax, fail-soft) with TT, killers, and history
    // -----------------------------------------------------------------------

    /// Recursive negamax alpha-beta search (fail-soft variant).
    ///
    /// Returns the score of `board` from `side`'s point of view.
    /// Positive = good for `side`, negative = bad.
    ///
    /// Parameters:
    ///   `depth`  — remaining plies to search.  Counts down to 0, where
    ///              quiescence search takes over.
    ///   `alpha`  — lower bound: we already know a line scoring at least this.
    ///              Any child scoring ≤ alpha is ignored (it cannot improve our
    ///              result).
    ///   `beta`   — upper bound from the opponent's perspective: if we find a
    ///              move scoring ≥ beta the opponent would never allow this
    ///              position, so we can stop searching (beta cutoff).
    ///   `ply`    — distance from the root, used to index the killer-move table.
    ///
    /// Fail-soft means the return value can lie outside [alpha, beta], which
    /// gives the caller more information than fail-hard (clamped to the window).
    #[allow(clippy::too_many_arguments)]
    fn alpha_beta(
        &mut self,
        board: &mut Board,
        depth: u8,
        mut alpha: i32,
        mut beta: i32,
        side: PlayerSide,
        ply: usize,
        can_null: bool,
    ) -> i32 {
        if self.time_up || self.timed_out() {
            self.time_up = true;
            return 0;
        }
        self.nodes += 1;

        // --- Check extension ---
        // Extend by 1 ply when in check: the reply set is small and often
        // tactical.  Without this, mating attacks are frequently missed
        // just beyond the search horizon.
        let in_check = MoveGenerator::is_in_check(board, side);
        let depth = if in_check { depth.saturating_add(1) } else { depth };

        if depth == 0 {
            if ply < MAX_PV_PLY {
                self.pv_length[ply] = 0;
            }
            return self.quiescence(board, alpha, beta, side);
        }

        if ply < MAX_PV_PLY {
            self.pv_length[ply] = 0;
        }

        let original_alpha = alpha;
        let sig = board.zobrist();

        // --- Transposition table probe ---
        // If we've seen this exact position before at equal or greater depth,
        // we can reuse or tighten the bounds.  Always extract the best move
        // for move ordering even when the score isn't directly usable.
        let mut tt_move: Option<Move> = None;
        if let Some(entry) = self.tt.probe(sig) {
            tt_move = entry.best_move;
            if entry.depth >= depth && entry.score.abs() < MATE_THRESHOLD {
                match entry.flag {
                    TTFlag::Exact => return entry.score,
                    TTFlag::LowerBound => alpha = alpha.max(entry.score),
                    TTFlag::UpperBound => beta = beta.min(entry.score),
                }
                if alpha >= beta {
                    return entry.score;
                }
            }
        }

        // --- Null-move pruning (NMP) ---
        // Idea: "if I skip my turn and my opponent still can't beat beta,
        // my position is so strong that this node will almost certainly
        // produce a beta cutoff."
        //
        // Guards against unsound results:
        //   can_null:    never do two null moves in a row.
        //   !in_check:   passing while in check is illegal.
        //   non-king:    zugzwang guard (K-only positions can lose by passing).
        //   beta < mate: don't corrupt mate-distance scoring.
        if can_null
            && !in_check
            && depth >= NMP_MIN_DEPTH
            && beta.abs() < MATE_THRESHOLD
            && Self::has_non_king_material(board, side)
        {
            board.make_null_move();
            let nmp_r: u8 = 3 + depth / 6;
            let reduced = depth.saturating_sub(1 + nmp_r);
            let null_score = -self.alpha_beta(
                board,
                reduced,
                -beta,
                -beta + 1,
                side.opponent(),
                ply + 1,
                false,
            );
            board.undo_null_move();
            if self.time_up {
                return 0;
            }
            if null_score >= beta {
                return null_score;
            }
        }

        // --- Reverse futility pruning (static null-move pruning) ---
        // At shallow depths when not in check, if static eval is far above
        // beta, the position is so good that no move can change the cutoff.
        if !in_check
            && (depth as usize) < REVERSE_FUTILITY_MARGIN.len()
            && beta.abs() < MATE_THRESHOLD
        {
            let static_eval = self.evaluator.evaluate(board, side);
            if static_eval - REVERSE_FUTILITY_MARGIN[depth as usize] >= beta {
                return static_eval;
            }
        }

        // --- Move generation ---
        let moves_raw = MoveGenerator::legal_moves_for_options(board, side, false);
        if moves_raw.is_empty() {
            // No legal moves = checkmate (if in check) or stalemate.
            return if MoveGenerator::is_in_check(board, side) {
                -Self::mate_score(depth)
            } else {
                0
            };
        }

        // Order: TT move → captures (MVV-LVA) → killers → history → rest
        let moves = self.ordered_moves(moves_raw, tt_move, ply);

        let mut best_score = i32::MIN / 2;
        let mut best_move_found: Option<Move> = None;
        let new_depth = depth - 1;

        // --- Futility pruning setup ---
        let can_futility = !in_check && (depth as usize) < FUTILITY_MARGIN.len();
        let futility_threshold = if can_futility {
            let static_eval = self.evaluator.evaluate(board, side);
            Some(static_eval + FUTILITY_MARGIN[depth as usize])
        } else {
            None
        };

        // --- Late Move Pruning (LMP) limit ---
        let lmp_limit = if !in_check && (depth as usize) < LMP_MOVE_LIMITS.len() {
            Some(LMP_MOVE_LIMITS[depth as usize])
        } else {
            None
        };

        for (i, mv) in moves.iter().enumerate() {
            if self.time_up {
                break;
            }
            let mv = *mv;

            let is_quiet = mv.capture.is_none() && !mv.promote;

            // --- Late Move Pruning (LMP) ---
            // At shallow depths, after searching enough moves, skip ALL
            // remaining quiet moves.  The best move was almost certainly
            // among the well-ordered moves already searched.
            if is_quiet && i > 0
                && let Some(limit) = lmp_limit
                    && i >= limit {
                        continue;
                    }

            // --- Futility pruning ---
            if is_quiet
                && i > 0
                && let Some(threshold) = futility_threshold
                && threshold <= alpha
            {
                continue;
            }

            let undo = board.make_move(mv);

            // --- Late Move Reductions (LMR) ---
            // Logarithmic formula: later moves at deeper nodes get larger
            // reductions.  Move 4 at depth 6 ≈ 1 ply; move 20 at depth 12 ≈ 3 ply.
            let reduction: u8 = if i >= LMR_MOVE_THRESHOLD
                && depth >= LMR_MIN_DEPTH
                && is_quiet
                && !in_check
            {
                let r = ((depth as f64).ln() * (i as f64).ln() / LMR_DIVISOR) as u8;
                r.max(1).min(depth - 1)
            } else {
                0
            };
            let reduced_depth = new_depth.saturating_sub(reduction);

            // --- PVS (Principal Variation Search) ---
            // Move 0 (TT/best-ordered): full window [-beta, -alpha].
            // Later moves: zero-window probe [-alpha-1, -alpha].
            //   - If LMR reduced + probe beats alpha → re-search full depth, null window.
            //   - If null-window probe beats alpha AND < beta → re-search full window
            //     (this move might be the new PV).
            let score;
            if i == 0 {
                score = -self.alpha_beta(
                    board,
                    new_depth,
                    -beta,
                    -alpha,
                    side.opponent(),
                    ply + 1,
                    true,
                );
            } else {
                // Scout search (possibly reduced).
                let mut s = -self.alpha_beta(
                    board,
                    reduced_depth,
                    -alpha - 1,
                    -alpha,
                    side.opponent(),
                    ply + 1,
                    true,
                );
                // LMR re-search: reduction was wrong, try full depth.
                if reduction > 0 && s > alpha && !self.time_up {
                    s = -self.alpha_beta(
                        board,
                        new_depth,
                        -alpha - 1,
                        -alpha,
                        side.opponent(),
                        ply + 1,
                        true,
                    );
                }
                // PVS re-search: score lies in (alpha, beta) — get exact value.
                if s > alpha && s < beta && !self.time_up {
                    s = -self.alpha_beta(
                        board,
                        new_depth,
                        -beta,
                        -alpha,
                        side.opponent(),
                        ply + 1,
                        true,
                    );
                }
                score = s;
            }

            board.undo_move(mv, undo);

            if score > best_score {
                best_score = score;
                best_move_found = Some(mv);
            }
            if score > alpha {
                alpha = score;
                if ply < MAX_PV_PLY {
                    self.update_pv(ply, mv);
                }
            }
            // --- Beta cutoff ---
            // The opponent would never allow this line (we already have a
            // better alternative elsewhere).  Record the move in killer/
            // history tables so it gets searched early at sibling nodes.
            if alpha >= beta {
                if mv.capture.is_none() {
                    self.store_killer(mv, ply);
                    self.update_history(mv, depth);
                }
                break;
            }
        }

        // --- TT store ---
        // Classify the result and cache it.  Skip if time ran out (partial
        // results would pollute the TT with unreliable scores).  The fixed-
        // size concurrent TT uses an always-replace policy, so there is no
        // capacity check here.
        if !self.time_up {
            let flag = if best_score >= beta {
                TTFlag::LowerBound  // we cut off — true value ≥ best_score
            } else if best_score > original_alpha {
                TTFlag::Exact       // we raised alpha — true value = best_score
            } else {
                TTFlag::UpperBound  // never beat alpha — true value ≤ best_score
            };
            self.tt.store(
                sig,
                TtEntry {
                    depth,
                    score: best_score,
                    flag,
                    best_move: best_move_found,
                },
            );
        }

        best_score
    }

    // -----------------------------------------------------------------------
    // Quiescence search
    // -----------------------------------------------------------------------
    // When the main search reaches depth 0, the position may be in the middle
    // of a capture exchange (e.g. a rook just took a pawn, and the opponent
    // can recapture).  Evaluating statically here would see the hanging pawn
    // as material — a wildly wrong score.
    //
    // Quiescence search resolves this "horizon effect" by continuing to search
    // only "loud" moves (captures and promotions) until the position is quiet.
    //
    // **Stand-pat**: at each node we can choose not to capture (stand pat).
    // The static eval is the lower bound on our score — we only search
    // captures that might improve on it.
    //
    // **Delta pruning**: before searching a capture, check if
    //   stand_pat + captured_piece_value + margin < alpha.
    // If so, even winning the capture can't raise alpha — skip it.
    //
    // Uses `MoveGenerator::loud_moves()` which only generates captures and
    // promotions (no quiet moves, no drops), roughly halving the work
    // compared to full generation + filter.

    fn quiescence(&mut self, board: &mut Board, mut alpha: i32, beta: i32, side: PlayerSide) -> i32 {
        if self.time_up || self.timed_out() {
            self.time_up = true;
            return 0;
        }
        self.nodes += 1;

        // Stand-pat: evaluate the position assuming we don't capture.
        let stand_pat = self.evaluator.evaluate(board, side);

        // If standing pat already beats beta, the opponent wouldn't allow this.
        if stand_pat >= beta {
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        // Generate only captures + promotions, ordered by MVV-LVA.
        let moves = MoveGenerator::loud_moves(board, side);
        let ordered = self.ordered_captures(moves);

        let mut best_score = stand_pat;
        for mv in &ordered {
            if self.time_up {
                break;
            }
            let mv = *mv;

            // Delta pruning: skip captures that can't possibly reach alpha.
            if let Some(captured) = mv.capture {
                let optimistic = stand_pat + DELTA_PIECE_VALUES[captured.index()] + DELTA_MARGIN;
                if optimistic < alpha {
                    continue;
                }
            }

            let undo = board.make_move(mv);
            let score = -self.quiescence(board, -beta, -alpha, side.opponent());
            board.undo_move(mv, undo);

            if score > best_score {
                best_score = score;
            }
            if score > alpha {
                alpha = score;
            }
            if alpha >= beta {
                break;
            }
        }
        best_score
    }

    // -----------------------------------------------------------------------
    // Killer moves
    // -----------------------------------------------------------------------

    fn store_killer(&mut self, mv: Move, ply: usize) {
        if self.killers.len() <= ply {
            self.killers.resize(ply + 1, [None; 2]);
        }
        let slot = &mut self.killers[ply];
        // Don't store duplicates; shift the existing killer to slot[1]
        if slot[0] != Some(mv) {
            slot[1] = slot[0];
            slot[0] = Some(mv);
        }
    }

    // -----------------------------------------------------------------------
    // History heuristic
    // -----------------------------------------------------------------------

    fn update_history(&mut self, mv: Move, depth: u8) {
        let from_idx = self.history_from_idx(mv);
        let to_idx = mv.to.index() as usize;
        // Bonus grows with depth^2 so deep cutoffs matter more; cap to prevent overflow
        self.history[from_idx][to_idx] =
            (self.history[from_idx][to_idx] + (depth as i32) * (depth as i32)).min(50_000);
    }

    #[inline]
    fn history_from_idx(&self, mv: Move) -> usize {
        mv.from
            .map(|s| s.index() as usize)
            .unwrap_or(HISTORY_FROM_SQUARES + mv.piece.index())
    }

    // -----------------------------------------------------------------------
    // Move ordering
    // -----------------------------------------------------------------------
    // Good move ordering is critical for alpha-beta efficiency.  Searching the
    // best move first produces a cutoff immediately, pruning the rest of the
    // tree.  The priority order is:
    //
    //   1. TT move (+1M):      the best move from a prior search of this position.
    //   2. Captures (+100k):   ordered by MVV-LVA (Most Valuable Victim, Least
    //                          Valuable Attacker) — prefer taking a rook with a
    //                          pawn over taking a pawn with a rook.
    //   3. Killer moves (+80-90k): quiet moves that caused beta cutoffs at this
    //                          ply in sibling nodes — often tactically important.
    //   4. History (+0-70k):   quiet moves that historically cause cutoffs,
    //                          weighted by depth² so deep cutoffs matter more.
    //   5. Promotions (+500):  minor bonus for promotion moves.
    //   6. Drops (+piece/2):   drops of valuable pieces get a small bonus.
    //   7. Everything else:    unscored quiet moves searched last.

    fn ordered_moves(&self, mut moves: MoveList, tt_move: Option<Move>, ply: usize) -> MoveList {
        moves.sort_by(|a, b| {
            self.move_score(*b, tt_move, ply)
                .cmp(&self.move_score(*a, tt_move, ply))
        });
        moves
    }

    fn ordered_captures(&self, mut moves: MoveList) -> MoveList {
        moves.sort_by_key(|m| std::cmp::Reverse(self.capture_score(*m)));
        moves
    }

    fn move_score(&self, mv: Move, tt_move: Option<Move>, ply: usize) -> i32 {
        // TT / PV move gets the highest priority
        if tt_move == Some(mv) {
            return 1_000_000;
        }

        let mut score = 0;

        if let Some(capture) = mv.capture {
            // MVV-LVA: Most Valuable Victim minus Least Valuable Attacker
            score += 100_000
                + MOVE_ORDER_VALUES[capture.index()] * 10
                - MOVE_ORDER_VALUES[mv.piece.index()];
        } else {
            // Killer moves (quiet moves that caused cutoffs at this ply before)
            if let Some(killers) = self.killers.get(ply) {
                if killers[0] == Some(mv) {
                    score += 90_000;
                } else if killers[1] == Some(mv) {
                    score += 80_000;
                }
            }
            // History heuristic bonus
            let from_idx = self.history_from_idx(mv);
            let to_idx = mv.to.index() as usize;
            score += self.history[from_idx][to_idx].min(70_000);
        }

        if mv.promote {
            score += 500;
        }
        if mv.kind == MoveKind::Drop {
            score += MOVE_ORDER_VALUES[mv.piece.index()] / 2;
        }
        if mv.piece == PieceKind::Pawn && !mv.promote {
            score += 50;
        }
        score
    }

    fn capture_score(&self, mv: Move) -> i32 {
        let mut score = 0;
        if let Some(capture) = mv.capture {
            score += 10_000 + MOVE_ORDER_VALUES[capture.index()] * 10
                - MOVE_ORDER_VALUES[mv.piece.index()];
        }
        if mv.promote {
            score += 500;
        }
        score
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn mate_score(depth: u8) -> i32 {
        MATE_SCORE - depth as i32
    }

    /// Returns true if `side` owns any non-king piece on the board or in hand.
    /// Used as a zugzwang guard for null-move pruning: in pure king-only
    /// positions the "passing is no worse than moving" assumption breaks down.
    fn has_non_king_material(board: &Board, side: PlayerSide) -> bool {
        for &kind in &PieceKind::ALL {
            if kind == PieceKind::King {
                continue;
            }
            if !board.bitboards().piece(side, kind).is_empty() {
                return true;
            }
            if board.hand(side).count(kind) > 0 {
                return true;
            }
        }
        false
    }

    /// Returns true if the search must stop immediately.
    ///
    /// Two stop conditions are checked, with different frequencies:
    ///
    /// 1. **Abort flag** (checked every call, O(1)):
    ///    An external thread (e.g. the USI "stop" command handler) can set this
    ///    flag at any time.  It is checked on every call because the whole point
    ///    of an abort flag is low latency — we want the search to stop within
    ///    milliseconds, not thousands of nodes.  `Relaxed` ordering is sufficient
    ///    because we only need to eventually see the write; strict ordering is not
    ///    required for correctness here.
    ///
    /// 2. **Deadline** (checked every `TIME_CHECK_INTERVAL` nodes):
    ///    `Instant::now()` is a syscall that costs ~10–50 ns on most platforms.
    ///    Calling it on every node (potentially millions per second) wastes CPU
    ///    time and causes the search to overshoot the deadline on each unwind step.
    ///    Instead, `check_counter` counts down from `TIME_CHECK_INTERVAL`; only
    ///    when it reaches 0 do we call `Instant::now()` and reset the counter.
    ///    With 1024 nodes per interval and ~100k–1M nodes/sec, the worst-case
    ///    overshoot is 1–10 ms — acceptable for typical think times of 1–30 s.
    ///
    /// Note: callers should check `self.time_up` (a plain bool) **before** calling
    /// this function.  Once `time_up` is true, every node on the unwind path would
    /// otherwise still call `timed_out()` needlessly.
    fn timed_out(&mut self) -> bool {
        // 1. Abort flag — checked every call, cheap atomic load.
        if let Some(flag) = &self.abort_flag
            && flag.load(Ordering::Relaxed) {
                return true;
            }
        // 2. Deadline — only check Instant::now() every TIME_CHECK_INTERVAL calls.
        if self.check_counter > 0 {
            self.check_counter -= 1;
            return false; // Haven't reached the check interval yet
        }
        // Reset the counter and perform the actual wall-clock check.
        self.check_counter = TIME_CHECK_INTERVAL;
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline {
                return true;
            }
        false
    }

    // -----------------------------------------------------------------------
    // PV (Principal Variation) tracking
    // -----------------------------------------------------------------------

    /// Copies the child PV from ply+1 into ply, prepending `mv`.
    fn update_pv(&mut self, ply: usize, mv: Move) {
        if ply + 1 >= MAX_PV_PLY {
            self.pv_length[ply] = 1;
            self.pv_table[ply][0] = Some(mv);
            return;
        }
        self.pv_table[ply][0] = Some(mv);
        let child_len = self.pv_length[ply + 1];
        for i in 0..child_len.min(MAX_PV_PLY - 1) {
            self.pv_table[ply][i + 1] = self.pv_table[ply + 1][i];
        }
        self.pv_length[ply] = 1 + child_len;
    }

    /// Outputs a USI `info` line with depth, score, time, nodes, nps, and PV.
    fn output_info_usi(&self, depth: u8, score: i32) {
        let elapsed_ms = self
            .search_start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let nps = if elapsed_ms > 0 {
            self.nodes * 1000 / elapsed_ms
        } else {
            0
        };

        // Format score: "score cp X" or "score mate X"
        let score_str = if score.abs() >= MATE_THRESHOLD {
            let mate_ply = MATE_SCORE - score.abs();
            // Convert ply distance to move count (round up).
            let mate_moves = (mate_ply + 1) / 2;
            if score > 0 {
                format!("score mate {}", mate_moves)
            } else {
                format!("score mate -{}", mate_moves)
            }
        } else {
            format!("score cp {}", score)
        };

        // Format PV as space-separated USI moves.
        let pv_len = self.pv_length[0].min(MAX_PV_PLY);
        let mut pv_str = String::new();
        for i in 0..pv_len {
            if let Some(mv) = self.pv_table[0][i] {
                if !pv_str.is_empty() {
                    pv_str.push(' ');
                }
                pv_str.push_str(&Self::format_move_usi(mv));
            }
        }

        let hashfull = self.tt.hashfull();

        if pv_str.is_empty() {
            println!(
                "info depth {} {} time {} nodes {} nps {} hashfull {}",
                depth, score_str, elapsed_ms, self.nodes, nps, hashfull
            );
        } else {
            println!(
                "info depth {} {} time {} nodes {} nps {} hashfull {} pv {}",
                depth, score_str, elapsed_ms, self.nodes, nps, hashfull, pv_str
            );
        }
    }

    /// Outputs a human-readable progress line for `think` mode.
    ///
    /// Format:
    ///   `depth 12 | eval +42 | 1.5M nodes (2.1M nps) | pv: 7776 8384 2838+`
    /// Mate scores render as `M3` (mate in 3 moves) or `-M2`.
    fn output_info_think(&self, depth: u8, score: i32) {
        let elapsed_ms = self
            .search_start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let nps = if elapsed_ms > 0 {
            self.nodes * 1000 / elapsed_ms
        } else {
            0
        };

        let score_str = if score.abs() >= MATE_THRESHOLD {
            let mate_ply = MATE_SCORE - score.abs();
            let mate_moves = (mate_ply + 1) / 2;
            if score > 0 {
                format!("M{}", mate_moves)
            } else {
                format!("-M{}", mate_moves)
            }
        } else {
            format!("{:+}", score)
        };

        let pv_len = self.pv_length[0].min(MAX_PV_PLY);
        let mut pv_str = String::new();
        for i in 0..pv_len {
            if let Some(mv) = self.pv_table[0][i] {
                if !pv_str.is_empty() {
                    pv_str.push(' ');
                }
                pv_str.push_str(&Self::format_move(mv));
            }
        }
        if pv_str.is_empty() {
            pv_str.push_str("(none)");
        }

        println!(
            "depth {:>2} | eval {:>6} | {} nodes ({}/s) | pv: {}",
            depth,
            score_str,
            format_node_count(self.nodes),
            format_node_count(nps),
            pv_str,
        );
    }

    fn format_move_usi(mv: Move) -> String {
        match mv.kind {
            MoveKind::Drop => {
                let piece = mv.piece.short_name().chars().next().unwrap_or('P');
                let to = Self::square_to_usi(mv.to);
                format!("{}*{}", piece.to_ascii_lowercase(), to)
            }
            _ => {
                let from = Self::square_to_usi(mv.from.expect("from square"));
                let to = Self::square_to_usi(mv.to);
                if mv.promote {
                    format!("{}{}+", from, to)
                } else {
                    format!("{}{}", from, to)
                }
            }
        }
    }

    fn square_to_usi(square: Square) -> String {
        let file = 9 - square.file();
        let rank = (b'a' + square.rank()) as char;
        format!("{}{}", file, rank)
    }

    fn format_move(mv: Move) -> String {
        match mv.kind {
            MoveKind::Drop => format!("{}*{}", mv.piece.short_name().to_ascii_uppercase(), mv.to),
            _ => {
                let mut text = format!("{}{}", mv.from.unwrap(), mv.to);
                if mv.promote {
                    text.push('+');
                }
                text
            }
        }
    }
}

/// Combines the outcomes of every search thread into a single result:
///   * total node count across all threads (so the caller's NPS reflects
///     the real aggregate work);
///   * best move is taken from the deepest result that actually has one,
///     breaking ties by score;
///   * stop reason is inherited from the main thread.
///
/// When no thread produced a usable `best_move` (e.g. every worker was
/// aborted before depth 1), returns a `Default::default()` outcome with
/// the summed node count preserved.
fn aggregate_outcomes(
    outcomes: Vec<SearchOutcome>,
    main_stop_reason: StopReason,
) -> SearchOutcome {
    let total_nodes: u64 = outcomes.iter().map(|o| o.nodes).sum();
    let best = outcomes
        .into_iter()
        .filter(|o| o.best_move.is_some())
        .max_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.score.cmp(&b.score))
        });
    match best {
        Some(mut outcome) => {
            outcome.nodes = total_nodes;
            outcome.stop_reason = main_stop_reason;
            outcome
        }
        None => SearchOutcome {
            nodes: total_nodes,
            stop_reason: main_stop_reason,
            ..SearchOutcome::default()
        },
    }
}

/// Renders a large node count in compact form: `1.2M`, `950k`, `5200`.
fn format_node_count(n: u64) -> String {
    if n >= 10_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{}", n)
    }
}
