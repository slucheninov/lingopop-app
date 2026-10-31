//! One shared process-lifetime hook thread. Every handle controls the same backend.
//! start/stop post commands; callbacks use only atomics and bounded try_send, never a lock.
//! Installation failures are printed as a fixed API name and numeric code, and returned by
//! the next start. Construction waits only for message-queue readiness, never for hooks.
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::OnceLock;

use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_RWIN, VK_SHIFT,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_ALTDOWN, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
};

use super::util::{map_err, os_err};
use crate::{classify_click, DismissEvent, DismissWatcher, PlatformError, Point, Rect};

const START: u32 = WM_APP + 41;
const STOP: u32 = WM_APP + 42;
// Outside the i32 OS-error range: even an installation failure with code 0 is retained.
const NO_ERROR: i64 = i64::MIN;
struct State {
    active: AtomicBool,
    installed: AtomicBool,
    x: AtomicI32,
    y: AtomicI32,
    width: AtomicI32,
    height: AtomicI32,
    error: AtomicI64,
    sender: crossbeam_channel::Sender<DismissEvent>,
    receiver: crossbeam_channel::Receiver<DismissEvent>,
}
impl State {
    fn rect(&self) -> Rect {
        Rect::new(
            self.x.load(Ordering::SeqCst),
            self.y.load(Ordering::SeqCst),
            self.width.load(Ordering::SeqCst),
            self.height.load(Ordering::SeqCst),
        )
    }
}
static STATE: OnceLock<State> = OnceLock::new();
static BACKEND: OnceLock<Result<u32, PlatformError>> = OnceLock::new();
fn state() -> &'static State {
    STATE.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::bounded(32);
        State {
            active: false.into(),
            installed: false.into(),
            x: 0.into(),
            y: 0.into(),
            width: 0.into(),
            height: 0.into(),
            error: NO_ERROR.into(),
            sender,
            receiver,
        }
    })
}

unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0
        && state().active.load(Ordering::SeqCst)
        && matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN)
    {
        // SAFETY: for a nonnegative low-level keyboard hook code Windows supplies this structure.
        let key = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if !key.flags.contains(LLKHF_INJECTED)
            && key.vkCode == u32::from(VK_ESCAPE.0)
            && !key.flags.contains(LLKHF_ALTDOWN)
            && ![VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN].iter().any(|vk| {
                // SAFETY: pure, non-blocking key-state query; the high bit means held down.
                unsafe { GetAsyncKeyState(i32::from(vk.0)) < 0 }
            })
        {
            let _ = state().sender.try_send(DismissEvent::Escape);
            return LRESULT(1);
        }
    }
    // SAFETY: forwards the unchanged hook arguments; the hook handle is ignored by this API.
    unsafe { CallNextHookEx(None, code, w, l) }
}
unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0
        && state().active.load(Ordering::SeqCst)
        && matches!(
            w.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        // SAFETY: for a nonnegative low-level mouse hook code Windows supplies this structure.
        let click = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
        let event = if click.flags & LLMHF_INJECTED == 0 {
            classify_click(
                Point {
                    x: click.pt.x,
                    y: click.pt.y,
                },
                state().rect(),
            )
        } else {
            None
        };
        if let Some(event) = event {
            let _ = state().sender.try_send(event);
        }
    }
    // SAFETY: forwards the unchanged hook arguments; mouse input is never swallowed.
    unsafe { CallNextHookEx(None, code, w, l) }
}

struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns a hook installed on this thread.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}
fn install() -> Result<(Hook, Hook), PlatformError> {
    // SAFETY: a null module name queries this executable's loaded module.
    let module =
        unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(map_err("GetModuleHandleW"))?;
    let keyboard = Hook(
        // SAFETY: callbacks have static code lifetime; dwThreadId=0 installs global low-level hooks.
        unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard),
                Some(HINSTANCE::from(module)),
                0,
            )
        }
        .map_err(map_err("SetWindowsHookExW"))?,
    );
    let mouse = Hook(
        // SAFETY: same lifetime and thread rules as the keyboard hook above.
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(HINSTANCE::from(module)), 0) }
            .map_err(map_err("SetWindowsHookExW"))?,
    );
    Ok((keyboard, mouse))
}
fn backend() -> &'static Result<u32, PlatformError> {
    BACKEND.get_or_init(|| {
        state();
        let (ready, receive) = std::sync::mpsc::sync_channel(1);
        let _thread = std::thread::Builder::new()
            .name("popup-dismiss".into())
            .spawn(move || {
                let _dpi = super::util::DpiGuard::per_monitor_v2();
                let mut msg = MSG::default();
                // SAFETY: valid MSG out-pointer; PeekMessage creates this thread's message queue.
                let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                // SAFETY: pure query of the current thread ID, used only for posting messages.
                if ready.send(unsafe { GetCurrentThreadId() }).is_err() {
                    return;
                }
                let mut hooks = None;
                loop {
                    // SAFETY: valid MSG buffer, messages belong to this dedicated thread.
                    let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
                    if result <= 0 {
                        break;
                    }
                    if msg.message == STOP {
                        hooks = None;
                        state().installed.store(false, Ordering::SeqCst);
                    } else if msg.message == START
                        && state().active.load(Ordering::SeqCst)
                        && hooks.is_none()
                    {
                        match install() {
                            Ok(installed) => {
                                hooks = Some(installed);
                                state().installed.store(true, Ordering::SeqCst);
                            }
                            Err(PlatformError::Os { code, .. }) => {
                                state().error.store(i64::from(code), Ordering::SeqCst);
                                state().active.store(false, Ordering::SeqCst);
                                eprintln!("SetWindowsHookExW failed (code {code})");
                            }
                            Err(_) => unreachable!("install only returns OS errors"),
                        }
                    } else {
                        // SAFETY: dispatch the message just obtained from this thread's queue.
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }
                state().active.store(false, Ordering::SeqCst);
                drop(hooks);
                state().installed.store(false, Ordering::SeqCst);
            })
            .map_err(|_| PlatformError::Unsupported("dismiss thread unavailable"))?;
        receive
            .recv()
            .map_err(|_| PlatformError::Unsupported("dismiss thread unavailable"))
    })
}

/// One process-wide backend; all constructed handles share hooks and a competing receiver.
/// Construct once at app startup. Dropping a handle stops watching without joining the thread.
pub struct WindowsDismissWatcher {
    backend: &'static Result<u32, PlatformError>,
}
impl Default for WindowsDismissWatcher {
    fn default() -> Self {
        Self::new()
    }
}
impl WindowsDismissWatcher {
    pub fn new() -> Self {
        Self { backend: backend() }
    }
    /// Whether the hook thread has finished installing both hooks (diagnostics only).
    /// start/stop remain asynchronous; this does not wait for either command.
    pub fn hooks_installed(&self) -> bool {
        state().installed.load(Ordering::SeqCst)
    }
    fn post(&self, message: u32) -> Result<(), PlatformError> {
        let id = self.backend.as_ref().map_err(Clone::clone)?;
        // SAFETY: id identifies the dedicated thread whose queue was initialized before publication.
        unsafe { PostThreadMessageW(*id, message, WPARAM(0), LPARAM(0)) }
            .map_err(map_err("PostThreadMessageW"))
    }
}
impl DismissWatcher for WindowsDismissWatcher {
    fn start(&self, rect: Rect) -> Result<(), PlatformError> {
        self.update_rect(rect);
        let error = state().error.swap(NO_ERROR, Ordering::SeqCst);
        while state().receiver.try_recv().is_ok() {}
        state().active.store(true, Ordering::SeqCst);
        if let Err(e) = self.post(START) {
            state().active.store(false, Ordering::SeqCst);
            return Err(e);
        }
        if error != NO_ERROR {
            return Err(os_err("SetWindowsHookExW", error as i32));
        }
        Ok(())
    }
    fn update_rect(&self, rect: Rect) {
        state().x.store(rect.x, Ordering::SeqCst);
        state().y.store(rect.y, Ordering::SeqCst);
        state().width.store(rect.width, Ordering::SeqCst);
        state().height.store(rect.height, Ordering::SeqCst);
    }
    fn stop(&self) {
        state().active.store(false, Ordering::SeqCst);
        let _ = self.post(STOP);
    }
    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent> {
        state().receiver.clone()
    }
}
impl Drop for WindowsDismissWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
