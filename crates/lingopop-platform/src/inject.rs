//! `replace_selection`: paste text over the selection of a target window, then put the clipboard back.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::clipboard::{Clipboard, RestoreGuard};
use crate::error::{LogFn, PlatformError};
use crate::foreground::{ForegroundWindow, WindowHandle};
use crate::keys::KeySynth;
use crate::sleeper::Sleeper;

#[derive(Clone)]
pub struct ReplaceOptions {
    pub foreground_timeout: Duration,
    pub foreground_poll: Duration,
    pub restore_delay: Duration,
    pub log: Option<Arc<LogFn>>,
}

impl Default for ReplaceOptions {
    fn default() -> Self {
        Self {
            foreground_timeout: Duration::from_millis(300),
            foreground_poll: Duration::from_millis(20),
            restore_delay: Duration::from_millis(150),
            log: None,
        }
    }
}

impl fmt::Debug for ReplaceOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplaceOptions")
            .field("foreground_timeout", &self.foreground_timeout)
            .field("foreground_poll", &self.foreground_poll)
            .field("restore_delay", &self.restore_delay)
            .field("log", &self.log.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceOutcome {
    Pasted,
    /// The target could not be brought to the front; the text is on the clipboard as plain text.
    ForegroundLost,
}

pub fn replace_selection(
    fg: &dyn ForegroundWindow,
    clip: &dyn Clipboard,
    keys: &dyn KeySynth,
    sleeper: &dyn Sleeper,
    target: WindowHandle,
    text: &str,
    opts: &ReplaceOptions,
) -> Result<ReplaceOutcome, PlatformError> {
    if fg.is_elevated(target) {
        if let Some(l) = &opts.log {
            l("input blocked: target runs elevated");
        }
        return Err(PlatformError::InputBlocked);
    }

    if !bring_to_front(fg, sleeper, target, opts) {
        clip.write_text(text)?;
        return Ok(ReplaceOutcome::ForegroundLost);
    }

    let snapshot = clip.snapshot()?;
    let guard = RestoreGuard::new(clip, snapshot, opts.log.clone());
    clip.write_text_transient(text)?;
    guard.observe(clip.sequence_number());
    keys.send_paste()?;
    sleeper.sleep(opts.restore_delay);
    drop(guard);
    Ok(ReplaceOutcome::Pasted)
}

fn bring_to_front(
    fg: &dyn ForegroundWindow,
    sleeper: &dyn Sleeper,
    target: WindowHandle,
    opts: &ReplaceOptions,
) -> bool {
    if fg.restore(target).is_err() {
        return false;
    }
    let start = sleeper.now();
    loop {
        if fg.current() == Some(target) {
            return true;
        }
        if sleeper.now().saturating_sub(start) >= opts.foreground_timeout {
            return false;
        }
        sleeper.sleep(opts.foreground_poll);
    }
}
