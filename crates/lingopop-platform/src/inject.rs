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
    /// The target could not be brought to the front, or the foreground moved away before the
    /// keystroke (`ForegroundWindow::is_current_target` is not atomic); nothing was pasted and
    /// the text is on the clipboard as plain text.
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
        // No snapshot is taken on this path, so make sure overwriting loses nothing.
        clip.check_preservable()?;
        clip.write_text(text)?;
        return Ok(ReplaceOutcome::ForegroundLost);
    }

    let snapshot = clip.snapshot()?;
    let guard = RestoreGuard::new(clip, snapshot, opts.log.clone());
    let written = clip.write_text_transient(text);
    // Even a failed write may have emptied the clipboard: whatever it did is ours to undo. The
    // sequence is the one our own write produced when the implementation can say so exactly
    // (macOS: what `clearContents` returned); a foreign write right after it is then never
    // mistaken for ours.
    guard.observe(
        clip.last_write_sequence()
            .unwrap_or_else(|| clip.sequence_number()),
    );
    written?;
    // The last look before the keystroke. NOT atomic: the user can still switch apps between
    // this answer and `send_paste`, so this narrows the race, it does not remove it. A
    // foreground that moved after the restore means the paste must go nowhere: the text stays
    // on the clipboard for a manual paste instead. A failed check (macOS: Accessibility
    // revoked) is returned as it is and nothing is sent.
    if !fg.is_current_target(target)? {
        if let Some(l) = &opts.log {
            l("replace skipped: target is no longer in front");
        }
        // Undo our transient write first (the guard only restores over our own write), then
        // leave the text as an ordinary entry, like the restore-failed path above.
        drop(guard);
        clip.check_preservable()?;
        clip.write_text(text)?;
        return Ok(ReplaceOutcome::ForegroundLost);
    }
    keys.send_paste()?;
    sleeper.sleep(opts.restore_delay);
    drop(guard);
    Ok(ReplaceOutcome::Pasted)
}

/// Restores `target` and polls `current()` until it is the foreground window or
/// `opts.foreground_timeout` passes (through the injected sleeper). `false` = could not.
pub fn bring_to_front(
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
