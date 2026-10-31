//! `Clipboard` over the Win32 clipboard API.

use std::fmt;
use std::sync::Arc;

use ::windows::core::PCWSTR;
use ::windows::Win32::System::DataExchange::{
    EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, RegisterClipboardFormatW,
};
use ::windows::Win32::System::Ole::{
    CF_BITMAP, CF_DIB, CF_DIBV5, CF_ENHMETAFILE, CF_HDROP, CF_LOCALE, CF_METAFILEPICT, CF_OEMTEXT,
    CF_PALETTE, CF_TEXT, CF_UNICODETEXT,
};

use super::util::{
    map_err, os_err, read_hglobal, set_clipboard_bytes, wide, ClipboardGuard, MessageWindow,
};
use crate::clipboard::{
    protection_unreadable, unreadable_label, Clipboard, ClipboardEntry, ClipboardFormat,
    ClipboardSnapshot,
};
use crate::error::PlatformError;
use crate::sleeper::Sleeper;

/// Formats captured by `snapshot`, in this order. `CF_TEXT` / `CF_OEMTEXT` / `CF_LOCALE` /
/// `CF_DIBV5` are not stored: Windows synthesizes them from the stored ones (and from a
/// `CF_DIBV5`-only source it synthesizes `CF_DIB`, which drops alpha).
///
/// The three exclusion markers are captured too, as ordinary entries with their exact bytes, so a
/// restore puts back the protection of e.g. a password copied from a password manager (and absence
/// stays absence).
const SNAPSHOT_ORDER: [ClipboardFormat; 9] = [
    ClipboardFormat::UnicodeText,
    ClipboardFormat::Html,
    ClipboardFormat::Rtf,
    ClipboardFormat::Png,
    ClipboardFormat::Dib,
    ClipboardFormat::FileList,
    ClipboardFormat::ExcludeFromMonitors,
    ClipboardFormat::CanIncludeInHistory,
    ClipboardFormat::CanUploadToCloud,
];

// Windows synthesizes these standard formats from other clipboard representations.
// Do not report them as unsaved private formats (e.g. Office/Excel registered formats).
fn is_synthesized_format(id: u32) -> bool {
    [
        CF_TEXT,
        CF_BITMAP,
        CF_METAFILEPICT,
        CF_OEMTEXT,
        CF_PALETTE,
        CF_ENHMETAFILE,
        CF_LOCALE,
        CF_DIBV5,
    ]
    .iter()
    .any(|format| u32::from(format.0) == id)
}

/// Registered format names that keep our temporary write out of clipboard history, cloud sync
/// and clipboard monitors.
const EXCLUDE_FROM_MONITORS: &str = "ExcludeClipboardContentFromMonitorProcessing";
const CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
const CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";

fn registered(name: &str) -> Result<u32, PlatformError> {
    let w = wide(name);
    // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) };
    if id == 0 {
        Err(os_err(
            "RegisterClipboardFormatW",
            super::util::last_error(),
        ))
    } else {
        Ok(id)
    }
}

fn format_id(format: ClipboardFormat) -> Result<u32, PlatformError> {
    match format {
        ClipboardFormat::UnicodeText => Ok(u32::from(CF_UNICODETEXT.0)),
        ClipboardFormat::Html => registered("HTML Format"),
        ClipboardFormat::Rtf => registered("Rich Text Format"),
        ClipboardFormat::Png => registered("PNG"),
        ClipboardFormat::Dib => Ok(u32::from(CF_DIB.0)),
        ClipboardFormat::FileList => Ok(u32::from(CF_HDROP.0)),
        ClipboardFormat::ExcludeFromMonitors => registered(EXCLUDE_FROM_MONITORS),
        ClipboardFormat::CanIncludeInHistory => registered(CAN_INCLUDE_IN_HISTORY),
        ClipboardFormat::CanUploadToCloud => registered(CAN_UPLOAD_TO_CLOUD),
    }
}

fn format_name(format: ClipboardFormat) -> &'static str {
    match format {
        ClipboardFormat::UnicodeText => "CF_UNICODETEXT",
        ClipboardFormat::Html => "HTML Format",
        ClipboardFormat::Rtf => "Rich Text Format",
        ClipboardFormat::Png => "PNG",
        ClipboardFormat::Dib => "CF_DIB",
        ClipboardFormat::FileList => "CF_HDROP",
        ClipboardFormat::ExcludeFromMonitors => EXCLUDE_FROM_MONITORS,
        ClipboardFormat::CanIncludeInHistory => CAN_INCLUDE_IN_HISTORY,
        ClipboardFormat::CanUploadToCloud => CAN_UPLOAD_TO_CLOUD,
    }
}

fn available(id: u32) -> bool {
    // SAFETY: a pure query; needs no open clipboard.
    unsafe { IsClipboardFormatAvailable(id) }.is_ok()
}

/// Reads one format from the open clipboard: `Ok(None)` when absent, `Err(())` when present but
/// unreadable (null handle, zero size, lock failure). `GetClipboardData` renders delayed formats
/// synchronously in the owner process, so an unresponsive owner can stall this call.
fn read_format(id: u32) -> Result<Option<Vec<u8>>, ()> {
    if !available(id) {
        return Ok(None);
    }
    // SAFETY: the caller holds the clipboard open; the handle stays valid until it closes.
    let handle = unsafe { GetClipboardData(id) }.map_err(|_| ())?;
    read_hglobal(handle).map(Some).ok_or(())
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The three exclusion markers with the values of a protected write: the monitor-exclusion marker
/// (1 byte) and both "may include / upload" flags as DWORD 0.
fn protection_markers() -> Result<Vec<(u32, Vec<u8>)>, PlatformError> {
    Ok(vec![
        (registered(EXCLUDE_FROM_MONITORS)?, vec![0u8]),
        (
            registered(CAN_INCLUDE_IN_HISTORY)?,
            0u32.to_le_bytes().to_vec(),
        ),
        (
            registered(CAN_UPLOAD_TO_CLOUD)?,
            0u32.to_le_bytes().to_vec(),
        ),
    ])
}

fn empty() -> Result<(), PlatformError> {
    // SAFETY: the caller holds the clipboard open with a window owner.
    unsafe { EmptyClipboard() }.map_err(map_err("EmptyClipboard"))
}

/// Windows clipboard. Writes use a message-only owner window created on the calling thread for
/// that write only; all data is set immediately (never delayed), so no `WM_RENDERFORMAT` can
/// reach the destroyed window.
pub struct WindowsClipboard {
    sleeper: Arc<dyn Sleeper>,
}

impl WindowsClipboard {
    pub fn new(sleeper: Arc<dyn Sleeper>) -> Self {
        Self { sleeper }
    }

    /// Shared body of `restore` / `restore_if`. With `expected`, the sequence number is re-read
    /// AFTER the clipboard is open (nobody can write while we hold it) and a mismatch returns
    /// `Ok(false)` with the clipboard untouched.
    fn restore_inner(
        &self,
        snapshot: &ClipboardSnapshot,
        expected: Option<u64>,
    ) -> Result<bool, PlatformError> {
        let ids = snapshot
            .entries
            .iter()
            .map(|e| format_id(e.format))
            .collect::<Result<Vec<_>, _>>()?;
        // Fail closed: a protection marker that was present but unreadable at snapshot time is
        // replaced by the full protected set, never dropped.
        let markers = if protection_unreadable(&snapshot.skipped) {
            protection_markers()?
        } else {
            Vec::new()
        };
        let owner = MessageWindow::new()?;
        let _guard = ClipboardGuard::open(Some(&owner), self.sleeper.as_ref())?;
        if let Some(expected) = expected {
            if self.sequence_number() != expected {
                return Ok(false);
            }
        }
        empty()?;
        // Best effort: put back every entry we can, report the first failure.
        let mut first_err = None;
        for (entry, id) in snapshot.entries.iter().zip(ids) {
            if entry.data.is_empty() {
                continue;
            }
            if let Err(e) = set_clipboard_bytes(id, &entry.data) {
                first_err.get_or_insert(e);
            }
        }
        for (id, data) in &markers {
            if let Err(e) = set_clipboard_bytes(*id, data) {
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(true), Err)
    }

    /// Opens, empties and fills the clipboard with `text`, plus the exclusion formats when
    /// `transient`. An `OpenClipboard` failure returns `ClipboardBusy` with nothing changed; a
    /// failure after `EmptyClipboard` returns the error (the clipboard is then partially written,
    /// and its sequence number has moved, so a `RestoreGuard` restores it).
    fn write(&self, text: &str, transient: bool) -> Result<(), PlatformError> {
        // Resolve every id first, so no failure can happen between opening and emptying.
        let text_id = format_id(ClipboardFormat::UnicodeText)?;
        let extra = if transient {
            protection_markers()?
        } else {
            Vec::new()
        };
        let owner = MessageWindow::new()?;
        let _guard = ClipboardGuard::open(Some(&owner), self.sleeper.as_ref())?;
        empty()?;
        set_clipboard_bytes(text_id, &utf16_bytes(text))?;
        for (id, data) in &extra {
            set_clipboard_bytes(*id, data)?;
        }
        Ok(())
    }
}

impl fmt::Debug for WindowsClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WindowsClipboard")
    }
}

impl Clipboard for WindowsClipboard {
    fn sequence_number(&self) -> u64 {
        // SAFETY: a pure query (0 when the process has no window-station clipboard access).
        u64::from(unsafe { GetClipboardSequenceNumber() })
    }

    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError> {
        let ids = SNAPSHOT_ORDER
            .iter()
            .map(|&f| format_id(f).map(|id| (f, id)))
            .collect::<Result<Vec<_>, _>>()?;
        let _guard = ClipboardGuard::open(None, self.sleeper.as_ref())?;
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        let mut id = 0;
        loop {
            // SAFETY: the clipboard is held open; zero begins enumeration and ends it.
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            if !is_synthesized_format(id) && !ids.iter().any(|(_, saved)| *saved == id) {
                skipped.push(format!("clipboard: skipped format id {id}"));
            }
        }
        for (format, id) in ids {
            match read_format(id) {
                Ok(Some(data)) => entries.push(ClipboardEntry { format, data }),
                Ok(None) => {}
                Err(()) => skipped.push(unreadable_label(format_name(format))),
            }
        }
        // Read while the clipboard is open: nobody else can write in between.
        let sequence = self.sequence_number();
        Ok(ClipboardSnapshot {
            entries,
            skipped,
            sequence,
        })
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), PlatformError> {
        self.restore_inner(snapshot, None).map(|_| ())
    }

    fn restore_if(
        &self,
        snapshot: &ClipboardSnapshot,
        expected_sequence: u64,
    ) -> Result<bool, PlatformError> {
        self.restore_inner(snapshot, Some(expected_sequence))
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        let id = format_id(ClipboardFormat::UnicodeText)?;
        let _guard = ClipboardGuard::open(None, self.sleeper.as_ref())?;
        let Ok(Some(bytes)) = read_format(id) else {
            return Ok(None);
        };
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        Ok(Some(String::from_utf16_lossy(&units)))
    }

    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError> {
        self.write(text, true)
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.write(text, false)
    }
}
