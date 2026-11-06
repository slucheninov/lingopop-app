//! Text recognition with Apple's Vision framework (`VNRecognizeTextRequest`), the same request
//! as the frozen Swift app (`Core/ScreenOCR.swift`): accurate recognition level, language
//! correction on, default languages, top candidate of every observation, observations ordered
//! top to bottom (Vision's Y-up `boundingBox.minY`, highest first).
//!
//! No permission is needed. The call is synchronous: run it from the platform gate / a worker
//! thread, never the main thread. Recognized text is returned to the caller and never logged.
//!
//! Image size limit: Apple documents none for `VNImageRequestHandler` (Windows' `MaxImageDimension`
//! is specific to `Windows.Media.Ocr`), so the image is passed as it is; the capture side already
//! bounds it to the displays.

use crate::capture::CapturedImage;
use crate::error::PlatformError;
use crate::ocr::{text_in_reading_order, Ocr, OcrText};
use core_graphics::base::kCGImageAlphaNoneSkipLast;
use core_graphics::color_space::{kCGColorSpaceSRGB, CGColorSpace};
use core_graphics::data_provider::CGDataProvider;
use core_graphics::image::CGImage;
use foreign_types::ForeignType;
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSDictionary};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};
use std::sync::Arc;

const PERFORM_API: &str = "VNImageRequestHandler.performRequests";
const IMAGE_API: &str = "CGImageCreate";

#[derive(Debug, Default, Clone, Copy)]
pub struct MacOcr;

impl MacOcr {
    pub fn new() -> Self {
        Self
    }
}

impl Ocr for MacOcr {
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError> {
        let expected = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|n| n.checked_mul(4));
        if image.width == 0 || image.height == 0 || expected != Some(image.rgba.len()) {
            return Err(PlatformError::InvalidRegion);
        }
        // A worker thread has no autorelease pool: Vision and CoreGraphics autorelease.
        autoreleasepool(|_| recognize_native(image))
    }
}

/// An RGBX sRGB `CGImage` over a copy of the pixels (the provider owns the copy, so nothing
/// borrows `image` and the CGImage may even outlive it).
fn cg_image(image: &CapturedImage) -> Result<CGImage, PlatformError> {
    let provider = CGDataProvider::from_buffer(Arc::new(image.rgba.clone()));
    let space = CGColorSpace::create_with_name(
        // SAFETY: a constant `CFStringRef` exported by CoreGraphics, valid for the process.
        unsafe { kCGColorSpaceSRGB },
    )
    .unwrap_or_else(CGColorSpace::create_device_rgb);
    // `CGImage::new` asserts a non-null result; keep a failure an error, not a panic.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CGImage::new(
            image.width as usize,
            image.height as usize,
            8,
            32,
            image.width as usize * 4,
            &space,
            kCGImageAlphaNoneSkipLast,
            &provider,
            false,
            0, // kCGRenderingIntentDefault
        )
    }))
    .map_err(|_| PlatformError::Os {
        api: IMAGE_API,
        code: 0,
    })
}

fn recognize_native(image: &CapturedImage) -> Result<OcrText, PlatformError> {
    let cg = cg_image(image)?;
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);

    let options: objc2::rc::Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
    // SAFETY: `cg.as_ptr()` is a live `CGImageRef` (owned by `cg`, which outlives the handler
    // below); `objc2_core_graphics::CGImage` is the same opaque CoreFoundation type as
    // `core_graphics::sys::CGImage`, so the pointer cast preserves layout. `options` is an empty
    // dictionary of the declared key / value types. The handler retains the image and never
    // modifies it.
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            &*cg.as_ptr().cast(),
            &options,
        )
    };
    let base: &VNRequest = &request;
    let requests = NSArray::from_slice(&[base]);
    handler
        .performRequests_error(&requests)
        .map_err(|e| PlatformError::Os {
            api: PERFORM_API,
            // The code only: the localized description may quote the image or the system.
            code: i32::try_from(e.code()).unwrap_or(i32::MAX),
        })?;

    let mut lines = Vec::new();
    if let Some(results) = request.results() {
        for observation in results.iter() {
            // SAFETY: `boundingBox` is a plain getter on a live observation.
            let y = unsafe { observation.boundingBox() }.origin.y;
            if let Some(top) = observation.topCandidates(1).iter().next() {
                lines.push((y, top.string().to_string()));
            }
        }
    }
    Ok(text_in_reading_order(lines))
}
