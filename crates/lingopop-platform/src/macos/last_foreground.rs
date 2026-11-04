//! The last external application: what a tray click acts on.
//!
//! A status-item click usually leaves the foreground alone, but a click from our own Settings or
//! History window makes us the frontmost app. A small poller therefore samples the foreground and
//! remembers the latest application that is not ours; `last_external` also samples on demand, so
//! it is right even between polls. The handle is a `MacForeground` registry handle: build this
//! from the same `MacForeground` the platform bundle uses.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::foreground::{MacForeground, SystemApps};
use crate::foreground::WindowHandle;
use crate::last_foreground::LastForeground;
use crate::macos_targets::LastExternalTracker;

const POLL: Duration = Duration::from_millis(250);

pub struct MacLastForeground {
    tracker: Arc<LastExternalTracker<SystemApps>>,
    stop: Arc<AtomicBool>,
    poller: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for MacLastForeground {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MacLastForeground")
    }
}

impl MacLastForeground {
    /// Tracks over `foreground`'s registry. A poller thread failing to start only loses the
    /// between-poll memory; `last_external` still samples on demand.
    pub fn new(foreground: &MacForeground) -> Self {
        let tracker = Arc::new(LastExternalTracker::new(
            foreground.tracker(),
            std::process::id() as i32,
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let poller = {
            let (tracker, stop) = (tracker.clone(), stop.clone());
            thread::Builder::new()
                .name("lingopop-last-foreground".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        tracker.observe_light();
                        thread::park_timeout(POLL);
                    }
                })
                .map_err(|_| crate::error::platform_log("last-foreground poller unavailable"))
                .ok()
        };
        Self {
            tracker,
            stop,
            poller,
        }
    }
}

impl Drop for MacLastForeground {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(poller) = self.poller.take() {
            poller.thread().unpark();
            let _ = poller.join();
        }
    }
}

impl LastForeground for MacLastForeground {
    fn last_external(&self) -> Option<WindowHandle> {
        self.tracker.last_external()
    }
}
