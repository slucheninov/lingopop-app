//! Clipboard seam: a lossless snapshot / restore pair plus the text primitives the flows need.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use crate::error::{LogFn, PlatformError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardFormat {
    UnicodeText,
    Html,
    Rtf,
    Png,
    Dib,
    FileList,
}

/// One clipboard format with its raw bytes. `Debug` prints the size only.
#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardEntry {
    pub format: ClipboardFormat,
    pub data: Vec<u8>,
}

impl fmt::Debug for ClipboardEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ClipboardEntry {{ format: {:?}, bytes: {} }}",
            self.format,
            self.data.len()
        )
    }
}

/// What the clipboard held at one moment. `skipped` holds the NAMES of formats that could not be read.
#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub entries: Vec<ClipboardEntry>,
    pub skipped: Vec<String>,
    pub sequence: u64,
}

impl fmt::Debug for ClipboardSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes: usize = self.entries.iter().map(|e| e.data.len()).sum();
        write!(
            f,
            "ClipboardSnapshot {{ entries: {}, bytes: {}, skipped: {}, sequence: {} }}",
            self.entries.len(),
            bytes,
            self.skipped.len(),
            self.sequence
        )
    }
}

pub trait Clipboard: Send + Sync {
    /// Changes on every clipboard write by any application, even with identical content.
    fn sequence_number(&self) -> u64;
    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError>;
    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), PlatformError>;
    /// `None` when the clipboard holds no text.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
    /// Text excluded from clipboard history and cloud sync (our own temporary write).
    /// Contract: a failure before the clipboard is emptied (e.g. `ClipboardBusy`) leaves it
    /// untouched, and a failure after that is returned as an error, never swallowed, so the
    /// caller's `RestoreGuard` puts the snapshot back over the partial write.
    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError>;
    /// An ordinary text entry the user may paste manually.
    fn write_text(&self, text: &str) -> Result<(), PlatformError>;
}

/// Restores the saved clipboard on drop (also on unwind), but never over a newer foreign write.
///
/// Rule: with `now = clip.sequence_number()`: `now == snapshot.sequence` means nothing changed (and
/// formats the snapshot could not read stay intact); `now == observed` (the sequence seen right
/// after OUR write or copy) means only we changed it, so restore; anything else is a newer write by
/// another application and is left alone.
pub(crate) struct RestoreGuard<'a> {
    clip: &'a dyn Clipboard,
    snapshot: ClipboardSnapshot,
    observed: Cell<Option<u64>>,
    log: Option<Arc<LogFn>>,
}

impl<'a> RestoreGuard<'a> {
    pub(crate) fn new(
        clip: &'a dyn Clipboard,
        snapshot: ClipboardSnapshot,
        log: Option<Arc<LogFn>>,
    ) -> Self {
        Self {
            clip,
            snapshot,
            observed: Cell::new(None),
            log,
        }
    }

    /// Records the sequence number that results from our own write or copy.
    pub(crate) fn observe(&self, sequence: u64) {
        self.observed.set(Some(sequence));
    }
}

impl Drop for RestoreGuard<'_> {
    fn drop(&mut self) {
        let now = self.clip.sequence_number();
        if now == self.snapshot.sequence || self.observed.get() != Some(now) {
            return;
        }
        if let Err(e) = self.clip.restore(&self.snapshot) {
            if let Some(log) = &self.log {
                log(&format!("clipboard restore failed: {e}"));
            }
        }
    }
}
