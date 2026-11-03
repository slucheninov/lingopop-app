//! Scriptable last-foreground tracker.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::foreground::WindowHandle;
use crate::last_foreground::LastForeground;

pub struct FakeLastForeground {
    window: Mutex<Option<WindowHandle>>,
    calls: AtomicUsize,
}

impl FakeLastForeground {
    pub fn new(window: Option<WindowHandle>) -> Arc<Self> {
        Arc::new(Self {
            window: Mutex::new(window),
            calls: AtomicUsize::new(0),
        })
    }

    pub fn set(&self, window: Option<WindowHandle>) {
        *self.window.lock().unwrap_or_else(|p| p.into_inner()) = window;
    }

    /// How many times `last_external` was called.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LastForeground for FakeLastForeground {
    fn last_external(&self) -> Option<WindowHandle> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.window.lock().unwrap_or_else(|p| p.into_inner())
    }
}
