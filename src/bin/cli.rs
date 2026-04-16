// Binary entry point for the interactive CLI game.
// Delegates entirely to `AppCli::run()` which handles setup, the game loop,
// human move input, and CPU move requests.
use shogi_codex::AppCli;

fn main() {
    AppCli::run();
}
