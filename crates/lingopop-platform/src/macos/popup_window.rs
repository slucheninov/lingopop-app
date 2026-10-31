//! macOS development fallback; the shell uses Tauri show and webview Escape.
use crate::{DismissEvent, DismissWatcher, NoActivateWindow, PlatformError, Rect, WindowHandle};
#[derive(Debug, Default)]
pub struct MacPopupWindow;
impl NoActivateWindow for MacPopupWindow {
    fn show_at(&self, _: WindowHandle, _: Rect) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("no-activate popup on macOS"))
    }
    fn hide(&self, _: WindowHandle) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("no-activate popup on macOS"))
    }
}
pub struct MacDismissWatcher {
    _sender: crossbeam_channel::Sender<DismissEvent>,
    receiver: crossbeam_channel::Receiver<DismissEvent>,
}
impl Default for MacDismissWatcher {
    fn default() -> Self {
        let (_sender, receiver) = crossbeam_channel::bounded(1);
        Self { _sender, receiver }
    }
}
impl DismissWatcher for MacDismissWatcher {
    fn start(&self, _: Rect) -> Result<(), PlatformError> {
        Ok(())
    }
    fn update_rect(&self, _: Rect) {}
    fn stop(&self) {}
    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent> {
        self.receiver.clone()
    }
}
