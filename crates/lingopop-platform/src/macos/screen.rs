//! The portable core of macOS screen capture, compiled on every OS (like `macos_geometry`) so
//! the whole decision logic runs on fakes. No native call and no `unsafe` lives here.
//!
//! `ScreenCaptureAdapter` executes a Task 11 `CapturePlan` through the `NativeScreen` seam and
//! owns the safety sequencing around every native call:
//!
//! 1. the region is validated and clipped and a plan is built from one topology snapshot;
//! 2. Screen Recording must be `Granted` BEFORE the first native call: without it the OS
//!    answers with a wallpaper-only image, not with an error, so the adapter never asks. It only
//!    reads the state (`Permissions::state` never prompts) and never calls `request`;
//! 3. before and after each native call the permission and the topology revision are checked
//!    again, so a revocation or a display reconfiguration discards the image instead of exposing
//!    it. This narrows the race; it cannot close it (the OS has no atomic capture-with-check).
//!
//! The native bitmap is an owned copy (`NativeBitmap`): no native buffer escapes its scope.

use crate::capture::{CapturedImage, ScreenCapture};
use crate::error::PlatformError;
use crate::macos_geometry::{CapturePlan, CaptureTile, MacGeometry, NativeRect, TopologyCache};
use crate::monitors::{validate_region, Rect};
use crate::permissions::{PermissionKind, PermissionState, Permissions};
use std::sync::Arc;

/// The OS call that failed, for the fixed-text `Os` errors of this module.
const CREATE_IMAGE_API: &str = "CGDisplayCreateImageForRect";
/// Fixed text of the error for a display layout that changed during a capture.
const TOPOLOGY_CHANGED_API: &str = "display topology changed";

/// Byte order of a native 8-bit-per-channel 4-byte pixel. The alpha byte is ignored on read
/// and the output alpha is always 255 (screen content is opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Bytes `R G B A`.
    Rgba,
    /// Bytes `B G R A` (CoreGraphics `kCGBitmapByteOrder32Little` + alpha first).
    Bgra,
}

/// One captured rectangle of one display, owned. Rows are top-down and may be padded:
/// `bytes_per_row >= width * 4`, and `data` holds at least `height` rows (the last row only needs
/// its meaningful `width * 4` bytes).
#[derive(Clone, PartialEq, Eq)]
pub struct NativeBitmap {
    pub width: u32,
    pub height: u32,
    pub bytes_per_row: usize,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for NativeBitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeBitmap {{ {}x{} }}", self.width, self.height)
    }
}

/// The native screenshot call. `rect` is in the POINTS of `display_id`, relative to that
/// display's top-left corner (the `CGDisplayCreateImageForRect` space). The implementation
/// returns a copy; the image itself is at the display's native resolution.
pub trait NativeScreen: Send + Sync {
    fn create_image(
        &self,
        display_id: u32,
        rect: NativeRect,
    ) -> Result<NativeBitmap, PlatformError>;
}

/// Where the current display topology comes from (read fresh before every check).
pub trait GeometrySource: Send + Sync {
    fn snapshot(&self) -> MacGeometry;
}

impl GeometrySource for TopologyCache {
    fn snapshot(&self) -> MacGeometry {
        TopologyCache::snapshot(self)
    }
}

pub struct ScreenCaptureAdapter<N: NativeScreen> {
    native: N,
    geometry: Arc<dyn GeometrySource>,
    permissions: Arc<dyn Permissions>,
}

impl<N: NativeScreen> ScreenCaptureAdapter<N> {
    pub fn new(
        native: N,
        geometry: Arc<dyn GeometrySource>,
        permissions: Arc<dyn Permissions>,
    ) -> Self {
        Self {
            native,
            geometry,
            permissions,
        }
    }

    fn require_permission(&self) -> Result<(), PlatformError> {
        if self.permissions.state(PermissionKind::ScreenRecording) == PermissionState::Granted {
            Ok(())
        } else {
            Err(PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording,
            ))
        }
    }

    /// Permission first, then the topology: both must hold right before and right after a call.
    fn guard(&self, plan: &CapturePlan) -> Result<(), PlatformError> {
        self.require_permission()?;
        // A plan that is no longer current was made from a layout that is gone: that is a
        // capture failure (retry), not a region the user drew too small, so it is not
        // `InvalidRegion` (which the flow reports as "selection too small").
        self.geometry
            .snapshot()
            .ensure_current(plan)
            .map_err(|_| PlatformError::Os {
                api: TOPOLOGY_CHANGED_API,
                code: 0,
            })
    }

    fn capture_tile(
        &self,
        plan: &CapturePlan,
        tile: &CaptureTile,
    ) -> Result<NativeBitmap, PlatformError> {
        self.guard(plan)?;
        let (c, scale) = (tile.pixel_crop, tile.backing_scale);
        // Display-local backing pixels -> display-local points (the native API's space).
        let rect = NativeRect::new(
            f64::from(c.x) / scale,
            f64::from(c.y) / scale,
            f64::from(c.width) / scale,
            f64::from(c.height) / scale,
        );
        let bitmap = self.native.create_image(tile.display_id, rect)?;
        // Re-check before the image is used: on failure it is dropped here, never exposed.
        self.guard(plan)?;
        Ok(bitmap)
    }
}

impl<N: NativeScreen> ScreenCapture for ScreenCaptureAdapter<N> {
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError> {
        // Never reach the native API unpermitted (it would return a wallpaper-only image).
        self.require_permission()?;
        let geometry = self.geometry.snapshot();
        let clipped = validate_region(region, &geometry.monitors())?;
        let plan = geometry.capture_plan(clipped)?;
        let (width, height) = plan.output_size;
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(PlatformError::InvalidRegion)?;
        // The whole output starts opaque black: a gap between displays never shows other content.
        let mut rgba = Vec::with_capacity(len);
        for _ in 0..len / 4 {
            rgba.extend_from_slice(&plan.background_rgba);
        }
        for tile in &plan.tiles {
            let bitmap = self.capture_tile(&plan, tile)?;
            blit(&mut rgba, width, height, tile.destination, &bitmap)?;
        }
        CapturedImage::new(width, height, rgba)
    }
}

fn os_error() -> PlatformError {
    PlatformError::Os {
        api: CREATE_IMAGE_API,
        code: 0,
    }
}

/// Checks the declared geometry against the buffer (checked arithmetic, never panics).
fn validate_bitmap(b: &NativeBitmap) -> Result<(), PlatformError> {
    let row_bytes = (b.width as usize).checked_mul(4).ok_or_else(os_error)?;
    if b.width == 0 || b.height == 0 || b.bytes_per_row < row_bytes {
        return Err(os_error());
    }
    let needed = (b.height as usize - 1)
        .checked_mul(b.bytes_per_row)
        .and_then(|n| n.checked_add(row_bytes))
        .ok_or_else(os_error)?;
    if b.data.len() < needed {
        return Err(os_error());
    }
    Ok(())
}

fn rgb_at(b: &NativeBitmap, x: u32, y: u32) -> [u8; 3] {
    let o = y as usize * b.bytes_per_row + x as usize * 4;
    let p = &b.data[o..o + 3];
    match b.format {
        PixelFormat::Rgba => [p[0], p[1], p[2]],
        PixelFormat::Bgra => [p[2], p[1], p[0]],
    }
}

/// Resamples `src` into `dest` (output pixel coordinates), clipped to the output, with alpha 255.
/// An equal size is a straight copy with channel reordering; otherwise bilinear sampling at pixel
/// centres, which is what scaling between mixed-DPI displays needs.
fn blit(
    out: &mut [u8],
    out_w: u32,
    out_h: u32,
    dest: Rect,
    src: &NativeBitmap,
) -> Result<(), PlatformError> {
    validate_bitmap(src)?;
    if dest.is_empty() {
        return Ok(());
    }
    let (dw, dh) = (dest.width as u32, dest.height as u32);
    let x0 = dest.x.max(0);
    let y0 = dest.y.max(0);
    let x1 = dest.right().min(out_w as i32);
    let y1 = dest.bottom().min(out_h as i32);
    let exact = src.width == dw && src.height == dh;
    let sx_ratio = f64::from(src.width) / f64::from(dw);
    let sy_ratio = f64::from(src.height) / f64::from(dh);
    for y in y0..y1 {
        let ty = (y - dest.y) as u32;
        for x in x0..x1 {
            let tx = (x - dest.x) as u32;
            let rgb = if exact {
                rgb_at(src, tx, ty)
            } else {
                bilinear(
                    src,
                    (f64::from(tx) + 0.5) * sx_ratio - 0.5,
                    (f64::from(ty) + 0.5) * sy_ratio - 0.5,
                )
            };
            let o = (y as usize * out_w as usize + x as usize) * 4;
            out[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    Ok(())
}

fn bilinear(src: &NativeBitmap, fx: f64, fy: f64) -> [u8; 3] {
    let max_x = f64::from(src.width - 1);
    let max_y = f64::from(src.height - 1);
    let (fx, fy) = (fx.clamp(0.0, max_x), fy.clamp(0.0, max_y));
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(src.width - 1), (y0 + 1).min(src.height - 1));
    let (wx, wy) = (fx - f64::from(x0), fy - f64::from(y0));
    let (a, b) = (rgb_at(src, x0, y0), rgb_at(src, x1, y0));
    let (c, d) = (rgb_at(src, x0, y1), rgb_at(src, x1, y1));
    let mut out = [0u8; 3];
    for i in 0..3 {
        let top = f64::from(a[i]) * (1.0 - wx) + f64::from(b[i]) * wx;
        let bottom = f64::from(c[i]) * (1.0 - wx) + f64::from(d[i]) * wx;
        out[i] = (top * (1.0 - wy) + bottom * wy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> NativeBitmap {
        NativeBitmap {
            width: w,
            height: h,
            bytes_per_row: w as usize * 4,
            format: PixelFormat::Rgba,
            data: [rgb[0], rgb[1], rgb[2], 0].repeat((w * h) as usize),
        }
    }

    #[test]
    fn bilinear_upscale_of_a_gradient_is_monotonic_and_keeps_the_ends() {
        let mut b = solid(2, 1, [0, 0, 0]);
        b.data[4..7].copy_from_slice(&[200, 200, 200]);
        let mut out = vec![0u8; 8 * 4];
        blit(&mut out, 8, 1, Rect::new(0, 0, 8, 1), &b).unwrap();
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds[0], 0);
        assert_eq!(reds[7], 200);
        assert!(reds.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn a_destination_beyond_the_output_is_clipped_not_a_panic() {
        let b = solid(4, 4, [9, 9, 9]);
        let mut out = vec![0u8; 2 * 2 * 4];
        blit(&mut out, 2, 2, Rect::new(1, 1, 4, 4), &b).unwrap();
        assert_eq!(&out[12..16], &[9, 9, 9, 255]);
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
    }
}
