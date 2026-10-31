//! Screen capture: the image type, PNG encoding and the capture trait.

use crate::error::PlatformError;
use crate::monitors::Rect;

/// A captured rectangle: 8-bit RGBA, row-major, top-down, alpha 255.
#[derive(Clone, PartialEq, Eq)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for CapturedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CapturedImage {{ {}x{} }}", self.width, self.height)
    }
}

impl CapturedImage {
    /// `rgba.len()` must equal `width * height * 4` (overflow counts as a mismatch).
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, PlatformError> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba.len()) {
            return Err(PlatformError::InvalidRegion);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn to_png(&self) -> Result<Vec<u8>, PlatformError> {
        let fail = |_| PlatformError::Unsupported("png encoding failed");
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(fail)?;
        writer.write_image_data(&self.rgba).map_err(fail)?;
        writer.finish().map_err(fail)?;
        Ok(out)
    }
}

pub trait ScreenCapture: Send + Sync {
    /// Captures `region` (physical virtual-screen pixels). A partly outside region is clipped.
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError>;
}
