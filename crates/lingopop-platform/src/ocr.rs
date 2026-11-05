//! OCR: the recognized-text type, the pure best-result selection and the recognizer trait.

use crate::capture::CapturedImage;
use crate::error::PlatformError;

/// Recognized text. `language` is the BCP-47 tag of the recognizer that produced it.
#[derive(Clone, PartialEq, Eq)]
pub struct OcrText {
    pub text: String,
    pub language: Option<String>,
}

impl OcrText {
    /// True when the text has no non-whitespace character (the caller shows "no text found").
    pub fn is_empty(&self) -> bool {
        self.text.chars().all(char::is_whitespace)
    }
}

impl std::fmt::Debug for OcrText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrText")
            .field("chars", &self.text.chars().count())
            .field("language", &self.language)
            .finish()
    }
}

/// The lines one recognizer found, in reading order.
#[derive(Clone, PartialEq, Eq)]
pub struct OcrCandidate {
    pub language: String,
    pub lines: Vec<String>,
}

impl std::fmt::Debug for OcrCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrCandidate")
            .field("language", &self.language)
            .field("lines", &self.lines.len())
            .finish()
    }
}

fn score(candidate: &OcrCandidate) -> usize {
    candidate
        .lines
        .iter()
        .map(|line| line.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

/// Picks the candidate with the most non-whitespace characters (Unicode scalars); a tie goes to
/// the earlier candidate (user-profile languages come first, English last). Its non-empty lines
/// are joined with `"\n"`. No candidates -> `None`; an all-empty best is returned with empty text.
pub fn select_best(candidates: Vec<OcrCandidate>) -> Option<OcrText> {
    let mut best: Option<(usize, OcrCandidate)> = None;
    for candidate in candidates {
        let s = score(&candidate);
        if best.as_ref().is_none_or(|(top, _)| s > *top) {
            best = Some((s, candidate));
        }
    }
    best.map(|(_, candidate)| OcrText {
        text: candidate
            .lines
            .iter()
            .filter(|line| !line.chars().all(char::is_whitespace))
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        language: Some(candidate.language),
    })
}

/// Builds the text of a recognizer that reports lines with a vertical position in a Y-UP space
/// (a larger value is higher on screen, e.g. Vision's normalized `boundingBox.minY`), in no
/// guaranteed order: the highest line comes first, lines at the same height keep the order the
/// recognizer gave them (stable), a non-finite position sorts last, blank lines are dropped and
/// the rest is joined with `"\n"`. `language` is `None` (the recognizer does not report one).
/// An empty or all-blank input is a REAL empty result, not an error.
pub fn text_in_reading_order(mut lines: Vec<(f64, String)>) -> OcrText {
    let key = |y: f64| if y.is_finite() { y } else { f64::NEG_INFINITY };
    lines.sort_by(|a, b| key(b.0).total_cmp(&key(a.0)));
    OcrText {
        text: lines
            .iter()
            .map(|(_, line)| line.as_str())
            .filter(|line| !line.chars().all(char::is_whitespace))
            .collect::<Vec<_>>()
            .join("\n"),
        language: None,
    }
}

pub trait Ocr: Send + Sync {
    /// Recognizes the text in `image`. `OcrUnavailable` when no recognizer language is installed.
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError>;
}
