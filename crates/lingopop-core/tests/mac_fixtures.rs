//! Checks the Rust types against files produced by the real macOS Swift models
//! (`scripts/mac-fixtures/run.sh generate`). The fixtures are generated, never hand-edited.

use std::path::PathBuf;
use std::sync::Arc;

use lingopop_core::envelope::MergeableEnvelope;
use lingopop_core::languages;
use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::{HistoryStore, LoadOutcome, ManualClock, Timestamp};
use serde_json::Value;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mac");

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

fn fixture_value(name: &str) -> Value {
    serde_json::from_slice(&fixture_bytes(name))
        .unwrap_or_else(|e| panic!("{name} is not JSON: {e}"))
}

fn ts(seconds: f64) -> Timestamp {
    Timestamp::from_apple_seconds(seconds)
}

fn decode_history(name: &str) -> (MergeableEnvelope<HistoryEntry>, bool) {
    let decoded =
        MergeableEnvelope::<HistoryEntry>::decode(&fixture_bytes(name)).expect("history decodes");
    (decoded.envelope, decoded.is_legacy)
}

fn decode_usage(name: &str) -> (MergeableEnvelope<UsageRecord>, bool) {
    let decoded =
        MergeableEnvelope::<UsageRecord>::decode(&fixture_bytes(name)).expect("usage decodes");
    (decoded.envelope, decoded.is_legacy)
}

#[test]
fn history_fixture_decodes_with_expected_values() {
    let (envelope, is_legacy) = decode_history("history.json");
    assert!(!is_legacy);
    assert_eq!(envelope.version, 2);
    assert_eq!(envelope.items.len(), 4);

    let ids: Vec<String> = envelope.items.iter().map(|e| e.id.to_string()).collect();
    assert_eq!(
        ids,
        [
            "16990d16-7e15-47c3-a299-ef2aec4400d1",
            "9b1f5c3a-2d44-4e8a-8f0b-6a7c1e2d3f40",
            "0a2b4c6d-8e0f-4a1b-9c3d-5e7f90a1b2c3",
            "d4e5f6a7-b8c9-4d0e-a1b2-c3d4e5f60718",
        ]
    );
    let providers: Vec<&str> = envelope.items.iter().map(|e| e.provider.as_str()).collect();
    assert_eq!(
        providers,
        ["Claude", "Google (Free)", "Microsoft Translator", "DeepL"]
    );
    let operations: Vec<&str> = envelope
        .items
        .iter()
        .map(|e| e.operation.as_str())
        .collect();
    assert_eq!(
        operations,
        ["translate", "screenTranslate", "magic", "grammarFix"]
    );
    assert!(envelope
        .items
        .iter()
        .all(|e| e.provider_type().is_some() && e.operation_type().is_some()));

    let timestamps: Vec<f64> = envelope
        .items
        .iter()
        .map(|e| e.timestamp.apple_seconds())
        .collect();
    assert_eq!(
        timestamps,
        [815059756.035193, 815059700.5, 815059000.0, 815058000.25]
    );

    assert_eq!(
        envelope.items[0].source_language.as_deref(),
        Some("English")
    );
    assert_eq!(envelope.items[1].source_language, None);
    assert_eq!(envelope.items[0].original, "Hello, world");
    assert_eq!(
        envelope.items[1].original,
        "Привіт 👋\n\"світ\" <b>&</b> a/b"
    );

    assert_eq!(envelope.deleted.len(), 2);
    assert_eq!(
        envelope.deleted[0].id.to_string(),
        "aaaaaaaa-1111-4222-8333-444444444444"
    );
    assert_eq!(envelope.deleted[0].at, ts(815059111.125));
    assert_eq!(envelope.deleted[1].at, ts(815058222.5));
    assert_eq!(envelope.cleared_before, Some(ts(800000000.5)));
}

#[test]
fn history_fixture_rust_encoding_equals_swift_json() {
    let (envelope, _) = decode_history("history.json");
    let rust = serde_json::to_value(&envelope).unwrap();
    // The whole-second timestamp (815059000) must serialize as an integer: `Value` equality
    // distinguishes 815059000 from 815059000.0.
    assert_eq!(rust, fixture_value("history.json"));
    let encoded: Value = serde_json::from_slice(&envelope.encode().unwrap()).unwrap();
    assert_eq!(encoded, fixture_value("history.json"));
}

#[test]
fn history_fixture_round_trip_equality() {
    let (envelope, _) = decode_history("history.json");
    let again = MergeableEnvelope::<HistoryEntry>::decode(&envelope.encode().unwrap()).unwrap();
    assert!(!again.is_legacy);
    assert_eq!(again.envelope, envelope);
}

#[test]
fn history_legacy_fixture_is_detected_as_legacy() {
    let (legacy, is_legacy) = decode_history("history.legacy.json");
    let (current, _) = decode_history("history.json");
    assert!(is_legacy);
    assert_eq!(legacy.items, current.items);
    assert!(legacy.deleted.is_empty());
    assert_eq!(legacy.cleared_before, None);
}

#[test]
fn usage_fixture_decodes_encodes_and_round_trips() {
    let (envelope, is_legacy) = decode_usage("usage.json");
    assert!(!is_legacy);
    assert_eq!(envelope.items.len(), 3);

    let r = &envelope.items;
    assert_eq!(r[0].id.to_string(), "432f3827-40a1-430b-a818-67b2b6a36970");
    assert_eq!(
        (
            r[0].provider.as_str(),
            r[0].model.as_str(),
            r[0].operation.as_str()
        ),
        ("Claude", "claude-haiku-5-5", "translate")
    );
    assert_eq!((r[0].input_tokens, r[0].output_tokens), (120, 45));
    assert_eq!(r[0].timestamp, ts(815059756.035515));
    assert_eq!(
        (
            r[1].provider.as_str(),
            r[1].model.as_str(),
            r[1].operation.as_str()
        ),
        ("Google Translate", "nmt", "screenTranslate")
    );
    assert_eq!((r[1].input_tokens, r[1].output_tokens), (0, 0));
    assert_eq!(
        (
            r[2].provider.as_str(),
            r[2].model.as_str(),
            r[2].operation.as_str()
        ),
        ("OpenAI", "gpt-6-luna", "magic")
    );
    assert_eq!((r[2].input_tokens, r[2].output_tokens), (1500, 800));
    assert_eq!(r[2].timestamp, ts(815060000.0));
    assert_eq!(envelope.deleted.len(), 1);
    assert_eq!(envelope.cleared_before, Some(ts(810000000.25)));

    assert_eq!(
        serde_json::to_value(&envelope).unwrap(),
        fixture_value("usage.json")
    );
    let again = MergeableEnvelope::<UsageRecord>::decode(&envelope.encode().unwrap()).unwrap();
    assert_eq!(again.envelope, envelope);
}

fn str_field<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {row}"))
}

fn opt_str_field<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    match &row[key] {
        Value::Null => None,
        Value::String(s) => Some(s.as_str()),
        other => panic!("{key} has unexpected value {other}"),
    }
}

#[test]
fn languages_match_swift_golden() {
    let golden = fixture_value("languages.golden.json");
    let rows = golden["languages"].as_array().expect("languages array");
    assert_eq!(rows.len(), languages::ALL.len());
    for (row, rust) in rows.iter().zip(languages::ALL) {
        assert_eq!(str_field(row, "name"), rust.name);
        assert_eq!(str_field(row, "code"), rust.code, "{}", rust.name);
        assert_eq!(
            opt_str_field(row, "deepLCode"),
            rust.deepl_code,
            "{}",
            rust.name
        );
        assert_eq!(
            opt_str_field(row, "microsoftCode"),
            rust.microsoft_code,
            "{}",
            rust.name
        );
        assert_eq!(
            row["inQuickList"].as_bool(),
            Some(rust.in_quick_list),
            "{}",
            rust.name
        );
    }

    let helpers = golden["helpers"].as_array().expect("helpers array");
    assert_eq!(helpers.len(), languages::ALL.len() + 1);
    assert_eq!(str_field(helpers.last().unwrap(), "name"), "Klingon");
    for row in helpers {
        let name = str_field(row, "name");
        assert_eq!(languages::code(name), str_field(row, "code"), "{name}");
        assert_eq!(
            languages::deepl_code(name),
            str_field(row, "deepLCode"),
            "{name}"
        );
        assert_eq!(
            languages::microsoft_code(name),
            str_field(row, "microsoftCode"),
            "{name}"
        );
    }
}

#[test]
fn store_opens_swift_written_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    std::fs::copy(fixture_path("history.json"), &path).unwrap();

    let clock = Arc::new(ManualClock::new(ts(815060000.0)));
    let (store, outcome) = HistoryStore::open(path, clock).unwrap();
    assert_eq!(outcome, LoadOutcome::Loaded);
    assert_eq!(store.entries().len(), 4);
    let stamps: Vec<Timestamp> = store.entries().iter().map(|e| e.timestamp).collect();
    let mut sorted = stamps.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(stamps, sorted, "newest first");
}

/// Manual gate (`scripts/mac-fixtures/run.sh check <dir>`): writes the Rust-encoded files so Swift
/// can decode them. `secrets.payload.json` is added with the secrets module.
#[test]
#[ignore = "writes files for the Swift reverse check; run explicitly"]
fn write_rust_outputs_for_swift_check() {
    let dir = std::env::var_os("LINGOPOP_ROUNDTRIP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/mac-roundtrip"
            ))
        });
    std::fs::create_dir_all(&dir).unwrap();

    let (history, _) = decode_history("history.json");
    std::fs::write(dir.join("history.json"), history.encode().unwrap()).unwrap();
    let (usage, _) = decode_usage("usage.json");
    std::fs::write(dir.join("usage.json"), usage.encode().unwrap()).unwrap();
}
