//! Screen capture is not implemented on macOS (dev build).

use crate::capture::{CapturedImage, ScreenCapture};
use crate::error::PlatformError;
use crate::monitors::Rect;

#[derive(Debug, Default, Clone, Copy)]
pub struct MacCapture;

impl MacCapture {
    pub fn new() -> Self {
        Self
    }
}

impl ScreenCapture for MacCapture {
    fn capture_region(&self, _region: Rect) -> Result<CapturedImage, PlatformError> {
        Err(PlatformError::Unsupported("screen capture on macOS"))
    }
}
