//! Local token-usage records (`usage.json`), ported from the macOS
//! `Core/UsageStore.swift` minus iCloud sync, polling and aggregation (Phase 4).

use std::path::PathBuf;
use std::sync::Arc;

use crate::envelope::MergeableEnvelope;
use crate::fsio::{LoadOutcome, StoreError};
use crate::records::UsageRecord;
use crate::store_common::EnvelopeFile;
use crate::time::{Clock, Timestamp};

const DAY_SECONDS: f64 = 86_400.0;
const RETENTION_DAYS: f64 = 31.0;

/// Usage records, oldest first, kept for 31 days. The macOS app counts from the
/// local `startOfDay`; the core has no time zone, so the cutoff here is UTC midnight
/// of today minus 31 days.
pub struct UsageStore {
    file: EnvelopeFile<UsageRecord>,
}

fn retention_cutoff(now: Timestamp) -> Timestamp {
    let midnight = (now.unix_seconds() / DAY_SECONDS).floor() * DAY_SECONDS;
    Timestamp::from_unix_seconds(midnight - RETENTION_DAYS * DAY_SECONDS)
}

/// Stable sort oldest first, then drop everything older than the cutoff.
fn normalize(envelope: &mut MergeableEnvelope<UsageRecord>, now: Timestamp) {
    let cutoff = retention_cutoff(now);
    envelope.items.sort_by_key(|r| r.timestamp);
    envelope.items.retain(|r| r.timestamp >= cutoff);
}

impl UsageStore {
    /// Loads the file without writing; the retention rule is applied in memory only.
    pub fn open(
        path: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Result<(UsageStore, LoadOutcome), StoreError> {
        let (mut file, outcome) = EnvelopeFile::load(path, clock)?;
        let now = file.now();
        file.adjust_in_memory(|envelope| normalize(envelope, now));
        Ok((UsageStore { file }, outcome))
    }

    /// Oldest first.
    pub fn records(&self) -> &[UsageRecord] {
        &self.file.envelope().items
    }

    pub fn append(&mut self, record: UsageRecord) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        next.items.push(record);
        normalize(&mut next, self.file.now());
        self.file.persist(next)
    }

    /// Sets the `clearedBefore` watermark and empties the list; records created
    /// afterwards survive.
    pub fn reset(&mut self) -> Result<(), StoreError> {
        let mut next = self.file.envelope().clone();
        next.mark_cleared(self.file.now());
        next.items.clear();
        self.file.persist(next)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::envelope::{MergeableEnvelope, Tombstone};
    use crate::{AIOperation, LoadOutcome, ManualClock, ProviderType, Timestamp, UsageRecord};

    /// 2026-10-30T13:29:16Z.
    const NOW_UNIX: f64 = 1_793_366_956.0;
    /// 2026-09-29T00:00:00Z: UTC midnight of `NOW_UNIX` minus 31 days.
    const CUTOFF_UNIX: f64 = 1_790_640_000.0;

    fn unix(seconds: f64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds)
    }

    fn record(ts: Timestamp) -> UsageRecord {
        UsageRecord::new(
            ProviderType::Claude,
            "model".to_owned(),
            AIOperation::Translate,
            10,
            20,
            ts,
        )
    }

    fn setup() -> (tempfile::TempDir, PathBuf, Arc<ManualClock>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        (dir, path, Arc::new(ManualClock::new(unix(NOW_UNIX))))
    }

    fn open(path: &Path, clock: &Arc<ManualClock>) -> (UsageStore, LoadOutcome) {
        UsageStore::open(path.to_path_buf(), clock.clone()).unwrap()
    }

    fn read_envelope(path: &Path) -> MergeableEnvelope<UsageRecord> {
        MergeableEnvelope::<UsageRecord>::decode(&fs::read(path).unwrap())
            .unwrap()
            .envelope
    }

    #[test]
    fn append_sorts_oldest_first_and_persists() {
        let (_dir, path, clock) = setup();
        let (mut store, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Missing);
        assert!(!path.exists());
        let (a, b, c) = (
            record(unix(NOW_UNIX - 100.0)),
            record(unix(NOW_UNIX - 300.0)),
            record(unix(NOW_UNIX - 200.0)),
        );
        for r in [&a, &b, &c] {
            store.append(r.clone()).unwrap();
        }
        assert_eq!(store.records(), &[b.clone(), c.clone(), a.clone()]);
        let (reopened, outcome) = open(&path, &clock);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(reopened.records(), &[b, c, a]);
    }

    #[test]
    fn retention_cutoff_is_utc_midnight_minus_31_days() {
        let (_dir, path, clock) = setup();
        let (mut store, _) = open(&path, &clock);
        let kept = record(unix(CUTOFF_UNIX));
        let dropped = record(unix(CUTOFF_UNIX - 1.0));
        store.append(dropped).unwrap();
        assert!(store.records().is_empty());
        store.append(kept.clone()).unwrap();
        assert_eq!(store.records(), std::slice::from_ref(&kept));

        // The same rule runs on open, in memory only.
        let (_dir2, path2, clock2) = setup();
        let both =
            MergeableEnvelope::with_items(vec![record(unix(CUTOFF_UNIX - 1.0)), kept.clone()]);
        let bytes = both.encode().unwrap();
        fs::write(&path2, &bytes).unwrap();
        let (opened, _) = open(&path2, &clock2);
        assert_eq!(opened.records(), &[kept]);
        assert_eq!(fs::read(&path2).unwrap(), bytes, "open must not write");
    }

    #[test]
    fn reset_sets_cleared_before_and_empties() {
        let (_dir, path, clock) = setup();
        let (mut store, _) = open(&path, &clock);
        store.append(record(unix(NOW_UNIX - 10.0))).unwrap();
        store.reset().unwrap();
        assert!(store.records().is_empty());
        let envelope = read_envelope(&path);
        assert!(envelope.items.is_empty());
        assert_eq!(envelope.cleared_before, Some(unix(NOW_UNIX)));
    }

    #[test]
    fn records_added_after_reset_survive_reopen() {
        let (_dir, path, clock) = setup();
        let (mut store, _) = open(&path, &clock);
        store.reset().unwrap();
        clock.advance(5.0);
        let fresh = record(unix(NOW_UNIX + 5.0));
        store.append(fresh.clone()).unwrap();
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.records(), &[fresh]);
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let (dir, path, clock) = setup();
        let original = b"{ not json at all";
        fs::write(&path, original).unwrap();
        let (mut store, outcome) = open(&path, &clock);
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert_eq!(backup, dir.path().join("usage.json.bak-1793366956"));
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(store.records().is_empty());
        assert!(!path.exists());
        store.append(record(unix(NOW_UNIX))).unwrap();
        assert_eq!(read_envelope(&path).items.len(), 1);
    }

    #[test]
    fn future_dated_record_is_kept() {
        let (_dir, path, clock) = setup();
        let (mut store, _) = open(&path, &clock);
        let future = record(unix(NOW_UNIX + 10.0 * 365.0 * 86_400.0));
        let huge = record(Timestamp::from_apple_seconds(1e300));
        store.append(huge.clone()).unwrap();
        store.append(future.clone()).unwrap();
        store.append(record(unix(NOW_UNIX))).unwrap();
        assert_eq!(store.records().len(), 3);
        assert_eq!(store.records()[1], future);
        assert_eq!(store.records()[2], huge);
        let (reopened, _) = open(&path, &clock);
        assert_eq!(reopened.records().len(), 3);
    }

    #[test]
    fn writes_envelope_v2_and_preserves_tombstones() {
        let (_dir, path, clock) = setup();
        let tomb = Tombstone {
            id: uuid::Uuid::new_v4(),
            at: unix(NOW_UNIX - 86_400.0),
        };
        let mut seed = MergeableEnvelope::<UsageRecord>::new();
        seed.deleted = vec![tomb.clone()];
        seed.cleared_before = Some(unix(NOW_UNIX - 1000.0));
        fs::write(&path, seed.encode().unwrap()).unwrap();
        let (mut store, _) = open(&path, &clock);
        store.append(record(unix(NOW_UNIX))).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        let envelope = read_envelope(&path);
        assert_eq!(envelope.deleted, vec![tomb]);
        assert_eq!(envelope.cleared_before, Some(unix(NOW_UNIX - 1000.0)));
    }
}
