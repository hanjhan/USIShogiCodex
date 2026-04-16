// Binary entry point for the GUI harness.
//
// This binary acts as a simple GUI driver that communicates with the engine
// over the USI protocol via subprocess stdin/stdout.  It is useful for
// running automated self-play or testing the USI implementation without a
// real GUI.
//
// Behaviour:
//  - If launched with "--engine", runs the USI engine directly (this is how
//    the subprocess is started).
//  - Otherwise, spawns itself as a child process with "--engine", then drives
//    a single game by sending USI commands and reading "bestmove" responses.
//
// The game loop runs for at most 100 turns.  If the engine resigns or becomes
// unresponsive, the loop exits early.

use shogi_codex::usi;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    // When launched as the engine subprocess, just run the USI loop.
    if std::env::args().any(|arg| arg == "--engine") {
        usi::run();
        return Ok(());
    }

    // GUI mode: spawn this binary again as the engine subprocess.
    let engine_path = std::env::current_exe()?;
    let mut child = Command::new(engine_path)
        .arg("--engine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut engine_stdin = child.stdin.take().expect("engine stdin");
    let stdout = child.stdout.take().expect("engine stdout");

    // Spawn a thread to read engine stdout lines and forward them via channel
    // so we can apply a timeout without blocking.
    let (line_tx, line_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = line_tx.send(line.trim().to_string());
                }
                Err(_) => break,
            }
        }
    });

    // USI handshake: "usi" → wait for "usiok"
    send_command(&mut engine_stdin, "usi")?;
    read_until_token(&line_rx, "usiok", Duration::from_secs(5))?;

    // Ready check: "isready" → wait for "readyok"
    send_command(&mut engine_stdin, "isready")?;
    read_until_token(&line_rx, "readyok", Duration::from_secs(5))?;

    // Play the game: send position + go each turn and collect bestmove responses.
    let mut moves_played: Vec<String> = Vec::new();
    let mut engine_alive = true;
    let mut reason = String::from("Normal termination");

    'game: for turn in 0..100 {
        // Build the position command from all moves played so far.
        let moves_string = if moves_played.is_empty() {
            String::new()
        } else {
            format!(" moves {}", moves_played.join(" "))
        };
        send_command(
            &mut engine_stdin,
            &format!("position startpos{}", moves_string),
        )?;
        send_command(&mut engine_stdin, "go")?;

        // Wait up to 31 seconds for a bestmove response.
        let best = match read_bestmove_with_timeout(&line_rx, Duration::from_secs(31))? {
            Some(mv) => mv,
            None => {
                // Timeout: send "stop" and wait for the engine to respond.
                println!("GUI: no bestmove after 31s, sending stop");
                send_command(&mut engine_stdin, "stop")?;
                match read_bestmove_with_timeout(&line_rx, Duration::from_secs(5))? {
                    Some(mv) => mv,
                    None => {
                        println!("GUI: engine unresponsive, killing process");
                        let _ = child.kill();
                        engine_alive = false;
                        reason = "Engine unresponsive".to_string();
                        break 'game;
                    }
                }
            }
        };

        println!("Turn {} bestmove {}", turn + 1, best);
        if best == "resign" {
            reason = "Engine resigned".to_string();
            break;
        }
        moves_played.push(best);
        std::thread::sleep(Duration::from_millis(100));
        if turn == 99 {
            reason = "Reached iteration limit".to_string();
        }
    }

    // Clean up the engine process.
    if engine_alive {
        send_command(&mut engine_stdin, "quit")?;
        let status = child.wait()?;
        if !status.success() {
            reason = format!("Engine terminated with status {:?}", status.code());
        }
    } else {
        let _ = child.wait();
    }
    println!("GUI: game ended ({})", reason);
    Ok(())
}

/// Writes `cmd` followed by a newline to `writer` and flushes.
fn send_command(writer: &mut dyn Write, cmd: &str) -> Result<(), Box<dyn Error>> {
    writeln!(writer, "{}", cmd)?;
    writer.flush()?;
    println!("GUI -> {}", cmd);
    Ok(())
}

/// Reads lines from `rx` until one starts with `token` or the deadline passes.
fn read_until_token(
    rx: &Receiver<String>,
    token: &str,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok(line) => {
                println!("ENGINE -> {}", line);
                if line.starts_with(token) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

/// Reads lines until a "bestmove <move>" line is found or the timeout expires.
/// Returns `Some(move_string)` on success, `None` on timeout.
fn read_bestmove_with_timeout(
    rx: &Receiver<String>,
    timeout: Duration,
) -> Result<Option<String>, Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        match rx.recv_timeout(deadline - now) {
            Ok(line) => {
                println!("ENGINE -> {}", line);
                if let Some(rest) = line.strip_prefix("bestmove ") {
                    return Ok(Some(rest.to_string()));
                }
            }
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}
