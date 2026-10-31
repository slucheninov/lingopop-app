//! `capture_selection`: copy the current selection through the clipboard and put the clipboard back.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::clipboard::{Clipboard, RestoreGuard};
use crate::error::{LogFn, PlatformError};
use crate::foreground::ForegroundWindow;
use crate::keys::KeySynth;
use crate::sleeper::Sleeper;

#[derive(Clone)]
pub struct CaptureOptions {
    pub modifier_release_timeout: Duration,
    pub copy_timeout: Duration,
    pub poll_interval: Duration,
    pub settle_cap: Duration,
    pub log: Option<Arc<LogFn>>,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            modifier_release_timeout: Duration::from_millis(300),
            copy_timeout: Duration::from_millis(300),
            poll_interval: Duration::from_millis(10),
            settle_cap: Duration::from_millis(100),
            log: None,
        }
    }
}

impl fmt::Debug for CaptureOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureOptions")
            .field("modifier_release_timeout", &self.modifier_release_timeout)
            .field("copy_timeout", &self.copy_timeout)
            .field("poll_interval", &self.poll_interval)
            .field("settle_cap", &self.settle_cap)
            .field("log", &self.log.is_some())
            .finish()
    }
}

/// Returns the selected text, `None` when nothing (or no text) was selected. The user's clipboard
/// is restored afterwards unless another application wrote to it in the meantime.
pub fn capture_selection(
    clip: &dyn Clipboard,
    keys: &dyn KeySynth,
    fg: &dyn ForegroundWindow,
    sleeper: &dyn Sleeper,
    opts: &CaptureOptions,
) -> Result<Option<String>, PlatformError> {
    if let Some(w) = fg.current() {
        if fg.is_elevated(w) {
            log(opts, "input blocked: target runs elevated");
            return Err(PlatformError::InputBlocked);
        }
    }
    // Wait BEFORE the snapshot: a foreign write during the wait would otherwise be clobbered by
    // restoring an older snapshot.
    wait_for_modifiers(keys, sleeper, opts)?;
    let snapshot = clip.snapshot()?;
    if !snapshot.skipped.is_empty() {
        log(
            opts,
            &format!(
                "clipboard snapshot skipped formats: {}",
                snapshot.skipped.join(", ")
            ),
        );
    }
    let guard = RestoreGuard::new(clip, snapshot, opts.log.clone());
    let result = capture_inner(&guard, clip, keys, sleeper, opts);
    drop(guard);
    result
}

fn capture_inner(
    guard: &RestoreGuard<'_>,
    clip: &dyn Clipboard,
    keys: &dyn KeySynth,
    sleeper: &dyn Sleeper,
    opts: &CaptureOptions,
) -> Result<Option<String>, PlatformError> {
    let before = clip.sequence_number();
    keys.send_copy()?;

    // Poll for the sequence change; the check at the deadline is the final re-check.
    let start = sleeper.now();
    let mut last = loop {
        let now_seq = clip.sequence_number();
        if now_seq != before {
            break now_seq;
        }
        if sleeper.now().saturating_sub(start) >= opts.copy_timeout {
            return Ok(None);
        }
        sleeper.sleep(opts.poll_interval);
    };
    guard.observe(last);

    // Settle: apps that "empty, then set" bump the sequence twice.
    let settle_start = sleeper.now();
    while sleeper.now().saturating_sub(settle_start) < opts.settle_cap {
        sleeper.sleep(opts.poll_interval);
        let cur = clip.sequence_number();
        if cur == last {
            break;
        }
        last = cur;
        guard.observe(last);
    }

    // Still empty (an app between "empty" and "set") while budget remains: keep polling.
    loop {
        if let Some(text) = clip.read_text()?.filter(|t| !t.trim().is_empty()) {
            // The text may belong to a foreign write that landed after `last`: only a sequence
            // that is still the one we observed proves it is our copy.
            if clip.sequence_number() != last {
                return Ok(None);
            }
            return Ok(Some(text));
        }
        if sleeper.now().saturating_sub(settle_start) >= opts.settle_cap {
            return Ok(None);
        }
        sleeper.sleep(opts.poll_interval);
        let cur = clip.sequence_number();
        if cur != last {
            last = cur;
            guard.observe(last);
        }
    }
}

fn wait_for_modifiers(
    keys: &dyn KeySynth,
    sleeper: &dyn Sleeper,
    opts: &CaptureOptions,
) -> Result<(), PlatformError> {
    if keys.modifiers_down().is_empty() {
        return Ok(());
    }
    let start = sleeper.now();
    loop {
        let held = keys.modifiers_down();
        if held.is_empty() {
            return Ok(());
        }
        if sleeper.now().saturating_sub(start) >= opts.modifier_release_timeout {
            return keys.release_modifiers(&held);
        }
        sleeper.sleep(opts.poll_interval);
    }
}

fn log(opts: &CaptureOptions, msg: &str) {
    if let Some(l) = &opts.log {
        l(msg);
    }
}
