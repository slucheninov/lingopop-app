//! macOS screen capture: `CGDisplayCreateImageForRect` behind the Screen Recording preflight.
//!
//! All sequencing (permission before and after every call, topology revisions, plan execution,
//! stride and colour conversion) is the portable `macos_screen::ScreenCaptureAdapter`; this file
//! only holds the native call. `CGDisplayCreateImageForRect` is obsoleted in the macOS 15 SDK
//! headers but still exported by CoreGraphics (the `core-graphics` crate already links it) and
//! is the one built-in region capture available at the macOS 13 floor.

use crate::macos::MacMonitors;
use crate::macos_geometry::{MacGeometry, NativeRect};
use crate::macos_screen::{
    GeometrySource, NativeBitmap, NativeScreen, PixelFormat, ScreenCaptureAdapter,
};
use crate::permissions::Permissions;
use crate::PlatformError;
use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color_space::{kCGColorSpaceSRGB, CGColorSpace};
use core_graphics::context::CGContext;
use core_graphics::display::CGDisplay;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::sys::{CGColorSpaceRef, CGContextRef};
use foreign_types::ForeignType;
use std::ffi::c_void;
use std::sync::Arc;

const CREATE_IMAGE_API: &str = "CGDisplayCreateImageForRect";
const CONTEXT_API: &str = "CGBitmapContextCreate";

/// The production screen capture service. Construct it with the SAME monitor cache and
/// permission service as the rest of the platform bundle (`native()` does).
pub type MacCapture = ScreenCaptureAdapter<CgScreen>;

impl MacCapture {
    pub fn with_services(monitors: MacMonitors, permissions: Arc<dyn Permissions>) -> Self {
        ScreenCaptureAdapter::new(CgScreen, Arc::new(monitors), permissions)
    }
}

impl GeometrySource for MacMonitors {
    fn snapshot(&self) -> MacGeometry {
        self.geometry()
    }
}

// The crate's `CGContext::create_bitmap_context` asserts a non-null result (a panic across our
// FFI-adjacent code), so the constructor is declared here and its null result is an error.
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
}

/// The real `NativeScreen`. Stateless; safe to call from any thread (no AppKit).
#[derive(Debug, Default, Clone, Copy)]
pub struct CgScreen;

fn os(api: &'static str) -> PlatformError {
    PlatformError::Os { api, code: 0 }
}

impl NativeScreen for CgScreen {
    fn create_image(
        &self,
        display_id: u32,
        rect: NativeRect,
    ) -> Result<NativeBitmap, PlatformError> {
        let bounds = CGRect::new(
            &CGPoint::new(rect.x, rect.y),
            &CGSize::new(rect.width, rect.height),
        );
        let image = CGDisplay::new(display_id)
            .image_for_rect(bounds)
            .ok_or_else(|| os(CREATE_IMAGE_API))?;
        let (width, height) = (image.width(), image.height());
        let width32 = u32::try_from(width).map_err(|_| os(CREATE_IMAGE_API))?;
        let height32 = u32::try_from(height).map_err(|_| os(CREATE_IMAGE_API))?;
        let bytes_per_row = width.checked_mul(4).ok_or_else(|| os(CONTEXT_API))?;
        let len = bytes_per_row
            .checked_mul(height)
            .ok_or_else(|| os(CONTEXT_API))?;
        if width == 0 || height == 0 {
            return Err(os(CREATE_IMAGE_API));
        }
        // Draw into an RGBA8 sRGB bitmap that WE own: whatever the display's native format is
        // (10-bit, wide gamut, BGRA), the result is a known layout in a known colour space, and
        // no CoreGraphics-owned buffer outlives this function.
        let mut data = vec![0u8; len];
        let space = CGColorSpace::create_with_name(
            // SAFETY: a constant `CFStringRef` exported by CoreGraphics, valid for the process.
            unsafe { kCGColorSpaceSRGB },
        )
        .unwrap_or_else(CGColorSpace::create_device_rgb);
        // SAFETY: `data` is a live buffer of `bytes_per_row * height` bytes (`len`), which is
        // what the context may write to; `space` is a valid colour space for the call. The
        // context is dropped below before `data` is read or moved, so it never outlives it.
        let raw = unsafe {
            CGBitmapContextCreate(
                data.as_mut_ptr().cast::<c_void>(),
                width,
                height,
                8,
                bytes_per_row,
                space.as_ptr(),
                kCGImageAlphaPremultipliedLast,
            )
        };
        if raw.is_null() {
            return Err(os(CONTEXT_API));
        }
        // SAFETY: `raw` is non-null and owned by us (a Create-rule result); `CGContext` releases it.
        let context = unsafe { CGContext::from_ptr(raw) };
        context.draw_image(
            CGRect::new(
                &CGPoint::new(0.0, 0.0),
                &CGSize::new(width as f64, height as f64),
            ),
            &image,
        );
        // The context writes into `data`; end its use before the buffer is read or moved.
        drop(context);
        Ok(NativeBitmap {
            width: width32,
            height: height32,
            bytes_per_row,
            format: PixelFormat::Rgba,
            data,
        })
    }
}
