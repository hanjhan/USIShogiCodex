use shogi_codex::usi;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().any(|arg| arg == "--engine") {
        usi::run();
        return Ok(());
    }

    let engine_path = std::env::current_exe()?;
    let mut child = Command::new(engine_path)
        .arg("--engine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut engine_stdin = child.stdin.take().expect("engine stdin");
    let stdout = child.stdout.take().expect("engine stdout");
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

    send_command(&mut engine_stdin, "usi")?;
    read_until_token(&line_rx, "usiok", Duration::from_secs(5))?;

    send_command(&mut engine_stdin, "isready")?;
    read_until_token(&line_rx, "readyok", Duration::from_secs(5))?;

    let mut moves_played: Vec<String> = Vec::new();
    let mut engine_alive = true;
    let mut reason = String::from("Normal termination");
    'game: for turn in 0..100 {
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
        let best = match read_bestmove_with_timeout(&line_rx, Duration::from_secs(31))? {
            Some(mv) => mv,
            None => {
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

fn send_command(writer: &mut dyn Write, cmd: &str) -> Result<(), Box<dyn Error>> {
    writeln!(writer, "{}", cmd)?;
    writer.flush()?;
    println!("GUI -> {}", cmd);
    Ok(())
}

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
