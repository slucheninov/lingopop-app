//! `LastForeground` over a `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` hook.
//!
//! Each instance owns a thread `lingopop-foreground-tracker` that installs an out-of-context
//! hook and pumps messages. Out-of-context hooks are called on the thread that installed them,
//! so the callback finds its instance through a `thread_local!` slot of that thread: there is no
//! process-wide state, and two instances never share anything. `Drop` posts a quit message, the
//! hook thread unhooks ON ITS OWN THREAD and leaves its loop, and `Drop` joins it.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use ::windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use ::windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use ::windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    IsWindow, PeekMessageW, PostThreadMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG,
    OBJID_WINDOW, PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
};

use super::util::{last_error, os_err};
use crate::error::PlatformError;
use crate::foreground::WindowHandle;
use crate::last_foreground::{is_trackable, LastForeground};

/// Posted to the hook thread by `Drop`.
const QUIT: u32 = WM_APP + 51;

thread_local! {
    /// The remembered-window slot of the tracker whose hook runs on this thread.
    static SLOT: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}

/// Remembers `hwnd` in `slot` when it is a window we may come back to.
fn remember(slot: &AtomicU64, hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let mut class = [0u16; 256];
    // SAFETY: `class` is a writable buffer; an invalid window yields 0.
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if len <= 0 {
        return;
    }
    let class = String::from_utf16_lossy(&class[..len as usize]);
    let mut pid = 0u32;
    // SAFETY: `pid` is writable; an invalid window yields thread id 0 and pid 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return;
    }
    // SAFETY: a pure query of our own process id.
    let own = unsafe { GetCurrentProcessId() };
    if is_trackable(&class, pid, own) {
        slot.store(hwnd.0 as usize as u64, Ordering::SeqCst);
    }
}

/// The hook callback. Out-of-context: runs on the installing thread, inside its message loop.
unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || id_object != OBJID_WINDOW.0 {
        return;
    }
    SLOT.with(|slot| {
        if let Some(slot) = slot.borrow().as_ref() {
            remember(slot, hwnd);
        }
    });
}

/// Windows tracker; `WindowHandle` holds the `HWND` value.
pub struct WindowsLastForeground {
    last: Arc<AtomicU64>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl WindowsLastForeground {
    /// Starts the hook thread and waits until the hook is installed (or failed).
    pub fn new() -> Result<Self, PlatformError> {
        let last = Arc::new(AtomicU64::new(0));
        let slot = last.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<u32, PlatformError>>(1);
        let thread = std::thread::Builder::new()
            .name("lingopop-foreground-tracker".into())
            .spawn(move || {
                let mut msg = MSG::default();
                // SAFETY: valid MSG out-pointer; PeekMessage creates this thread's message queue
                // so a quit posted right after `ready` is not lost.
                let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                SLOT.with(|s| *s.borrow_mut() = Some(slot.clone()));
                // SAFETY: `on_event` has static code lifetime; no module handle is needed for an
                // out-of-context hook; process and thread 0 = every window.
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(on_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.is_invalid() {
                    let err = os_err("SetWinEventHook", last_error());
                    SLOT.with(|s| *s.borrow_mut() = None);
                    let _ = ready_tx.send(Err(err));
                    return;
                }
                // The window in front right now counts when it is trackable.
                // SAFETY: a pure query.
                remember(&slot, unsafe { GetForegroundWindow() });
                // SAFETY: a pure query of the current thread id, used only for posting.
                let thread_id = unsafe { GetCurrentThreadId() };
                if ready_tx.send(Ok(thread_id)).is_err() {
                    // SAFETY: the hook was installed on this thread above.
                    let _ = unsafe { UnhookWinEvent(hook) };
                    SLOT.with(|s| *s.borrow_mut() = None);
                    return;
                }
                loop {
                    // SAFETY: valid MSG buffer; the queue belongs to this thread.
                    let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
                    if result <= 0 || msg.message == QUIT {
                        break;
                    }
                    // SAFETY: dispatch the message just taken from this thread's queue.
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                // SAFETY: the hook was installed on this very thread, which unhooks it.
                let _ = unsafe { UnhookWinEvent(hook) };
                SLOT.with(|s| *s.borrow_mut() = None);
            })
            .map_err(|_| PlatformError::Unsupported("foreground tracker thread unavailable"))?;
        let thread_id = match ready_rx.recv() {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(PlatformError::Unsupported(
                    "foreground tracker thread unavailable",
                ));
            }
        };
        Ok(Self {
            last,
            thread_id,
            thread: Some(thread),
        })
    }
}

impl LastForeground for WindowsLastForeground {
    fn last_external(&self) -> Option<WindowHandle> {
        let raw = self.last.load(Ordering::SeqCst);
        if raw == 0 {
            return None;
        }
        let hwnd = HWND(raw as usize as *mut std::ffi::c_void);
        // SAFETY: IsWindow accepts any value and only reports whether it names a window.
        unsafe { IsWindow(Some(hwnd)) }
            .as_bool()
            .then_some(WindowHandle(raw))
    }
}

impl Drop for WindowsLastForeground {
    fn drop(&mut self) {
        // SAFETY: `thread_id` names the hook thread, whose queue exists since before `new`
        // returned; posting to a thread that already ended just fails.
        let _ = unsafe { PostThreadMessageW(self.thread_id, QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
