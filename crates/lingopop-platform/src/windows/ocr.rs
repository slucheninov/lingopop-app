//! `Ocr` over `Windows.Media.Ocr` (WinRT): an engine for the user-profile languages plus an
//! English engine, the result with the most text wins (`select_best`).
//!
//! Call from a worker thread (Phase 4 uses `spawn_blocking`): the thread joins the WinRT
//! multithreaded apartment, and a blocking `join()` on an STA (UI) thread can deadlock.

use std::cell::Cell;

use ::windows::core::Error;
use ::windows::Globalization::Language;
use ::windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use ::windows::Media::Ocr::OcrEngine;
use ::windows::Storage::Streams::DataWriter;
use ::windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use ::windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use super::util::{map_err, os_err, win32_code};
use crate::capture::{downscale_to_fit, CapturedImage};
use crate::error::PlatformError;
use crate::ocr::{select_best, Ocr, OcrCandidate, OcrText};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsOcr;

impl WindowsOcr {
    pub fn new() -> Self {
        Self
    }
}

thread_local! {
    static RO_INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

/// Joins the calling thread to the WinRT multithreaded apartment once. A thread that already
/// runs in another apartment (`RPC_E_CHANGED_MODE`) keeps it.
fn ensure_ro_initialized() -> Result<(), PlatformError> {
    if RO_INITIALIZED.get() {
        return Ok(());
    }
    // SAFETY: only initializes the WinRT runtime for the calling thread (`S_FALSE` = already
    // initialized is a success). It is never uninitialized: it stays valid for the thread's life.
    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => {}
        Err(e) if e.code() == RPC_E_CHANGED_MODE => {}
        Err(e) => return Err(os_err("RoInitialize", win32_code(&e))),
    }
    RO_INITIALIZED.set(true);
    Ok(())
}

fn is_english(tag: &str) -> bool {
    tag.eq_ignore_ascii_case("en") || tag.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("en-"))
}

fn language_tag(language: &Language) -> Option<String> {
    language.LanguageTag().ok().map(|t| t.to_string_lossy())
}

/// The first installed English recognizer language with its tag.
fn english_language() -> Option<(Language, String)> {
    let languages = OcrEngine::AvailableRecognizerLanguages().ok()?;
    (&languages).into_iter().find_map(|language| {
        let tag = language_tag(&language)?;
        is_english(&tag).then_some((language, tag))
    })
}

/// The user-profile engine (if any) first, then English when it is a different language.
fn engines() -> Vec<OcrEngine> {
    let mut engines = Vec::new();
    let mut first_tag = None;
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        first_tag = engine
            .RecognizerLanguage()
            .ok()
            .and_then(|l| language_tag(&l));
        engines.push(engine);
    }
    if let Some((language, tag)) = english_language() {
        let duplicate = first_tag
            .as_deref()
            .is_some_and(|first| first.eq_ignore_ascii_case(&tag));
        if !duplicate {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                engines.push(engine);
            }
        }
    }
    engines
}

/// A BGRA8 software bitmap holding a copy of `image`.
fn bitmap(image: &CapturedImage) -> Result<SoftwareBitmap, PlatformError> {
    let width = i32::try_from(image.width).map_err(|_| PlatformError::InvalidRegion)?;
    let height = i32::try_from(image.height).map_err(|_| PlatformError::InvalidRegion)?;
    let mut bgra = image.rgba.clone();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(map_err("DataWriter::new"))?;
    writer
        .WriteBytes(&bgra)
        .map_err(map_err("DataWriter::WriteBytes"))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(map_err("DataWriter::DetachBuffer"))?;
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
        .map_err(map_err("SoftwareBitmap::CreateCopyFromBuffer"))
}

/// Runs one engine to completion (blocking) and collects its lines in reading order.
fn recognize_with(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<OcrCandidate, Error> {
    let result = engine.RecognizeAsync(bitmap)?.join()?;
    let mut lines = Vec::new();
    for line in &result.Lines()? {
        lines.push(line.Text()?.to_string_lossy());
    }
    let language = engine
        .RecognizerLanguage()?
        .LanguageTag()?
        .to_string_lossy();
    Ok(OcrCandidate { language, lines })
}

impl Ocr for WindowsOcr {
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError> {
        ensure_ro_initialized()?;
        let max = OcrEngine::MaxImageDimension().map_err(|_| PlatformError::OcrUnavailable)?;
        let image = downscale_to_fit(image, max);
        if image.width == 0 || image.height == 0 {
            return Err(PlatformError::InvalidRegion);
        }
        let engines = engines();
        if engines.is_empty() {
            return Err(PlatformError::OcrUnavailable);
        }
        let bitmap = bitmap(&image)?;
        let mut candidates = Vec::new();
        let mut last_error = None;
        for engine in &engines {
            match recognize_with(engine, &bitmap) {
                Ok(candidate) => candidates.push(candidate),
                Err(e) => last_error = Some(e),
            }
        }
        match (select_best(candidates), last_error) {
            (Some(best), _) => Ok(best),
            (None, Some(e)) => Err(os_err("OcrEngine::RecognizeAsync", win32_code(&e))),
            (None, None) => Err(PlatformError::OcrUnavailable),
        }
    }
}
