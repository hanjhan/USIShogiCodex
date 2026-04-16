// Game layer — sits between the pure engine and the front-end binaries.
//
// Sub-modules:
//   config     — GameConfig: mode, player descriptors, time control, debug flag
//   player     — PlayerDescriptor / PlayerKind (Human or CPU with strength)
//   timer      — TimeControl, TimeManager, PlayerClock (main time + byoyomi)
//   controller — GameController: state machine, move application, CPU requests

pub mod config;
pub mod controller;
pub mod player;
pub mod timer;

pub use config::{GameConfig, GameMode};
pub use controller::{AdvanceState, GameController, GameResult, GameStatus, MoveError};
pub use player::{PlayerDescriptor, PlayerKind};
pub use timer::{TimeControl, TimeManager};
