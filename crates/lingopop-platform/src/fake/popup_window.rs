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
    main_thread_calls: Mutex<usize>,
    fail_next: Mutex<Option<PlatformError>>,
}
impl FakePopupWindow {
    pub fn calls(&self) -> Vec<PopupWindowCall> {
        self.calls.lock().unwrap().clone()
    }
    /// Calls made while a `FakeMainThread` ran this thread as main.
    pub fn main_thread_calls(&self) -> usize {
        *self.main_thread_calls.lock().unwrap()
    }
    /// The next call is recorded, then fails with `error`.
    pub fn fail_next(&self, error: PlatformError) {
        *self.fail_next.lock().unwrap() = Some(error);
    }
    fn record(&self, call: PopupWindowCall) -> Result<(), PlatformError> {
        self.calls.lock().unwrap().push(call);
        if super::main_thread::on_fake_main() {
            *self.main_thread_calls.lock().unwrap() += 1;
        }
        self.fail_next.lock().unwrap().take().map_or(Ok(()), Err)
    }
}
impl NoActivateWindow for FakePopupWindow {
    fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError> {
        self.record(PopupWindowCall::ShowAt(window, rect))
    }
    fn hide(&self, window: WindowHandle) -> Result<(), PlatformError> {
        self.record(PopupWindowCall::Hide(window))
    }
}
