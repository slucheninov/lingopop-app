use crate::{NoActivateWindow, PlatformError, Rect, WindowHandle};
use std::sync::Mutex;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupWindowCall {
    ShowAt(WindowHandle, Rect),
    Hide(WindowHandle),
}
#[derive(Debug, Default)]
pub struct FakePopupWindow {
    calls: Mutex<Vec<PopupWindowCall>>,
}
impl FakePopupWindow {
    pub fn calls(&self) -> Vec<PopupWindowCall> {
        self.calls.lock().unwrap().clone()
    }
}
impl NoActivateWindow for FakePopupWindow {
    fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError> {
        self.calls
            .lock()
            .unwrap()
            .push(PopupWindowCall::ShowAt(window, rect));
        Ok(())
    }
    fn hide(&self, window: WindowHandle) -> Result<(), PlatformError> {
        self.calls
            .lock()
            .unwrap()
            .push(PopupWindowCall::Hide(window));
        Ok(())
    }
}
