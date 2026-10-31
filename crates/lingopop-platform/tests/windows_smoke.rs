//! Windows smoke tests against the real clipboard, keyboard and windows. They need an interactive
//! desktop, so they are `#[ignore]` and run in the dedicated CI step (or by hand on a Windows box).
//! The clipboard is global: every test serializes on `CLIP_LOCK`.
#![cfg(windows)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lingopop_platform::{
    capture_selection, CaptureOptions, Clipboard, ClipboardFormat, ClipboardSnapshot,
    ForegroundWindow, KeySynth, SystemSleeper, WindowHandle, WindowsClipboard, WindowsForeground,
    WindowsKeys,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, SendMessageW, TranslateMessage,
    HWND_MESSAGE, MSG, PM_REMOVE, WM_SETTEXT, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

static CLIP_LOCK: Mutex<()> = Mutex::new(());

/// `EM_SETSEL` (from `Win32_UI_Controls`, not worth a feature for one constant).
const EM_SETSEL: u32 = 0x00B1;

fn clip_lock() -> MutexGuard<'static, ()> {
    CLIP_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn clipboard() -> WindowsClipboard {
    WindowsClipboard::new(Arc::new(SystemSleeper::new()))
}

fn registered(name: &str) -> u32 {
    let w = wide(name);
    // SAFETY: `w` is NUL-terminated and outlives the call.
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) };
    assert_ne!(id, 0, "RegisterClipboardFormatW({name})");
    id
}

/// Replaces the clipboard with the given raw formats (owner: a temporary message-only window).
fn put_raw(formats: &[(u32, Vec<u8>)]) {
    let class = wide("STATIC");
    // SAFETY: a message-only window destroyed at the end of this function; every successful
    // OpenClipboard is paired with CloseClipboard; each global block is freed if SetClipboardData
    // does not take ownership.
    unsafe {
        let owner = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
        .expect("CreateWindowExW");
        OpenClipboard(Some(owner)).expect("OpenClipboard");
        EmptyClipboard().expect("EmptyClipboard");
        for (id, data) in formats {
            let mem = GlobalAlloc(GMEM_MOVEABLE, data.len()).expect("GlobalAlloc");
            let ptr = GlobalLock(mem);
            assert!(!ptr.is_null());
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u8>(), data.len());
            let _ = GlobalUnlock(mem);
            if SetClipboardData(*id, Some(HANDLE(mem.0))).is_err() {
                let _ = GlobalFree(Some(mem));
                panic!("SetClipboardData({id})");
            }
        }
        let _ = CloseClipboard();
        let _ = DestroyWindow(owner);
    }
}

/// The raw bytes of one format, `None` when absent.
fn read_raw(id: u32) -> Option<Vec<u8>> {
    // SAFETY: the handle from GetClipboardData is used only while the clipboard is open, and every
    // successful OpenClipboard / GlobalLock is balanced.
    unsafe {
        IsClipboardFormatAvailable(id).ok()?;
        OpenClipboard(None).expect("OpenClipboard");
        let out = (|| {
            let h = GetClipboardData(id).ok()?;
            let mem = HGLOBAL(h.0);
            let size = GlobalSize(mem);
            let ptr = GlobalLock(mem);
            if ptr.is_null() {
                return None;
            }
            let data = std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec();
            let _ = GlobalUnlock(mem);
            Some(data)
        })();
        let _ = CloseClipboard();
        out
    }
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    wide(s).into_iter().flat_map(u16::to_le_bytes).collect()
}

/// A 1x1 32-bit bottom-up DIB: BITMAPINFOHEADER (40 bytes) + one BGRA pixel.
fn tiny_dib() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&40u32.to_le_bytes()); // biSize
    v.extend_from_slice(&1i32.to_le_bytes()); // biWidth
    v.extend_from_slice(&1i32.to_le_bytes()); // biHeight
    v.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    v.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    v.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    v.extend_from_slice(&4u32.to_le_bytes()); // biSizeImage
    v.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    v.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    v.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    v.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    v.extend_from_slice(&[0x10, 0x20, 0x30, 0xFF]);
    v
}

/// A topmost `EDIT` popup owned by a helper thread that pumps its messages (so synthesized keys
/// reach it while the test thread blocks). Destroyed on drop.
struct TestWindow {
    hwnd: usize,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestWindow {
    fn handle(&self) -> WindowHandle {
        WindowHandle(self.hwnd as u64)
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn desktop_test_window(x: i32) -> TestWindow {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let thread_stop = stop.clone();
    let thread = std::thread::spawn(move || {
        let class = wide("EDIT");
        // SAFETY: the window is created, pumped and destroyed on this thread only; `msg` is a
        // valid MSG buffer for PeekMessageW.
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).expect("GetModuleHandleW");
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP | WS_VISIBLE,
                x,
                100,
                300,
                60,
                None,
                None,
                Some(HINSTANCE::from(module)),
                None,
            )
            .expect("CreateWindowExW(EDIT)");
            tx.send(hwnd.0 as usize).expect("send hwnd");
            let mut msg = MSG::default();
            while !thread_stop.load(Ordering::SeqCst) {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = DestroyWindow(hwnd);
        }
    });
    let hwnd = rx.recv().expect("window thread started");
    std::thread::sleep(Duration::from_millis(100));
    TestWindow {
        hwnd,
        stop,
        thread: Some(thread),
    }
}

fn wait_for_foreground(fg: &WindowsForeground, w: WindowHandle) -> bool {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if fg.current() == Some(w) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_roundtrip_text_html_image() {
    let _lock = clip_lock();
    let html = b"Version:0.9\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:0\r\nEndFragment:0\r\n<b>hi</b>".to_vec();
    let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    put_raw(&[
        (u32::from(CF_UNICODETEXT.0), utf16_bytes("héllo Привет 🙂")),
        (registered("HTML Format"), html),
        (registered("PNG"), png),
        (u32::from(CF_DIB.0), tiny_dib()),
    ]);
    let clip = clipboard();
    let before = clip.snapshot().expect("snapshot");
    let formats: Vec<ClipboardFormat> = before.entries.iter().map(|e| e.format).collect();
    assert_eq!(
        formats,
        vec![
            ClipboardFormat::UnicodeText,
            ClipboardFormat::Html,
            ClipboardFormat::Png,
            ClipboardFormat::Dib
        ]
    );
    assert!(before.skipped.is_empty(), "{:?}", before.skipped);
    assert_eq!(
        clip.read_text().expect("read_text").as_deref(),
        Some("héllo Привет 🙂")
    );

    clip.write_text_transient("x")
        .expect("write_text_transient");
    clip.restore(&before).expect("restore");

    let after = clip.snapshot().expect("snapshot after restore");
    assert_eq!(after.entries, before.entries, "every format byte-identical");
    assert!(after.skipped.is_empty(), "{:?}", after.skipped);
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_write_text_transient_sets_exclusion_formats() {
    let _lock = clip_lock();
    let clip = clipboard();
    clip.write_text_transient("transient").expect("write");
    assert_eq!(
        clip.read_text().expect("read").as_deref(),
        Some("transient")
    );
    assert!(read_raw(registered("ExcludeClipboardContentFromMonitorProcessing")).is_some());
    for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
        let data = read_raw(registered(name)).unwrap_or_else(|| panic!("{name} missing"));
        assert!(data.len() >= 4, "{name}: {} bytes", data.len());
        assert_eq!(&data[..4], &[0, 0, 0, 0], "{name} must hold DWORD 0");
    }
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_sequence_number_increments_on_write() {
    let _lock = clip_lock();
    let clip = clipboard();
    let before = clip.sequence_number();
    clip.write_text("same").expect("write");
    let middle = clip.sequence_number();
    clip.write_text("same").expect("write again");
    let after = clip.sequence_number();
    assert_ne!(before, middle);
    assert_ne!(middle, after, "identical content still bumps the sequence");
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_read_text_none_for_image_only_clipboard() {
    let _lock = clip_lock();
    put_raw(&[(u32::from(CF_DIB.0), tiny_dib())]);
    assert_eq!(clipboard().read_text().expect("read_text"), None);
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_restore_of_empty_snapshot_clears() {
    let _lock = clip_lock();
    let clip = clipboard();
    clip.write_text("to be cleared").expect("write");
    let empty = ClipboardSnapshot {
        entries: Vec::new(),
        skipped: Vec::new(),
        sequence: 0,
    };
    clip.restore(&empty).expect("restore");
    assert_eq!(clip.read_text().expect("read_text"), None);
    assert!(clip.snapshot().expect("snapshot").entries.is_empty());
}

#[test]
#[ignore = "needs interactive desktop"]
fn send_copy_copies_selection_from_edit_window() {
    let _lock = clip_lock();
    let clip = clipboard();
    let keys = WindowsKeys::new();
    let fg = WindowsForeground::new();
    clip.write_text("original-clip").expect("seed clipboard");

    let window = desktop_test_window(100);
    let text = wide("selected-123");
    // SAFETY: the window is alive (its thread pumps messages until drop); `text` outlives the
    // synchronous SendMessageW calls.
    unsafe {
        SendMessageW(
            window.hwnd(),
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(text.as_ptr() as isize)),
        );
        SendMessageW(window.hwnd(), EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    fg.restore(window.handle()).expect("restore foreground");
    assert!(
        wait_for_foreground(&fg, window.handle()),
        "edit window not in front"
    );

    let got = capture_selection(
        &clip,
        &keys,
        &fg,
        &SystemSleeper::new(),
        &CaptureOptions::default(),
    )
    .expect("capture_selection");
    assert_eq!(got.as_deref(), Some("selected-123"));
    assert_eq!(
        clip.read_text().expect("read_text").as_deref(),
        Some("original-clip"),
        "the original clipboard is restored"
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn foreground_current_and_restore_roundtrip() {
    let fg = WindowsForeground::new();
    let first = desktop_test_window(100);
    let second = desktop_test_window(500);
    fg.restore(first.handle()).expect("restore first");
    assert!(wait_for_foreground(&fg, first.handle()));
    fg.restore(second.handle()).expect("restore second");
    assert!(wait_for_foreground(&fg, second.handle()));
    assert!(
        !fg.is_elevated(first.handle()),
        "our own window is not elevated above us"
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn modifiers_down_is_empty_when_no_key_is_pressed() {
    assert!(WindowsKeys::new().modifiers_down().is_empty());
}
