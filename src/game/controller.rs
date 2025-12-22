use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::engine::{
    board::{Board, PositionSignature},
    movegen::MoveGenerator,
    movement::Move,
    search::{AlphaBetaSearcher, SearchConfig, SearchOutcome},
    state::PlayerSide,
};

use super::{
    config::GameConfig,
    timer::{TimeManager, TimeStatus},
};

#[derive(Clone, Debug)]
pub enum GameStatus {
    Ready,
    AwaitingMove { side: PlayerSide },
    Completed(GameResult),
}

#[derive(Clone, Debug)]
pub enum GameResult {
    Resignation { winner: PlayerSide },
    Checkmate { winner: PlayerSide },
    Repetition,
    Timeout { winner: PlayerSide },
}

#[derive(Clone, Debug)]
pub enum MoveError {
    GameAlreadyFinished,
    IllegalMove,
    Timeout { winner: PlayerSide },
}

#[derive(Clone, Debug)]
pub enum AdvanceState {
    Ongoing,
    Completed(GameResult),
}

pub struct GameController {
    config: GameConfig,
    board: Board,
    time_manager: TimeManager,
    searchers: [Option<AlphaBetaSearcher>; 2],
    status: GameStatus,
    history: HashMap<PositionSignature, u32>,
    move_log: Vec<Move>,
    clock_running: Option<PlayerSide>,
}

impl GameController {
    pub fn new(config: GameConfig, debug_log: Option<Arc<Mutex<File>>>) -> Self {
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

    pub fn bootstrap(&mut self) {
        let side = self.board.to_move();
        self.clock_running = None;
        self.status = GameStatus::AwaitingMove { side };
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        MoveGenerator::legal_moves_for(&self.board, self.board.to_move())
    }

    pub fn request_move(&mut self) -> Option<(SearchOutcome, Duration)> {
        let side = self.board.to_move();
        // println!("[TRACE] controller::request_move start side={:?}", side);
        let idx = side.index();
        let limit = self.think_time_for(side);
        // println!(
        //     "[TRACE] controller::request_move think_time side={:?} limit={:?}",
        //     side, limit
        // );
        match self.searchers[idx].as_mut() {
            Some(searcher) => {
                // println!(
                //     "[TRACE] controller::request_move invoking search side={:?}",
                //     side
                // );
                // let legal_moves = MoveGenerator::legal_moves_for(&self.board, side);
                // println!(
                //     "[TRACE] controller::request_move state ply={} total_legal={} moves={:?}",
                //     self.board.ply(),
                //     legal_moves.len(),
                //     legal_moves
                // );
                // println!(
                //     "[TRACE] controller::request_move hands Sente={:?} Gote={:?}",
                //     self.board.hand(PlayerSide::Sente),
                //     self.board.hand(PlayerSide::Gote)
                // );
                let outcome = searcher.search(&self.board, limit);
                // println!(
                //     "[TRACE] controller::request_move done side={:?} best_move={:?} score={} depth={} nodes={}",
                //     side, outcome.best_move, outcome.score, outcome.depth, outcome.nodes
                // );
                Some((outcome, limit))
            }
            None => {
                // println!(
                //     "[TRACE] controller::request_move no searcher configured for side={:?}",
                //     side
                // );
                None
            }
        }
    }

    pub fn apply_move(&mut self, mv: Move) -> Result<AdvanceState, MoveError> {
        let side = self.board.to_move();
        match self.status {
            GameStatus::Completed(_) => return Err(MoveError::GameAlreadyFinished),
            GameStatus::Ready => return Err(MoveError::GameAlreadyFinished),
            GameStatus::AwaitingMove { side: expected } if expected != side => {
                return Err(MoveError::GameAlreadyFinished);
            }
            _ => {}
        }

        let legal_moves = self.legal_moves();
        if !legal_moves.iter().any(|candidate| *candidate == mv) {
            return Err(MoveError::IllegalMove);
        }

        if let TimeStatus::Flagged = self.time_manager.stop_turn(side) {
            let result = GameResult::Timeout {
                winner: side.opponent(),
            };
            self.status = GameStatus::Completed(result.clone());
            return Err(MoveError::Timeout {
                winner: side.opponent(),
            });
        }

        self.board.apply_move(mv);
        self.move_log.push(mv);

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

    pub fn resign(&mut self, loser: PlayerSide) {
        let _ = self.time_manager.stop_turn(loser);
        self.status = GameStatus::Completed(GameResult::Resignation {
            winner: loser.opponent(),
        });
        self.clock_running = None;
    }

    fn evaluate_position(&mut self) -> Option<GameResult> {
        let sig = self.board.signature();
        let count = self.history.entry(sig).or_insert(0);
        *count += 1;
        if *count >= 4 {
            return Some(GameResult::Repetition);
        }

        let side = self.board.to_move();
        let legal = MoveGenerator::legal_moves_for(&self.board, side);
        if legal.is_empty() {
            return Some(GameResult::Checkmate {
                winner: side.opponent(),
            });
        }
        None
    }

    fn think_time_for(&self, side: PlayerSide) -> Duration {
        let (main, byoyomi) = self.time_manager.remaining(side);
        let max_think = Duration::from_secs(30);
        if main > Duration::from_secs(0) {
            let mut slice = main / 20;
            let min = Duration::from_millis(300);
            let max = Duration::from_secs(30);
            if slice.is_zero() {
                slice = if main < min { main } else { min };
            }
            if slice < min {
                slice = min;
            }
            if slice > max {
                slice = max;
            }
            if slice > main {
                slice = main;
            }
            slice.min(max_think)
        } else {
            let mut slice = byoyomi / 5;
            let min = Duration::from_millis(200);
            let max = Duration::from_secs(5);
            if slice.is_zero() {
                slice = if byoyomi < min { byoyomi } else { min };
            }
            if slice < min {
                slice = min;
            }
            if slice > max {
                slice = max;
            }
            if slice > byoyomi {
                slice = byoyomi;
            }
            slice.min(max_think)
        }
    }

    pub fn ensure_clock_started(&mut self) {
        if let GameStatus::AwaitingMove { side } = self.status {
            if self.clock_running != Some(side) {
                self.time_manager.start_turn(side);
                self.clock_running = Some(side);
            }
        }
    }
}
