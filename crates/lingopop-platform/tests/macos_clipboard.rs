//! Owner-run native pasteboard tests. Every test uses a PRIVATE, uniquely named pasteboard
//! (`NSPasteboard.pasteboardWithUniqueName`), never the user's general pasteboard, and is ignored
//! by default because it still talks to AppKit and the pasteboard server. Run by hand:
//!
//! `cargo test -p lingopop-platform --test macos_clipboard --locked -- --ignored --test-threads=1`
#![cfg(target_os = "macos")]

use lingopop_platform::pasteboard::{
    PasteboardBackend, PasteboardClipboard, CONCEALED_TYPE, TEXT_TYPE, TRANSIENT_TYPE,
    UNSUPPORTED_SNAPSHOT,
};
use lingopop_platform::{Clipboard, NsPasteboard, PlatformError};

type Item = Vec<(String, Vec<u8>)>;

fn item(types: &[(&str, &[u8])]) -> Item {
    types
        .iter()
        .map(|(t, d)| ((*t).to_owned(), d.to_vec()))
        .collect()
}

fn dump(pb: &NsPasteboard) -> Vec<Item> {
    pb.item_types()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, types)| {
            types
                .iter()
                .map(|t| (t.clone(), pb.read(i as u32, t).unwrap().expect("readable")))
                .collect()
        })
        .collect()
}

fn seed(pb: &NsPasteboard, items: &[Item]) {
    pb.clear();
    pb.write_items(items).unwrap();
}

#[test]
#[ignore = "owner-run: touches the real pasteboard server (private unique pasteboard)"]
fn native_two_items_roundtrip_keeps_boundaries_and_bytes() {
    let pb = NsPasteboard::unique_for_tests();
    let clip = PasteboardClipboard::with_backend(pb.clone());
    let original = vec![
        item(&[
            (TEXT_TYPE, b"hello"),
            ("public.rtf", b"{\\rtf1 hi}"),
            ("public.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 255]),
        ]),
        item(&[
            (TEXT_TYPE, "héllo 🙂".as_bytes()),
            (TRANSIENT_TYPE, b""),
            (CONCEALED_TYPE, b""),
        ]),
    ];
    seed(&pb, &original);
    let snap = clip.snapshot().expect("snapshot");

    seed(&pb, &[item(&[(TEXT_TYPE, b"other")])]);
    clip.restore(&snap).expect("restore");

    let mut restored = dump(&pb);
    let mut expected = original;
    // Type order inside an item is not guaranteed by AppKit; compare per item sorted.
    for it in restored.iter_mut().chain(expected.iter_mut()) {
        it.sort();
    }
    assert_eq!(restored, expected);
}

/// Documents the assumption `last_write_sequence` relies on: writing items after
/// `clearContents` does not move `changeCount` again.
#[test]
#[ignore = "owner-run: touches the real pasteboard server (private unique pasteboard)"]
fn native_clear_return_equals_change_count_after_the_write() {
    let pb = NsPasteboard::unique_for_tests();
    let clip = PasteboardClipboard::with_backend(pb.clone());
    clip.write_text_transient("x").expect("write");
    assert_eq!(clip.last_write_sequence(), Some(pb.change_count()));
}

#[test]
#[ignore = "owner-run: touches the real pasteboard server (private unique pasteboard)"]
fn native_promised_type_aborts_without_touching_anything() {
    let pb = NsPasteboard::unique_for_tests();
    let clip = PasteboardClipboard::with_backend(pb.clone());
    seed(
        &pb,
        &[item(&[
            (TEXT_TYPE, b"hi"),
            ("com.apple.pasteboard.promised-file-url", b"file:///x"),
        ])],
    );
    let before = pb.change_count();
    assert_eq!(
        clip.snapshot().unwrap_err(),
        PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT)
    );
    assert_eq!(pb.change_count(), before);
}

#[test]
#[ignore = "owner-run: touches the real pasteboard server (private unique pasteboard)"]
fn native_changed_sequence_prevents_restore() {
    let pb = NsPasteboard::unique_for_tests();
    let clip = PasteboardClipboard::with_backend(pb.clone());
    seed(&pb, &[item(&[(TEXT_TYPE, b"mine")])]);
    let snap = clip.snapshot().expect("snapshot");
    seed(&pb, &[item(&[(TEXT_TYPE, b"foreign")])]);
    assert_eq!(clip.restore_if(&snap, snap.sequence), Ok(false));
    assert_eq!(clip.read_text().unwrap().as_deref(), Some("foreign"));
}

#[test]
#[ignore = "owner-run: touches the real pasteboard server (private unique pasteboard)"]
fn native_transient_write_sets_both_markers_on_the_text_item() {
    let pb = NsPasteboard::unique_for_tests();
    let clip = PasteboardClipboard::with_backend(pb.clone());
    clip.write_text_transient("secret").expect("write");
    let mut items = dump(&pb);
    assert_eq!(items.len(), 1);
    items[0].sort();
    let mut expected = item(&[
        (TEXT_TYPE, b"secret"),
        (TRANSIENT_TYPE, b""),
        (CONCEALED_TYPE, b""),
    ]);
    expected.sort();
    assert_eq!(items[0], expected);
}
