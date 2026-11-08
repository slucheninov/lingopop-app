//! Local, private text clipboard history. File order is observation order, even
//! when the system clock moves backwards. Text is never included in diagnostics.
use crate::{
    envelope::{Mergeable, MergeableEnvelope},
    fsio::{LoadOutcome, StoreError},
    store_common::EnvelopeFile,
    Clock, Timestamp,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, path::PathBuf, sync::Arc};
use uuid::Uuid;

pub const MAX_CLIPBOARD_HISTORY_COUNT: u32 = 1000;
pub const MAX_CLIPBOARD_MENU_COUNT: u32 = 50;
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 65_536;
/// The retained text budget; JSON escaping fits inside the bounded file read.
const MAX_RETAINED_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistoryEntry {
    #[serde(with = "crate::serde_uuid")]
    pub id: Uuid,
    pub text: String,
    pub timestamp: Timestamp,
}
impl fmt::Debug for ClipboardHistoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardHistoryEntry")
            .field("id", &self.id)
            .field("text", &"[redacted]")
            .field("timestamp", &self.timestamp)
            .finish()
    }
}
impl Mergeable for ClipboardHistoryEntry {
    fn id(&self) -> Uuid {
        self.id
    }
    fn merge_timestamp(&self) -> Timestamp {
        self.timestamp
    }
}

pub struct ClipboardHistoryStore {
    file: EnvelopeFile<ClipboardHistoryEntry>,
}
impl ClipboardHistoryStore {
    /// Read-only open, with the standard missing/BOM/corruption rules. Oversized
    /// files and unreadable/locked files fail closed without replacing anything.
    pub fn open(path: PathBuf, clock: Arc<dyn Clock>) -> Result<(Self, LoadOutcome), StoreError> {
        let (mut file, outcome) = EnvelopeFile::load_bounded(path, clock, MAX_FILE_BYTES)?;
        file.adjust_in_memory(|next| Self::normalize(next, MAX_CLIPBOARD_HISTORY_COUNT));
        Ok((Self { file }, outcome))
    }
    /// Returns retained entries in newest-first order.
    pub fn entries(&self) -> &[ClipboardHistoryEntry] {
        &self.file.envelope().items
    }
    /// Exact text deduplication preserves the UUID. Empty/oversized text is
    /// ignored; zero disables retention and clears existing entries atomically.
    pub fn record(&mut self, text: String, max_count: u32) -> Result<Option<Uuid>, StoreError> {
        if max_count == 0 {
            if !self.entries().is_empty() {
                self.trim(0)?;
            }
            return Ok(None);
        }
        if text.is_empty() || text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            return Ok(None);
        }
        let mut next = self.file.envelope().clone();
        let id = next
            .items
            .iter()
            .find(|entry| entry.text == text)
            .map(|entry| entry.id)
            .unwrap_or_else(Uuid::new_v4);
        next.items.retain(|entry| entry.text != text);
        next.items.insert(
            0,
            ClipboardHistoryEntry {
                id,
                text,
                timestamp: self.file.now(),
            },
        );
        Self::normalize(&mut next, max_count);
        self.file.persist(next)?;
        Ok(Some(id))
    }
    /// A stale UUID is a no-op and cannot resurrect a removed entry.
    pub fn promote(&mut self, id: Uuid, max_count: u32) -> Result<bool, StoreError> {
        let Some(entry) = self.entries().iter().find(|entry| entry.id == id) else {
            return Ok(false);
        };
        let text = entry.text.clone();
        self.record(text, max_count)?;
        Ok(true)
    }
    /// Persists the newest entries up to the configured limit.
    pub fn trim(&mut self, max_count: u32) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        Self::normalize(&mut next, max_count);
        self.file.persist(next)
    }
    /// Removes an entry by its stable identifier.
    pub fn remove(&mut self, id: Uuid) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        next.items.retain(|entry| entry.id != id);
        // This store is local, never merged: avoid retaining deleted text or an
        // unbounded tombstone list for clipboard observations.
        self.file.persist(next)
    }
    /// Removes all retained entries from memory and disk.
    pub fn clear(&mut self) -> Result<(), StoreError> {
        self.file.persist(MergeableEnvelope::new())
    }
    fn normalize(next: &mut MergeableEnvelope<ClipboardHistoryEntry>, max_count: u32) {
        let mut texts = HashSet::new();
        let mut ids = HashSet::new();
        let mut bytes = 0;
        next.items.retain(|entry| {
            if entry.text.is_empty()
                || entry.text.len() > MAX_CLIPBOARD_TEXT_BYTES
                || bytes + entry.text.len() > MAX_RETAINED_TEXT_BYTES
                || !ids.insert(entry.id)
                || !texts.insert(entry.text.clone())
            {
                return false;
            }
            bytes += entry.text.len();
            true
        });
        next.items
            .truncate(max_count.min(MAX_CLIPBOARD_HISTORY_COUNT) as usize);
        next.deleted.clear();
        next.cleared_before = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ManualClock, Timestamp};
    use std::{fs, sync::Arc};

    fn setup() -> (tempfile::TempDir, ClipboardHistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let (store, outcome) = ClipboardHistoryStore::open(
            dir.path().join("clipboard-history.json"),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(100.0))),
        )
        .unwrap();
        assert_eq!(outcome, LoadOutcome::Missing);
        (dir, store)
    }

    #[test]
    fn clipboard_history_persists_deduplicates_and_promotes_exact_text() {
        let (dir, mut store) = setup();
        let first = store.record("a".into(), 100).unwrap().unwrap();
        store.record(" a".into(), 100).unwrap();
        assert_eq!(store.record("a".into(), 100).unwrap(), Some(first));
        assert_eq!(
            store
                .entries()
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["a", " a"]
        );
        let second = store.entries()[1].id;
        assert!(store.promote(second, 100).unwrap());
        let (reloaded, _) = ClipboardHistoryStore::open(
            dir.path().join("clipboard-history.json"),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(10.0))),
        )
        .unwrap();
        assert_eq!(reloaded.entries(), store.entries());
        assert!(!format!("{:?}", store.entries()[0]).contains("text: \" a\""));
    }

    #[test]
    fn clipboard_history_limits_delete_clear_and_ignored_inputs() {
        let (_dir, mut store) = setup();
        for text in ["a", "b", "c"] {
            store.record(text.into(), 2).unwrap();
        }
        assert_eq!(store.entries().len(), 2);
        let id = store.entries()[0].id;
        store.remove(id).unwrap();
        assert!(!store.promote(id, 2).unwrap());
        assert_eq!(store.record(String::new(), 2).unwrap(), None);
        assert_eq!(
            store
                .record("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1), 2)
                .unwrap(),
            None
        );
        store.trim(0).unwrap();
        assert!(store.entries().is_empty());
        assert_eq!(store.record("ignored".into(), 0).unwrap(), None);
        store.record("new".into(), 100).unwrap();
        store.clear().unwrap();
        assert!(store.entries().is_empty());
    }

    #[test]
    fn clipboard_history_zero_retention_skips_empty_store_write() {
        let (dir, mut store) = setup();
        let path = dir.path().join("clipboard-history.json");
        assert_eq!(store.record("ignored".into(), 0).unwrap(), None);
        assert!(!path.exists());
        store.record("saved".into(), 1).unwrap();
        assert_eq!(store.record("ignored".into(), 0).unwrap(), None);
        assert!(store.entries().is_empty());
    }

    #[test]
    fn clipboard_history_corrupt_and_unreadable_files_fail_safely() {
        let (dir, _) = setup();
        let path = dir.path().join("clipboard-history.json");
        fs::write(&path, b"private-sentinel malformed").unwrap();
        let (store, outcome) = ClipboardHistoryStore::open(
            path.clone(),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(100.0))),
        )
        .unwrap();
        assert!(matches!(outcome, LoadOutcome::CorruptBackedUp(_)));
        assert!(store.entries().is_empty());
        fs::create_dir(&path).unwrap();
        let error = ClipboardHistoryStore::open(
            path,
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(100.0))),
        )
        .err()
        .unwrap();
        assert!(!format!("{error:?} {error}").contains("private-sentinel"));
    }

    #[test]
    fn clipboard_history_retention_decrease_persists_and_debug_is_private() {
        let (dir, mut store) = setup();
        for text in ["older", "private-clipboard-sentinel", "newer"] {
            store.record(text.into(), 100).unwrap();
        }
        assert!(!format!("{:?}", store.entries()).contains("private-clipboard-sentinel"));
        store.trim(1).unwrap();
        let (reloaded, _) = ClipboardHistoryStore::open(
            dir.path().join("clipboard-history.json"),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(0.0))),
        )
        .unwrap();
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].text, "newer");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.path().join("clipboard-history.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn clipboard_history_oversized_file_read_is_bounded_and_original_is_kept() {
        let (dir, _) = setup();
        let path = dir.path().join("clipboard-history.json");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let error = ClipboardHistoryStore::open(
            path.clone(),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(0.0))),
        )
        .err()
        .unwrap();
        assert!(matches!(error, StoreError::Io(_)));
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_FILE_BYTES + 1);
    }

    #[cfg(windows)]
    #[test]
    fn clipboard_history_locked_file_is_not_replaced() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, mut store) = setup();
        store.record("locked-sentinel".into(), 100).unwrap();
        let path = dir.path().join("clipboard-history.json");
        let original = fs::read(&path).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let error = ClipboardHistoryStore::open(
            path.clone(),
            Arc::new(ManualClock::new(Timestamp::from_apple_seconds(0.0))),
        )
        .err()
        .unwrap();
        assert!(!format!("{error:?} {error}").contains("locked-sentinel"));
        assert!(store.clear().is_err());
        assert_eq!(store.entries().len(), 1);
        drop(lock);
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn clipboard_history_failed_mutations_do_not_publish() {
        let (dir, mut store) = setup();
        store.record("private-sentinel".into(), 100).unwrap();
        let before = store.entries().to_vec();
        let path = dir.path().join("clipboard-history.json");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let error = store.record("other".into(), 100).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-sentinel"));
        assert!(store.remove(before[0].id).is_err());
        assert!(store.promote(before[0].id, 100).is_err());
        assert!(store.trim(0).is_err());
        assert!(store.clear().is_err());
        assert_eq!(store.entries(), before);
    }
}
