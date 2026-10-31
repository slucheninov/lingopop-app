//! `ShellPopup`: the orchestrator's `PopupView` over the hidden `popup` Tauri window.
//!
//! RULES
//! - NEVER call Tauri `show()` / `set_focus()` on Windows: only the platform
//!   `NoActivateWindow` (`SetWindowPos(.., SWP_NOACTIVATE | SWP_SHOWWINDOW)`) shows the popup,
//!   so the user's app keeps focus. The macOS dev build falls back to Tauri `show()` because
//!   the platform returns `Unsupported` there (dev only).
//! - These methods run on async-runtime workers while the orchestrator holds its session
//!   mutex. They must never wait on the main thread, and every Tauri command in this crate is
//!   `async` (see `commands.rs`) so the main thread never waits on that mutex while a worker
//!   does a cross-thread `SetWindowPos`.
//! - The model JSON is the only place user text leaves Rust; failures log fixed text only.
//!
//! - The native window handle is cached ONCE in `setup` (on the main thread, when the window is
//!   built) and passed to `ShellPopup::new`. `WebviewWindow::hwnd()` synchronously dispatches
//!   to the main thread on Windows, so no `PopupView` method may ever call it.
//!
//! The orchestrator owns the geometry: it passes the origin and the LOGICAL size (what the
//! frontend measures, reported through `popup_resized`) to every `show_at` / `resize`, clamped
//! into the work area, and builds the dismiss rect from the same numbers. This shell only
//! converts logical to physical pixels with the scale of the monitor under the origin and places
//! the window. `resize` is called under the orchestrator's commit mutex and only while the popup
//! is visible (a resize never re-shows a popup that Close just hid).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use lingopop_orchestrator::{PopupView, PopupViewModel};
use lingopop_platform::{monitor_at, physical_size, Platform, Point, Rect, WindowHandle};
#[cfg(not(windows))]
use tauri::Manager;
use tauri::{AppHandle, Emitter, WebviewWindow};

pub const POPUP_LABEL: &str = "popup";
pub const MODEL_EVENT: &str = "popup://model";

/// The newest model, kept so `ready` can re-emit one that arrived before the webview listened.
struct Pending {
    last_model: Option<serde_json::Value>,
}

pub struct ShellPopup {
    app: AppHandle,
    platform: Platform,
    /// Native handle cached at construction (see the module docs); `None` = unavailable.
    handle: Option<WindowHandle>,
    visible: AtomicBool,
    pending: Mutex<Pending>,
}

/// Reads the native handle. On Windows this blocks on the main thread: call it only from
/// `setup` (main thread), never from `PopupView` methods.
#[cfg(windows)]
pub fn native_handle(window: &WebviewWindow) -> Option<WindowHandle> {
    window
        .hwnd()
        .ok()
        .map(|hwnd| WindowHandle(hwnd.0 as usize as u64))
}

#[cfg(not(windows))]
pub fn native_handle(_window: &WebviewWindow) -> Option<WindowHandle> {
    Some(WindowHandle(0))
}

impl ShellPopup {
    pub fn new(app: AppHandle, platform: Platform, handle: Option<WindowHandle>) -> Self {
        Self {
            app,
            platform,
            handle,
            visible: AtomicBool::new(false),
            pending: Mutex::new(Pending { last_model: None }),
        }
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(not(windows))]
    fn window(&self) -> Option<WebviewWindow> {
        let window = self.app.get_webview_window(POPUP_LABEL);
        if window.is_none() {
            log::warn!("popup window is missing");
        }
        window
    }

    fn rect_for(&self, origin: Point, logical: (f64, f64)) -> Rect {
        let monitors = self.platform.monitors.monitors().unwrap_or_default();
        let scale = monitor_at(origin, &monitors).map_or(1.0, |m| m.scale_factor);
        let size = physical_size(logical.0, logical.1, scale);
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    fn emit_model(&self, value: &serde_json::Value) {
        if self.app.emit_to(POPUP_LABEL, MODEL_EVENT, value).is_err() {
            log::warn!("popup model event failed");
        }
    }

    /// Shows or moves the window to `rect` without activating it.
    fn place(&self, rect: Rect) {
        let Some(handle) = self.handle else {
            log::warn!("popup window handle is unavailable");
            return;
        };
        match self.platform.popup_window.show_at(handle, rect) {
            Ok(()) => {}
            // macOS dev only: the platform has no no-activate show; Tauri `show()` may activate.
            #[cfg(not(windows))]
            Err(lingopop_platform::PlatformError::Unsupported(_)) => {
                let Some(window) = self.window() else { return };
                let _ = window.set_position(tauri::PhysicalPosition::new(rect.x, rect.y));
                let _ = window.set_size(tauri::PhysicalSize::new(
                    rect.width.max(1) as u32,
                    rect.height.max(1) as u32,
                ));
                let _ = window.show();
            }
            Err(e) => log::warn!("popup show failed: {e}"),
        }
    }

    /// The frontend mounted and listens: re-emits a model that arrived before that.
    pub fn ready(&self) {
        let pending = { self.pending().last_model.clone() };
        if let Some(model) = pending {
            self.emit_model(&model);
        }
    }
}

impl PopupView for ShellPopup {
    fn show_at(&self, origin: Point, size_logical: (f64, f64), model: &PopupViewModel) {
        let value = serde_json::to_value(model).ok();
        if let Some(value) = &value {
            self.pending().last_model = Some(value.clone());
            self.emit_model(value);
        }
        self.place(self.rect_for(origin, size_logical));
        self.visible.store(true, Ordering::Release);
    }

    fn update(&self, model: &PopupViewModel) {
        let Ok(value) = serde_json::to_value(model) else {
            return;
        };
        self.pending().last_model = Some(value.clone());
        self.emit_model(&value);
    }

    fn resize(&self, origin: Point, size_logical: (f64, f64)) {
        if self.is_visible() {
            self.place(self.rect_for(origin, size_logical));
        }
    }

    fn hide(&self) {
        self.visible.store(false, Ordering::Release);
        let Some(handle) = self.handle else { return };
        match self.platform.popup_window.hide(handle) {
            Ok(()) => {}
            #[cfg(not(windows))]
            Err(lingopop_platform::PlatformError::Unsupported(_)) => {
                if let Some(window) = self.window() {
                    let _ = window.hide();
                }
            }
            Err(e) => log::warn!("popup hide failed: {e}"),
        }
    }

    fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }
}
