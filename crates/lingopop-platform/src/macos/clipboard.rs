//! `Clipboard` over `NSPasteboard`: the item-based engine in `crate::pasteboard` with an AppKit
//! backend. Item boundaries and every readable type survive a snapshot and restore; types that
//! cannot be preserved make the snapshot abort (see the engine's docs for the rules, the
//! never-read skip list and the `changeCount` retry).
//!
//! NSPasteboard has no cross-process compare-and-swap: `restore_if` compares `changeCount`
//! right before `clearContents`, so a foreign write inside that gap is not detected. This is
//! weaker than the Windows clipboard, which stays open across check and write.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardType, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};

use crate::error::PlatformError;
use crate::pasteboard::{PasteboardBackend, PasteboardClipboard, PasteboardItem};

/// `NSPasteboard` as a `PasteboardBackend`. It holds only the pasteboard's NAME (`None` = the
/// general pasteboard) and looks the object up per call, so it is `Send + Sync` and never keeps
/// AppKit objects across threads.
#[derive(Debug, Clone, Default)]
pub struct NsPasteboard {
    name: Option<String>,
}

impl NsPasteboard {
    /// The general (user) pasteboard.
    pub fn general() -> Self {
        Self { name: None }
    }

    /// A private pasteboard with a unique name, for owner-run native tests: it is not the user's
    /// clipboard.
    #[doc(hidden)]
    pub fn unique_for_tests() -> Self {
        Self {
            name: Some(NSPasteboard::pasteboardWithUniqueName().name().to_string()),
        }
    }

    fn pasteboard(&self) -> Retained<NSPasteboard> {
        match &self.name {
            None => NSPasteboard::generalPasteboard(),
            Some(name) => NSPasteboard::pasteboardWithName(&NSString::from_str(name)),
        }
    }
}

fn string_type() -> &'static NSPasteboardType {
    // SAFETY: `NSPasteboardTypeString` is an immutable constant exported by AppKit.
    unsafe { NSPasteboardTypeString }
}

fn write_error() -> PlatformError {
    PlatformError::Os {
        api: "NSPasteboard.writeObjects",
        code: 0,
    }
}

impl PasteboardBackend for NsPasteboard {
    fn change_count(&self) -> u64 {
        // A change count is never negative.
        u64::try_from(self.pasteboard().changeCount()).unwrap_or(0)
    }

    fn item_types(&self) -> Result<Vec<Vec<String>>, PlatformError> {
        autoreleasepool(|_| {
            let pb = self.pasteboard();
            // `types` only lists names; no data is requested.
            Ok(pb
                .pasteboardItems()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| item.types().iter().map(|t| t.to_string()).collect())
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn read(&self, item: u32, type_name: &str) -> Result<Option<Vec<u8>>, PlatformError> {
        autoreleasepool(|_| {
            let pb = self.pasteboard();
            let Some(items) = pb.pasteboardItems() else {
                return Ok(None);
            };
            let Some(entry) = items.iter().nth(item as usize) else {
                return Ok(None);
            };
            // The single `dataForType:` read each type gets; nil = unreadable.
            Ok(entry
                .dataForType(&NSString::from_str(type_name))
                .map(|data| data.to_vec()))
        })
    }

    fn clear(&self) -> u64 {
        u64::try_from(self.pasteboard().clearContents()).unwrap_or(0)
    }

    fn write_items(&self, items: &[PasteboardItem]) -> Result<(), PlatformError> {
        autoreleasepool(|_| {
            let mut objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
                Vec::with_capacity(items.len());
            for types in items {
                let item = NSPasteboardItem::new();
                for (type_name, data) in types {
                    if !item
                        .setData_forType(&NSData::with_bytes(data), &NSString::from_str(type_name))
                    {
                        return Err(write_error());
                    }
                }
                objects.push(ProtocolObject::from_retained(item));
            }
            if self
                .pasteboard()
                .writeObjects(&NSArray::from_retained_slice(&objects))
            {
                Ok(())
            } else {
                Err(write_error())
            }
        })
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Ok(self
            .pasteboard()
            .stringForType(string_type())
            .map(|s| s.to_string()))
    }
}

/// The macOS clipboard over the general pasteboard.
pub type MacClipboard = PasteboardClipboard<NsPasteboard>;

impl MacClipboard {
    pub fn new() -> Self {
        Self::with_backend(NsPasteboard::general())
    }
}

impl Default for MacClipboard {
    fn default() -> Self {
        Self::new()
    }
}
