//! Pasteboard snapshot and restore over an item-based backend (macOS `NSPasteboard`; a fake in
//! tests). Portable, so the rules below are tested on every OS.
//!
//! Rules:
//! - Snapshots keep item boundaries: every entry is `ClipboardFormat::NativePasteboard { item,
//!   type_name }`, and a restore rebuilds one pasteboard item per snapshot item. Transient and
//!   concealed markers are ordinary (empty) types of their item, so they survive.
//! - Types named `com.apple.pasteboard.promised-*`, `NSFilePromise*` and `dyn.*` are NEVER read:
//!   reading a lazy provider can trigger it (a file promise starts a copy). They are recorded by
//!   name in `skipped`. Public AppKit cannot tell a lazy provider from an eager one without
//!   reading, so a lazy provider that is not on that list may still be triggered by the single
//!   read each type gets (the frozen Swift app does the same).
//! - Every other type is read once. `nil` data (unreadable) and a type larger than
//!   `MAX_TYPE_BYTES` are skipped (the bytes dropped at once).
//! - All or nothing: if anything non-empty would be omitted (skipped, unreadable, oversized, or a
//!   name-listed type, which is assumed non-empty), `snapshot()` fails with
//!   `Unsupported("pasteboard snapshot")` BEFORE the caller copies or clears anything.
//!   Consequence: a previous clipboard that carries a `dyn.*` type makes the text operation abort
//!   with that message instead of risking the user's clipboard.
//! - The snapshot is taken between two `changeCount` reads and retried up to three times, then
//!   `ClipboardBusy`.
//! - There is no cross-process compare-and-swap on `NSPasteboard`: `restore_if` compares
//!   `changeCount` right before `clearContents`, and a foreign write that lands between that
//!   check and the clear (microseconds) is not detected. This is weaker than the Windows
//!   implementation, which holds the clipboard open across check and write; do not claim
//!   atomic exclusion for it.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::clipboard::{
    unreadable_label, Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot,
};
use crate::error::{platform_log, PlatformError};

/// The `Unsupported` text of an unpreservable pasteboard (the orchestrator maps it to a message).
pub const UNSUPPORTED_SNAPSHOT: &str = "pasteboard snapshot";
/// Largest type that is kept in a snapshot (16 MiB).
pub const MAX_TYPE_BYTES: usize = 16 * 1024 * 1024;
/// Snapshot attempts before giving up on a pasteboard that keeps changing.
pub const SNAPSHOT_ATTEMPTS: usize = 3;

/// Markers from nspasteboard.org that clipboard managers honour, set like the mac app's
/// `Core/PasteboardSnapshot.swift` does for its temporary write.
pub const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
pub const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
/// `NSPasteboardTypeString`.
pub const TEXT_TYPE: &str = "public.utf8-plain-text";

/// One pasteboard item: its types with their bytes.
pub type PasteboardItem = Vec<(String, Vec<u8>)>;

/// The pasteboard operations the engine needs. Implementations must not read data in
/// `item_types`.
pub trait PasteboardBackend: Send + Sync {
    /// `NSPasteboard.changeCount`.
    fn change_count(&self) -> u64;
    /// Type names of every pasteboard item, in item order. Never reads data. `Err` when the
    /// pasteboard could not be enumerated (AppKit returns nil): never an empty list, which would
    /// look like an empty clipboard and make a later restore wipe it.
    fn item_types(&self) -> Result<Vec<Vec<String>>, PlatformError>;
    /// The data of one type of one item; `Ok(None)` when the owner provides none (nil).
    fn read(&self, item: u32, type_name: &str) -> Result<Option<Vec<u8>>, PlatformError>;
    /// `clearContents`: returns the change count right after the clear.
    fn clear(&self) -> u64;
    /// Writes `items` as separate pasteboard items (after `clear`).
    fn write_items(&self, items: &[PasteboardItem]) -> Result<(), PlatformError>;
    /// The plain-text string, independent of the item snapshot.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
}

/// True for the types that are never read (see the module docs).
pub fn is_unreadable_by_name(type_name: &str) -> bool {
    type_name.starts_with("com.apple.pasteboard.promised-")
        || type_name.contains("NSFilePromise")
        || type_name.starts_with("dyn.")
}

/// The result of one snapshot collection. `complete` is false when anything was skipped; then the
/// snapshot only documents what was skipped (by name) and must not be used for a restore.
#[derive(Debug, Clone)]
pub struct Collected {
    pub snapshot: ClipboardSnapshot,
    pub complete: bool,
}

/// Collects the pasteboard between two change-count reads, retrying up to `SNAPSHOT_ATTEMPTS`
/// times, then `ClipboardBusy`.
pub fn collect(backend: &dyn PasteboardBackend) -> Result<Collected, PlatformError> {
    for _ in 0..SNAPSHOT_ATTEMPTS {
        let before = backend.change_count();
        let (entries, skipped) = read_all(backend)?;
        if backend.change_count() != before {
            continue;
        }
        return Ok(Collected {
            complete: skipped.is_empty(),
            snapshot: ClipboardSnapshot {
                entries,
                skipped,
                sequence: before,
            },
        });
    }
    Err(PlatformError::ClipboardBusy)
}

fn read_all(
    backend: &dyn PasteboardBackend,
) -> Result<(Vec<ClipboardEntry>, Vec<String>), PlatformError> {
    let types = backend.item_types()?;
    // Name check first, over the whole pasteboard: nothing is read when a listed type is present.
    let listed: Vec<String> = types
        .iter()
        .flatten()
        .filter(|t| is_unreadable_by_name(t))
        .cloned()
        .collect();
    if !listed.is_empty() {
        return Ok((Vec::new(), listed));
    }
    let mut entries = Vec::new();
    for (index, item_types) in types.iter().enumerate() {
        let item =
            u32::try_from(index).map_err(|_| PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT))?;
        for type_name in item_types {
            match backend.read(item, type_name)? {
                None => return Ok((Vec::new(), vec![unreadable_label(type_name)])),
                Some(data) if data.len() > MAX_TYPE_BYTES => {
                    drop(data);
                    return Ok((Vec::new(), vec![type_name.clone()]));
                }
                Some(data) => entries.push(ClipboardEntry {
                    format: ClipboardFormat::NativePasteboard {
                        item,
                        type_name: type_name.clone(),
                    },
                    data,
                }),
            }
        }
    }
    Ok((entries, Vec::new()))
}

/// Rebuilds the pasteboard items of a snapshot; any entry that is not a native pasteboard entry
/// (or a snapshot that records skipped types) cannot be restored faithfully.
fn items_of(snapshot: &ClipboardSnapshot) -> Result<Vec<PasteboardItem>, PlatformError> {
    if !snapshot.skipped.is_empty() {
        return Err(PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT));
    }
    let mut items: Vec<(u32, PasteboardItem)> = Vec::new();
    for entry in &snapshot.entries {
        let ClipboardFormat::NativePasteboard { item, type_name } = &entry.format else {
            return Err(PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT));
        };
        let payload = (type_name.clone(), entry.data.clone());
        match items.iter_mut().find(|(i, _)| i == item) {
            Some((_, types)) => types.push(payload),
            None => items.push((*item, vec![payload])),
        }
    }
    items.sort_by_key(|(i, _)| *i);
    Ok(items.into_iter().map(|(_, types)| types).collect())
}

/// `Clipboard` over a `PasteboardBackend`.
pub struct PasteboardClipboard<B: PasteboardBackend> {
    backend: B,
    last_write: Mutex<Option<u64>>,
}

impl<B: PasteboardBackend> PasteboardClipboard<B> {
    pub fn with_backend(backend: B) -> Self {
        Self {
            backend,
            last_write: Mutex::new(None),
        }
    }

    fn last_write(&self) -> MutexGuard<'_, Option<u64>> {
        self.last_write.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `clearContents`, then one item with the text (and the markers when `transient`). The
    /// sequence returned by the clear is remembered BEFORE the write, so a failed or partial
    /// write is still ours to undo.
    fn write_text_item(&self, text: &str, transient: bool) -> Result<(), PlatformError> {
        *self.last_write() = None;
        let sequence = self.backend.clear();
        *self.last_write() = Some(sequence);
        let mut item: PasteboardItem = vec![(TEXT_TYPE.to_owned(), text.as_bytes().to_vec())];
        if transient {
            item.push((TRANSIENT_TYPE.to_owned(), Vec::new()));
            item.push((CONCEALED_TYPE.to_owned(), Vec::new()));
        }
        self.backend.write_items(&[item])
    }

    fn restore_inner(
        &self,
        snapshot: &ClipboardSnapshot,
        expected: Option<u64>,
    ) -> Result<bool, PlatformError> {
        // Everything that can be refused is refused before the pasteboard is touched.
        let items = items_of(snapshot)?;
        if let Some(expected) = expected {
            // As close to the clear as the API permits; see the module docs for the remaining gap.
            if self.backend.change_count() != expected {
                return Ok(false);
            }
        }
        self.backend.clear();
        if items.is_empty() {
            return Ok(true);
        }
        self.backend.write_items(&items).map(|()| true)
    }
}

impl<B: PasteboardBackend> Clipboard for PasteboardClipboard<B> {
    fn sequence_number(&self) -> u64 {
        self.backend.change_count()
    }

    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError> {
        let collected = collect(&self.backend)?;
        if !collected.complete {
            // Type names of the blocking types only; never bytes.
            let names: Vec<String> = collected
                .snapshot
                .skipped
                .iter()
                .map(|n| crate::clipboard::loggable_type_name(n))
                .collect();
            platform_log(&format!(
                "pasteboard snapshot aborted, not preserved: {}",
                names.join(", ")
            ));
            return Err(PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT));
        }
        Ok(collected.snapshot)
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
        self.backend.read_text()
    }

    fn check_preservable(&self) -> Result<(), PlatformError> {
        // Same completeness rule as `snapshot()`, without keeping anything.
        let collected = collect(&self.backend)?;
        if collected.complete {
            Ok(())
        } else {
            Err(PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT))
        }
    }

    fn write_text_transient(&self, text: &str) -> Result<(), PlatformError> {
        self.write_text_item(text, true)
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.write_text_item(text, false)
    }

    fn last_write_sequence(&self) -> Option<u64> {
        *self.last_write()
    }
}

impl<B: PasteboardBackend> std::fmt::Debug for PasteboardClipboard<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasteboardClipboard")
    }
}

impl<T: PasteboardBackend + ?Sized> PasteboardBackend for Arc<T> {
    fn change_count(&self) -> u64 {
        (**self).change_count()
    }
    fn item_types(&self) -> Result<Vec<Vec<String>>, PlatformError> {
        (**self).item_types()
    }
    fn read(&self, item: u32, type_name: &str) -> Result<Option<Vec<u8>>, PlatformError> {
        (**self).read(item, type_name)
    }
    fn clear(&self) -> u64 {
        (**self).clear()
    }
    fn write_items(&self, items: &[PasteboardItem]) -> Result<(), PlatformError> {
        (**self).write_items(items)
    }
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        (**self).read_text()
    }
}
