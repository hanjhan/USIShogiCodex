use std::io::{self, Write};

/// Prints `prompt` and reads one trimmed line from stdin.
/// Returns an `io::Error` if stdin is closed or a read error occurs.
pub fn read_line(prompt: &str) -> io::Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Prints `prompt`, reads a line, and interprets it as a menu selection.
///
/// - Valid input: a number in [1, max_option].  Returns that number.
/// - Empty input or out-of-range value: returns `default`.
/// - Read error: returns `default`.
pub fn read_selection(prompt: &str, max_option: usize, default: usize) -> usize {
    match read_line(prompt) {
        Ok(input) => {
            if input.is_empty() {
                return default;
            }
            if let Ok(value) = input.parse::<usize>()
                && value >= 1 && value <= max_option {
                    return value;
                }
            default
        }
        Err(_) => default,
    }
}
