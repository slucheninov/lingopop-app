//! OCR is not implemented on macOS (dev build).

use crate::capture::CapturedImage;
use crate::error::PlatformError;
use crate::ocr::{Ocr, OcrText};

#[derive(Debug, Default, Clone, Copy)]
pub struct MacOcr;

impl MacOcr {
    pub fn new() -> Self {
        Self
    }
}

impl Ocr for MacOcr {
    fn recognize(&self, _image: &CapturedImage) -> Result<OcrText, PlatformError> {
        Err(PlatformError::Unsupported("OCR on macOS"))
    }
}
