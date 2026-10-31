//! Shared Win32 helpers: wide strings, error mapping, the clipboard open guard and `HGLOBAL` copies.

use std::time::Duration;

use ::windows::core::{Error, PCWSTR};
use ::windows::Win32::Foundation::{GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND};
use ::windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, SetClipboardData};
use ::windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use ::windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

use crate::error::PlatformError;
use crate::sleeper::{retry_busy, Sleeper};

/// NUL-terminated UTF-16 copy of `s`.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The calling thread's last Win32 error code.
pub(crate) fn last_error() -> i32 {
    // SAFETY: GetLastError only reads thread-local state.
    unsafe { GetLastError() }.0 as i32
}

pub(crate) fn os_err(api: &'static str, code: i32) -> PlatformError {
    PlatformError::Os { api, code }
}

/// The Win32 error code inside a `windows` error (an `HRESULT_FROM_WIN32` value is unwrapped).
pub(crate) fn win32_code(e: &Error) -> i32 {
    let hr = e.code().0;
    if (hr as u32) & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

pub(crate) fn map_err(api: &'static str) -> impl Fn(Error) -> PlatformError {
    move |e| os_err(api, win32_code(&e))
}

/// A message-only window owned by the calling thread, destroyed on drop. Used as the clipboard
/// owner for writes (an owner-less `EmptyClipboard` makes `SetClipboardData` fail per the docs).
pub(crate) struct MessageWindow(pub(crate) HWND);

impl MessageWindow {
    pub(crate) fn new() -> Result<Self, PlatformError> {
        let class = wide("STATIC");
        // SAFETY: `class` is a NUL-terminated UTF-16 string that outlives the call; the window is
        // message-only (HWND_MESSAGE parent) and destroyed in Drop on this same thread.
        let hwnd = unsafe {
            CreateWindowExW(
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
        }
        .map_err(map_err("CreateWindowExW"))?;
        Ok(Self(hwnd))
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        // SAFETY: the window was created by this thread in `new` and is destroyed exactly once.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// The clipboard, open for the lifetime of the guard (`CloseClipboard` on drop).
pub(crate) struct ClipboardGuard(());

impl ClipboardGuard {
    /// Opens the clipboard, retrying 10 times 10 ms apart; any failure is `ClipboardBusy` and leaves
    /// the clipboard untouched.
    pub(crate) fn open(
        owner: Option<&MessageWindow>,
        sleeper: &dyn Sleeper,
    ) -> Result<Self, PlatformError> {
        let owner = owner.map(|w| w.0);
        retry_busy(10, Duration::from_millis(10), sleeper, || {
            // SAFETY: `owner` is None or a live window of this thread; a successful open is paired
            // with CloseClipboard in Drop.
            unsafe { OpenClipboard(owner) }.map_err(|_| PlatformError::ClipboardBusy)
        })?;
        Ok(Self(()))
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: this guard exists only while the clipboard is open by this thread.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Copies the bytes of a global memory block. `None` when the block is empty or cannot be locked.
///
/// The caller must have the clipboard open and `handle` must come from `GetClipboardData`.
pub(crate) fn read_hglobal(handle: HANDLE) -> Option<Vec<u8>> {
    let mem = HGLOBAL(handle.0);
    // SAFETY: `mem` is a clipboard-owned global memory handle valid while the clipboard is open.
    let size = unsafe { GlobalSize(mem) };
    if size == 0 {
        return None;
    }
    // SAFETY: as above; a non-null lock result is unlocked below.
    let ptr = unsafe { GlobalLock(mem) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: GlobalLock returned a pointer to at least `size` readable bytes, valid until unlock.
    let data = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
    // SAFETY: balances the successful GlobalLock above.
    let _ = unsafe { GlobalUnlock(mem) };
    Some(data)
}

/// Allocates a movable global block holding `data` (must be non-empty).
fn alloc_hglobal(data: &[u8]) -> Result<HGLOBAL, PlatformError> {
    // SAFETY: plain allocation; the handle is freed by the caller or owned by the system.
    let mem = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) }.map_err(map_err("GlobalAlloc"))?;
    // SAFETY: `mem` was just allocated by us.
    let ptr = unsafe { GlobalLock(mem) };
    if ptr.is_null() {
        let code = last_error();
        // SAFETY: `mem` is ours and not shared with anyone.
        let _ = unsafe { GlobalFree(Some(mem)) };
        return Err(os_err("GlobalLock", code));
    }
    // SAFETY: the block holds at least `data.len()` writable bytes and does not overlap `data`.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u8>(), data.len()) };
    // SAFETY: balances the GlobalLock above. A FALSE result with no error just means "unlocked".
    let _ = unsafe { GlobalUnlock(mem) };
    Ok(mem)
}

/// Puts `data` on the open clipboard under `format`. On success the system owns the memory; on
/// failure it is freed here.
pub(crate) fn set_clipboard_bytes(format: u32, data: &[u8]) -> Result<(), PlatformError> {
    let mem = alloc_hglobal(data)?;
    // SAFETY: the caller has the clipboard open and emptied; `mem` is a movable global block.
    match unsafe { SetClipboardData(format, Some(HANDLE(mem.0))) } {
        Ok(_) => Ok(()),
        Err(e) => {
            // SAFETY: ownership did not move to the system, so the block is still ours.
            let _ = unsafe { GlobalFree(Some(mem)) };
            Err(os_err("SetClipboardData", win32_code(&e)))
        }
    }
}
