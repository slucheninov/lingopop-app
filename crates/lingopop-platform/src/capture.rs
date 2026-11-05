//! Capture regions use desktop coordinates; CapturedImage dimensions and RGBA data always
//! use physical image pixels, including on Retina displays.
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
    /// Captures `region` (desktop coordinates: Windows physical pixels, macOS Quartz points).
    /// The returned image uses physical backing pixels. A partly outside region is clipped.
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError>;
    /// Like [`capture_region`](Self::capture_region), for a region picked against the display
    /// topology `revision` (`RegionOverlay::pick_with_revision`). When the live topology has a
    /// different revision the region may be in the coordinates of a layout that is gone, so the
    /// capture fails (`Os { api: "display topology changed" }`) instead of capturing a stale
    /// rectangle. `None` = no revision to check (Windows; the default ignores it).
    fn capture_region_at(
        &self,
        region: Rect,
        revision: Option<u64>,
    ) -> Result<CapturedImage, PlatformError> {
        let _ = revision;
        self.capture_region(region)
    }
}

/// Box-filter downsampling for OCR. A zero limit is treated as one pixel.
/// Empty images are left unchanged (the OCR boundary rejects them).
pub fn downscale_to_fit(image: &CapturedImage, max_dim: u32) -> CapturedImage {
    let limit = max_dim.max(1);
    let longest = image.width.max(image.height);
    if longest <= limit || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let width = ((u64::from(image.width) * u64::from(limit)) / u64::from(longest)).max(1) as u32;
    let height = ((u64::from(image.height) * u64::from(limit)) / u64::from(longest)).max(1) as u32;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    let sx = f64::from(image.width) / f64::from(width);
    let sy = f64::from(image.height) / f64::from(height);
    for y in 0..height {
        let (top, bottom) = (f64::from(y) * sy, f64::from(y + 1) * sy);
        for x in 0..width {
            let (left, right) = (f64::from(x) * sx, f64::from(x + 1) * sx);
            let mut sums = [0.0; 4];
            let mut area = 0.0;
            for iy in top.floor() as u32..(bottom.ceil() as u32).min(image.height) {
                for ix in left.floor() as u32..(right.ceil() as u32).min(image.width) {
                    let weight = (right.min(f64::from(ix + 1)) - left.max(f64::from(ix)))
                        * (bottom.min(f64::from(iy + 1)) - top.max(f64::from(iy)));
                    let offset = (iy as usize * image.width as usize + ix as usize) * 4;
                    for (channel, sum) in sums.iter_mut().enumerate() {
                        *sum += f64::from(image.rgba[offset + channel]) * weight;
                    }
                    area += weight;
                }
            }
            rgba.extend(sums.map(|sum| (sum / area).round() as u8));
        }
    }
    CapturedImage {
        width,
        height,
        rgba,
    }
}
