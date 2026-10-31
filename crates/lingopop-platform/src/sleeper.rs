//! Time seam: lets timing-sensitive flows run on a virtual clock in tests.

use std::time::{Duration, Instant};

pub trait Sleeper: Send + Sync {
    /// Monotonic time since an arbitrary start.
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

/// Real clock: `Instant` and `thread::sleep`.
#[derive(Debug, Clone, Copy)]
pub struct SystemSleeper {
    start: Instant,
}

impl SystemSleeper {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemSleeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Sleeper for SystemSleeper {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_sleeper_now_is_monotonic() {
        let s = SystemSleeper::new();
        let a = s.now();
        s.sleep(Duration::from_millis(2));
        let b = s.now();
        assert!(b >= a + Duration::from_millis(2));
    }
}
