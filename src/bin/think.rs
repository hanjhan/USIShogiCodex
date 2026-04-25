// Binary entry point for the interactive thinking-mode analyser.
// Delegates to `think::run()`, which prompts for a starting position and
// then loops between background search and user input.
use shogi_codex::think;

fn main() {
    think::run();
}
