//! Dev stand-in: the frontmost application is the "last external" window.

use crate::foreground::{ForegroundWindow, WindowHandle};
use crate::last_foreground::LastForeground;
use crate::macos::MacForeground;

#[derive(Debug, Default, Clone, Copy)]
pub struct MacLastForeground;

impl MacLastForeground {
    pub fn new() -> Self {
        Self
    }
}

impl LastForeground for MacLastForeground {
    fn last_external(&self) -> Option<WindowHandle> {
        MacForeground::new().current()
    }
}
