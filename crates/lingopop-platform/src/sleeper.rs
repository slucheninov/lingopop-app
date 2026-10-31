//! Time seam: lets timing-sensitive flows run on a virtual clock in tests.

use std::time::{Duration, Instant};

use crate::error::PlatformError;

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

/// Runs `op` up to `attempts` times, retrying only on `ClipboardBusy` and sleeping `delay` between
/// attempts (not after the last). Any other result is returned at once.
pub fn retry_busy<T>(
    attempts: u32,
    delay: Duration,
    sleeper: &dyn Sleeper,
    mut op: impl FnMut() -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let attempts = attempts.max(1);
    let mut attempt = 1;
    loop {
        match op() {
            Err(PlatformError::ClipboardBusy) if attempt < attempts => {
                sleeper.sleep(delay);
                attempt += 1;
            }
            other => return other,
        }
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
