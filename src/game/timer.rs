use std::time::{Duration, Instant};

use crate::engine::state::PlayerSide;

#[derive(Clone, Copy, Debug)]
pub struct TimeControl {
    pub main_time: Duration,
    pub byoyomi: Duration,
}

impl TimeControl {
    pub fn new(main_time: Duration, byoyomi: Duration) -> Self {
        Self { main_time, byoyomi }
    }
}

impl Default for TimeControl {
    fn default() -> Self {
        Self {
            main_time: Duration::from_secs(10 * 60),
            byoyomi: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlayerClock {
    pub remaining_main: Duration,
    last_start: Option<Instant>,
}

impl PlayerClock {
    fn new(control: TimeControl) -> Self {
        Self {
            remaining_main: control.main_time,
            last_start: None,
        }
    }
}

pub struct TimeManager {
    control: TimeControl,
    clocks: [PlayerClock; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeStatus {
    Ok,
    Flagged,
}

impl TimeManager {
    pub fn new(control: TimeControl) -> Self {
        Self {
            clocks: [PlayerClock::new(control), PlayerClock::new(control)],
            control,
        }
    }

    pub fn start_turn(&mut self, side: PlayerSide) {
        self.clocks[side.index()].last_start = Some(Instant::now());
    }

    pub fn stop_turn(&mut self, side: PlayerSide) -> TimeStatus {
        if let Some(started) = self.clocks[side.index()].last_start.take() {
            let elapsed = started.elapsed();
            let clock = &mut self.clocks[side.index()];
            if elapsed >= clock.remaining_main {
                let overflow = elapsed - clock.remaining_main;
                clock.remaining_main = Duration::from_secs(0);
                if overflow > self.control.byoyomi {
                    return TimeStatus::Flagged;
                }
            } else {
                clock.remaining_main -= elapsed;
            }
        }
        TimeStatus::Ok
    }

    pub fn remaining(&self, side: PlayerSide) -> (Duration, Duration) {
        let clock = &self.clocks[side.index()];
        (clock.remaining_main, self.control.byoyomi)
    }

    pub fn in_byoyomi(&self, side: PlayerSide) -> bool {
        self.clocks[side.index()].remaining_main.is_zero()
    }

    pub fn control(&self) -> TimeControl {
        self.control
    }
}
