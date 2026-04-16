// CLI front-end for the shogi engine.
//
// Sub-modules:
//   app          — `AppCli`: top-level game setup, main loop, human/CPU turn handling
//   board_render — `BoardRenderer`: ASCII board display and USI SFEN string generation
//   input        — stdin helpers: `read_line`, `read_selection`

pub mod app;
pub mod board_render;
pub mod input;

pub use app::AppCli;
pub use board_render::BoardRenderer;
