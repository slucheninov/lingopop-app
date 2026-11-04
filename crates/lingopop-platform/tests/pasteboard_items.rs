//! The pasteboard snapshot / restore engine over `FakePasteboard`, on every OS: item boundaries,
//! the never-read skip list, the all-or-nothing rule and sequence-guarded restoration.

use std::sync::Arc;

use lingopop_platform::fake::{
    FakeClipboard, FakeForeground, FakeKeys, FakePasteboard, FakeSleeper, KeyCall,
};
use lingopop_platform::pasteboard::{
    collect, is_unreadable_by_name, PasteboardClipboard, CONCEALED_TYPE, MAX_TYPE_BYTES,
    TRANSIENT_TYPE, UNSUPPORTED_SNAPSHOT,
};
use lingopop_platform::{
    capture_selection, replace_selection, CaptureOptions, Clipboard, ClipboardEntry,
    ClipboardFormat, ClipboardSnapshot, PlatformError, ReplaceOptions, ReplaceOutcome,
    WindowHandle,
};

const PROMISED: &str = "com.apple.pasteboard.promised-file-url";
const TEXT: &str = "public.utf8-plain-text";
const TARGET: WindowHandle = WindowHandle(7);

type Item = Vec<(String, Vec<u8>)>;

fn item(types: &[(&str, &[u8])]) -> Item {
    types
        .iter()
        .map(|(t, d)| ((*t).to_owned(), d.to_vec()))
        .collect()
}

fn native(item: u32, name: &str) -> ClipboardFormat {
    ClipboardFormat::NativePasteboard {
        item,
        type_name: name.to_owned(),
    }
}

/// Everything `capture_selection` / `replace_selection` need around a pasteboard.
struct Rig {
    pb: FakePasteboard,
    clip: PasteboardClipboard<FakePasteboard>,
    keys: Arc<FakeKeys>,
    fg: Arc<FakeForeground>,
    sleeper: Arc<FakeSleeper>,
}

fn rig(items: Vec<Item>) -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    // `FakeKeys` records into the log of the (unused) text clipboard it is built around.
    let unused = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(unused, sleeper.clone());
    let pb = FakePasteboard::with_items(items);
    Rig {
        clip: PasteboardClipboard::with_backend(pb.clone()),
        pb,
        keys,
        fg: FakeForeground::new(Some(WindowHandle(1))),
        sleeper,
    }
}

fn send_copy_count(r: &Rig) -> usize {
    r.keys
        .events()
        .iter()
        .filter(|e| **e == KeyCall::SendCopy)
        .count()
}

fn capture(r: &Rig) -> Result<Option<String>, PlatformError> {
    capture_selection(
        &r.clip,
        &*r.keys,
        &*r.fg,
        &*r.sleeper,
        &CaptureOptions::default(),
    )
}

fn unsupported() -> PlatformError {
    PlatformError::Unsupported("pasteboard snapshot")
}

#[test]
fn unsupported_constant_is_the_documented_text() {
    assert_eq!(UNSUPPORTED_SNAPSHOT, "pasteboard snapshot");
}

#[test]
fn two_items_roundtrip_byte_for_byte_and_keep_boundaries() {
    let original = vec![
        item(&[
            (TEXT, b"hello"),
            ("public.rtf", b"{\\rtf1 hi}"),
            ("public.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 255]),
            ("public.file-url", b"file:///tmp/a.txt"),
            (TRANSIENT_TYPE, b""),
        ]),
        item(&[(TEXT, "héllo 🙂".as_bytes()), (CONCEALED_TYPE, b"")]),
    ];
    let r = rig(original.clone());
    let snap = r.clip.snapshot().unwrap();

    assert!(snap.skipped.is_empty());
    assert_eq!(snap.sequence, r.pb.change_count());
    let shape: Vec<(ClipboardFormat, Vec<u8>)> = snap
        .entries
        .iter()
        .map(|e| (e.format.clone(), e.data.clone()))
        .collect();
    let expected: Vec<(ClipboardFormat, Vec<u8>)> = original
        .iter()
        .enumerate()
        .flat_map(|(i, it)| {
            it.iter()
                .map(move |(t, d)| (native(i as u32, t), d.clone()))
        })
        .collect();
    assert_eq!(shape, expected);

    r.pb.foreign_write(vec![item(&[(TEXT, b"other")])]);
    r.clip.restore(&snap).unwrap();
    assert_eq!(r.pb.items(), original);
}

#[test]
fn empty_pasteboard_restores_as_empty() {
    let r = rig(Vec::new());
    let snap = r.clip.snapshot().unwrap();
    assert!(snap.entries.is_empty());
    r.pb.foreign_write(vec![item(&[(TEXT, b"x")])]);
    r.clip.restore(&snap).unwrap();
    assert!(r.pb.items().is_empty());
}

#[test]
fn transient_and_concealed_markers_survive_a_restore() {
    let original = vec![item(&[
        (TEXT, b"secret"),
        (TRANSIENT_TYPE, b""),
        (CONCEALED_TYPE, b""),
    ])];
    let r = rig(original.clone());
    let snap = r.clip.snapshot().unwrap();
    r.clip.write_text_transient("tmp").unwrap();
    r.clip.restore(&snap).unwrap();
    assert_eq!(r.pb.items(), original);
}

#[test]
fn transient_write_marks_the_text_item_and_read_text_is_independent() {
    let r = rig(vec![item(&[("public.png", b"img")])]);
    r.clip.write_text_transient("x").unwrap();
    assert_eq!(
        r.pb.items(),
        vec![item(&[
            (TEXT, b"x"),
            (TRANSIENT_TYPE, b""),
            (CONCEALED_TYPE, b"")
        ])]
    );
    assert_eq!(r.clip.read_text().unwrap().as_deref(), Some("x"));

    r.clip.write_text("plain").unwrap();
    assert_eq!(r.pb.items(), vec![item(&[(TEXT, b"plain")])]);
}

#[test]
fn skip_list_matches_promised_file_promise_and_dyn_types_only() {
    for name in [
        "com.apple.pasteboard.promised-file-url",
        "com.apple.pasteboard.promised-suggested-file-name",
        "NSFilePromiseReceiver",
        "com.apple.NSFilePromiseItemMetaData",
        "dyn.ah62d4rv4gu8yc6durvwwaznwmuuha2pxsvw0e55bsmwca7d3sbwu",
    ] {
        assert!(is_unreadable_by_name(name), "{name}");
    }
    for name in [
        TEXT,
        "public.png",
        "public.file-url",
        "com.apple.pasteboard.other",
    ] {
        assert!(!is_unreadable_by_name(name), "{name}");
    }
}

/// Regression anchor: promised types are never read and make the snapshot abort before any copy.
#[test]
fn promised_type_aborts_without_read() {
    let big = vec![0u8; MAX_TYPE_BYTES + 1024 * 1024];
    let r = rig(vec![
        item(&[(TEXT, b"hi"), (PROMISED, b"file:///x")]),
        item(&[("public.png", &big)]),
    ]);

    let collected = collect(&r.pb).unwrap();
    let promised_read_calls = r.pb.read_count_of(PROMISED);
    assert_eq!(promised_read_calls, 0);
    assert!(collected.snapshot.skipped.contains(&PROMISED.to_string()));
    assert!(!collected.complete);

    let first_fixture_result = capture(&r);
    assert_eq!(first_fixture_result, Err(unsupported()));
    assert_eq!((send_copy_count(&r), r.pb.clear_calls()), (0, 0));
    assert_eq!(r.pb.read_count_of(PROMISED), 0);

    let only_promised = rig(vec![item(&[(PROMISED, b"file:///x")])]);
    assert_eq!(capture(&only_promised), Err(unsupported()));
    assert_eq!(
        (
            send_copy_count(&only_promised),
            only_promised.pb.clear_calls()
        ),
        (0, 0)
    );
    assert_eq!(only_promised.pb.read_count_of(PROMISED), 0);
}

#[test]
fn dyn_and_file_promise_types_abort_without_read() {
    for name in [
        "dyn.ah62d4rv4gu8yc6durvwwaznwmuuha2pxsvw0e55bsmwca7d3sbwu",
        "NSFilePromiseReceiver",
    ] {
        let r = rig(vec![item(&[(TEXT, b"hi"), (name, b"x")])]);
        assert_eq!(capture(&r), Err(unsupported()), "{name}");
        assert_eq!(r.pb.read_count_of(name), 0, "{name}");
        assert_eq!((send_copy_count(&r), r.pb.clear_calls()), (0, 0));
    }
}

#[test]
fn oversized_type_aborts_before_copy() {
    let exactly = vec![1u8; MAX_TYPE_BYTES];
    let ok = rig(vec![item(&[("public.png", &exactly)])]);
    assert!(ok.clip.snapshot().is_ok(), "the cap itself is allowed");

    let big = vec![1u8; MAX_TYPE_BYTES + 1];
    let r = rig(vec![item(&[(TEXT, b"hi"), ("public.png", &big)])]);
    let collected = collect(&r.pb).unwrap();
    assert!(!collected.complete);
    assert!(collected
        .snapshot
        .skipped
        .contains(&"public.png".to_string()));
    assert!(
        collected
            .snapshot
            .entries
            .iter()
            .all(|e| e.data.len() <= MAX_TYPE_BYTES),
        "oversized bytes are dropped at once"
    );
    assert_eq!(capture(&r), Err(unsupported()));
    assert_eq!((send_copy_count(&r), r.pb.clear_calls()), (0, 0));
}

#[test]
fn unreadable_type_next_to_readable_text_aborts_before_copy() {
    let r = rig(vec![item(&[(TEXT, b"hi"), ("public.png", b"img")])]);
    r.pb.set_unreadable(0, "public.png");
    assert_eq!(capture(&r), Err(unsupported()));
    assert_eq!((send_copy_count(&r), r.pb.clear_calls()), (0, 0));
    assert!(r.clip.snapshot().is_err_and(|e| e == unsupported()));
}

#[test]
fn snapshot_aborts_on_concurrent_foreign_write() {
    let r = rig(vec![item(&[(TEXT, b"hi")])]);
    // Every read of data is followed by "another application" touching the pasteboard.
    r.pb.on_read(Box::new(|pb| pb.foreign_touch()));
    assert_eq!(r.clip.snapshot().unwrap_err(), PlatformError::ClipboardBusy);
    assert_eq!(r.pb.read_calls().len(), 3, "three attempts, then give up");
    assert_eq!(r.pb.clear_calls(), 0);
}

#[test]
fn snapshot_retries_a_single_foreign_write_and_then_succeeds() {
    let r = rig(vec![item(&[(TEXT, b"hi")])]);
    let touched = std::sync::atomic::AtomicBool::new(false);
    r.pb.on_read(Box::new(move |pb| {
        if !touched.swap(true, std::sync::atomic::Ordering::SeqCst) {
            pb.foreign_touch();
        }
    }));
    let snap = r.clip.snapshot().unwrap();
    assert_eq!(snap.sequence, r.pb.change_count());
    assert_eq!(r.pb.read_calls().len(), 2);
}

#[test]
fn a_changed_sequence_prevents_restore() {
    let original = vec![item(&[(TEXT, b"mine")])];
    let r = rig(original.clone());
    let snap = r.clip.snapshot().unwrap();
    r.pb.foreign_write(vec![item(&[(TEXT, b"foreign")])]);
    assert_eq!(r.clip.restore_if(&snap, snap.sequence), Ok(false));
    assert_eq!(r.pb.clear_calls(), 0);
    assert_eq!(r.pb.items(), vec![item(&[(TEXT, b"foreign")])]);

    // The matching sequence restores.
    let now = r.clip.sequence_number();
    assert_eq!(r.clip.restore_if(&snap, now), Ok(true));
    assert_eq!(r.pb.items(), original);
}

#[test]
fn restore_refuses_an_incomplete_snapshot_before_clearing() {
    let r = rig(vec![item(&[(TEXT, b"mine")])]);
    let snap = ClipboardSnapshot {
        entries: vec![ClipboardEntry {
            format: native(0, TEXT),
            data: b"mine".to_vec(),
        }],
        skipped: vec!["public.png".into()],
        sequence: r.pb.change_count(),
    };
    assert_eq!(r.clip.restore(&snap), Err(unsupported()));
    let foreign_format = ClipboardSnapshot {
        entries: vec![ClipboardEntry {
            format: ClipboardFormat::UnicodeText,
            data: b"x".to_vec(),
        }],
        skipped: Vec::new(),
        sequence: 1,
    };
    assert_eq!(r.clip.restore(&foreign_format), Err(unsupported()));
    assert_eq!(r.pb.clear_calls(), 0);
}

#[test]
fn own_write_sequence_is_the_value_returned_by_clear() {
    let r = rig(vec![item(&[(TEXT, b"mine")])]);
    assert_eq!(r.clip.last_write_sequence(), None);
    r.clip.write_text_transient("x").unwrap();
    assert_eq!(r.clip.last_write_sequence(), Some(r.pb.change_count()));
    assert_eq!(r.clip.last_write_sequence(), Some(r.clip.sequence_number()));
}

#[test]
fn partial_own_write_reports_failure_and_the_guard_restores() {
    let original = vec![
        item(&[(TEXT, b"mine"), ("public.rtf", b"{\\rtf1}")]),
        item(&[("public.png", b"img")]),
    ];
    let r = rig(original.clone());
    r.pb.fail_next_write();
    let result = replace_selection(
        &*r.fg,
        &r.clip,
        &*r.keys,
        &*r.sleeper,
        TARGET,
        "NEW",
        &ReplaceOptions::default(),
    );
    assert!(
        matches!(result, Err(PlatformError::Os { .. })),
        "{result:?}"
    );
    assert_eq!(r.pb.items(), original, "the guard put the snapshot back");
}

#[test]
fn replace_restores_the_snapshot_after_a_paste() {
    let original = vec![item(&[(TEXT, b"mine")]), item(&[("public.png", b"img")])];
    let r = rig(original.clone());
    assert_eq!(
        replace_selection(
            &*r.fg,
            &r.clip,
            &*r.keys,
            &*r.sleeper,
            TARGET,
            "NEW",
            &ReplaceOptions::default(),
        ),
        Ok(ReplaceOutcome::Pasted)
    );
    assert_eq!(r.pb.items(), original);
}

#[test]
fn replace_does_not_clobber_a_foreign_write_after_the_paste() {
    let r = rig(vec![item(&[(TEXT, b"mine")])]);
    let pb = r.pb.clone();
    r.keys.on_paste(Box::new(move |_| {
        pb.foreign_write(vec![item(&[(TEXT, b"foreign")])]);
    }));
    assert_eq!(
        replace_selection(
            &*r.fg,
            &r.clip,
            &*r.keys,
            &*r.sleeper,
            TARGET,
            "NEW",
            &ReplaceOptions::default(),
        ),
        Ok(ReplaceOutcome::Pasted)
    );
    assert_eq!(r.pb.items(), vec![item(&[(TEXT, b"foreign")])]);
}

#[test]
fn capture_logs_the_failed_snapshot_without_contents() {
    let r = rig(vec![item(&[
        (TEXT, b"SECRET-TEXT"),
        (PROMISED, b"SECRET-URL"),
    ])]);
    let lines = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let opts = CaptureOptions {
        log: Some(Arc::new(move |m: &str| {
            sink.lock().unwrap().push(m.to_owned())
        })),
        ..CaptureOptions::default()
    };
    let result = capture_selection(&r.clip, &*r.keys, &*r.fg, &*r.sleeper, &opts);
    assert_eq!(result, Err(unsupported()));
    let lines = lines.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("clipboard snapshot failed")),
        "{lines:?}"
    );
    for l in lines.iter() {
        assert!(!l.contains("SECRET"), "{l}");
    }
}

#[test]
fn debug_prints_counts_and_allowed_type_names_only() {
    let secret_type = "com.acme-merger.client-list";
    let snap = ClipboardSnapshot {
        entries: vec![
            ClipboardEntry {
                format: native(0, "public.png"),
                data: b"SECRET-BYTES".to_vec(),
            },
            ClipboardEntry {
                format: native(1, secret_type),
                data: b"SECRET-BYTES".to_vec(),
            },
        ],
        skipped: vec![secret_type.to_owned()],
        sequence: 5,
    };
    let shown = format!("{snap:?} {:?} {:?}", snap.entries[0], snap.entries[1]);
    assert!(!shown.contains("SECRET-BYTES"), "{shown}");
    assert!(!shown.contains("acme-merger"), "{shown}");
    assert!(shown.contains("public.png"), "{shown}");
    assert!(
        shown.contains("entries: 2") && shown.contains("skipped: 1"),
        "{shown}"
    );
    // The fake itself is not a place contents may leak either.
    let r = rig(vec![item(&[(TEXT, b"SECRET-BYTES")])]);
    assert!(!format!("{:?}", r.pb).contains("SECRET-BYTES"));
}

#[test]
fn clipboard_format_stays_hashable_and_clonable_without_copy() {
    use std::collections::HashSet;
    let a = native(0, TEXT);
    let mut set = HashSet::new();
    set.insert(a.clone());
    assert!(set.contains(&a));
    assert_ne!(a, native(1, TEXT));
    assert_ne!(a, ClipboardFormat::UnicodeText);
}
