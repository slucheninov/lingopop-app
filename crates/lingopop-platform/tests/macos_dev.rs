//! The macOS dev implementation (runs on the macOS CI leg and on the dev Mac). The pasteboard test
//! touches the user's real clipboard and is `#[ignore]`; the owner runs it locally.
#![cfg(target_os = "macos")]

use lingopop_platform::{native, CapturedImage, Clipboard, MacClipboard, PlatformError, Rect};

#[test]
fn native_bundle_on_macos_has_unsupported_capture_and_ocr() {
    let platform = native();
    assert_eq!(
        platform.capture.capture_region(Rect::new(0, 0, 10, 10)),
        Err(PlatformError::Unsupported("screen capture on macOS"))
    );
    let image = CapturedImage::new(1, 1, vec![0, 0, 0, 255]).unwrap();
    assert_eq!(
        platform.ocr.recognize(&image).map(|_| ()),
        Err(PlatformError::Unsupported("OCR on macOS"))
    );
    assert_eq!(platform.protector.protect(b"abc").unwrap(), b"abc");
}

#[test]
fn mac_monitors_report_a_main_display() {
    let monitors = native().monitors.monitors().unwrap();
    assert!(!monitors.is_empty(), "no active display");
    for m in &monitors {
        assert!(!m.bounds.is_empty(), "{m:?}");
        assert_eq!(m.bounds, m.work_area);
        assert!(m.scale_factor >= 1.0, "{m:?}");
    }
    assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1);
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
