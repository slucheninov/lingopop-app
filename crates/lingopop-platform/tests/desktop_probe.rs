//! Diagnostic for CI: prints whether this Windows session has an interactive desktop (clipboard,
//! foreground window, input, hotkeys). Never asserts; the verdict is the last `PROBE` line.
#![cfg(windows)]

use std::ffi::c_void;

use windows::Win32::Foundation::{GetLastError, GlobalFree, HANDLE, POINT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetUserObjectInformationW, UOI_NAME,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOD_ALT, MOD_NOREPEAT, VIRTUAL_KEY, VK_F9,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, SM_CMONITORS, SM_CXVIRTUALSCREEN,
};

fn last_error() -> u32 {
    // SAFETY: reads thread-local state only.
    unsafe { GetLastError() }.0
}

fn window_station_name() -> String {
    // SAFETY: the process window-station handle is a pseudo-owned handle that must not be closed.
    let Ok(station) = (unsafe { GetProcessWindowStation() }) else {
        return format!("<GetProcessWindowStation failed: {}>", last_error());
    };
    let mut buf = [0u16; 256];
    let mut needed = 0u32;
    // SAFETY: `buf` is writable for the byte length passed; `needed` is writable.
    let ok = unsafe {
        GetUserObjectInformationW(
            HANDLE(station.0),
            UOI_NAME,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            std::mem::size_of_val(&buf) as u32,
            Some(&mut needed),
        )
    };
    if ok.is_err() {
        return format!("<GetUserObjectInformationW failed: {}>", last_error());
    }
    let len = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Writes a short text with the raw API and reads it back; returns whether the roundtrip matched.
fn clipboard_roundtrip() -> bool {
    let text: Vec<u16> = "lingopop-probe".encode_utf16().chain([0]).collect();
    // SAFETY: every successful OpenClipboard is paired with CloseClipboard; the global block is
    // freed on SetClipboardData failure and owned by the system on success; the read handle is
    // only used while the clipboard is open.
    unsafe {
        if OpenClipboard(None).is_err() {
            println!("probe: write OpenClipboard failed: {}", last_error());
            return false;
        }
        let written = (|| {
            EmptyClipboard().ok()?;
            let bytes = text.len() * 2;
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let ptr = GlobalLock(mem);
            if ptr.is_null() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            std::ptr::copy_nonoverlapping(text.as_ptr(), ptr.cast::<u16>(), text.len());
            let _ = GlobalUnlock(mem);
            if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))).is_err() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            Some(())
        })();
        let _ = CloseClipboard();
        if written.is_none() {
            println!("probe: clipboard write failed: {}", last_error());
            return false;
        }
        if OpenClipboard(None).is_err() {
            println!("probe: read OpenClipboard failed: {}", last_error());
            return false;
        }
        let read = (|| {
            let h = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
            let mem = windows::Win32::Foundation::HGLOBAL(h.0);
            let ptr = GlobalLock(mem);
            if ptr.is_null() {
                return None;
            }
            let back = std::slice::from_raw_parts(ptr.cast::<u16>(), text.len()).to_vec();
            let _ = GlobalUnlock(mem);
            Some(back)
        })();
        let _ = CloseClipboard();
        read.as_deref() == Some(&text[..])
    }
}

#[test]
#[ignore = "diagnostic: prints whether the runner has an interactive desktop"]
fn desktop_probe() {
    // SAFETY: OpenClipboard(None) is paired with CloseClipboard on success.
    let open = unsafe { OpenClipboard(None) };
    let open_err = last_error();
    if open.is_ok() {
        // SAFETY: we hold the clipboard open.
        let _ = unsafe { CloseClipboard() };
    }
    println!(
        "probe: OpenClipboard ok={} last_error={open_err}",
        open.is_ok()
    );

    // SAFETY: plain queries.
    let seq = unsafe { GetClipboardSequenceNumber() };
    println!("probe: GetClipboardSequenceNumber={seq} (0 = no window-station access)");

    let clipboard_ok = clipboard_roundtrip();
    println!("probe: clipboard CF_UNICODETEXT roundtrip ok={clipboard_ok}");

    // SAFETY: a pure query.
    let fg = unsafe { GetForegroundWindow() };
    println!("probe: GetForegroundWindow null={}", fg.is_invalid());

    let mut pt = POINT::default();
    // SAFETY: `pt` is writable.
    let cursor = unsafe { GetCursorPos(&mut pt) };
    println!(
        "probe: GetCursorPos ok={} at=({}, {}) last_error={}",
        cursor.is_ok(),
        pt.x,
        pt.y,
        last_error()
    );

    // SAFETY: pure queries.
    let (monitors, virtual_width) = unsafe {
        (
            GetSystemMetrics(SM_CMONITORS),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
        )
    };
    println!("probe: SM_CMONITORS={monitors} SM_CXVIRTUALSCREEN={virtual_width}");

    let station = window_station_name();
    println!("probe: window station={station} (WinSta0 = interactive)");

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(lingopop_platform::keys::VK_MASK),
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: one valid keyboard INPUT (a key-up of an unassigned key, harmless).
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    println!(
        "probe: SendInput inserted={sent} last_error={}",
        last_error()
    );

    // SAFETY: a thread hotkey registration without a window, unregistered right after.
    let hotkey = unsafe { RegisterHotKey(None, 1, MOD_ALT | MOD_NOREPEAT, u32::from(VK_F9.0)) };
    let hotkey_err = last_error();
    if hotkey.is_ok() {
        // SAFETY: undoes the registration above.
        let _ = unsafe { UnregisterHotKey(None, 1) };
    }
    println!(
        "probe: RegisterHotKey ok={} last_error={hotkey_err}",
        hotkey.is_ok()
    );

    let interactive = station.eq_ignore_ascii_case("WinSta0") && !fg.is_invalid();
    println!(
        "PROBE interactive={interactive} clipboard={} hotkey={} sendinput={sent}",
        if clipboard_ok { "ok" } else { "fail" },
        if hotkey.is_ok() { "ok" } else { "fail" },
    );
}
