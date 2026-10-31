//! Scriptable monitors.

use crate::error::PlatformError;
use crate::monitors::{MonitorInfo, Monitors, Point};

pub struct FakeMonitors {
    cursor: Point,
    monitors: Vec<MonitorInfo>,
}

impl FakeMonitors {
    pub fn new(cursor: Point, monitors: Vec<MonitorInfo>) -> Self {
        Self { cursor, monitors }
    }
}

impl Monitors for FakeMonitors {
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        Ok(self.cursor)
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        Ok(self.monitors.clone())
    }
}
