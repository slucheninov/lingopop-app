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
        // Display-local backing pixels -> display-local points (the native API's space). The
        // points can be fractional (scale 1.5, fractional display origins) and the OS widens a
        // fractional rect on its own (CGRectIntegral-style, by an amount we cannot see), so the
        // request is widened HERE to the integral points that enclose it, and the result is cut
        // back to the plan's exact pixel crop below.
        let wanted = NativeRect::new(
            f64::from(c.x) / scale,
            f64::from(c.y) / scale,
            f64::from(c.width) / scale,
            f64::from(c.height) / scale,
        );
        let enclosing = wanted.enclosing_rect()?;
        let bitmap = self
            .native
            .create_image(tile.display_id, NativeRect::from(enclosing))?;
        // Re-check before the image is used: on failure it is dropped here, never exposed.
        self.guard(plan)?;
        crop_to_plan(bitmap, c, enclosing, scale)
    }
}

/// Cuts the bitmap of the integral-point rect `enclosing` back to the plan's exact pixel crop
/// `crop` (display-local backing pixels). The offset is `crop - enclosing * scale` pixels, so a
/// crop that already is integral in points is returned untouched. A bitmap whose size does not
/// match `enclosing * scale` (an OS that resampled it), an invalid one, or a crop outside it
/// cannot be indexed exactly: it fails with the fixed-text capture error (never returned whole,
/// which would resample the widened area, pixels outside the plan's crop, into the selection).
fn crop_to_plan(
    bitmap: NativeBitmap,
    crop: Rect,
    enclosing: Rect,
    scale: f64,
) -> Result<NativeBitmap, PlatformError> {
    let expect_w = f64::from(enclosing.width) * scale;
    let expect_h = f64::from(enclosing.height) * scale;
    let ox = (f64::from(crop.x) - f64::from(enclosing.x) * scale).round();
    let oy = (f64::from(crop.y) - f64::from(enclosing.y) * scale).round();
    let sized = (f64::from(bitmap.width) - expect_w).abs() < 1.0
        && (f64::from(bitmap.height) - expect_h).abs() < 1.0;
    let (Ok(cw), Ok(ch)) = (u32::try_from(crop.width), u32::try_from(crop.height)) else {
        return Err(os_error());
    };
    let fits = ox >= 0.0
        && oy >= 0.0
        && cw > 0
        && ch > 0
        && ox + f64::from(cw) <= f64::from(bitmap.width)
        && oy + f64::from(ch) <= f64::from(bitmap.height);
    validate_bitmap(&bitmap)?;
    if !sized || !fits {
        return Err(os_error());
    }
    let (ox, oy) = (ox as usize, oy as usize);
    if ox == 0 && oy == 0 && cw == bitmap.width && ch == bitmap.height {
        return Ok(bitmap);
    }
    let row = cw as usize * 4;
    let mut data = Vec::with_capacity(row * ch as usize);
    for y in 0..ch as usize {
        let start = (oy + y) * bitmap.bytes_per_row + ox * 4;
        data.extend_from_slice(&bitmap.data[start..start + row]);
    }
    Ok(NativeBitmap {
        width: cw,
        height: ch,
        bytes_per_row: row,
        format: bitmap.format,
        data,
    })
}

impl<N: NativeScreen> ScreenCapture for ScreenCaptureAdapter<N> {
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError> {
        self.capture_region_at(region, None)
    }

    fn ensure_topology(&self, revision: Option<u64>) -> Result<(), PlatformError> {
        // The region was picked against another layout: its coordinates are not ours any more.
        if revision.is_some_and(|r| r != self.geometry.snapshot().topology_revision()) {
            return Err(PlatformError::Os {
                api: TOPOLOGY_CHANGED_API,
                code: 0,
            });
        }
        Ok(())
    }

    fn capture_region_at(
        &self,
        region: Rect,
        revision: Option<u64>,
    ) -> Result<CapturedImage, PlatformError> {
        // Never reach the native API unpermitted (it would return a wallpaper-only image).
        self.require_permission()?;
        self.ensure_topology(revision)?;
        let geometry = self.geometry.snapshot();
        let clipped = validate_region(region, &geometry.monitors())?;
        let plan = geometry.capture_plan(clipped)?;
        let (width, height) = plan.output_size;
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(PlatformError::InvalidRegion)?;
        // The whole output starts opaque black: a gap between displays never shows other content.
        let mut rgba = plan.background_rgba.repeat(len / 4);
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

    fn numbered(w: u32, h: u32) -> NativeBitmap {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[x as u8, y as u8, 0, 0]);
            }
        }
        NativeBitmap {
            width: w,
            height: h,
            bytes_per_row: w as usize * 4,
            format: PixelFormat::Rgba,
            data,
        }
    }

    #[test]
    fn crop_cuts_the_enclosing_bitmap_to_the_exact_pixel_rect() {
        // Scale 1.5: points (0,0,4,4) -> 6x6 pixels; the plan's crop is pixels (1,1,5,5).
        let out = crop_to_plan(
            numbered(6, 6),
            Rect::new(1, 1, 5, 5),
            Rect::new(0, 0, 4, 4),
            1.5,
        )
        .unwrap();
        assert_eq!((out.width, out.height, out.bytes_per_row), (5, 5, 20));
        assert_eq!(rgb_at(&out, 0, 0), [1, 1, 0]);
        assert_eq!(rgb_at(&out, 4, 4), [5, 5, 0]);
    }

    #[test]
    fn an_integral_crop_is_returned_untouched() {
        let b = numbered(4, 4);
        let out =
            crop_to_plan(b.clone(), Rect::new(2, 2, 4, 4), Rect::new(1, 1, 2, 2), 2.0).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn a_bitmap_that_does_not_match_the_scale_is_an_error_not_the_widened_area() {
        // 9x9 for an enclosing 4x4 at scale 1.5 (expected 6x6): the exact crop cannot be
        // established, so the capture fails with the fixed-text error.
        let err = crop_to_plan(
            numbered(9, 9),
            Rect::new(1, 1, 5, 5),
            Rect::new(0, 0, 4, 4),
            1.5,
        )
        .unwrap_err();
        assert_eq!(err, os_error());
    }

    #[test]
    fn a_crop_outside_the_bitmap_or_an_invalid_bitmap_is_an_error() {
        let outside = crop_to_plan(
            numbered(6, 6),
            Rect::new(3, 3, 5, 5),
            Rect::new(0, 0, 4, 4),
            1.5,
        );
        assert_eq!(outside.unwrap_err(), os_error());
        let mut broken = numbered(6, 6);
        broken.data.truncate(10);
        let invalid = crop_to_plan(broken, Rect::new(1, 1, 5, 5), Rect::new(0, 0, 4, 4), 1.5);
        assert_eq!(invalid.unwrap_err(), os_error());
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
