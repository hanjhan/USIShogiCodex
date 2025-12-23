use crate::engine::{
    board::Board,
    movegen::MoveGenerator,
    movement::{Move, MoveKind},
    search::{AlphaBetaSearcher, SearchConfig, SearchOutcome},
    state::{PieceKind, PlayerSide, Square},
};
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn run() {
    let mut engine = UsiEngine::new();
    engine.run();
}

struct SearchTask {
    abort: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    rx: Receiver<SearchOutcome>,
}

struct UsiEngine {
    board: Board,
    search_task: Option<SearchTask>,
    side_fixed: bool,
    log_file: Option<BufWriter<File>>,
}

impl UsiEngine {
    fn new() -> Self {
        Self {
            board: Board::new_standard(),
            search_task: None,
            side_fixed: false,
            log_file: Self::create_log_file(),
        }
    }

    fn create_log_file() -> Option<BufWriter<File>> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let pid = std::process::id();
        let filename = format!("usi-log-{}-{}.log", pid, timestamp);
        File::create(filename).ok().map(BufWriter::new)
    }

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
            self.poll_search();
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    let trimmed = line.trim();
                    if !self.handle_command(trimmed) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

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
                self.board = Board::new_standard();
                self.side_fixed = false;
            }
            "position" => {
                let rest = line["position".len()..].trim();
                self.log_line(line);
                self.handle_position(rest);
            }
            "go" => {
                self.start_search();
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

    fn handle_position(&mut self, args: &str) {
        self.board = Board::new_standard();
        let mut tokens = args.split_whitespace();
        let mut move_count = 0usize;
        if tokens.next() == Some("startpos") {
            if tokens.next() == Some("moves") {
                for mv_text in tokens {
                    if !self.apply_usi_move(mv_text) {
                        break;
                    }
                    move_count += 1;
                }
            }
        }
        if !self.side_fixed {
            self.update_side_to_move(move_count);
            self.side_fixed = true;
        }
    }

    fn update_side_to_move(&mut self, move_count: usize) {
        let side = if move_count % 2 == 0 {
            PlayerSide::Sente
        } else {
            PlayerSide::Gote
        };
        self.board.set_to_move(side);
    }

    fn start_search(&mut self) {
        if self.search_task.is_some() {
            return;
        }
        let board_clone = self.board.clone();
        let (tx, rx) = mpsc::channel();
        let abort = Arc::new(AtomicBool::new(false));
        let search_abort = abort.clone();
        let handle = thread::spawn(move || {
            let mut searcher = AlphaBetaSearcher::new(SearchConfig::default(), None);
            searcher.set_abort_flag(Some(search_abort));
            let outcome = searcher.search(&board_clone, Duration::from_secs(30));
            let _ = tx.send(outcome);
        });
        self.search_task = Some(SearchTask {
            abort,
            handle: Some(handle),
            rx,
        });
    }

    fn stop_search(&mut self) {
        if let Some(mut task) = self.search_task.take() {
            task.abort.store(true, Ordering::SeqCst);
            let outcome = task.rx.recv().unwrap_or_else(|_| SearchOutcome::default());
            if let Some(handle) = task.handle.take() {
                let _ = handle.join();
            }
            self.output_bestmove(outcome);
        }
    }

    fn poll_search(&mut self) {
        if let Some(task) = &mut self.search_task {
            if let Ok(outcome) = task.rx.try_recv() {
                if let Some(handle) = task.handle.take() {
                    let _ = handle.join();
                }
                self.search_task = None;
                self.output_bestmove(outcome);
            }
        }
    }

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

    fn apply_usi_move(&mut self, text: &str) -> bool {
        let legal = MoveGenerator::legal_moves(&self.board);
        if let Some(mv) = Self::select_usi_move(&legal, text) {
            self.board.apply_move(mv);
            true
        } else {
            eprintln!("Failed to parse move: {}", text);
            false
        }
    }

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

    fn parse_square(text: &str) -> Option<Square> {
        if text.len() != 2 {
            return None;
        }
        Square::from_text(text)
    }

    fn format_usi_move(mv: Move) -> String {
        match mv.kind {
            MoveKind::Drop => {
                let mut piece = mv.piece.short_name().chars().next().unwrap_or('P').to_ascii_uppercase();
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

    fn square_to_usi(square: Square) -> String {
        let file = 9 - square.file();
        let rank = (b'a' + square.rank()) as char;
        format!("{}{}", file, rank)
    }

    fn piece_from_char(ch: char) -> Option<PieceKind> {
        match ch.to_ascii_uppercase() {
            'K' => Some(PieceKind::King),
            'R' => Some(PieceKind::Rook),
            'B' => Some(PieceKind::Bishop),
            'G' => Some(PieceKind::Gold),
            'S' => Some(PieceKind::Silver),
            'N' => Some(PieceKind::Knight),
            'L' => Some(PieceKind::Lance),
            'P' => Some(PieceKind::Pawn),
            _ => None,
        }
    }
}
