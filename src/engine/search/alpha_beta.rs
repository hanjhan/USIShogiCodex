use std::time::{Duration, Instant};

use super::{evaluator::MaterialEvaluator, strength::SearchStrength};
use crate::engine::{
    board::Board,
    movegen::MoveGenerator,
    movement::{Move, MoveKind},
    state::{PieceKind, PlayerSide},
};
use std::{
    fs::File,
    io::Write,
    sync::{Arc, Mutex},
};

const MATE_SCORE: i32 = 30_000;
const MAX_SEARCH_TIME: Duration = Duration::from_secs(30);
const MOVE_ORDER_VALUES: [i32; PieceKind::ALL.len()] = [
    0,   // King
    900, // Rook
    850, // Bishop
    600, // Gold
    500, // Silver
    350, // Knight
    300, // Lance
    100, // Pawn
];

#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    pub strength: SearchStrength,
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

#[derive(Default, Debug)]
pub struct SearchOutcome {
    pub best_move: Option<Move>,
    pub score: i32,
    pub nodes: u64,
    pub depth: u8,
}

pub struct AlphaBetaSearcher {
    evaluator: MaterialEvaluator,
    config: SearchConfig,
    nodes: u64,
    deadline: Option<Instant>,
    time_up: bool,
    debug_log: Option<Arc<Mutex<File>>>,
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
        }
    }

    pub fn search(&mut self, board: &Board, time_limit: Duration) -> SearchOutcome {
        self.nodes = 0;
        self.time_up = false;
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

        let max_depth = self.config.strength.depth();
        let mut best_move = None;
        let mut best_score = self.evaluator.evaluate(board, board.to_move());
        let mut best_depth = 0;
        self.log_line(&format!(
            "START search limit={:?} side={:?}",
            slice,
            board.to_move()
        ));
        println!("START search limit={:?} side={:?}", slice, board.to_move());
        for depth in 1..=max_depth {
            let (cand_move, cand_score) = self.search_root(board, depth);
            if cand_move.is_some() || best_move.is_none() {
                best_move = cand_move;
                best_score = cand_score;
                best_depth = depth;
            }
            if self.time_up {
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
        }
    }

    fn search_root(&mut self, board: &Board, depth: u8) -> (Option<Move>, i32) {
        let side = board.to_move();
        let moves = self.order_moves(MoveGenerator::legal_moves_for(board, side));
        if moves.is_empty() {
            let score = if MoveGenerator::is_in_check(board, side) {
                -Self::mate_score(depth)
            } else {
                0
            };
            return (None, score);
        }

        let mut best_move = Some(moves[0]);
        let mut alpha = i32::MIN / 2;
        let beta = i32::MAX / 2;
        for mv in moves {
            if self.time_up {
                break;
            }
            let mut next = board.clone();
            next.apply_move(mv);
            let score = -self.alpha_beta(&next, depth - 1, -beta, -alpha, side.opponent());
            if score > alpha {
                alpha = score;
                best_move = Some(mv);
            }
        }

        (best_move, alpha)
    }

    fn alpha_beta(
        &mut self,
        board: &Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        side: PlayerSide,
    ) -> i32 {
        if self.timed_out() {
            self.time_up = true;
            return self.evaluator.evaluate(board, side);
        }
        self.nodes += 1;
        if depth == 0 {
            return self.evaluator.evaluate(board, side);
        }

        let mut best_score = i32::MIN / 2;
        let moves = self.order_moves(MoveGenerator::legal_moves_for(board, side));
        if moves.is_empty() {
            return if MoveGenerator::is_in_check(board, side) {
                -Self::mate_score(depth)
            } else {
                0
            };
        }

        for mv in moves {
            if self.time_up {
                break;
            }
            let mut next = board.clone();
            next.apply_move(mv);
            let score = -self.alpha_beta(&next, depth - 1, -beta, -alpha, side.opponent());
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

    fn mate_score(depth: u8) -> i32 {
        MATE_SCORE - depth as i32
    }

    fn timed_out(&self) -> bool {
        if let Some(deadline) = self.deadline {
            Instant::now() >= deadline
        } else {
            false
        }
    }

    fn order_moves(&self, moves: Vec<Move>) -> Vec<Move> {
        let mut moves = moves;
        moves.sort_by(|a, b| self.move_order_score(*b).cmp(&self.move_order_score(*a)));
        moves
    }

    fn move_order_score(&self, mv: Move) -> i32 {
        let mut score = 0;
        if let Some(capture) = mv.capture {
            score += 10_000 + MOVE_ORDER_VALUES[capture.index()];
            score -= MOVE_ORDER_VALUES[mv.piece.index()] / 10;
        }
        if mv.promote {
            score += 500;
        }
        match mv.kind {
            MoveKind::Drop => score += MOVE_ORDER_VALUES[mv.piece.index()] / 2,
            MoveKind::Capture => score += 200,
            MoveKind::Quiet => {}
        }
        if mv.piece == PieceKind::Pawn && !mv.promote {
            score += 50;
        }
        score
    }

    fn log_line(&self, message: &str) {
        if let Some(writer) = &self.debug_log {
            if let Ok(mut guard) = writer.lock() {
                let _ = writeln!(guard, "{}", message);
            }
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
