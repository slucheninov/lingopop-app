//! The macOS dev implementation (runs on the macOS CI leg and on the dev Mac). The pasteboard test
//! touches the user's real clipboard and is `#[ignore]`; the owner runs it locally.
#![cfg(target_os = "macos")]

use lingopop_platform::{native, CapturedImage, Clipboard, MacClipboard, PlatformError, Rect};

#[test]
fn native_bundle_on_macos_capture_never_reaches_the_screen_before_setup() {
    let platform = native();
    // Before shell setup the topology cache is empty, so a granted permission ends in
    // InvalidRegion and a missing one in PermissionDenied: either way nothing is captured
    // (the state read never prompts).
    let result = platform.capture.capture_region(Rect::new(0, 0, 10, 10));
    assert!(
        matches!(
            result,
            Err(PlatformError::InvalidRegion | PlatformError::PermissionDenied(_))
        ),
        "{result:?}"
    );
    let image = CapturedImage::new(1, 1, vec![0, 0, 0, 255]).unwrap();
    assert_eq!(
        platform.ocr.recognize(&image).map(|_| ()),
        Err(PlatformError::Unsupported("OCR on macOS"))
    );
    // Construction must not access the real login keychain.
}

#[test]
fn mac_monitors_start_with_an_empty_cache() {
    // Native topology is initialized only in main-thread shell setup. Construction and
    // worker reads are display-independent, including on headless CI.
    let monitors = native().monitors.monitors().unwrap();
    assert!(monitors.is_empty());
}

#[test]
#[ignore = "touches the real pasteboard"]
fn mac_clipboard_roundtrip_text() {
    let clip = MacClipboard::new();
    let original = clip.read_text().unwrap();
    let snapshot = clip.snapshot().unwrap();
    assert_eq!(snapshot.sequence, clip.sequence_number());

    clip.write_text_transient("x").unwrap();
    assert_ne!(clip.sequence_number(), snapshot.sequence);
    assert_eq!(clip.read_text().unwrap().as_deref(), Some("x"));

    clip.restore(&snapshot).unwrap();
    assert_eq!(clip.read_text().unwrap(), original);
}
