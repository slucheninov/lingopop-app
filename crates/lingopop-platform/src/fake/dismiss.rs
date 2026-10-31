use crate::{DismissEvent, DismissWatcher, PlatformError, Rect};
use std::sync::Mutex;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissCall {
    Start(Rect),
    UpdateRect(Rect),
    Stop,
}
pub struct FakeDismissWatcher {
    calls: Mutex<Vec<DismissCall>>,
    active: std::sync::atomic::AtomicBool,
    sender: crossbeam_channel::Sender<DismissEvent>,
    receiver: crossbeam_channel::Receiver<DismissEvent>,
}
impl Default for FakeDismissWatcher {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self {
            calls: Mutex::default(),
            active: false.into(),
            sender,
            receiver,
        }
    }
}
impl FakeDismissWatcher {
    pub fn calls(&self) -> Vec<DismissCall> {
        self.calls.lock().unwrap().clone()
    }
    pub fn fire(&self, event: DismissEvent) {
        if self.active.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = self.sender.try_send(event);
        }
    }
}
impl DismissWatcher for FakeDismissWatcher {
    fn start(&self, rect: Rect) -> Result<(), PlatformError> {
        self.calls.lock().unwrap().push(DismissCall::Start(rect));
        self.active.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn update_rect(&self, rect: Rect) {
        self.calls
            .lock()
            .unwrap()
            .push(DismissCall::UpdateRect(rect));
    }
    fn stop(&self) {
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.calls.lock().unwrap().push(DismissCall::Stop);
    }
    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent> {
        self.receiver.clone()
    }
}
