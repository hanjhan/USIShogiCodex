use std::time::{Duration, Instant};

use crate::engine::state::PlayerSide;

// Shogi time control: each player starts with a "main time" pool.  When main
// time runs out, they enter "byoyomi" — they must complete each move within
// the byoyomi period, but there is no accumulation (it resets each move).
//
// `TimeManager` tracks both players' clocks.  The game controller calls:
//   start_turn(side)  — records the start time for side's current move
//   stop_turn(side)   — calculates elapsed time, deducts from main time, and
//                       returns Flagged if both main time and byoyomi are
//                       exceeded.

/// Time-control settings for a single game.  Main time is tracked per side
/// (so Sente and Gote can start with different amounts of thinking time),
/// while byoyomi is a shared per-move period by convention.
#[derive(Clone, Copy, Debug)]
pub struct TimeControl {
    /// Starting main-time pool for each side, indexed by `PlayerSide::index()`.
    main_time: [Duration; 2],
    /// Byoyomi period per move once main time is exhausted.
    pub byoyomi: Duration,
}

impl TimeControl {
    /// Creates a `TimeControl` with the same main time for both players.
    pub fn new(main_time: Duration, byoyomi: Duration) -> Self {
        Self {
            main_time: [main_time, main_time],
            byoyomi,
        }
    }

    /// Creates a `TimeControl` with a possibly-different main time per side.
    pub fn with_per_side(
        sente_main: Duration,
        gote_main: Duration,
        byoyomi: Duration,
    ) -> Self {
        Self {
            main_time: [sente_main, gote_main],
            byoyomi,
        }
    }

    /// Returns the starting main-time pool for `side`.
    pub fn main_time(&self, side: PlayerSide) -> Duration {
        self.main_time[side.index()]
    }
}

/// Default: 10 minutes main time for both sides + 10 seconds byoyomi.
impl Default for TimeControl {
    fn default() -> Self {
        let main = Duration::from_secs(10 * 60);
        Self {
            main_time: [main, main],
            byoyomi: Duration::from_secs(10),
        }
    }
}

/// Clock state for a single player.
#[derive(Clone, Debug)]
pub struct PlayerClock {
    /// How much main time the player has left.  Zero means they are in byoyomi.
    pub remaining_main: Duration,
    /// The `Instant` when the player's current turn started, or None if not
    /// currently their turn.
    last_start: Option<Instant>,
}

impl PlayerClock {
    fn new(main_time: Duration) -> Self {
        Self {
            remaining_main: main_time,
            last_start: None,
        }
    }
}

/// Manages the clocks for both players.
pub struct TimeManager {
    control: TimeControl,
    clocks: [PlayerClock; 2],
}

/// Result of stopping a player's clock at the end of their move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeStatus {
    /// Move was completed within the time limit.
    Ok,
    /// Player exceeded both main time and byoyomi — they lose on time.
    Flagged,
}

impl TimeManager {
    pub fn new(control: TimeControl) -> Self {
        Self {
            clocks: [
                PlayerClock::new(control.main_time(PlayerSide::Sente)),
                PlayerClock::new(control.main_time(PlayerSide::Gote)),
            ],
            control,
        }
    }

    /// Records the start of `side`'s turn.  Called before waiting for their move.
    pub fn start_turn(&mut self, side: PlayerSide) {
        self.clocks[side.index()].last_start = Some(Instant::now());
    }

    /// Stops `side`'s clock and deducts elapsed time.
    ///
    /// - If the player has main time remaining and elapsed < remaining_main,
    ///   just deduct from main time.
    /// - If elapsed >= remaining_main, the overflow is checked against byoyomi.
    ///   If overflow > byoyomi, the player has flagged (time out).
    pub fn stop_turn(&mut self, side: PlayerSide) -> TimeStatus {
        if let Some(started) = self.clocks[side.index()].last_start.take() {
            let elapsed = started.elapsed();
            let clock = &mut self.clocks[side.index()];
            if elapsed >= clock.remaining_main {
                // Main time is exhausted; check byoyomi
                let overflow = elapsed - clock.remaining_main;
                clock.remaining_main = Duration::from_secs(0);
                if overflow > self.control.byoyomi {
                    return TimeStatus::Flagged;
                }
                // Byoyomi survived — clock stays at zero, resets next turn
            } else {
                clock.remaining_main -= elapsed;
            }
        }
        TimeStatus::Ok
    }

    /// Returns `(main_time_remaining, byoyomi_period)` for `side`.
    /// Note: byoyomi is a fixed period (not per-player state), so it is
    /// the same value for both players.
    pub fn remaining(&self, side: PlayerSide) -> (Duration, Duration) {
        let clock = &self.clocks[side.index()];
        (clock.remaining_main, self.control.byoyomi)
    }

    /// Returns true if `side` has exhausted their main time and is now relying
    /// on byoyomi.
    pub fn in_byoyomi(&self, side: PlayerSide) -> bool {
        self.clocks[side.index()].remaining_main.is_zero()
    }

    pub fn control(&self) -> TimeControl {
        self.control
    }
}
