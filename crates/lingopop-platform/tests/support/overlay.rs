#![allow(dead_code)]
use lingopop_platform::{
    Monitors, PlatformError, Rect, RegionOverlay, WindowsMonitors, WindowsOverlay,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, RECT},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

pub fn window() -> Option<HWND> {
    // SAFETY: static class string; no output pointer.
    unsafe { FindWindowW(w!("LingoPopRegionOverlay"), None).ok() }
}
pub struct Pick {
    pub cancel: crossbeam_channel::Sender<()>,
    result: mpsc::Receiver<Result<Option<Rect>, PlatformError>>,
}
impl Pick {
    pub fn start() -> Self {
        assert!(window().is_none());
        let (cancel, rx) = crossbeam_channel::bounded(1);
        let (tx, result) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(WindowsOverlay::new().pick(rx));
        });
        let pick = Self { cancel, result };
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(hwnd) = window() {
                // SAFETY: querying a discovered window; only used for readiness.
                if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                    break;
                }
            }
            assert!(Instant::now() < end, "overlay did not appear");
            std::thread::sleep(Duration::from_millis(5));
        }
        pick
    }
    pub fn finish(&self) -> Option<Rect> {
        self.result
            .recv_timeout(Duration::from_millis(500))
            .expect("overlay hung")
            .expect("overlay failed")
    }
}
impl Drop for Pick {
    fn drop(&mut self) {
        let _ = self.cancel.try_send(());
        // Wait for native cleanup even when a test unwinds before finish().
        let _ = self.result.recv_timeout(Duration::from_secs(1));
    }
}
pub fn bounds() -> Rect {
    lingopop_platform::virtual_screen(&WindowsMonitors::new().monitors().unwrap())
}
pub fn window_rect() -> Rect {
    let mut r = RECT::default();
    // SAFETY: valid out pointer and current overlay window.
    unsafe { GetWindowRect(window().unwrap(), &mut r) }.unwrap();
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}
pub fn escape() {
    let input = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_ESCAPE,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_ESCAPE,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];
    // SAFETY: initialized keyboard records, with correct structure size.
    assert_eq!(
        unsafe { SendInput(&input, std::mem::size_of::<INPUT>() as i32) },
        2
    );
}
pub fn mouse(x: i32, y: i32, flags: MOUSE_EVENT_FLAGS) {
    let b = bounds();
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: ((i64::from(x - b.x) * 65536) / i64::from(b.width)) as i32,
                dy: ((i64::from(y - b.y) * 65536) / i64::from(b.height)) as i32,
                dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    };
    // SAFETY: initialized absolute mouse record in virtual-screen coordinates.
    assert_eq!(
        unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) },
        1
    );
    std::thread::sleep(Duration::from_millis(70));
}
