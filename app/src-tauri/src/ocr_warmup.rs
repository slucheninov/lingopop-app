//! Background warm-up of the text recognizer at launch.
//!
//! The first Vision recognition in a process loads its models: about 27 s on a developer Mac
//! and up to a minute on a cold CI runner, while every later call takes milliseconds. Screen
//! Translate runs OCR inside the platform gate with its 3 s deadline, so without a warm-up the
//! first Screen Translate after launch ends with "took too long" and holds the gate until the
//! native call returns. One recognition of a small blank image on its own thread, started at
//! launch and outside the gate, pays that cost before the user can need it. A Screen Translate
//! started while the warm-up still runs can still time out once.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use lingopop_platform::{CapturedImage, Ocr};

/// Side of the blank square recognized by the warm-up.
pub const WARMUP_SIDE: u32 = 64;

/// Starts the warm-up thread. Logs only a fixed line with the duration or the error's
/// `Display` (no user data: the image is blank).
pub fn spawn(ocr: Arc<dyn Ocr>) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("lingopop-ocr-warmup".into())
        .spawn(move || {
            let side = WARMUP_SIDE as usize;
            let Ok(image) =
                CapturedImage::new(WARMUP_SIDE, WARMUP_SIDE, vec![255; side * side * 4])
            else {
                return;
            };
            let start = Instant::now();
            match ocr.recognize(&image) {
                Ok(_) => log::info!("ocr warm-up finished in {} ms", start.elapsed().as_millis()),
                Err(e) => log::info!("ocr warm-up failed: {e}"),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lingopop_platform::fake::FakeOcr;
    use lingopop_platform::{OcrText, PlatformError};

    #[test]
    fn warm_up_recognizes_one_blank_square_off_thread() {
        let ocr = Arc::new(FakeOcr::new(Ok(OcrText {
            text: String::new(),
            language: None,
        })));
        spawn(ocr.clone()).unwrap().join().unwrap();
        assert_eq!(ocr.images(), vec![(WARMUP_SIDE, WARMUP_SIDE)]);
    }

    #[test]
    fn warm_up_failure_is_not_fatal() {
        let ocr = Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable)));
        spawn(ocr.clone()).unwrap().join().unwrap();
        assert_eq!(ocr.images().len(), 1);
    }
}
