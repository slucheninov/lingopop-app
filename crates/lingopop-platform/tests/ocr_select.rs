//! The pure OCR best-result selection, `OcrText` and the scripted fake.

use lingopop_platform::fake::FakeOcr;
use lingopop_platform::{select_best, CapturedImage, Ocr, OcrCandidate, OcrText, PlatformError};

fn candidate(language: &str, lines: &[&str]) -> OcrCandidate {
    OcrCandidate {
        language: language.to_string(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
    }
}

#[test]
fn select_best_picks_candidate_with_most_non_whitespace_chars() {
    let best = select_best(vec![
        candidate("en", &["Hel lo"]),
        candidate("uk", &["Привіт", "сві"]),
    ])
    .expect("some");
    assert_eq!(best.language.as_deref(), Some("uk"));
    assert_eq!(best.text, "Привіт\nсві");
}

#[test]
fn select_best_ignores_whitespace_when_scoring() {
    let best = select_best(vec![
        candidate("en", &["          ", "\t\t\t", "   "]),
        candidate("uk", &["ab"]),
    ])
    .expect("some");
    assert_eq!(best.language.as_deref(), Some("uk"));
    assert_eq!(best.text, "ab");
}

#[test]
fn select_best_tie_goes_to_earlier_candidate() {
    let best = select_best(vec![
        candidate("uk", &["abc"]),
        candidate("en-US", &["a b c"]),
    ])
    .expect("some");
    assert_eq!(best.language.as_deref(), Some("uk"));
    assert_eq!(best.text, "abc");
}

#[test]
fn select_best_none_for_no_candidates() {
    assert_eq!(select_best(Vec::new()), None);
}

#[test]
fn select_best_returns_empty_text_when_every_candidate_is_empty() {
    let best =
        select_best(vec![candidate("uk", &[]), candidate("en-US", &["  ", ""])]).expect("some");
    assert_eq!(best.text, "");
    assert!(best.is_empty());
    assert_eq!(best.language.as_deref(), Some("uk"));
}

#[test]
fn select_best_joins_non_empty_lines_with_newline() {
    let best = select_best(vec![candidate(
        "en-US",
        &["first line", "", "   ", "second line", "third"],
    )])
    .expect("some");
    assert_eq!(best.text, "first line\nsecond line\nthird");
}

#[test]
fn select_best_counts_scalars_not_bytes() {
    // "ab" is 2 bytes and 2 scalars; "ыы" is 4 bytes but 2 scalars and comes later, so the tie
    // keeps "ab". "😀😀😀" is 12 bytes and 3 scalars and beats "abc d" (4 scalars) by bytes only.
    let best = select_best(vec![candidate("en", &["ab"]), candidate("uk", &["ыы"])]).unwrap();
    assert_eq!(best.language.as_deref(), Some("en"));
    let best = select_best(vec![
        candidate("en", &["abc d"]),
        candidate("x", &["😀😀😀"]),
    ])
    .unwrap();
    assert_eq!(best.language.as_deref(), Some("en"));
}

#[test]
fn ocr_text_is_empty_means_no_non_whitespace_char() {
    let text = |t: &str| OcrText {
        text: t.to_string(),
        language: None,
    };
    assert!(text("").is_empty());
    assert!(text(" \n\t ").is_empty());
    assert!(!text(" x ").is_empty());
}

#[test]
fn ocr_text_debug_prints_length_only() {
    let text = OcrText {
        text: "Hello world".to_string(),
        language: Some("en-US".to_string()),
    };
    let shown = format!("{text:?}");
    assert_eq!(shown, r#"OcrText { chars: 11, language: Some("en-US") }"#);
    assert!(!shown.contains("Hello"));
    let cyrillic = OcrText {
        text: "Привіт".to_string(),
        language: None,
    };
    assert_eq!(
        format!("{cyrillic:?}"),
        "OcrText { chars: 6, language: None }"
    );
}

#[test]
fn fake_ocr_returns_scripted_result_and_error() {
    let image = CapturedImage::new(2, 1, vec![0; 8]).unwrap();
    let scripted = OcrText {
        text: "hello".to_string(),
        language: Some("en-US".to_string()),
    };
    let ok = FakeOcr::new(Ok(scripted.clone()));
    assert_eq!(ok.recognize(&image), Ok(scripted.clone()));
    assert_eq!(ok.recognize(&image), Ok(scripted));
    assert_eq!(ok.images(), vec![(2, 1), (2, 1)]);

    let failing = FakeOcr::new(Err(PlatformError::OcrUnavailable));
    assert_eq!(
        failing.recognize(&image),
        Err(PlatformError::OcrUnavailable)
    );
    assert_eq!(failing.images(), vec![(2, 1)]);
}

#[test]
fn ocr_is_object_safe_send_sync() {
    fn check<T: Send + Sync + ?Sized>() {}
    check::<dyn Ocr>();
    let _boxed: Box<dyn Ocr> = Box::new(FakeOcr::new(Err(PlatformError::OcrUnavailable)));
}
