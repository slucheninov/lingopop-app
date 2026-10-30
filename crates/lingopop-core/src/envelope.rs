//! Versioned `history.json` / `usage.json` file format and the pure merge logic,
//! ported from the macOS app's `Core/CloudMerge.swift` (`MergeableEnvelope`).
//!
//! ```text
//! { "version": 2, "items": [...], "deleted": [{"id": "...", "at": ...}], "clearedBefore": ... }
//! ```
//!
//! Nothing is synced on Windows, so only the pure part is ported: the stores use
//! `mark_deleted` / `mark_cleared` / `pruned`; `merged` is kept because the spec
//! mandates it. Merge = union of items by id, minus tombstoned ids, minus items
//! not newer than `clearedBefore`, union of tombstones (latest `at` wins) and the
//! later watermark. Tombstones older than 90 days are pruned.

use std::collections::{HashMap, HashSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fsio::strip_bom;
use crate::records::{HistoryEntry, UsageRecord};
use crate::serde_uuid;
use crate::time::Timestamp;

/// Tombstones older than this (90 days) are dropped.
pub const TOMBSTONE_RETENTION_SECONDS: f64 = 7_776_000.0;

/// The envelope version this build writes.
const CURRENT_VERSION: u32 = 2;

/// An immutable, UUID-identified record that can be merged (history entries, usage records).
pub trait Mergeable {
    fn id(&self) -> Uuid;
    /// Creation time; compared against `MergeableEnvelope::cleared_before`.
    fn merge_timestamp(&self) -> Timestamp;
}

impl Mergeable for HistoryEntry {
    fn id(&self) -> Uuid {
        self.id
    }
    fn merge_timestamp(&self) -> Timestamp {
        self.timestamp
    }
}

impl Mergeable for UsageRecord {
    fn id(&self) -> Uuid {
        self.id
    }
    fn merge_timestamp(&self) -> Timestamp {
        self.timestamp
    }
}

/// Marks an id as deleted at `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tombstone {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    pub at: Timestamp,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeableEnvelope<T> {
    #[serde(default = "default_version")]
    pub version: u32,
    pub items: Vec<T>,
    #[serde(default)]
    pub deleted: Vec<Tombstone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_before: Option<Timestamp>,
}

/// Result of [`MergeableEnvelope::decode`]; `is_legacy` is true for a bare array.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedEnvelope<T> {
    pub envelope: MergeableEnvelope<T>,
    pub is_legacy: bool,
}

/// Borrowed write form: `version` is always 2 and `deleted` is always present
/// (Swift's synthesized decoder requires both).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EncodedEnvelope<'a, T> {
    version: u32,
    items: &'a [T],
    deleted: &'a [Tombstone],
    #[serde(skip_serializing_if = "Option::is_none")]
    cleared_before: Option<Timestamp>,
}

/// Single retention rule shared by `merged` and `pruned`; `>=` keeps a tombstone
/// exactly 90 days old and any future-dated one.
fn within_retention(at: Timestamp, now: Timestamp) -> bool {
    at >= now.plus_seconds(-TOMBSTONE_RETENTION_SECONDS)
}

impl<T> Default for MergeableEnvelope<T> {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            items: Vec::new(),
            deleted: Vec::new(),
            cleared_before: None,
        }
    }
}

impl<T: Mergeable + Serialize + DeserializeOwned + Clone> MergeableEnvelope<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_items(items: Vec<T>) -> Self {
        Self {
            items,
            ..Self::default()
        }
    }

    /// Decodes the envelope or, failing that, a legacy bare array. A leading
    /// UTF-8 BOM is tolerated. `None` when the bytes are neither.
    pub fn decode(bytes: &[u8]) -> Option<DecodedEnvelope<T>> {
        let bytes = strip_bom(bytes);
        if let Ok(envelope) = serde_json::from_slice::<MergeableEnvelope<T>>(bytes) {
            return Some(DecodedEnvelope {
                envelope,
                is_legacy: false,
            });
        }
        let items = serde_json::from_slice::<Vec<T>>(bytes).ok()?;
        Some(DecodedEnvelope {
            envelope: Self::with_items(items),
            is_legacy: true,
        })
    }

    /// Always writes `version: 2` and `deleted`; omits `clearedBefore` when `None`.
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&EncodedEnvelope {
            version: CURRENT_VERSION,
            items: &self.items,
            deleted: &self.deleted,
            cleared_before: self.cleared_before,
        })
    }

    /// Records tombstones at `at`, replacing any existing tombstone of the same id.
    pub fn mark_deleted(&mut self, ids: &[Uuid], at: Timestamp) {
        let mut unique = HashSet::with_capacity(ids.len());
        let ordered: Vec<Uuid> = ids
            .iter()
            .copied()
            .filter(|id| unique.insert(*id))
            .collect();
        self.deleted.retain(|t| !unique.contains(&t.id));
        self.deleted
            .extend(ordered.into_iter().map(|id| Tombstone { id, at }));
    }

    /// Sets the watermark; it never moves backwards.
    pub fn mark_cleared(&mut self, at: Timestamp) {
        self.cleared_before = Some(self.cleared_before.map_or(at, |current| current.max(at)));
    }

    /// Pure merge; items keep `self` order first, then remote-only items. Callers sort/cap afterwards.
    pub fn merged(&self, other: &Self, now: Timestamp) -> Self {
        let cleared_before = match (self.cleared_before, other.cleared_before) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };

        let mut tombstones: HashMap<Uuid, Timestamp> =
            HashMap::with_capacity(self.deleted.len() + other.deleted.len());
        for t in self.deleted.iter().chain(&other.deleted) {
            tombstones
                .entry(t.id)
                .and_modify(|at| *at = (*at).max(t.at))
                .or_insert(t.at);
        }

        let mut seen = HashSet::with_capacity(self.items.len() + other.items.len());
        let mut items = Vec::new();
        for item in self.items.iter().chain(&other.items) {
            let id = item.id();
            if !seen.insert(id) || tombstones.contains_key(&id) {
                continue;
            }
            if cleared_before.is_some_and(|mark| item.merge_timestamp() <= mark) {
                continue;
            }
            items.push(item.clone());
        }

        let mut deleted: Vec<Tombstone> = tombstones
            .into_iter()
            .filter(|(_, at)| within_retention(*at, now))
            .map(|(id, at)| Tombstone { id, at })
            .collect();
        // Raw UUID bytes order exactly like the uppercase hyphenated string
        // (fixed-width hex, '0'-'9' < 'A'-'F'), without allocating a string per compare.
        deleted.sort_by(|a, b| {
            a.at.cmp(&b.at)
                .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
        });

        Self {
            version: CURRENT_VERSION,
            items,
            deleted,
            cleared_before,
        }
    }

    /// Drops tombstones older than 90 days, nothing else.
    pub fn pruned(&self, now: Timestamp) -> Self {
        Self {
            version: CURRENT_VERSION,
            items: self.items.clone(),
            deleted: self
                .deleted
                .iter()
                .filter(|t| within_retention(t.at, now))
                .cloned()
                .collect(),
            cleared_before: self.cleared_before,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use crate::{AIOperation, HistoryEntry, ProviderType, Timestamp, UsageRecord};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct T {
        #[serde(with = "crate::serde_uuid")]
        id: Uuid,
        ts: Timestamp,
    }

    impl Mergeable for T {
        fn id(&self) -> Uuid {
            self.id
        }
        fn merge_timestamp(&self) -> Timestamp {
            self.ts
        }
    }

    type Env = MergeableEnvelope<T>;

    const DAY: f64 = 86_400.0;

    fn now() -> Timestamp {
        Timestamp::from_unix_seconds(1_800_000_000.0)
    }

    fn at(offset: f64) -> Timestamp {
        now().plus_seconds(offset)
    }

    fn item_at(ts: Timestamp) -> T {
        T {
            id: Uuid::new_v4(),
            ts,
        }
    }

    fn item() -> T {
        item_at(now())
    }

    fn tomb(id: Uuid, ts: Timestamp) -> Tombstone {
        Tombstone { id, at: ts }
    }

    fn env(items: Vec<T>, deleted: Vec<Tombstone>, cleared_before: Option<Timestamp>) -> Env {
        Env {
            version: 2,
            items,
            deleted,
            cleared_before,
        }
    }

    fn ids(items: &[T]) -> Vec<Uuid> {
        items.iter().map(|i| i.id).collect()
    }

    // --- ports of CloudMergeTests ---

    #[test]
    fn merged_unions_items_by_id() {
        let (x, y, z) = (item(), item(), item());
        let y_id = y.id;
        let result = Env::with_items(vec![x.clone(), y.clone()])
            .merged(&Env::with_items(vec![y, z.clone()]), now());
        let got: std::collections::HashSet<Uuid> = ids(&result.items).into_iter().collect();
        assert_eq!(got, [x.id, y_id, z.id].into_iter().collect());
        assert_eq!(result.items.len(), 3);
    }

    #[test]
    fn merged_drops_tombstoned_items() {
        let x = item();
        let t = tomb(x.id, now());
        let result =
            Env::with_items(vec![x.clone()]).merged(&env(vec![], vec![t.clone()], None), now());
        assert!(!result.items.iter().any(|i| i.id == x.id));
        assert_eq!(result.deleted, vec![t]);
    }

    #[test]
    fn merged_drops_items_at_or_before_watermark() {
        let old = item_at(at(-20.0));
        let edge = item_at(at(-10.0));
        let fresh = item_at(at(0.0));
        let mark = at(-10.0);
        let result = Env::with_items(vec![old, edge, fresh.clone()])
            .merged(&env(vec![], vec![], Some(mark)), now());
        assert_eq!(ids(&result.items), vec![fresh.id]);
        assert_eq!(result.cleared_before, Some(mark));
    }

    #[test]
    fn merged_tombstone_latest_at_wins() {
        let id = Uuid::new_v4();
        let a = env(vec![], vec![tomb(id, at(-5.0))], None);
        let b = env(vec![], vec![tomb(id, at(-1.0))], None);
        let result = a.merged(&b, now());
        assert_eq!(result.deleted, vec![tomb(id, at(-1.0))]);
        let reverse = b.merged(&a, now());
        assert_eq!(reverse.deleted, vec![tomb(id, at(-1.0))]);
    }

    #[test]
    fn merged_prunes_tombstones_older_than_retention() {
        let stale = tomb(Uuid::new_v4(), at(-91.0 * DAY));
        let recent = tomb(Uuid::new_v4(), at(-89.0 * DAY));
        let edge = tomb(Uuid::new_v4(), at(-90.0 * DAY));
        let result =
            env(vec![], vec![stale, recent.clone(), edge.clone()], None).merged(&Env::new(), now());
        let mut expected = vec![recent, edge];
        expected.sort_by_key(|a| a.at);
        assert_eq!(result.deleted, expected);
    }

    #[test]
    fn decode_legacy_bare_array() {
        let data = serde_json::to_vec(&vec![item(), item()]).unwrap();
        let decoded = Env::decode(&data).unwrap();
        assert!(decoded.is_legacy);
        assert_eq!(decoded.envelope.items.len(), 2);
        assert!(decoded.envelope.deleted.is_empty());
        assert_eq!(decoded.envelope.cleared_before, None);
        assert_eq!(decoded.envelope.version, 2);
    }

    #[test]
    fn decode_empty_bare_array_is_legacy() {
        let decoded = Env::decode(b"[]").unwrap();
        assert!(decoded.is_legacy);
        assert!(decoded.envelope.items.is_empty());
    }

    #[test]
    fn decode_envelope_round_trip() {
        let t = tomb(Uuid::new_v4(), now());
        let e = env(vec![item(), item()], vec![t.clone()], Some(at(-3.0)));
        let decoded = Env::decode(&e.encode().unwrap()).unwrap();
        assert!(!decoded.is_legacy);
        assert_eq!(decoded.envelope, e);
        assert_eq!(decoded.envelope.deleted, vec![t]);
    }

    #[test]
    fn decode_garbage_returns_none() {
        assert!(Env::decode(b"nope").is_none());
        assert!(Env::decode(b"").is_none());
        assert!(Env::decode(b"{\"items\":5}").is_none());
    }

    #[test]
    fn mark_cleared_only_moves_forward() {
        let mut e = Env::new();
        e.mark_cleared(now());
        e.mark_cleared(at(-10.0));
        assert_eq!(e.cleared_before, Some(now()));
    }

    #[test]
    fn mark_deleted_replaces_existing_tombstone() {
        let id = Uuid::new_v4();
        let mut e = Env::new();
        e.mark_deleted(&[id], at(-5.0));
        e.mark_deleted(&[id], now());
        assert_eq!(e.deleted, vec![tomb(id, now())]);
    }

    #[test]
    fn mark_deleted_dedupes_repeated_ids_and_keeps_other_tombstones() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut e = Env::new();
        e.mark_deleted(&[a], at(-5.0));
        e.mark_deleted(&[b, b, a], now());
        assert_eq!(e.deleted.len(), 2);
        assert!(e.deleted.iter().all(|t| t.at == now()));
    }

    // --- new tests ---

    #[test]
    fn encode_always_writes_version_2_and_deleted_and_omits_cleared_before_when_none() {
        let mut e = Env::new();
        e.version = 7;
        let value: serde_json::Value = serde_json::from_slice(&e.encode().unwrap()).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["deleted"], serde_json::json!([]));
        assert_eq!(value["items"], serde_json::json!([]));
        assert!(value.get("clearedBefore").is_none());

        e.cleared_before = Some(now());
        let value: serde_json::Value = serde_json::from_slice(&e.encode().unwrap()).unwrap();
        assert!(value["clearedBefore"].is_number());
        let decoded = Env::decode(&e.encode().unwrap()).unwrap();
        assert_eq!(decoded.envelope.cleared_before, Some(now()));
    }

    #[test]
    fn decode_tolerates_missing_deleted_and_version() {
        let decoded = Env::decode(br#"{"items":[]}"#).unwrap();
        assert!(!decoded.is_legacy);
        assert_eq!(decoded.envelope.version, 2);
        assert!(decoded.envelope.deleted.is_empty());
        assert_eq!(decoded.envelope.cleared_before, None);
    }

    #[test]
    fn decode_accepts_future_version_number() {
        let decoded = Env::decode(br#"{"version":3,"items":[],"deleted":[]}"#).unwrap();
        assert!(!decoded.is_legacy);
        let value: serde_json::Value =
            serde_json::from_slice(&decoded.envelope.encode().unwrap()).unwrap();
        assert_eq!(value["version"], 2);
    }

    #[test]
    fn decode_strips_utf8_bom() {
        let e = env(vec![item()], vec![tomb(Uuid::new_v4(), now())], None);
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(e.encode().unwrap());
        let decoded = Env::decode(&bytes).unwrap();
        assert!(!decoded.is_legacy);
        assert_eq!(decoded.envelope, e);

        let mut legacy = vec![0xEF, 0xBB, 0xBF];
        legacy.extend(serde_json::to_vec(&vec![item()]).unwrap());
        assert!(Env::decode(&legacy).unwrap().is_legacy);
    }

    #[test]
    fn merged_keeps_self_order_then_remote_only_items() {
        let (a, b, c, d) = (item(), item(), item(), item());
        let mine = Env::with_items(vec![c.clone(), a.clone()]);
        let theirs = Env::with_items(vec![d.clone(), a.clone(), b.clone()]);
        let result = mine.merged(&theirs, now());
        assert_eq!(ids(&result.items), vec![c.id, a.id, d.id, b.id]);
    }

    #[test]
    fn merged_first_occurrence_of_an_id_wins() {
        let id = Uuid::new_v4();
        let mine = T { id, ts: at(-1.0) };
        let theirs = T { id, ts: at(-2.0) };
        let result =
            Env::with_items(vec![mine.clone()]).merged(&Env::with_items(vec![theirs]), now());
        assert_eq!(result.items, vec![mine]);
    }

    #[test]
    fn merged_deleted_sorted_by_at_then_uppercase_uuid_string() {
        let mut tombs = Vec::new();
        for n in 0..40u128 {
            // Few distinct `at` values so the uuid tie-break is exercised.
            let id = Uuid::from_u128(n.wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835));
            tombs.push(tomb(id, at(-((n % 3) as f64))));
        }
        let (left, right) = tombs.split_at(17);
        let result =
            env(vec![], left.to_vec(), None).merged(&env(vec![], right.to_vec(), None), now());
        let mut expected = tombs.clone();
        let upper = |id: Uuid| {
            id.hyphenated()
                .encode_upper(&mut Uuid::encode_buffer())
                .to_owned()
        };
        expected.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| upper(a.id).cmp(&upper(b.id))));
        assert_eq!(result.deleted, expected);
    }

    #[test]
    fn uuid_case_does_not_matter_between_item_and_tombstone() {
        let upper = "16990D16-7E15-47C3-A299-EF2AEC4400D1";
        let lower = upper.to_lowercase();
        let ts = now().apple_seconds();

        let items_lower: Env =
            Env::decode(format!(r#"{{"items":[{{"id":"{lower}","ts":{ts}}}]}}"#).as_bytes())
                .unwrap()
                .envelope;
        let tombs_upper: Env = Env::decode(
            format!(r#"{{"items":[],"deleted":[{{"id":"{upper}","at":{ts}}}]}}"#).as_bytes(),
        )
        .unwrap()
        .envelope;
        let result = items_lower.merged(&tombs_upper, now());
        assert!(result.items.is_empty());
        assert_eq!(result.deleted.len(), 1);

        let items_upper: Env =
            Env::decode(format!(r#"{{"items":[{{"id":"{upper}","ts":{ts}}}]}}"#).as_bytes())
                .unwrap()
                .envelope;
        let tombs_lower: Env = Env::decode(
            format!(r#"{{"items":[],"deleted":[{{"id":"{lower}","at":{ts}}}]}}"#).as_bytes(),
        )
        .unwrap()
        .envelope;
        assert!(items_upper.merged(&tombs_lower, now()).items.is_empty());

        // The written form is uppercase either way.
        let written = String::from_utf8(tombs_lower.encode().unwrap()).unwrap();
        assert!(written.contains(upper));
        assert!(!written.contains(&lower));

        // A legacy array holding the same id in two cases is one id after a merge.
        let legacy = format!(r#"[{{"id":"{lower}","ts":{ts}}},{{"id":"{upper}","ts":{ts}}}]"#);
        let decoded = Env::decode(legacy.as_bytes()).unwrap();
        assert!(decoded.is_legacy);
        assert_eq!(decoded.envelope.items.len(), 2);
        assert_eq!(decoded.envelope.merged(&Env::new(), now()).items.len(), 1);
    }

    #[test]
    fn future_dated_data_is_kept() {
        let future = at(10.0 * 365.0 * DAY);
        let fresh = item_at(future);
        let result =
            Env::with_items(vec![fresh.clone()]).merged(&env(vec![], vec![], Some(now())), now());
        assert_eq!(ids(&result.items), vec![fresh.id]);

        let t = tomb(Uuid::new_v4(), future);
        let result = env(vec![], vec![t.clone()], None).merged(&Env::new(), now());
        assert_eq!(result.deleted, vec![t]);

        let mut e = Env::new();
        e.mark_cleared(future);
        e.mark_cleared(now());
        assert_eq!(e.cleared_before, Some(future));
    }

    #[test]
    fn merge_of_50k_items_and_50k_tombstones_is_linear() {
        let make = |n: u128| T {
            id: Uuid::from_u128(n + 1),
            ts: now(),
        };
        // Items 0..50_000 on self, 25_000..75_000 on other: 75_000 distinct ids.
        let mine_items: Vec<T> = (0..50_000).map(make).collect();
        let theirs_items: Vec<T> = (25_000..75_000).map(make).collect();
        // Tombstones for ids 70_000..120_000 (50_000), split across both sides with an overlap;
        // 5_000 of them (70_000..75_000) hit items.
        let tombs = |range: std::ops::Range<u128>| -> Vec<Tombstone> {
            range.map(|n| tomb(Uuid::from_u128(n + 1), now())).collect()
        };
        let mine = env(mine_items, tombs(70_000..100_000), None);
        let theirs = env(theirs_items, tombs(90_000..120_000), None);
        let result = mine.merged(&theirs, now());
        assert_eq!(result.items.len(), 70_000);
        assert_eq!(result.deleted.len(), 50_000);
        let bytes = result.encode().unwrap();
        assert_eq!(Env::decode(&bytes).unwrap().envelope.items.len(), 70_000);

        // mark_deleted over a large tombstone list stays linear too.
        let mut big = env(vec![], tombs(0..50_000), None);
        let new_ids: Vec<Uuid> = (25_000..75_000u128)
            .map(|n| Uuid::from_u128(n + 1))
            .collect();
        big.mark_deleted(&new_ids, now());
        assert_eq!(big.deleted.len(), 75_000);
    }

    #[test]
    fn pruned_only_drops_old_tombstones() {
        let stale = tomb(Uuid::new_v4(), at(-91.0 * DAY));
        let edge = tomb(Uuid::new_v4(), at(-90.0 * DAY));
        let recent = tomb(Uuid::new_v4(), at(-1.0));
        let old_item = item_at(at(-200.0 * DAY));
        let e = env(
            vec![old_item.clone(), old_item.clone()],
            vec![recent.clone(), stale, edge.clone()],
            Some(at(-300.0 * DAY)),
        );
        let result = e.pruned(now());
        // Items (even duplicates and ones older than the watermark), order of the
        // surviving tombstones and the watermark are untouched.
        assert_eq!(result.items, e.items);
        assert_eq!(result.deleted, vec![recent, edge]);
        assert_eq!(result.cleared_before, e.cleared_before);
        assert_eq!(result.version, 2);
    }

    #[test]
    fn app_records_implement_mergeable() {
        let ts = Timestamp::from_apple_seconds(42.0);
        let h = HistoryEntry::new(
            "a".into(),
            "b".into(),
            None,
            "German".into(),
            ProviderType::Claude,
            AIOperation::Translate,
            ts,
        );
        assert_eq!(Mergeable::id(&h), h.id);
        assert_eq!(h.merge_timestamp(), ts);
        let u = UsageRecord::new(
            ProviderType::Claude,
            "m".into(),
            AIOperation::Translate,
            1,
            2,
            ts,
        );
        assert_eq!(Mergeable::id(&u), u.id);
        assert_eq!(u.merge_timestamp(), ts);
    }
}
