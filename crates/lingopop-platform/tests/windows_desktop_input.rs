//! Optional tests: synthesized input depends on desktop ownership by the runner.
#![cfg(windows)]
#[path = "support/overlay.rs"]
mod overlay;
use lingopop_platform::Rect;
use overlay::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_esc_cancels() {
    let pick = Pick::start();
    escape();
    assert_eq!(pick.finish(), None);
    assert!(window().is_none());
}
fn drag(dx: i32, dy: i32, backwards: bool, interior: bool, steal: bool) -> Option<Rect> {
    let pick = Pick::start();
    let b = bounds();
    let (x, y) = (b.x + 100, b.y + 100);
    let (start, end) = if backwards {
        ((x + dx, y + dy), (x, y))
    } else {
        ((x, y), (x + dx, y + dy))
    };
    mouse(start.0, start.1, MOUSEEVENTF_LEFTDOWN);
    mouse(end.0, end.1, MOUSE_EVENT_FLAGS(0));
    if interior {
        mouse(end.0 + 20, end.1 + 20, MOUSE_EVENT_FLAGS(0));
    }
    if steal {
        use windows::{core::w, Win32::UI::WindowsAndMessaging::*};
        // SAFETY: temporary test window created and destroyed on this thread.
        unsafe {
            let hwnd = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("overlay capture loss"),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                50,
                50,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let _ = SetForegroundWindow(hwnd);
            let _ = DestroyWindow(hwnd);
        }
    }
    mouse(end.0, end.1, MOUSEEVENTF_LEFTUP);
    pick.finish()
}
#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_drag_below_threshold_is_none() {
    assert_eq!(drag(5, 5, false, false, false), None);
}
#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_drag_returns_rect() {
    let r = drag(100, 60, false, false, false).unwrap();
    assert!((r.width - 100).abs() <= 1 && (r.height - 60).abs() <= 1);
}
#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_backward_drag_normalizes() {
    let r = drag(100, 60, true, false, false).unwrap();
    let b = bounds();
    assert!((r.x - b.x - 100).abs() <= 1 && (r.y - b.y - 100).abs() <= 1);
    assert!((r.width - 100).abs() <= 1 && (r.height - 60).abs() <= 1);
}
#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_release_over_interior_finishes() {
    assert!(drag(100, 60, false, true, false).is_some());
}
#[test]
#[ignore = "needs interactive desktop, input-driven"]
fn overlay_survives_capture_loss() {
    let _ = drag(100, 60, false, false, true);
}
