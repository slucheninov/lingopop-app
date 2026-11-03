use crate::{PlatformError, Rect, RegionOverlay};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct FakeOverlay {
    result: Result<Option<Rect>, PlatformError>,
    block: AtomicBool,
    picks: AtomicUsize,
    active: AtomicUsize,
    max: AtomicUsize,
}
impl FakeOverlay {
    pub fn new(result: Result<Option<Rect>, PlatformError>) -> Self {
        Self {
            result,
            block: false.into(),
            picks: 0.into(),
            active: 0.into(),
            max: 0.into(),
        }
    }
    pub fn block_until_cancel(&self, block: bool) {
        self.block.store(block, Ordering::SeqCst);
    }
    pub fn picks(&self) -> usize {
        self.picks.load(Ordering::SeqCst)
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
    pub fn max_active(&self) -> usize {
        self.max.load(Ordering::SeqCst)
    }
}
impl RegionOverlay for FakeOverlay {
    fn pick(&self, cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError> {
        self.picks.fetch_add(1, Ordering::SeqCst);
        let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(n, Ordering::SeqCst);
        let out = if self.block.load(Ordering::SeqCst) {
            let _ = cancel.recv();
            Ok(None)
        } else {
            self.result.clone()
        };
        self.active.fetch_sub(1, Ordering::SeqCst);
        out
    }
}
