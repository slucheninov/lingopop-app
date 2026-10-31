use std::sync::Mutex;
use std::time::Duration;

use crate::sleeper::Sleeper;

#[derive(Debug, Default)]
struct State {
    now: Duration,
    slept: Vec<Duration>,
}

/// Virtual clock: `sleep` advances `now` without waiting and records the duration.
#[derive(Debug, Default)]
pub struct FakeSleeper {
    state: Mutex<State>,
}

impl FakeSleeper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every `sleep` call so far, in order.
    pub fn slept(&self) -> Vec<Duration> {
        self.lock().slept.clone()
    }

    /// Moves the clock forward without recording a sleep.
    pub fn advance(&self, d: Duration) {
        self.lock().now += d;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Sleeper for FakeSleeper {
    fn now(&self) -> Duration {
        self.lock().now
    }

    fn sleep(&self, d: Duration) {
        let mut s = self.lock();
        s.now += d;
        s.slept.push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fake_sleeper_advances_virtual_time_without_waiting() {
        let s = FakeSleeper::new();
        let wall = Instant::now();
        s.sleep(Duration::from_millis(300));
        assert_eq!(s.now(), Duration::from_millis(300));
        assert!(wall.elapsed() < Duration::from_millis(50));
        s.advance(Duration::from_millis(10));
        assert_eq!(s.now(), Duration::from_millis(310));
        assert_eq!(s.slept(), vec![Duration::from_millis(300)]);
    }

    #[test]
    fn fake_sleeper_records_every_sleep_in_order() {
        let s = FakeSleeper::new();
        for ms in [5, 1, 20] {
            s.sleep(Duration::from_millis(ms));
        }
        assert_eq!(s.slept(), [5, 1, 20].map(Duration::from_millis).to_vec());
    }
}
