use super::util::{map_err, DpiGuard};
use crate::{NoActivateWindow, PlatformError, Rect, WindowHandle};
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
};
#[derive(Debug, Default)]
pub struct WindowsPopupWindow;
impl NoActivateWindow for WindowsPopupWindow {
    fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError> {
        let _dpi = DpiGuard::per_monitor_v2();
        // SAFETY: the shell owns this HWND; coordinates are physical pixels under the DPI guard.
        unsafe {
            SetWindowPos(
                HWND(window.0 as *mut _),
                Some(HWND_TOPMOST),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(map_err("SetWindowPos"))
    }
    fn hide(&self, window: WindowHandle) -> Result<(), PlatformError> {
        // SAFETY: the shell owns this HWND. SW_HIDE never activates it; BOOL is previous visibility.
        let _ = unsafe { ShowWindow(HWND(window.0 as *mut _), SW_HIDE) };
        Ok(())
    }
}
