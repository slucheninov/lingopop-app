//! Clipboard seam: a lossless snapshot / restore pair plus the text primitives the flows need.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use crate::error::{LogFn, PlatformError};

/// One clipboard representation. Not `Copy` since `NativePasteboard` owns its type name; the
/// other variants keep their identity (Windows code clones or borrows them).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ClipboardFormat {
    UnicodeText,
    Html,
    Rtf,
    Png,
    Dib,
    FileList,
    /// Registered `ExcludeClipboardContentFromMonitorProcessing` marker (Windows).
    ExcludeFromMonitors,
    /// Registered `CanIncludeInClipboardHistory` marker (Windows).
    CanIncludeInHistory,
    /// Registered `CanUploadToCloudClipboard` marker (Windows).
    CanUploadToCloud,
    /// One pasteboard type of one pasteboard item (macOS): `item` is the position of the item on
    /// the pasteboard, `type_name` the UTI / pasteboard type. Item boundaries and arbitrary
    /// readable types survive a snapshot and restore this way.
    NativePasteboard {
        item: u32,
        type_name: String,
    },
}

/// `Debug` never prints a native type name unless it is a well-known system type (see
/// `loggable_type_name`): app-defined type names can say something about the user's content.
impl fmt::Debug for ClipboardFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnicodeText => f.write_str("UnicodeText"),
            Self::Html => f.write_str("Html"),
            Self::Rtf => f.write_str("Rtf"),
            Self::Png => f.write_str("Png"),
            Self::Dib => f.write_str("Dib"),
            Self::FileList => f.write_str("FileList"),
            Self::ExcludeFromMonitors => f.write_str("ExcludeFromMonitors"),
            Self::CanIncludeInHistory => f.write_str("CanIncludeInHistory"),
            Self::CanUploadToCloud => f.write_str("CanUploadToCloud"),
            Self::NativePasteboard { item, type_name } => write!(
                f,
                "NativePasteboard {{ item: {item}, type: {} }}",
                loggable_type_name(type_name)
            ),
        }
    }
}

/// The type name as it may appear in `Debug` output and logs: system-defined names (`public.*`,
/// `com.apple.*`, `org.nspasteboard.*`) verbatim, dynamic `dyn.*` types as `dyn.*`, anything else
/// (app-defined) as `<custom type>`.
#[doc(hidden)]
pub fn loggable_type_name(name: &str) -> String {
    if name.starts_with("dyn.") {
        "dyn.*".to_owned()
    } else if ["public.", "com.apple.", "org.nspasteboard."]
        .iter()
        .any(|p| name.starts_with(p))
    {
        name.to_owned()
    } else {
        "<custom type>".to_owned()
    }
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

/// What the clipboard held at one moment. `skipped` holds known-format unreadable labels and
/// numeric ids of enumerated formats that were not saved (never clipboard contents).
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

/// Registered clipboard format names that protect content from history, cloud sync and monitors.
/// Windows only; kept here so the fail-closed rule below is pure and testable on every OS.
#[doc(hidden)]
pub const PROTECTION_FORMAT_NAMES: [&str; 3] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
];

/// The `skipped` entry for a format that is present but could not be read (the NAME only).
#[doc(hidden)]
pub fn unreadable_label(name: &str) -> String {
    format!("format {name} unreadable")
}

/// True when any protection marker was present but unreadable in the snapshot: a restore must then
/// mark the content as protected instead of silently dropping the protection (fail closed).
#[doc(hidden)]
pub fn protection_unreadable(skipped: &[String]) -> bool {
    PROTECTION_FORMAT_NAMES
        .iter()
        .any(|n| skipped.contains(&unreadable_label(n)))
}

pub trait Clipboard: Send + Sync {
    /// Changes on every clipboard write by any application, even with identical content.
    fn sequence_number(&self) -> u64;
    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError>;
    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), PlatformError>;
    /// Restores `snapshot` only when the sequence number still equals `expected_sequence` at the
    /// moment the clipboard is held (on Windows: re-read after `OpenClipboard` succeeded, so a
    /// foreign write during the open retries is never clobbered). `Ok(true)` = restored,
    /// `Ok(false)` = the sequence differed and nothing was touched.
    fn restore_if(
        &self,
        snapshot: &ClipboardSnapshot,
        expected_sequence: u64,
    ) -> Result<bool, PlatformError>;
    /// `None` when the clipboard holds no text.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
    /// Text excluded from clipboard history and cloud sync (our own temporary write).
    /// Contract: a failure before the clipboard is emptied (e.g. `ClipboardBusy`) leaves it
    /// untouched, and a failure after that is returned as an error, never swallowed, so the
    /// caller's `RestoreGuard` puts the snapshot back over the partial write.
    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError>;
    /// An ordinary text entry the user may paste manually.
    fn write_text(&self, text: &str) -> Result<(), PlatformError>;
    /// The sequence number the clipboard had right after OUR most recent write (also a failed or
    /// partial one), when the implementation can report it exactly (macOS: the value
    /// `clearContents` returned). `None` = unknown; callers then read `sequence_number()` after
    /// the write, which is exact on Windows because the clipboard is held across the write.
    fn last_write_sequence(&self) -> Option<u64> {
        None
    }
    /// Whether the current contents could be snapshotted and restored faithfully, WITHOUT
    /// changing the clipboard. Callers that overwrite it without a snapshot (the Replace
    /// fallback that leaves text for a manual paste) check this first and write nothing on
    /// `Err`. Default `Ok(())`: Windows keeps its behaviour and opens no extra clipboard. macOS
    /// returns `Unsupported("pasteboard snapshot")` for content a restore would drop.
    fn check_preservable(&self) -> Result<(), PlatformError> {
        Ok(())
    }
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
        // The decision above is only a pre-check: `restore_if` repeats it. Windows re-checks
        // while holding the clipboard open (a write cannot land in between); macOS compares
        // `changeCount` immediately before `clearContents`, best effort: NSPasteboard has no
        // cross-process compare-and-swap.
        if let Err(e) = self.clip.restore_if(&self.snapshot, now) {
            if let Some(log) = &self.log {
                log(&format!("clipboard restore failed: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_protection_marker_is_detected_by_name_only() {
        assert!(!protection_unreadable(&[]));
        assert!(!protection_unreadable(&[unreadable_label("PNG")]));
        for n in PROTECTION_FORMAT_NAMES {
            assert!(protection_unreadable(&["x".into(), unreadable_label(n)]));
        }
    }

    #[test]
    fn native_format_is_clone_hash_eq_and_its_debug_hides_custom_names() {
        use std::collections::HashSet;
        let png = ClipboardFormat::NativePasteboard {
            item: 1,
            type_name: "public.png".into(),
        };
        let mut set = HashSet::new();
        set.insert(png.clone());
        assert!(set.contains(&png));
        assert_eq!(
            format!("{png:?}"),
            "NativePasteboard { item: 1, type: public.png }"
        );
        let custom = ClipboardFormat::NativePasteboard {
            item: 0,
            type_name: "com.acme.client-list".into(),
        };
        assert_eq!(
            format!("{custom:?}"),
            "NativePasteboard { item: 0, type: <custom type> }"
        );
        assert_eq!(loggable_type_name("dyn.ah62d4rv4gu8"), "dyn.*");
        assert_eq!(format!("{:?}", ClipboardFormat::Png), "Png");
    }
}
