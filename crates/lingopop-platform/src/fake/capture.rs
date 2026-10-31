//! A capture that returns a solid colour and records the regions asked for.

use std::sync::Mutex;

use crate::capture::{CapturedImage, ScreenCapture};
use crate::error::PlatformError;
use crate::monitors::Rect;

pub struct FakeCapture {
    colour: [u8; 4],
    regions: Mutex<Vec<Rect>>,
}

impl FakeCapture {
    pub fn new(colour: [u8; 4]) -> Self {
        Self {
            colour,
            regions: Mutex::new(Vec::new()),
        }
    }

    /// Every region passed to `capture_region`, in order (invalid ones included).
    pub fn regions(&self) -> Vec<Rect> {
        self.regions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl ScreenCapture for FakeCapture {
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError> {
        self.regions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(region);
        if region.is_empty() {
            return Err(PlatformError::InvalidRegion);
        }
        let pixels = region.width as usize * region.height as usize;
        let rgba = self.colour.repeat(pixels);
        CapturedImage::new(region.width as u32, region.height as u32, rgba)
    }
}
