// Binary entry point for the USI engine.
// Runs `usi::run()` which reads USI commands from stdin and writes responses
// to stdout, allowing any USI-compatible GUI (e.g. Shogidroid, ShogiGUI) to
// use this engine.
use shogi_codex::usi;

fn main() {
    usi::run();
}
