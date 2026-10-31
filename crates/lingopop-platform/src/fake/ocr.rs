//! An OCR that returns a scripted result and records the image sizes it was given.

use std::sync::Mutex;

use crate::capture::CapturedImage;
use crate::error::PlatformError;
use crate::ocr::{Ocr, OcrText};

pub struct FakeOcr {
    result: Result<OcrText, PlatformError>,
    images: Mutex<Vec<(u32, u32)>>,
}

impl FakeOcr {
    /// Every `recognize` call returns a clone of `result`.
    pub fn new(result: Result<OcrText, PlatformError>) -> Self {
        Self {
            result,
            images: Mutex::new(Vec::new()),
        }
    }

    /// `(width, height)` of every image passed to `recognize`, in order.
    pub fn images(&self) -> Vec<(u32, u32)> {
        self.images
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Ocr for FakeOcr {
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError> {
        self.images
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((image.width, image.height));
        self.result.clone()
    }
}
