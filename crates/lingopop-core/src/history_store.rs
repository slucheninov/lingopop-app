//! Local translation history (`history.json`), ported from the macOS
//! `Core/TranslationHistoryStore.swift` minus iCloud sync and polling.

use std::path::PathBuf;
use std::sync::Arc;

use uuid::Uuid;

use crate::envelope::MergeableEnvelope;
use crate::fsio::{LoadOutcome, StoreError};
use crate::records::HistoryEntry;
use crate::store_common::EnvelopeFile;
use crate::time::Clock;

/// History, newest first. Cap overflow is never tombstoned; `remove` records a
/// tombstone and `clear` sets the `clearedBefore` watermark.
pub struct HistoryStore {
    file: EnvelopeFile<HistoryEntry>,
}

/// Stable sort, newest first.
fn sort_newest_first(entries: &mut [HistoryEntry]) {
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
}

impl HistoryStore {
    /// Loads the file without writing. Entries are sorted newest first but not
    /// capped: the cap is applied by the next `add` / `trim`, as on macOS.
    pub fn open(
        path: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Result<(HistoryStore, LoadOutcome), StoreError> {
        let (mut file, outcome) = EnvelopeFile::load(path, clock)?;
        file.adjust_in_memory(|envelope| sort_newest_first(&mut envelope.items));
        Ok((HistoryStore { file }, outcome))
    }

    /// Newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.file.envelope().items
    }

    /// Inserts the entry, sorts newest first and caps at `max_count`. A
    /// `max_count` of 0 (history disabled) empties the list without a tombstone
    /// or watermark.
    pub fn add(&mut self, entry: HistoryEntry, max_count: u32) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        if max_count == 0 {
            next.items.clear();
        } else {
            next.items.insert(0, entry);
            Self::normalize(&mut next, max_count);
        }
        self.file.persist(next)
    }

    /// Re-applies the cap (e.g. after the user lowered it).
    pub fn trim(&mut self, max_count: u32) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        Self::normalize(&mut next, max_count);
        self.file.persist(next)
    }

    pub fn remove(&mut self, id: Uuid) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        next.mark_deleted(&[id], self.file.now());
        next.items.retain(|e| e.id != id);
        self.file.persist(next)
    }

    pub fn clear(&mut self) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        next.mark_cleared(self.file.now());
        next.items.clear();
        self.file.persist(next)
    }

    fn normalize(envelope: &mut MergeableEnvelope<HistoryEntry>, max_count: u32) {
        sort_newest_first(&mut envelope.items);
        envelope.items.truncate(max_count as usize);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::envelope::{MergeableEnvelope, Tombstone};
    use crate::{AIOperation, HistoryEntry, LoadOutcome, ManualClock, ProviderType, Timestamp};

    const DAY: f64 = 86_400.0;

    fn t(apple: f64) -> Timestamp {
        Timestamp::from_apple_seconds(apple)
    }

    fn entry(apple: f64) -> HistoryEntry {
        HistoryEntry::new(
            format!("original {apple}"),
            format!("translated {apple}"),
            None,
            "German".to_owned(),
            ProviderType::Claude,
            AIOperation::Translate,
            t(apple),
        )
    }

    fn setup(start: f64) -> (tempfile::TempDir, PathBuf, Arc<ManualClock>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        (dir, path, Arc::new(ManualClock::new(t(start))))
    }

    fn open(path: &Path, clock: &Arc<ManualClock>) -> (HistoryStore, LoadOutcome) {
        HistoryStore::open(path.to_path_buf(), clock.clone()).unwrap()
    }

    fn read_envelope(path: &Path) -> MergeableEnvelope<HistoryEntry> {
        MergeableEnvelope::<HistoryEntry>::decode(&fs::read(path).unwrap())
            .unwrap()
            .envelope
    }

    fn write_envelope(path: &Path, envelope: &MergeableEnvelope<HistoryEntry>) {
        fs::write(path, envelope.encode().unwrap()).unwrap();
    }

    fn ids(entries: &[HistoryEntry]) -> Vec<Uuid> {
        entries.iter().map(|e| e.id).collect()
    }

    #[test]
    fn open_missing_file_is_empty_and_writes_nothing() {
        let (_dir, path, clock) = setup(1000.0);
        let (store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Missing);
        assert!(store.entries().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn add_inserts_newest_first_and_persists() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        let (a, b, c) = (entry(10.0), entry(30.0), entry(20.0));
        for e in [&a, &b, &c] {
            store.add(e.clone(), 100).unwrap();
        }
        assert_eq!(
            ids(store.entries()),
            ids(&[b.clone(), c.clone(), a.clone()])
        );
        let (reopened, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(reopened.entries(), &[b, c, a]);
    }

    #[test]
    fn add_caps_at_max_count() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        for i in 1..=5 {
            store.add(entry(f64::from(i)), 3).unwrap();
        }
        let stamps: Vec<f64> = store
            .entries()
            .iter()
            .map(|e| e.timestamp.apple_seconds())
            .collect();
        assert_eq!(stamps, [5.0, 4.0, 3.0]);
        let envelope = read_envelope(&path);
        assert_eq!(envelope.items.len(), 3);
        assert!(
            envelope.deleted.is_empty(),
            "cap overflow is never tombstoned"
        );
        assert_eq!(envelope.cleared_before, None);
    }

    #[test]
    fn add_with_zero_max_count_empties_without_tombstone_or_watermark() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        store.add(entry(1.0), 10).unwrap();
        store.add(entry(2.0), 0).unwrap();
        assert!(store.entries().is_empty());
        let envelope = read_envelope(&path);
        assert!(envelope.items.is_empty());
        assert!(envelope.deleted.is_empty());
        assert_eq!(envelope.cleared_before, None);
    }

    #[test]
    fn trim_applies_new_cap() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        for i in 1..=5 {
            store.add(entry(f64::from(i)), 10).unwrap();
        }
        store.trim(2).unwrap();
        assert_eq!(store.entries().len(), 2);
        assert_eq!(store.entries()[0].timestamp, t(5.0));
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.entries().len(), 2);
        assert!(read_envelope(&path).deleted.is_empty());
    }

    #[test]
    fn remove_records_tombstone_and_persists() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        let (a, b) = (entry(1.0), entry(2.0));
        store.add(a.clone(), 10).unwrap();
        store.add(b.clone(), 10).unwrap();
        clock.set(t(1234.0));
        store.remove(a.id).unwrap();
        assert_eq!(store.entries(), std::slice::from_ref(&b));
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.entries(), &[b]);
        assert_eq!(
            read_envelope(&path).deleted,
            vec![Tombstone {
                id: a.id,
                at: t(1234.0)
            }]
        );
    }

    #[test]
    fn clear_sets_cleared_before_and_keeps_items_added_later() {
        let (_dir, path, clock) = setup(100.0);
        let (mut store, _) = open(&path, &clock);
        store.add(entry(50.0), 10).unwrap();
        store.clear().unwrap();
        assert!(store.entries().is_empty());
        clock.advance(10.0);
        let fresh = entry(110.0);
        store.add(fresh.clone(), 10).unwrap();
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.entries(), &[fresh]);
        assert_eq!(read_envelope(&path).cleared_before, Some(t(100.0)));
    }

    #[test]
    fn clock_going_backwards_never_lowers_cleared_before() {
        let (_dir, path, clock) = setup(100.0);
        let (mut store, _) = open(&path, &clock);
        store.clear().unwrap();
        clock.set(t(50.0));
        store.clear().unwrap();
        assert_eq!(read_envelope(&path).cleared_before, Some(t(100.0)));
    }

    #[test]
    fn future_dated_entries_are_kept_and_sorted_first() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        let future = entry(1000.0 + 10.0 * 365.0 * DAY);
        store.add(entry(900.0), 10).unwrap();
        store.add(future.clone(), 10).unwrap();
        store.add(entry(950.0), 10).unwrap();
        assert_eq!(store.entries()[0], future);

        let (_dir2, path2, clock2) = setup(1000.0);
        write_envelope(
            &path2,
            &MergeableEnvelope::with_items(vec![entry(1e300), entry(5.0)]),
        );
        let (mut absurd, outcome) = open(&path2, &clock2);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(absurd.entries()[0].timestamp, t(1e300));
        absurd.add(entry(7.0), 10).unwrap();
        assert_eq!(absurd.entries().len(), 3);
        assert_eq!(absurd.entries()[0].timestamp, t(1e300));
    }

    #[test]
    fn writes_envelope_v2() {
        let (_dir, path, clock) = setup(1000.0);
        let (mut store, _) = open(&path, &clock);
        store.add(entry(1.0), 10).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["deleted"], serde_json::json!([]));
        assert!(value.get("clearedBefore").is_none());
    }

    #[test]
    fn legacy_bare_array_is_read_and_upgraded_on_next_write() {
        let (_dir, path, clock) = setup(1000.0);
        let old = entry(1.0);
        let bare = serde_json::to_vec(&vec![old.clone()]).unwrap();
        fs::write(&path, &bare).unwrap();
        let (mut store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Legacy);
        assert_eq!(fs::read(&path).unwrap(), bare, "open must not write");
        let new = entry(2.0);
        store.add(new.clone(), 10).unwrap();
        let envelope = read_envelope(&path);
        assert_eq!(envelope.version, 2);
        assert_eq!(envelope.items, vec![new, old]);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value.is_object());
    }

    #[test]
    fn tombstones_older_than_90_days_are_pruned_on_write() {
        let (_dir, path, clock) = setup(100.0 * DAY);
        let (old, recent) = (Uuid::new_v4(), Uuid::new_v4());
        let mut envelope = MergeableEnvelope::<HistoryEntry>::new();
        envelope.deleted = vec![
            Tombstone {
                id: old,
                at: t(100.0 * DAY - 91.0 * DAY),
            },
            Tombstone {
                id: recent,
                at: t(100.0 * DAY - 1.0 * DAY),
            },
        ];
        write_envelope(&path, &envelope);
        let (mut store, _) = open(&path, &clock);
        assert_eq!(read_envelope(&path).deleted.len(), 2, "open must not write");
        store.add(entry(1.0), 10).unwrap();
        let deleted = read_envelope(&path).deleted;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].id, recent);
    }

    #[test]
    fn unknown_provider_in_stored_entry_survives_rewrite() {
        let (_dir, path, clock) = setup(1000.0);
        let mut future = entry(5.0);
        future.provider = "FutureAI".to_owned();
        future.operation = "futureOp".to_owned();
        write_envelope(&path, &MergeableEnvelope::with_items(vec![future.clone()]));
        let (mut store, _) = open(&path, &clock);
        store.add(entry(6.0), 10).unwrap();
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.entries().len(), 2);
        let kept = reopened
            .entries()
            .iter()
            .find(|e| e.id == future.id)
            .unwrap();
        assert_eq!(kept.provider, "FutureAI");
        assert_eq!(kept.operation, "futureOp");
        assert_eq!(kept.provider_type(), None);
    }

    #[test]
    fn utf8_bom_file_loads() {
        let (_dir, path, clock) = setup(1000.0);
        let e = entry(1.0);
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(
            MergeableEnvelope::with_items(vec![e.clone()])
                .encode()
                .unwrap(),
        );
        fs::write(&path, bytes).unwrap();
        let (store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.entries(), &[e]);
    }

    #[test]
    fn blank_file_is_missing_with_no_backup() {
        for content in [&b""[..], b"  \n\t", &[0xEF, 0xBB, 0xBF]] {
            let (dir, path, clock) = setup(1000.0);
            fs::write(&path, content).unwrap();
            let (store, outcome) = open(&path, &clock);
            assert_eq!(outcome, LoadOutcome::Missing);
            assert!(store.entries().is_empty());
            let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
            assert_eq!(names.len(), 1, "no .bak must be created");
        }
    }

    #[test]
    fn truncated_file_is_quarantined_not_overwritten() {
        let (dir, path, clock) = setup(1_793_366_956.0 - 978_307_200.0);
        let original = br#"{"version":2,"items":[{"id""#;
        fs::write(&path, original).unwrap();
        let (mut store, outcome) = open(&path, &clock);
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert_eq!(backup, dir.path().join("history.json.bak-1793366956"));
        assert!(store.entries().is_empty());
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(!path.exists(), "the bad file is moved, nothing written yet");

        let e = entry(1.0);
        store.add(e.clone(), 10).unwrap();
        assert_eq!(read_envelope(&path).items, vec![e]);
        assert_eq!(fs::read(&backup).unwrap(), original);
    }

    #[test]
    fn failed_write_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        let path = sub.join("history.json");
        let clock = Arc::new(ManualClock::new(t(1000.0)));
        let (mut store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Missing);
        fs::write(&sub, b"now a regular file").unwrap();
        assert!(store.add(entry(1.0), 10).is_err());
        assert!(store.entries().is_empty());
        assert!(store.remove(Uuid::new_v4()).is_err());
        assert!(store.clear().is_err());
        assert!(store.trim(5).is_err());
        assert!(store.entries().is_empty());
    }

    #[test]
    fn fifty_thousand_entry_file_loads_and_first_add_trims_to_cap() {
        let (_dir, path, clock) = setup(1_000_000.0);
        let items: Vec<HistoryEntry> = (0..50_000).map(|i| entry(f64::from(i))).collect();
        write_envelope(&path, &MergeableEnvelope::with_items(items));
        let (mut store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.entries().len(), 50_000, "open does not cap");
        assert_eq!(store.entries()[0].timestamp, t(49_999.0));
        store.add(entry(60_000.0), 10).unwrap();
        assert_eq!(store.entries().len(), 10);
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.entries().len(), 10);
        assert_eq!(reopened.entries()[0].timestamp, t(60_000.0));
    }

    #[test]
    fn lowercase_uuid_file_loads_and_is_rewritten_uppercase() {
        let (_dir, path, clock) = setup(1000.0);
        let line = r#"[{"original":"a","targetLanguage":"German","translated":"b","operation":"translate","timestamp":815059756.035193,"provider":"Claude","id":"16990d16-7e15-47c3-a299-ef2aec4400d1"}]"#;
        fs::write(&path, line).unwrap();
        let (mut store, _) = open(&path, &clock);
        assert_eq!(store.entries().len(), 1);
        store.add(entry(1.0), 10).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("16990D16-7E15-47C3-A299-EF2AEC4400D1"));
        assert!(!text.contains("16990d16"));
    }
}
