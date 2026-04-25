use crate::{
    cli::{board_render::BoardRenderer, input},
    engine::{
        board::Board,
        movement::{Move, MoveKind},
        search::{SearchStrength, StopReason},
        state::{PieceKind, PlayerSide, Square},
    },
    game::{
        AdvanceState, GameConfig, GameController, GameResult, GameStatus, MoveError,
        config::GameMode,
        player::{PlayerDescriptor, PlayerKind},
        timer::{TimeControl, TimeManager},
    },
};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

pub struct AppCli;

impl AppCli {
    pub fn run() {
        Self::print_banner();
        let (config, debug_mode) = Self::configure_game();
        let log_handle = if debug_mode {
            match File::create("debug.log") {
                Ok(file) => Some(Arc::new(Mutex::new(file))),
                Err(err) => {
                    eprintln!("Failed to create debug.log: {}", err);
                    None
                }
            }
        } else {
            None
        };
        Self::announce(&config);
        let mut controller = GameController::new(config, log_handle.clone());
        controller.bootstrap();
        controller.ensure_clock_started();
        let (tx, rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            let stdin = io::stdin();
            loop {
                let mut buf = String::new();
                if stdin.read_line(&mut buf).is_err() {
                    break;
                }
                if tx.send(buf).is_err() {
                    break;
                }
            }
        });
        Self::game_loop(&mut controller, debug_mode, &log_handle, rx);
    }

    fn game_loop(
        controller: &mut GameController,
        debug_mode: bool,
        log_handle: &Option<Arc<Mutex<File>>>,
        rx: Receiver<String>,
    ) {
        let mut pending_input = VecDeque::new();
        loop {
            println!("\n{}", BoardRenderer::render(controller.board()));
            println!("USI: {}", BoardRenderer::render_usi(controller.board()));
            Self::print_clock_info(controller);
            if Self::process_global_commands(controller, &rx, &mut pending_input)
                && matches!(controller.status(), GameStatus::Completed(_)) {
                    if let GameStatus::Completed(result) = controller.status().clone() {
                        Self::print_result(&result);
                    }
                    break;
                }
            match controller.status().clone() {
                GameStatus::Completed(result) => {
                    Self::print_result(&result);
                    Self::print_move_history(controller.move_log());
                    break;
                }
                GameStatus::Ready => {
                    controller.bootstrap();
                    controller.ensure_clock_started();
                }
                GameStatus::AwaitingMove { side } => {
                    controller.ensure_clock_started();
                    let descriptor = controller.config().player(side);
                    if descriptor.is_human() {
                        Self::handle_human_turn(
                            controller,
                            side,
                            log_handle,
                            &rx,
                            &mut pending_input,
                        );
                    } else {
                        Self::handle_cpu_turn(
                            controller,
                            side,
                            debug_mode,
                            log_handle,
                            &rx,
                            &mut pending_input,
                        );
                    }
                }
            }
        }
    }

    fn handle_human_turn(
        controller: &mut GameController,
        side: PlayerSide,
        log_handle: &Option<Arc<Mutex<File>>>,
        rx: &Receiver<String>,
        pending_input: &mut VecDeque<String>,
    ) {
        let legal_moves = controller.legal_moves();
        if legal_moves.is_empty() {
            // Should already be handled by controller state, but guard just in case.
            println!("{} has no legal moves.", side.label());
            return;
        }
        println!(
            "{} to move. Enter a move like 7776 (+ for promotion) or P*77 for drops (legacy letter ranks still accepted).",
            side.label()
        );
        loop {
            match Self::next_line(rx, pending_input, "> ") {
                Some(line) if line.is_empty() => continue,
                Some(line) => match Self::parse_command(&line) {
                    None => continue,
                    Some(UserCommand::Help) => {
                        Self::print_help();
                    }
                    Some(UserCommand::ListMoves) => {
                        Self::print_move_list(legal_moves.as_slice());
                    }
                    Some(UserCommand::Resign) => {
                        println!("{} resigns.", side.label());
                        controller.resign(side);
                        return;
                    }
                    Some(UserCommand::ForceResign(resign_side)) => {
                        controller.resign(resign_side);
                        return;
                    }
                    Some(UserCommand::Move(spec)) => {
                        if let Some(chosen) = Self::select_move(&spec, legal_moves.as_slice()) {
                            if Self::apply_move(controller, side, chosen, log_handle) {
                                return;
                            }
                        } else {
                            println!("Move not legal in this position.");
                        }
                    }
                    Some(UserCommand::Unknown(text)) => {
                        println!("Unrecognized command: {}", text);
                    }
                },
                None => {
                    println!("Input stream closed. {} resigns.", side.label());
                    controller.resign(side);
                    return;
                }
            }
        }
    }

    fn handle_cpu_turn(
        controller: &mut GameController,
        side: PlayerSide,
        debug_mode: bool,
        log_handle: &Option<Arc<Mutex<File>>>,
        rx: &Receiver<String>,
        pending_input: &mut VecDeque<String>,
    ) {
        println!("{} (CPU) thinking...", side.label());
        if Self::process_global_commands(controller, rx, pending_input) {
            return;
        }
        let legal_moves = controller.legal_moves();
        if legal_moves.is_empty() {
            println!("{} has no legal moves and resigns.", side.label());
            controller.resign(side);
            return;
        }
        println!("Requesting move...");
        let search_start = std::time::Instant::now();
        if let Some((outcome, limit)) = controller.request_move() {
            if debug_mode {
                let elapsed = search_start.elapsed().as_secs_f32();
                let secs = limit.as_secs_f32();
                let reason = match outcome.stop_reason {
                    StopReason::Confident => "confident",
                    StopReason::TimeUp => "time up",
                };
                match outcome.best_move {
                    Some(mv) => println!(
                        "[DEBUG] {} best move {} score {} depth {} nodes {} ({:.2}s / {:.2}s limit, {})",
                        side.label(),
                        Self::format_move(mv),
                        outcome.score,
                        outcome.depth,
                        outcome.nodes,
                        elapsed,
                        secs,
                        reason,
                    ),
                    None => println!(
                        "[DEBUG] {} search found no move ({:.2}s / {:.2}s limit, nodes {})",
                        side.label(),
                        elapsed,
                        secs,
                        outcome.nodes
                    ),
                }
            }
            if let Some(mv) = outcome.best_move {
                let _ = Self::apply_move(controller, side, mv, log_handle);
            } else {
                println!("{} cannot find a move and resigns.", side.label());
                controller.resign(side);
            }
        } else {
            println!("CPU has no move generator configured; resigning.");
            controller.resign(side);
        }
    }

    fn apply_move(
        controller: &mut GameController,
        side: PlayerSide,
        mv: Move,
        log_handle: &Option<Arc<Mutex<File>>>,
    ) -> bool {
        match controller.apply_move(mv) {
            Ok(AdvanceState::Ongoing) => {
                println!("{} plays {}", side.label(), Self::format_move(mv));
                Self::log_player_move(log_handle, controller.time_manager(), side, mv);
                true
            }
            Ok(AdvanceState::Completed(_)) => {
                println!("{} plays {}", side.label(), Self::format_move(mv));
                Self::log_player_move(log_handle, controller.time_manager(), side, mv);
                true
            }
            Err(MoveError::IllegalMove) => {
                println!("Illegal move, please try again.");
                false
            }
            Err(MoveError::GameAlreadyFinished) => true,
            Err(MoveError::Timeout { winner }) => {
                println!("{} loses on time. {} wins.", side.label(), winner.label());
                true
            }
        }
    }

    fn configure_game() -> (GameConfig, bool) {
        println!("Select game mode:");
        println!("  [1] Player vs CPU");
        println!("  [2] CPU vs CPU");
        let mode_choice = input::read_selection("Mode (default 2): ", 2, 2);
        let mode = if mode_choice == 1 {
            GameMode::PlayerVsCpu
        } else {
            GameMode::CpuVsCpu
        };

        let time_control = Self::prompt_time_control();
        let think_secs = Self::prompt_think_time();
        let think_time = Duration::from_secs(think_secs);
        let sente = match mode {
            GameMode::PlayerVsCpu => PlayerDescriptor::new(PlayerSide::Sente, PlayerKind::Human),
            GameMode::CpuVsCpu => PlayerDescriptor::new(
                PlayerSide::Sente,
                PlayerKind::Cpu {
                    strength: Self::prompt_strength("Sente"),
                },
            ),
        };
        let gote_strength_label = match mode {
            GameMode::PlayerVsCpu => "Gote (CPU)",
            GameMode::CpuVsCpu => "Gote",
        };
        let gote = PlayerDescriptor::new(
            PlayerSide::Gote,
            PlayerKind::Cpu {
                strength: Self::prompt_strength(gote_strength_label),
            },
        );

        let debug_mode = Self::ask_debug_mode();

        (
            GameConfig::new(mode, sente, gote, time_control, think_time, debug_mode),
            debug_mode,
        )
    }

    fn announce(config: &GameConfig) {
        println!("\nConfiguration summary:");
        println!("  Mode: {:?}", config.mode);
        println!("  Sente: {}", config.sente.label());
        println!("  Gote: {}", config.gote.label());
        println!(
            "  Time control: Sente {} min / Gote {} min + {} sec byoyomi",
            config.time_control.main_time(PlayerSide::Sente).as_secs() / 60,
            config.time_control.main_time(PlayerSide::Gote).as_secs() / 60,
            config.time_control.byoyomi.as_secs()
        );
        println!(
            "  Debug mode: {}",
            if config.debug_mode() {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "Commands: move = <from><to>[+], drop = <piece>*<square>, 'moves' to list, 'help' for help, 'resign' to resign."
        );
        println!("Type '/resign' (or ':resign') at any time to immediately resign as Sente.");
    }

    fn print_banner() {
        println!("================ Shogi CLI ================");
    }

    fn print_clock_info(controller: &GameController) {
        for &side in &PlayerSide::ALL {
            let (main, byoyomi) = controller.time_manager().remaining(side);
            let phase = if controller.time_manager().in_byoyomi(side) {
                "byoyomi"
            } else {
                "main"
            };
            println!(
                "{} time: {} ({} sec byoyomi, currently in {})",
                side.label(),
                Self::format_duration(main),
                byoyomi.as_secs(),
                phase
            );
        }
    }

    fn format_duration(duration: Duration) -> String {
        let total = duration.as_secs();
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{:02}:{:02}", minutes, seconds)
    }

    fn print_result(result: &GameResult) {
        match result {
            GameResult::Resignation { winner } => {
                println!("Game over: {} wins by resignation.", winner.label());
            }
            GameResult::Checkmate { winner } => {
                println!("Game over: {} wins by checkmate.", winner.label());
            }
            GameResult::Repetition => println!("Game over: draw by repetition (sennichite)."),
            GameResult::Timeout { winner } => {
                println!("Game over: {} wins on time.", winner.label());
            }
        }
    }

    /// Prints the full move list in traditional shogi-style notation using
    /// Romanised Japanese piece names (Fu, Kyou, Kei, Gin, Kin, Kaku, Hi, Ou;
    /// promoted names: To, NariKyou, NariKei, NariGin, Uma, Ryu).
    ///
    /// Format per line: `<turn>. <Side> <dest><piece>[+]([from])`
    ///   Normal move:    "  1. Sente 76Fu(77)"
    ///   Promotion:      " 25. Sente 22Kaku+(88)"
    ///   Moving promoted: " 40. Gote  33Ryu(23)"
    ///   Drop:           " 10. Sente 82Kaku()"
    ///
    /// To render "Kaku vs Uma" (base vs promoted) correctly for pieces that
    /// were already promoted *before* the move, we replay the game from the
    /// starting position and consult the board's `promoted` bitboard at each
    /// source square.
    fn print_move_history(history: &[Move]) {
        if history.is_empty() {
            println!("No moves were played.");
            return;
        }
        println!("\nFull move list:");
        let mut board = Board::new_standard();
        for (idx, mv) in history.iter().enumerate() {
            let turn = idx + 1;
            let player = if mv.player == PlayerSide::Sente {
                "Sente"
            } else {
                "Gote"
            };
            // Detect whether the moving piece was already promoted before this
            // move was applied (only meaningful for non-drop moves).
            let was_promoted = match mv.from {
                Some(from) => board
                    .bitboards()
                    .promoted(mv.player, mv.piece)
                    .is_set(from),
                None => false, // drops are always unpromoted
            };
            println!(
                "{:>3}. {:<5} {}",
                turn,
                player,
                Self::format_move_japanese(*mv, was_promoted)
            );
            board.apply_move(*mv);
        }
    }

    /// Formats a single move in traditional shogi notation with Japanese
    /// piece names.  See [`print_move_history`] for the format spec.
    fn format_move_japanese(mv: Move, was_promoted: bool) -> String {
        let dest = mv.to.to_string(); // Square Display → "76" format
        let piece = Self::piece_name_japanese(mv.piece, was_promoted);
        match mv.kind {
            MoveKind::Drop => format!("{}{}()", dest, piece),
            _ => {
                let from = mv
                    .from
                    .map(|sq| sq.to_string())
                    .unwrap_or_default();
                // `mv.promote` means the piece is *promoting on this move*.
                // We show "+" after the (still-unpromoted) piece name to
                // indicate the promotion decision, matching notation like
                // "76Fu+(77)".
                let promotion = if mv.promote { "+" } else { "" };
                format!("{}{}{}({})", dest, piece, promotion, from)
            }
        }
    }

    /// Returns the Romanised Japanese name for a piece.  If `promoted` is
    /// true, returns the promoted form (e.g. "Ryu" for promoted Rook).
    fn piece_name_japanese(kind: PieceKind, promoted: bool) -> &'static str {
        match (kind, promoted) {
            (PieceKind::King, _) => "Ou",
            (PieceKind::Rook, false) => "Hi",
            (PieceKind::Rook, true) => "Ryu",
            (PieceKind::Bishop, false) => "Kaku",
            (PieceKind::Bishop, true) => "Uma",
            (PieceKind::Gold, _) => "Kin",
            (PieceKind::Silver, false) => "Gin",
            (PieceKind::Silver, true) => "NariGin",
            (PieceKind::Knight, false) => "Kei",
            (PieceKind::Knight, true) => "NariKei",
            (PieceKind::Lance, false) => "Kyou",
            (PieceKind::Lance, true) => "NariKyou",
            (PieceKind::Pawn, false) => "Fu",
            (PieceKind::Pawn, true) => "To",
        }
    }

    fn print_help() {
        println!("Move input examples (rows accept 1-9 or legacy a-i):");
        println!("  - 7776    : move piece from 77 to 76 (7g7f also accepted)");
        println!("  - 28 88+  : move from 28 to 88 with promotion (input as 2888+)");
        println!("  - P*57    : drop a pawn on 57 (P*5e also accepted)");
        println!(
            "Commands: 'moves' lists legal moves, 'resign' resigns, 'help' shows this message, '/resign' forces an immediate Sente resignation."
        );
    }

    fn print_move_list(moves: &[Move]) {
        let mut encoded: Vec<String> = moves.iter().map(|mv| Self::format_move(*mv)).collect();
        encoded.sort();
        println!("Legal moves ({}):", encoded.len());
        for chunk in encoded.chunks(8) {
            println!("  {}", chunk.join("  "));
        }
    }

    fn parse_command(input: &str) -> Option<UserCommand> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let lowered = trimmed.to_ascii_lowercase();
        if Self::is_sente_resign_command(&lowered) {
            return Some(UserCommand::ForceResign(PlayerSide::Sente));
        }
        if lowered == "resign" {
            return Some(UserCommand::Resign);
        }
        if lowered == "help" {
            return Some(UserCommand::Help);
        }
        if lowered == "moves" {
            return Some(UserCommand::ListMoves);
        }
        if let Some(spec) = Self::parse_move_spec(trimmed) {
            return Some(UserCommand::Move(spec));
        }
        Some(UserCommand::Unknown(trimmed.to_string()))
    }

    fn parse_move_spec(token: &str) -> Option<ParsedMove> {
        if let Some((piece_part, rest)) = token.split_once('*') {
            let piece_char = piece_part.chars().next()?.to_ascii_uppercase();
            let kind = Self::piece_from_char(piece_char)?;
            let destination = Square::from_text(rest.trim())?;
            return Some(ParsedMove {
                from: None,
                to: destination,
                drop: Some(kind),
                promote: Some(false),
            });
        }
        let mut text = token.trim();
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
        Some(ParsedMove {
            from: Some(from),
            to,
            drop: None,
            promote,
        })
    }

    fn select_move(spec: &ParsedMove, moves: &[Move]) -> Option<Move> {
        let mut candidates: Vec<Move> = moves
            .iter()
            .copied()
            .filter(|mv| match spec.drop {
                Some(kind) => mv.kind == MoveKind::Drop && mv.piece == kind && mv.to == spec.to,
                None => mv.from == spec.from && mv.to == spec.to,
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
            && let Some(non_promo) = candidates.iter().find(|mv| !mv.promote) {
                return Some(*non_promo);
            }
        candidates.into_iter().next()
    }

    fn piece_from_char(ch: char) -> Option<PieceKind> {
        PieceKind::from_char(ch)
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

    fn ask_debug_mode() -> bool {
        println!("Enable debug mode (show CPU thinking)? [y/N]");
        match input::read_line("Debug mode: ") {
            Ok(ans) => matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
            Err(_) => false,
        }
    }

    fn prompt_think_time() -> u64 {
        println!("\nCPU think time per move in seconds (1-60, default 5):");
        let choice = input::read_selection("Think time: ", 60, 5);
        choice.max(1) as u64
    }

    /// Interactively prompts the user for the time control: Sente's main time
    /// in minutes, Gote's main time in minutes, and the byoyomi period in
    /// seconds.  Empty or invalid input falls back to the default.
    fn prompt_time_control() -> TimeControl {
        println!("\nTime control — press Enter to accept the default in parentheses.");
        let sente_min = Self::prompt_u64("Sente main time in minutes", 10);
        let gote_min = Self::prompt_u64("Gote main time in minutes", 10);
        let byoyomi_sec = Self::prompt_u64("Byoyomi per move in seconds", 10);
        TimeControl::with_per_side(
            Duration::from_secs(sente_min * 60),
            Duration::from_secs(gote_min * 60),
            Duration::from_secs(byoyomi_sec),
        )
    }

    /// Reads a non-negative integer from stdin with a default fallback.
    /// Used by `prompt_time_control` where values of 0 should be accepted
    /// (e.g. a byoyomi-only game with 0 main time).
    fn prompt_u64(label: &str, default: u64) -> u64 {
        let prompt = format!("{} (default {}): ", label, default);
        match input::read_line(&prompt) {
            Ok(ref s) if s.is_empty() => default,
            Ok(s) => s.parse().unwrap_or(default),
            Err(_) => default,
        }
    }

    fn prompt_strength(label: &str) -> SearchStrength {
        println!("\nSelect {} strength:", label);
        println!("  [1] Weak\n  [2] Normal\n  [3] Strong");
        let input_label = format!("{} strength (default 2): ", label);
        let choice = input::read_selection(&input_label, 3, 2);
        match choice {
            1 => SearchStrength::Weak,
            3 => SearchStrength::Strong,
            _ => SearchStrength::Normal,
        }
    }

    fn log_player_move(
        log_handle: &Option<Arc<Mutex<File>>>,
        times: &TimeManager,
        side: PlayerSide,
        mv: Move,
    ) {
        if let Some(handle) = log_handle
            && let Ok(mut file) = handle.lock() {
                let (s_main, s_byoyomi) = times.remaining(PlayerSide::Sente);
                let (g_main, g_byoyomi) = times.remaining(PlayerSide::Gote);
                let _ = writeln!(
                    file,
                    "PLAY {} {} | Sente {} +{}s | Gote {} +{}s",
                    side.label(),
                    Self::format_move(mv),
                    Self::format_duration(s_main),
                    s_byoyomi.as_secs(),
                    Self::format_duration(g_main),
                    g_byoyomi.as_secs()
                );
            }
    }

    fn process_global_commands(
        controller: &mut GameController,
        rx: &Receiver<String>,
        pending: &mut VecDeque<String>,
    ) -> bool {
        loop {
            match rx.try_recv() {
                Ok(raw) => {
                    let trimmed = raw.trim().to_ascii_lowercase();
                    if Self::is_sente_resign_command(&trimmed) {
                        println!("Sente resigns by request.");
                        controller.resign(PlayerSide::Sente);
                        return true;
                    }
                    if !trimmed.is_empty() {
                        pending.push_back(raw);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        false
    }

    fn next_line(
        rx: &Receiver<String>,
        pending: &mut VecDeque<String>,
        prompt: &str,
    ) -> Option<String> {
        if let Some(buffered) = pending.pop_front() {
            return Some(buffered.trim().to_string());
        }
        print!("{}", prompt);
        io::stdout().flush().ok();
        rx.recv().ok().map(|line| line.trim().to_string())
    }

    fn is_sente_resign_command(input: &str) -> bool {
        matches!(
            input,
            "/resign"
                | ":resign"
                | "/rs"
                | ":rs"
                | "resign sente"
                | "sente resign"
                | "/resign sente"
        )
    }
}

#[derive(Clone, Debug)]
enum UserCommand {
    Move(ParsedMove),
    Resign,
    ForceResign(PlayerSide),
    Help,
    ListMoves,
    Unknown(String),
}

#[derive(Clone, Debug)]
struct ParsedMove {
    from: Option<Square>,
    to: Square,
    drop: Option<PieceKind>,
    promote: Option<bool>,
}
