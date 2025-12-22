use std::io::{self, Write};

pub fn read_line(prompt: &str) -> io::Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

pub fn read_selection(prompt: &str, max_option: usize, default: usize) -> usize {
    match read_line(prompt) {
        Ok(input) => {
            if input.is_empty() {
                return default;
            }
            if let Ok(value) = input.parse::<usize>() {
                if value >= 1 && value <= max_option {
                    return value;
                }
            }
            default
        }
        Err(_) => default,
    }
}
