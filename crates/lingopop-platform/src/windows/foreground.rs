//! `ForegroundWindow` over the Win32 window and process-token APIs.

use std::ffi::c_void;

use ::windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, HWND};
use ::windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use ::windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

use super::util::{last_error, os_err, win32_code};
use crate::error::PlatformError;
use crate::foreground::{integrity_blocks, ForegroundWindow, WindowHandle};

/// `ERROR_INVALID_WINDOW_HANDLE`.
const INVALID_WINDOW_HANDLE: i32 = 1400;

/// Windows foreground control; `WindowHandle` holds the `HWND` value.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsForeground;

impl WindowsForeground {
    pub fn new() -> Self {
        Self
    }
}

fn hwnd(w: WindowHandle) -> HWND {
    HWND(w.0 as usize as *mut c_void)
}

/// `TokenIsElevated` of a process token; `None` when the token cannot be opened or queried.
fn process_elevated(process: HANDLE) -> Option<bool> {
    let mut token = HANDLE::default();
    // SAFETY: `process` is a valid process handle with query rights; `token` receives a handle
    // that is closed below.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of exactly the size passed; `len` is writable.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    // SAFETY: `token` was opened above and is closed exactly once.
    let _ = unsafe { CloseHandle(token) };
    queried.ok()?;
    Some(elevation.TokenIsElevated != 0)
}

impl ForegroundWindow for WindowsForeground {
    fn current(&self) -> Option<WindowHandle> {
        // SAFETY: a pure query.
        let h = unsafe { GetForegroundWindow() };
        (!h.is_invalid()).then_some(WindowHandle(h.0 as usize as u64))
    }

    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError> {
        let h = hwnd(window);
        // SAFETY: IsWindow accepts any value and only reports whether it names a window.
        if !unsafe { IsWindow(Some(h)) }.as_bool() {
            return Err(os_err("IsWindow", INVALID_WINDOW_HANDLE));
        }
        // SAFETY: `h` names an existing window (it may vanish meanwhile; the calls then fail).
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            if SetForegroundWindow(h).as_bool() {
                return Ok(());
            }
        }
        // The foreground lock refused us: attach our input queue to the current foreground
        // thread for one more try, then detach.
        // SAFETY: plain queries and AttachThreadInput calls that are paired (attach, detach).
        let ok = unsafe {
            let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let me = GetCurrentThreadId();
            if fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool()
            {
                let ok = SetForegroundWindow(h).as_bool();
                let _ = BringWindowToTop(h);
                let _ = AttachThreadInput(me, fg_thread, false);
                ok
            } else {
                false
            }
        };
        if ok {
            Ok(())
        } else {
            Err(os_err("SetForegroundWindow", last_error()))
        }
    }

    fn is_elevated(&self, window: WindowHandle) -> bool {
        let mut pid = 0u32;
        // SAFETY: `pid` is writable; an invalid window yields thread id 0 and pid 0.
        unsafe { GetWindowThreadProcessId(hwnd(window), Some(&mut pid)) };
        if pid == 0 {
            return false;
        }
        // SAFETY: the pseudo handle of our own process needs no closing.
        let self_elevated = process_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        // SAFETY: a plain open; the handle is closed below.
        match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(process) => {
                let target = process_elevated(process);
                // SAFETY: `process` was opened above and is closed exactly once.
                let _ = unsafe { CloseHandle(process) };
                // Never block on a guess: an unreadable token counts as not elevated.
                target.is_some_and(|t| integrity_blocks(t, self_elevated))
            }
            // A process we may not even open while we are not elevated runs above us.
            Err(e) => win32_code(&e) == ERROR_ACCESS_DENIED.0 as i32 && !self_elevated,
        }
    }
}
