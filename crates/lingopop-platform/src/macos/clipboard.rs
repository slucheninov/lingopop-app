//! `Clipboard` over `NSPasteboard.generalPasteboard`.
//!
//! DEV-ONLY LIMITATION: the snapshot holds the plain-text string only. Other pasteboard types
//! (images, rich text, files) are listed by name in `skipped` and are lost if a restore runs
//! after the clipboard changed. Acceptable because the macOS build is never shipped.

use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardType, NSPasteboardTypeString};
use objc2_foundation::{NSData, NSString};

use crate::clipboard::{Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot};
use crate::error::PlatformError;

/// Markers from nspasteboard.org that clipboard managers honour, set like the mac app's
/// `Core/PasteboardSnapshot.swift` does for its temporary write.
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

fn pasteboard() -> Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

fn string_type() -> &'static NSPasteboardType {
    // SAFETY: `NSPasteboardTypeString` is an immutable constant exported by AppKit.
    unsafe { NSPasteboardTypeString }
}

fn write_error() -> PlatformError {
    PlatformError::Os {
        api: "NSPasteboard.setString",
        code: 0,
    }
}

fn set_string(pb: &NSPasteboard, text: &str) -> Result<(), PlatformError> {
    if pb.setString_forType(&NSString::from_str(text), string_type()) {
        Ok(())
    } else {
        Err(write_error())
    }
}

/// Dev only (see the module docs). Entries hold UTF-8 bytes.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacClipboard;

impl MacClipboard {
    pub fn new() -> Self {
        Self
    }
}

impl Clipboard for MacClipboard {
    fn sequence_number(&self) -> u64 {
        pasteboard().changeCount() as u64
    }

    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError> {
        let pb = pasteboard();
        let sequence = pb.changeCount() as u64;
        let entries = pb
            .stringForType(string_type())
            .map(|s| ClipboardEntry {
                format: ClipboardFormat::UnicodeText,
                data: s.to_string().into_bytes(),
            })
            .into_iter()
            .collect();
        let skipped = pb
            .types()
            .map(|types| {
                types
                    .iter()
                    .filter(|t| &**t != string_type())
                    .map(|t| t.to_string())
                    .collect()
            })
            .unwrap_or_default();
        Ok(ClipboardSnapshot {
            entries,
            skipped,
            sequence,
        })
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), PlatformError> {
        let pb = pasteboard();
        pb.clearContents();
        match snapshot
            .entries
            .iter()
            .find(|e| e.format == ClipboardFormat::UnicodeText)
        {
            Some(entry) => set_string(&pb, &String::from_utf8_lossy(&entry.data)),
            None => Ok(()),
        }
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Ok(pasteboard()
            .stringForType(string_type())
            .map(|s| s.to_string()))
    }

    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError> {
        let pb = pasteboard();
        pb.clearContents();
        set_string(&pb, text)?;
        let empty = NSData::new();
        for marker in [TRANSIENT_TYPE, CONCEALED_TYPE] {
            if !pb.setData_forType(Some(&empty), &NSString::from_str(marker)) {
                return Err(write_error());
            }
        }
        Ok(())
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        let pb = pasteboard();
        pb.clearContents();
        set_string(&pb, text)
    }
}
