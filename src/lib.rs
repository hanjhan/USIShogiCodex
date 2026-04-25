// shogi-codex library crate
//
// Top-level modules:
//   engine — pure game logic (bitboards, move generation, search, evaluation)
//   game   — game-layer coordination (controller, config, clocks, player types)
//   cli    — terminal front-end (interactive loop, board rendering, input)
//   usi    — USI protocol engine (stdin/stdout command handler for GUIs)
//
// Public re-exports used by the binary crates:
//   AppCli       — entry point for `cargo run --bin cli`
//   GameConfig   — full game configuration (mode, players, time control)
//   GameController — state machine that drives a single game

pub mod cli;
pub mod engine;
pub mod game;
pub mod think;
pub mod usi;

pub use cli::AppCli;
pub use game::{config::GameConfig, controller::GameController};
