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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{evaluator::MaterialEvaluator, strength::{SearchStrength, MAX_DEPTH}};
use crate::engine::{
    board::Board,
    movegen::MoveGenerator,
    movelist::MoveList,
    movement::{Move, MoveKind},
    state::{PieceKind, PlayerSide},
};
use std::{
    fs::File,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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

// ~1M positions.  Larger tables hit more but consume more memory.
const TT_MAX_SIZE: usize = 1 << 20;

// How often to poll the wall clock.  1024 nodes ≈ 0.6 ms at 1.6M nps.
const TIME_CHECK_INTERVAL: u32 = 1024;

// History table dimensions: 81 board squares + 8 drop piece kinds = 89 "from" slots,
// 81 "to" squares.  history[from][to] stores a score for how often this
// (from, to) pair caused a beta cutoff — used for quiet-move ordering.
const HISTORY_FROM_SQUARES: usize = 81;
const HISTORY_FROM_SIZE: usize = HISTORY_FROM_SQUARES + PieceKind::ALL.len(); // 89
const HISTORY_TO_SIZE: usize = 81;

// --- Pruning / reduction tuning constants ---

// Null-move pruning: skip the full search when a reduced-depth null-window
// search (after passing the turn) still beats beta.
// R = 2 is standard; R = 3 is faster but risks missing shogi-specific
// zugzwang and tactical sequences.
const NMP_MIN_DEPTH: u8 = 3;
const NMP_REDUCTION: u8 = 2;

// Late Move Reductions: after LMR_MOVE_THRESHOLD well-ordered moves,
// search quiet moves at (depth - reduction) — re-search at full depth only
// if the reduced search beats alpha.  Safe because a fail-high always
// triggers a re-search.
const LMR_MIN_DEPTH: u8 = 3;
const LMR_MOVE_THRESHOLD: usize = 3;

// Futility pruning margins indexed by depth (depth 0 unused).
// At depth d, if static_eval + FUTILITY_MARGIN[d] <= alpha, quiet moves
// are skipped.  Larger margins are more aggressive (prune more, risk more).
const FUTILITY_MARGIN: [i32; 3] = [0, 300, 500];

// Delta pruning in quiescence: skip a capture if
//   stand_pat + captured_piece_value + DELTA_MARGIN < alpha.
// The margin accounts for positional value beyond raw material.
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
// Each entry stores:
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TTFlag {
    Exact,
    LowerBound,
    UpperBound,
}

#[derive(Clone, Copy)]
struct TTEntry {
    depth: u8,
    score: i32,
    flag: TTFlag,
    best_move: Option<Move>,
}

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    pub strength: SearchStrength,
    /// Default think time when no explicit time limit is provided.
    pub time_per_move: Duration,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            strength: SearchStrength::Normal,
            time_per_move: Duration::from_secs(1),
        }
    }
}

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
    debug_log: Option<Arc<Mutex<File>>>,
    /// External stop signal (e.g. USI "stop" command).
    abort_flag: Option<Arc<AtomicBool>>,
    /// Transposition table — persists across moves so accumulated knowledge
    /// carries over.  Keyed by 64-bit Zobrist hash.
    tt: HashMap<u64, TTEntry>,
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
}

impl AlphaBetaSearcher {
    pub fn new(config: SearchConfig, debug_log: Option<Arc<Mutex<File>>>) -> Self {
        Self {
            evaluator: MaterialEvaluator::default(),
            config,
            nodes: 0,
            deadline: None,
            time_up: false,
            debug_log,
            abort_flag: None,
            tt: HashMap::with_capacity(1 << 16),
            killers: Vec::new(),
            history: Box::new([[0; HISTORY_TO_SIZE]; HISTORY_FROM_SIZE]),
            check_counter: TIME_CHECK_INTERVAL,
        }
    }

    pub fn set_abort_flag(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.abort_flag = flag;
    }

    /// Main entry point: runs iterative deepening from depth 1 up to the
    /// strength limit.  Each completed iteration improves move ordering for
    /// the next (via TT and history).  The best move from the last *completed*
    /// iteration is returned — incomplete iterations are discarded.
    ///
    /// A fallback move is seeded before the loop so that even if depth 1
    /// times out mid-search, we still return a legal move instead of None.
    pub fn search(&mut self, board: &Board, time_limit: Duration) -> SearchOutcome {
        self.nodes = 0;
        self.time_up = false;
        self.check_counter = TIME_CHECK_INTERVAL;
        // Killers and history are per-search; TT is kept across moves.
        self.killers.clear();
        *self.history = [[0; HISTORY_TO_SIZE]; HISTORY_FROM_SIZE];

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

        eprintln!("START search limit={:?} side={:?}", slice, board.to_move());

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

        if let Some(mv) = best_move {
            self.log_line(&format!(
                "FINISH best_move={} score={} depth={} nodes={}",
                Self::format_move(mv),
                best_score,
                best_depth,
                self.nodes
            ));
        } else {
            self.log_line(&format!(
                "FINISH no-move score={} depth={} nodes={}",
                best_score, best_depth, self.nodes
            ));
        }
        SearchOutcome {
            best_move,
            score: best_score,
            nodes: self.nodes,
            depth: best_depth,
            stop_reason,
        }
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
        let tt_move = self.tt.get(&board.zobrist()).and_then(|e| e.best_move);
        let moves = self.ordered_moves(MoveGenerator::legal_moves_for(board, side), tt_move, 0);
        if moves.is_empty() {
            let score = if MoveGenerator::is_in_check(board, side) {
                -Self::mate_score(depth)
            } else {
                0
            };
            return (None, score);
        }

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

        // --- Leaf → quiescence ---
        // Don't evaluate statically at depth 0 — hand off to quiescence
        // search which resolves all captures/promotions first.
        if depth == 0 {
            return self.quiescence(board, alpha, beta, side);
        }

        let original_alpha = alpha;
        let sig = board.zobrist();

        // --- Transposition table probe ---
        // If we've seen this exact position before at equal or greater depth,
        // we can reuse or tighten the bounds.  Always extract the best move
        // for move ordering even when the score isn't directly usable.
        let mut tt_move: Option<Move> = None;
        if let Some(entry) = self.tt.get(&sig) {
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
            let reduced = depth.saturating_sub(1 + NMP_REDUCTION);
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

        // --- Move generation ---
        // Skips the uchi-fu-zume (pawn-drop-mate) check inside the search
        // tree for performance.  The root move is validated by the controller.
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
        // At depth 1-2, compute static eval + margin once.  If even with the
        // margin we can't reach alpha, all quiet moves are futile.
        let can_futility = !in_check && (depth as usize) < FUTILITY_MARGIN.len();
        let futility_threshold = if can_futility {
            let static_eval = self.evaluator.evaluate(board, side);
            Some(static_eval + FUTILITY_MARGIN[depth as usize])
        } else {
            None
        };

        for (i, mv) in moves.iter().enumerate() {
            if self.time_up {
                break;
            }
            let mv = *mv;

            // --- Futility pruning ---
            // Skip quiet moves (no capture, no promotion) past the PV move
            // when static eval + margin can't reach alpha.
            let is_quiet = mv.capture.is_none() && !mv.promote;
            if is_quiet
                && i > 0
                && let Some(threshold) = futility_threshold
                && threshold <= alpha
            {
                continue;
            }

            let undo = board.make_move(mv);

            // --- Late Move Reductions (LMR) ---
            // Quiet moves beyond the first few well-ordered ones are unlikely
            // to be best.  Search them at reduced depth; if they beat alpha,
            // re-search at full depth to verify.
            let reduction: u8 = if i >= LMR_MOVE_THRESHOLD
                && depth >= LMR_MIN_DEPTH
                && is_quiet
                && !in_check
            {
                if depth >= 6 { 2 } else { 1 }
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
        // results would pollute the TT with unreliable scores).
        if !self.time_up {
            let flag = if best_score >= beta {
                TTFlag::LowerBound  // we cut off — true value ≥ best_score
            } else if best_score > original_alpha {
                TTFlag::Exact       // we raised alpha — true value = best_score
            } else {
                TTFlag::UpperBound  // never beat alpha — true value ≤ best_score
            };
            if self.tt.contains_key(&sig) || self.tt.len() < TT_MAX_SIZE {
                self.tt.insert(
                    sig,
                    TTEntry {
                        depth,
                        score: best_score,
                        flag,
                        best_move: best_move_found,
                    },
                );
            }
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

    fn log_line(&self, message: &str) {
        if let Some(writer) = &self.debug_log
            && let Ok(mut guard) = writer.lock() {
                let _ = writeln!(guard, "{}", message);
            }
    }

    fn format_move(mv: Move) -> String {
        match mv.kind {
            MoveKind::Drop => format!("{}*{}", mv.piece.short_name(), mv.to),
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
