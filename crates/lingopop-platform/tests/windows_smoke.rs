//! Windows smoke tests against the real clipboard, keyboard, windows and OCR. The tests that need
//! an interactive desktop are `#[ignore]` and run in the dedicated CI step (or by hand on a Windows
//! box); the OCR tests (no language pack = a printed skip), the DPAPI tests, the login-item
//! registry tests (HKCU `Software\LingoPopTest\` only) and the `native()` bundle test need no
//! desktop and run by default.
//! The clipboard is global: every test serializes on `CLIP_LOCK`.
#![cfg(windows)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lingopop_platform::{
    capture_selection, virtual_screen, CaptureOptions, CapturedImage, Clipboard, ClipboardFormat,
    ClipboardSnapshot, ForegroundWindow, KeySynth, LastForeground, Monitors, Ocr, OcrText,
    PlatformError, Rect, ScreenCapture, SystemSleeper, WindowHandle, WindowsCapture,
    WindowsClipboard, WindowsForeground, WindowsKeys, WindowsLastForeground, WindowsMonitors,
    WindowsOcr,
};
use windows::core::PCWSTR;
use windows::Media::Ocr::OcrEngine;
use windows::Win32::Foundation::{
    GlobalFree, COLORREF, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::CreateSolidBrush;
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
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW,
    SendMessageW, TranslateMessage, HWND_MESSAGE, MSG, PM_REMOVE, WM_SETTEXT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

const PROTECTION_NAMES: [&str; 3] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
];

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
    test_window(x, false, false)
}
fn test_window(x: i32, no_activate: bool, probe: bool) -> TestWindow {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let thread_stop = stop.clone();
    let thread = std::thread::spawn(move || {
        let class = wide(if probe {
            "LingoPopEscapeProbe"
        } else if no_activate {
            "STATIC"
        } else {
            "EDIT"
        });
        // SAFETY: the window is created, pumped and destroyed on this thread only; `msg` is a
        // valid MSG buffer for PeekMessageW.
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).expect("GetModuleHandleW");
            if probe {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(escape_probe_proc),
                    hInstance: HINSTANCE::from(module),
                    lpszClassName: PCWSTR(class.as_ptr()),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let hwnd = CreateWindowExW(
                if no_activate {
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
                } else {
                    WS_EX_TOPMOST
                },
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                if no_activate {
                    WS_POPUP
                } else {
                    WS_POPUP | WS_VISIBLE
                },
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
        // Protection markers of a password manager: they must survive a snapshot / restore.
        (
            registered("ExcludeClipboardContentFromMonitorProcessing"),
            vec![1u8],
        ),
        (
            registered("CanIncludeInClipboardHistory"),
            0u32.to_le_bytes().to_vec(),
        ),
        (
            registered("CanUploadToCloudClipboard"),
            0u32.to_le_bytes().to_vec(),
        ),
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
            ClipboardFormat::Dib,
            ClipboardFormat::ExcludeFromMonitors,
            ClipboardFormat::CanIncludeInHistory,
            ClipboardFormat::CanUploadToCloud,
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
fn clipboard_restore_keeps_absence_of_exclusion_formats() {
    let _lock = clip_lock();
    put_raw(&[(u32::from(CF_UNICODETEXT.0), utf16_bytes("plain"))]);
    let clip = clipboard();
    let before = clip.snapshot().expect("snapshot");
    clip.write_text_transient("x").expect("transient");
    clip.restore(&before).expect("restore");
    assert_eq!(clip.read_text().expect("read").as_deref(), Some("plain"));
    for name in PROTECTION_NAMES {
        assert!(
            read_raw(registered(name)).is_none(),
            "{name} must stay absent"
        );
    }
}

#[test]
#[ignore = "needs interactive desktop"]
fn clipboard_restore_if_skips_when_sequence_differs() {
    let _lock = clip_lock();
    let clip = clipboard();
    clip.write_text("original").expect("write");
    let snap = clip.snapshot().expect("snapshot");
    clip.write_text("newer").expect("newer write");
    let stale = snap.sequence;
    assert!(!clip.restore_if(&snap, stale).expect("restore_if"));
    assert_eq!(clip.read_text().expect("read").as_deref(), Some("newer"));
    let now = clip.sequence_number();
    assert!(clip.restore_if(&snap, now).expect("restore_if"));
    assert_eq!(clip.read_text().expect("read").as_deref(), Some("original"));
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

unsafe extern "system" fn magenta_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: forwards the arguments of a window procedure call unchanged.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A topmost 200x100 popup at (100,100) painted solid magenta by its class background brush.
fn magenta_window() -> TestWindow {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let thread_stop = stop.clone();
    let thread = std::thread::spawn(move || {
        let class = wide("LingoPopMagenta");
        // SAFETY: the class, brush and window are created, pumped and destroyed on this thread
        // only; `msg` is a valid MSG buffer; the class brush is owned by the class
        // for the life of the process (a one-off test leak).
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).expect("GetModuleHandleW");
            let brush = CreateSolidBrush(COLORREF(0x00FF_00FF));
            let wc = WNDCLASSW {
                lpfnWndProc: Some(magenta_proc),
                hInstance: HINSTANCE::from(module),
                lpszClassName: PCWSTR(class.as_ptr()),
                hbrBackground: brush,
                ..Default::default()
            };
            // Registration fails harmlessly when a previous test already registered the class.
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP | WS_VISIBLE,
                100,
                100,
                200,
                100,
                None,
                None,
                Some(HINSTANCE::from(module)),
                None,
            )
            .expect("CreateWindowExW(magenta)");
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
    // Give the compositor time to paint the window before it is captured.
    std::thread::sleep(Duration::from_millis(300));
    TestWindow {
        hwnd,
        stop,
        thread: Some(thread),
    }
}

#[test]
#[ignore = "needs interactive desktop"]
fn monitors_report_at_least_one_primary_with_positive_scale() {
    let monitors = WindowsMonitors::new().monitors().expect("monitors");
    assert!(monitors.iter().any(|m| m.primary), "{monitors:?}");
    for m in &monitors {
        assert!(m.scale_factor >= 1.0, "{m:?}");
        assert!(m.bounds.width > 0 && m.bounds.height > 0, "{m:?}");
        assert_eq!(m.work_area.intersect(m.bounds), Some(m.work_area), "{m:?}");
    }
}

#[test]
#[ignore = "needs interactive desktop"]
fn cursor_position_is_inside_virtual_screen() {
    let source = WindowsMonitors::new();
    let screen = virtual_screen(&source.monitors().expect("monitors"));
    let p = source.cursor_position().expect("cursor_position");
    assert!(screen.contains(p), "{p:?} not in {screen:?}");
}

#[test]
#[ignore = "needs interactive desktop"]
fn capture_region_returns_known_window_colour() {
    let _window = magenta_window();
    let image = WindowsCapture::new()
        .capture_region(Rect::new(175, 125, 50, 50))
        .expect("capture_region");
    assert_eq!((image.width, image.height), (50, 50));
    assert!(
        image.rgba.chunks(4).all(|px| px == [255, 0, 255, 255]),
        "first pixel {:?}",
        &image.rgba[..4]
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn capture_region_out_of_screen_is_invalid_region() {
    let capture = WindowsCapture::new();
    assert_eq!(
        capture.capture_region(Rect::new(-100_000, -100_000, 50, 50)),
        Err(PlatformError::InvalidRegion)
    );
    assert_eq!(
        capture.capture_region(Rect::new(10, 10, 0, 50)),
        Err(PlatformError::InvalidRegion)
    );
}

/// The OCR fixture ("Hello world", black on white) as an RGBA image.
fn hello_image() -> CapturedImage {
    let bytes = include_bytes!("fixtures/ocr/hello.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes[..]))
        .read_info()
        .expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buf.truncate(info.buffer_size());
    CapturedImage::new(info.width, info.height, buf).expect("rgba image")
}

/// `None` (after printing the skip line) when the runner has no OCR language pack.
fn ocr_or_skip(result: Result<OcrText, PlatformError>) -> Option<OcrText> {
    match result {
        Err(PlatformError::OcrUnavailable) => {
            eprintln!("skipped: OcrUnavailable (no OCR language pack installed on this runner)");
            None
        }
        other => Some(other.expect("recognize")),
    }
}

#[test]
fn ocr_recognizes_fixture_png() {
    let Some(text) = ocr_or_skip(WindowsOcr::new().recognize(&hello_image())) else {
        return;
    };
    eprintln!("ocr result: {text:?}");
    assert!(text.text.to_lowercase().contains("hello"), "{text:?}");
}

#[test]
fn ocr_without_language_packs_is_ocr_unavailable_or_text() {
    let blank = CapturedImage::new(64, 32, [255u8; 4].repeat(64 * 32)).expect("image");
    match WindowsOcr::new().recognize(&blank) {
        Ok(text) => eprintln!("ocr of a blank image: {text:?}"),
        Err(e) => assert_eq!(e, PlatformError::OcrUnavailable),
    }
}

#[test]
fn ocr_image_larger_than_max_dimension_is_downscaled() {
    let Ok(max) = OcrEngine::MaxImageDimension() else {
        assert_eq!(
            WindowsOcr::new().recognize(&hello_image()),
            Err(PlatformError::OcrUnavailable)
        );
        return;
    };
    for (width, height) in [(max + 1, 32), (32, max + 1)] {
        let image = CapturedImage::new(
            width,
            height,
            vec![255; width as usize * height as usize * 4],
        )
        .expect("image");
        match WindowsOcr::new().recognize(&image) {
            Ok(_) | Err(PlatformError::OcrUnavailable | PlatformError::Os { .. }) => {}
            Err(e) => panic!("downscaled OCR failed: {e}"),
        }
    }
}

/// Ctrl+Alt+F9: the chord the hotkey tests register (F9 is rarely bound globally).
fn f9_chord() -> lingopop_core::hotkey::Hotkey {
    use lingopop_core::hotkey::{Hotkey, Modifier};
    Hotkey::new("F9", [Modifier::Ctrl, Modifier::Alt])
}

/// Runs `f` on a fresh thread: the hotkey manager is `!Send` and owns a window that must be
/// created and used on one thread.
fn on_hotkey_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().expect("hotkey thread")
}

#[test]
#[ignore = "needs interactive desktop"]
fn hotkey_register_conflict_and_unregister() {
    use lingopop_platform::{GlobalHotkeyService, HotkeyService};
    on_hotkey_thread(|| {
        // One live service only: RegisterHotKey reports the same chord twice on one window too.
        let first = GlobalHotkeyService::new().expect("service");
        let id = first.register(&f9_chord()).expect("first register");
        assert_eq!(
            first.register(&f9_chord()),
            Err(PlatformError::HotkeyConflict)
        );
        first.unregister(id).expect("unregister");
        first.register(&f9_chord()).expect("register again");
    });
}

#[test]
#[ignore = "needs interactive desktop"]
fn hotkey_press_delivers_event() {
    use lingopop_platform::{GlobalHotkeyService, HotkeyService};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_F9, VK_MENU,
    };

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    on_hotkey_thread(|| {
        let service = GlobalHotkeyService::new().expect("service");
        let id = service.register(&f9_chord()).expect("register");
        let events = service.events();
        let inputs = [
            key(VK_CONTROL, false),
            key(VK_MENU, false),
            key(VK_F9, false),
            key(VK_F9, true),
            key(VK_MENU, true),
            key(VK_CONTROL, true),
        ];
        // SAFETY: `inputs` is a valid slice of keyboard INPUTs and the size is that of one INPUT.
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        assert_eq!(sent as usize, inputs.len(), "SendInput inserted all events");
        // The hotkey window lives on this thread: pump its messages for 500 ms.
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut msg = MSG::default();
        while Instant::now() < deadline {
            // SAFETY: `msg` is a valid MSG buffer; messages of this thread's own windows only.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let event = events
            .recv_timeout(Duration::from_secs(1))
            .expect("hotkey event");
        assert_eq!(event.id, id);
    });
}

#[test]
#[ignore = "needs interactive desktop"]
fn hotkey_service_drop_joins_forwarder_and_allows_recreate() {
    use lingopop_platform::{GlobalHotkeyService, HotkeyService};
    on_hotkey_thread(|| {
        let service = GlobalHotkeyService::new().expect("service");
        service.register(&f9_chord()).expect("register");
        let started = Instant::now();
        drop(service);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop must join the forwarder within a poll interval"
        );
        let again = GlobalHotkeyService::new().expect("recreated service");
        again.register(&f9_chord()).expect("chord is free again");
    });
}

// DPAPI: no desktop needed, these run in the default `cargo test`.

const DPAPI_FIXTURE: &[u8] = "{\"apiKey.Claude\":\"sk-ant-FIXTURE\",\"n\":\"Привет\"}".as_bytes();

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn dpapi_roundtrip_ascii_unicode_and_empty_payload() {
    use lingopop_core::secrets::SecretProtector;
    let p = lingopop_platform::WindowsProtector::new();
    for plain in [DPAPI_FIXTURE, b"".as_slice()] {
        let sealed = p.protect(plain).expect("protect");
        assert!(
            !sealed.is_empty(),
            "DPAPI output carries a header even for empty input"
        );
        assert_eq!(p.unprotect(&sealed).expect("unprotect"), plain);
    }
}

#[test]
fn dpapi_protected_bytes_do_not_contain_plaintext() {
    use lingopop_core::secrets::SecretProtector;
    let sealed = lingopop_platform::WindowsProtector::new()
        .protect(DPAPI_FIXTURE)
        .expect("protect");
    assert!(!contains(&sealed, b"sk-ant-FIXTURE"));
}

#[test]
fn dpapi_tampered_ciphertext_fails_to_unprotect() {
    use lingopop_core::secrets::{ProtectError, SecretProtector};
    let p = lingopop_platform::WindowsProtector::new();
    let sealed = p.protect(DPAPI_FIXTURE).expect("protect");
    let mut flipped = sealed.clone();
    *flipped.last_mut().unwrap() ^= 0x01;
    let truncated = &sealed[..sealed.len() - 1];
    for bad in [flipped.as_slice(), truncated] {
        match p.unprotect(bad) {
            Err(ProtectError::Failed(message)) => {
                assert!(
                    message.starts_with("CryptUnprotectData failed (code "),
                    "{message}"
                );
                assert!(!message.contains("sk-ant-FIXTURE"));
            }
            Ok(_) => panic!("tampered DPAPI data must not unprotect"),
        }
    }
}

#[test]
fn dpapi_garbage_input_fails_to_unprotect() {
    use lingopop_core::secrets::SecretProtector;
    let p = lingopop_platform::WindowsProtector::new();
    assert!(p.unprotect(b"not dpapi data").is_err());
}

#[test]
fn dpapi_works_through_secret_store() {
    use lingopop_core::fsio::LoadOutcome;
    use lingopop_core::provider::ProviderType;
    use lingopop_core::secrets::SecretStore;
    use lingopop_core::time::SystemClock;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("secrets.dat");
    let protector = Arc::new(lingopop_platform::WindowsProtector::new());
    let (mut store, outcome) =
        SecretStore::open(path.clone(), protector.clone(), Arc::new(SystemClock)).expect("open");
    assert!(matches!(outcome, LoadOutcome::Missing));
    store
        .set_api_key(ProviderType::Claude, "sk-ant-FIXTURE")
        .expect("set key");

    let bytes = std::fs::read(&path).expect("read secrets.dat");
    assert!(!contains(&bytes, b"sk-ant-FIXTURE"));

    let (reloaded, outcome) =
        SecretStore::open(path, protector, Arc::new(SystemClock)).expect("reopen");
    assert!(matches!(outcome, LoadOutcome::Loaded));
    assert_eq!(
        reloaded.api_key(ProviderType::Claude),
        Some("sk-ant-FIXTURE")
    );
}

#[test]
fn native_bundle_constructs() {
    let platform = lingopop_platform::native();
    let sealed = platform.protector.protect(b"bundle").expect("protect");
    assert_eq!(
        platform.protector.unprotect(&sealed).expect("unprotect"),
        b"bundle"
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn show_at_does_not_change_foreground() {
    use lingopop_platform::{NoActivateWindow, WindowsPopupWindow};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindowVisible};
    let _lock = clip_lock();
    let target = desktop_test_window(100);
    let fg = WindowsForeground::new();
    fg.restore(target.handle()).expect("foreground");
    assert!(wait_for_foreground(&fg, target.handle()));
    let popup = test_window(500, true, false);
    // SAFETY: pure foreground query; both test windows remain alive.
    let before = unsafe { GetForegroundWindow() };
    let native = WindowsPopupWindow;
    native
        .show_at(popup.handle(), Rect::new(500, 100, 200, 80))
        .expect("show");
    // SAFETY: queries of the live popup and current foreground.
    unsafe {
        assert_eq!(GetForegroundWindow(), before);
        assert!(IsWindowVisible(popup.hwnd()).as_bool());
    }
    native.hide(popup.handle()).expect("hide");
    // SAFETY: popup is still alive.
    assert!(!unsafe { IsWindowVisible(popup.hwnd()) }.as_bool());
}

#[test]
#[ignore = "needs interactive desktop"]
fn dismiss_watcher_stops_on_hide() {
    use lingopop_platform::{DismissWatcher, WindowsDismissWatcher};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VK_ESCAPE,
    };
    fn wait_hooks(w: &WindowsDismissWatcher, expected: bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while w.hooks_installed() != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(w.hooks_installed(), expected);
    }
    let _lock = clip_lock();
    let target = test_window(100, false, true);
    ESCAPES.store(0, Ordering::SeqCst);
    let fg = WindowsForeground::new();
    fg.restore(target.handle()).expect("foreground");
    assert!(wait_for_foreground(&fg, target.handle()));
    let watcher = WindowsDismissWatcher::new();
    let events = watcher.events();
    watcher.start(Rect::new(500, 100, 200, 80)).expect("start");
    wait_hooks(&watcher, true);
    let inputs = [false, true].map(|up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_ESCAPE,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                ..Default::default()
            },
        },
    });
    // SAFETY: valid keyboard INPUT slice and matching structure size.
    assert_eq!(
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) },
        2
    );
    assert!(
        events.recv_timeout(Duration::from_millis(150)).is_err(),
        "injected Esc must be ignored"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while ESCAPES.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        ESCAPES.load(Ordering::SeqCst) > 0,
        "injected Esc must reach foreground"
    );
    watcher.stop();
    wait_hooks(&watcher, false);
    assert!(events.try_recv().is_err());
    watcher
        .start(Rect::new(500, 100, 200, 80))
        .expect("restart");
    wait_hooks(&watcher, true);
    watcher.stop();
    wait_hooks(&watcher, false);
}

#[test]
#[ignore = "needs interactive desktop"]
fn dismiss_watcher_reports_injected_outside_click() {
    use lingopop_platform::{DismissEvent, DismissWatcher, WindowsDismissWatcher};
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let _lock = clip_lock();
    let mut cursor = POINT::default();
    // SAFETY: valid out pointer to a POINT.
    unsafe { GetCursorPos(&mut cursor) }.expect("cursor");
    let watcher = WindowsDismissWatcher::new();
    let events = watcher.events();
    // The rectangle lies far to the right of the cursor, so a click at the cursor is outside it.
    watcher
        .start(Rect::new(cursor.x + 1000, cursor.y, 200, 80))
        .expect("start");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !watcher.hooks_installed() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(watcher.hooks_installed());
    // VM guest tools and remote desktops deliver the mouse as injected input, so SendInput stands in for them.
    let inputs = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..Default::default()
            },
        },
    });
    // SAFETY: valid mouse INPUT slice and matching structure size.
    assert_eq!(
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) },
        2
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)),
        Ok(DismissEvent::ClickOutside),
        "injected outside click must be reported"
    );
    watcher.stop();
}

static ESCAPES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static OVERLAY_PROBE_CLICKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn escape_probe_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    use windows::Win32::UI::{Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::WM_KEYDOWN};
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN {
        OVERLAY_PROBE_CLICKS.fetch_add(1, Ordering::SeqCst);
    }
    if msg == WM_KEYDOWN && w.0 == usize::from(VK_ESCAPE.0) {
        ESCAPES.fetch_add(1, Ordering::SeqCst);
    }
    // SAFETY: forwards the original window-procedure arguments unchanged.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

#[test]
#[ignore = "needs interactive desktop"]
fn ui_languages_non_empty_on_desktop() {
    assert!(!lingopop_platform::native().system.ui_languages().is_empty());
}

fn test_subkey(name: &str) -> String {
    let subkey = format!(r"Software\LingoPopTest\{name}-{}", std::process::id());
    assert!(
        subkey.starts_with(r"Software\LingoPopTest\"),
        "refusing to touch a key outside the test hive"
    );
    assert!(
        !subkey.contains("CurrentVersion"),
        "refusing to touch the real Run key"
    );
    subkey
}

/// Deletes only `HKCU\Software\LingoPopTest\...`. Never the real Run key.
struct DeleteTestKey(String);

impl Drop for DeleteTestKey {
    fn drop(&mut self) {
        if !self.0.starts_with(r"Software\LingoPopTest\") || self.0.contains("CurrentVersion") {
            return;
        }
        let name = wide(&self.0);
        // SAFETY: `name` is NUL-terminated and names only the per-test subkey.
        let _ = unsafe {
            windows::Win32::System::Registry::RegDeleteKeyW(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                PCWSTR(name.as_ptr()),
            )
        };
    }
}

fn query_sz(subkey: &str, value: &str) -> Result<String, u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    };
    let sub = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: `sub` is NUL-terminated and outlives the call; a null key is not closed.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            Some(0),
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(status.0);
    }
    let read = (|| {
        let name = wide(value);
        let mut size = 0u32;
        // SAFETY: `name` is NUL-terminated; a null data pointer requests the byte count.
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(status.0);
        }
        let mut buf = vec![0u8; size as usize];
        let mut got = size;
        // SAFETY: `buf` has `size` bytes and lives for the call.
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut got),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(status.0);
        }
        buf.truncate(got as usize);
        let units: Vec<u16> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes(*chunk))
            .take_while(|unit| *unit != 0)
            .collect();
        Ok(String::from_utf16_lossy(&units))
    })();
    // SAFETY: `key` was opened above and is closed exactly once.
    let _ = unsafe { RegCloseKey(key) };
    read
}

#[test]
fn login_item_sets_quoted_value_and_removes_it() {
    use lingopop_platform::{run_value, LoginItem, WindowsLoginItem};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    let subkey = test_subkey("login_item_sets_quoted_value_and_removes_it");
    let _cleanup = DeleteTestKey(subkey.clone());
    let item = WindowsLoginItem::with_subkey(&subkey);
    let exe = std::path::PathBuf::from(r"C:\Program Files\LingoPop\lingopop.exe");
    item.set_enabled(true, &exe).expect("enable");
    assert_eq!(
        query_sz(&subkey, "LingoPop").expect("query"),
        run_value(&exe)
    );
    item.set_enabled(false, &exe).expect("disable");
    assert_eq!(
        query_sz(&subkey, "LingoPop").expect_err("removed"),
        ERROR_FILE_NOT_FOUND.0
    );
}

#[test]
fn login_item_disable_when_absent_is_ok() {
    use lingopop_platform::{LoginItem, WindowsLoginItem};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    let subkey = test_subkey("login_item_disable_when_absent_is_ok");
    let _cleanup = DeleteTestKey(subkey.clone());
    let item = WindowsLoginItem::with_subkey(&subkey);
    item.set_enabled(false, std::path::Path::new("unused"))
        .expect("absent value is success");
    assert_eq!(
        query_sz(&subkey, "LingoPop").expect_err("still absent"),
        ERROR_FILE_NOT_FOUND.0
    );
}

// ---- Last-foreground tracker -------------------------------------------------------------------
//
// The tracker hooks with WINEVENT_SKIPOWNPROCESS, so the "external" windows below belong to
// helper PROCESSES (`tests/foreground_helper.rs`, built with `--test foreground_helper`) that the
// test spawns and that end when their stdin closes.

/// A `foreground_helper` process with one topmost window. Killed on drop.
struct Helper {
    child: std::process::Child,
    hwnd: u64,
}

fn helper_exe() -> std::path::PathBuf {
    let dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("exe dir")
        .to_path_buf();
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).expect("read exe dir").flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("foreground_helper-") && name.ends_with(".exe")) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| {
        panic!(
            "foreground_helper-*.exe not found in {}: run the test with `--test foreground_helper` too",
            dir.display()
        )
    })
}

impl Helper {
    fn spawn(title: &str) -> Self {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let mut child = Command::new(helper_exe())
            .args(["--window", title])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn foreground_helper");
        let stdout = child.stdout.take().expect("helper stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut helper = Self { child, hwnd: 0 };
        let line = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("helper printed READY within 10 s");
        helper.hwnd = line
            .trim()
            .strip_prefix("READY ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("unexpected helper output: {line:?}"));
        helper
    }

    fn handle(&self) -> WindowHandle {
        WindowHandle(self.hwnd)
    }

    /// Closes stdin (the helper then exits) and waits up to `timeout` for the process to end.
    fn close_and_wait(&mut self, timeout: Duration) -> bool {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if !self.close_and_wait(Duration::from_secs(2)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Polls `cond` for up to `timeout` (generous: the hook is asynchronous).
fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// Brings `w` to the front. The foreground lock may refuse a process that has not received the
/// last input, so each retry first taps Shift (it opens no menu), which grants that right.
fn force_foreground(fg: &WindowsForeground, w: WindowHandle) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_SHIFT};
    for _ in 0..3 {
        let _ = fg.restore(w);
        if wait_until(Duration::from_millis(700), || fg.current() == Some(w)) {
            return;
        }
        // SAFETY: a plain synthesized Shift press and release, no pointers involved.
        unsafe {
            keybd_event(VK_SHIFT.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_SHIFT.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        }
    }
    panic!("could not bring window {} to the foreground", w.0);
}

#[test]
#[ignore = "needs interactive desktop"]
fn tracker_remembers_foreground_switch_between_external_windows() {
    let tracker = WindowsLastForeground::new().expect("tracker");
    let fg = WindowsForeground::new();
    let a = Helper::spawn("lingopop-tracker-a");
    let b = Helper::spawn("lingopop-tracker-b");
    for target in [&a, &b, &a] {
        force_foreground(&fg, target.handle());
        assert!(
            wait_until(Duration::from_secs(2), || tracker.last_external()
                == Some(target.handle())),
            "last_external() = {:?}, expected {:?}",
            tracker.last_external(),
            target.handle()
        );
    }
}

#[test]
#[ignore = "needs interactive desktop"]
fn tracker_ignores_own_process_windows() {
    let tracker = WindowsLastForeground::new().expect("tracker");
    let fg = WindowsForeground::new();
    let external = Helper::spawn("lingopop-tracker-external");
    force_foreground(&fg, external.handle());
    assert!(
        wait_until(Duration::from_secs(2), || tracker.last_external()
            == Some(external.handle())),
        "helper window not remembered: {:?}",
        tracker.last_external()
    );

    let own = desktop_test_window(500);
    force_foreground(&fg, own.handle());
    // Give a (wrong) update every chance to arrive.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        tracker.last_external(),
        Some(external.handle()),
        "a window of our own process replaced the remembered one"
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn tracker_returns_none_after_window_destroyed() {
    let tracker = WindowsLastForeground::new().expect("tracker");
    let fg = WindowsForeground::new();
    let mut external = Helper::spawn("lingopop-tracker-gone");
    let handle = external.handle();
    force_foreground(&fg, handle);
    assert!(
        wait_until(Duration::from_secs(2), || tracker.last_external()
            == Some(handle)),
        "helper window not remembered: {:?}",
        tracker.last_external()
    );

    assert!(
        external.close_and_wait(Duration::from_secs(5)),
        "helper did not exit after its stdin closed"
    );
    // The destroyed window is never reported. Windows may hand the foreground to another
    // external window at that moment (which is then remembered), so `None` is not guaranteed.
    assert!(
        wait_until(Duration::from_secs(2), || tracker.last_external()
            != Some(handle)),
        "destroyed window still reported: {:?}",
        tracker.last_external()
    );
}

#[test]
#[ignore = "needs interactive desktop"]
fn two_trackers_do_not_share_state() {
    let first = WindowsLastForeground::new().expect("first tracker");
    let second = WindowsLastForeground::new().expect("second tracker");
    let fg = WindowsForeground::new();
    let a = Helper::spawn("lingopop-tracker-shared-a");
    let b = Helper::spawn("lingopop-tracker-shared-b");

    force_foreground(&fg, a.handle());
    for tracker in [&first, &second] {
        assert!(
            wait_until(Duration::from_secs(2), || tracker.last_external()
                == Some(a.handle())),
            "a tracker missed the first switch"
        );
    }

    drop(first);
    force_foreground(&fg, b.handle());
    assert!(
        wait_until(Duration::from_secs(2), || second.last_external()
            == Some(b.handle())),
        "the surviving tracker stopped updating after the other was dropped"
    );
    drop(second);

    // Creating and dropping in a row neither hangs nor panics.
    for _ in 0..5 {
        drop(WindowsLastForeground::new().expect("tracker"));
    }
}

#[path = "support/overlay.rs"]
mod overlay_test;

#[test]
#[ignore = "needs interactive desktop"]
fn overlay_creates_and_destroys_window() {
    let pick = overlay_test::Pick::start();
    let hwnd = overlay_test::window().unwrap();
    // SAFETY: pure thread-id query for a live overlay window.
    let id =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, None) };
    assert!(id != 0);
    assert_eq!(overlay_thread_windows(id), 1);
    pick.cancel.send(()).unwrap();
    assert_eq!(pick.finish(), None);
    assert!(overlay_test::window().is_none());
}
fn overlay_thread_windows(id: u32) -> usize {
    use windows::{core::BOOL, Win32::UI::WindowsAndMessaging::EnumThreadWindows};
    unsafe extern "system" fn count(_: HWND, l: LPARAM) -> BOOL {
        // SAFETY: EnumThreadWindows forwards our live usize pointer synchronously.
        unsafe {
            *(l.0 as *mut usize) += 1;
        }
        BOOL(1)
    }
    let mut n = 0usize;
    // SAFETY: synchronous enumeration; count's out pointer remains live through the call.
    let _ = unsafe { EnumThreadWindows(id, Some(count), LPARAM((&mut n as *mut usize) as isize)) };
    n
}
#[test]
#[ignore = "needs interactive desktop"]
fn overlay_destroys_window_on_cancel() {
    let probe = test_window(100, false, true);
    let fg = WindowsForeground::new();
    fg.restore(probe.handle()).unwrap();
    ESCAPES.store(0, Ordering::SeqCst);
    OVERLAY_PROBE_CLICKS.store(0, Ordering::SeqCst);
    let pick = overlay_test::Pick::start();
    pick.cancel.send(()).unwrap();
    assert_eq!(pick.finish(), None);
    assert!(overlay_test::window().is_none());
    overlay_test::escape();
    assert!(wait_until(Duration::from_secs(1), || ESCAPES
        .load(Ordering::SeqCst)
        > 0));
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP};
    overlay_test::mouse(200, 120, MOUSEEVENTF_LEFTDOWN);
    overlay_test::mouse(200, 120, MOUSEEVENTF_LEFTUP);
    assert!(wait_until(Duration::from_secs(1), || OVERLAY_PROBE_CLICKS
        .load(Ordering::SeqCst)
        > 0));
}
#[test]
#[ignore = "needs interactive desktop"]
fn overlay_window_origin_is_virtual_screen_origin() {
    let pick = overlay_test::Pick::start();
    assert_eq!(overlay_test::window_rect(), overlay_test::bounds());
    pick.cancel.send(()).unwrap();
    assert_eq!(pick.finish(), None);
}
