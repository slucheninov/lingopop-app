//! Real Vision text recognition on the committed fixture (`fixtures/ocr/hello.png`, "Hello
//! world"). It needs no permission and never touches the screen: the pixels come from a file.
//! Recognized text is never printed; only `contains("hello")` style assertions run.
#![cfg(target_os = "macos")]

use lingopop_platform::{CapturedImage, MacOcr, Ocr, PlatformError};

fn fixture() -> CapturedImage {
    let bytes = include_bytes!("fixtures/ocr/hello.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buf.truncate(info.buffer_size());
    CapturedImage::new(info.width, info.height, buf).expect("fixture image")
}

fn recognizes_hello(image: &CapturedImage) -> bool {
    MacOcr::new()
        .recognize(image)
        .map(|t| t.text.to_lowercase().contains("hello"))
        .expect("recognition succeeds")
}

#[test]
fn vision_reads_the_fixture() {
    let text = MacOcr::new().recognize(&fixture()).expect("recognize");
    assert!(!text.is_empty());
    assert!(text.text.to_lowercase().contains("hello"));
    assert!(text.text.to_lowercase().contains("world"));
    // Vision reports no language; blank lines are dropped.
    assert_eq!(text.language, None);
    assert!(text.text.lines().all(|l| !l.trim().is_empty()));
}

#[test]
fn a_blank_image_is_a_real_empty_result_not_an_error() {
    let white = CapturedImage::new(200, 80, vec![255; 200 * 80 * 4]).unwrap();
    let text = MacOcr::new().recognize(&white).expect("no error");
    assert!(text.is_empty());
}

#[test]
fn a_zero_sized_or_inconsistent_image_is_an_invalid_region() {
    let ocr = MacOcr::new();
    let zero = CapturedImage {
        width: 0,
        height: 10,
        rgba: vec![],
    };
    assert_eq!(
        ocr.recognize(&zero).map(|_| ()),
        Err(PlatformError::InvalidRegion)
    );
    let short = CapturedImage {
        width: 10,
        height: 10,
        rgba: vec![0; 12],
    };
    assert_eq!(
        ocr.recognize(&short).map(|_| ()),
        Err(PlatformError::InvalidRegion)
    );
}

#[test]
fn the_same_image_can_be_recognized_twice() {
    // The vision retry keeps the captured image; recognition must not consume or alter it.
    let image = fixture();
    let before = image.clone();
    assert!(recognizes_hello(&image));
    assert!(recognizes_hello(&image));
    assert_eq!(image, before);
}

#[test]
fn recognition_runs_off_the_main_thread() {
    // The gate runs blocking work on worker threads; Vision must work there.
    let image = fixture();
    let handle = std::thread::spawn(move || recognizes_hello(&image));
    assert!(handle.join().expect("worker"));
}

/// Rotates clockwise by `quarter_turns` * 90 degrees (pure pixel moves).
fn rotate(image: &CapturedImage, quarter_turns: u32) -> CapturedImage {
    let mut current = image.clone();
    for _ in 0..quarter_turns % 4 {
        let (w, h) = (current.width as usize, current.height as usize);
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                // (x, y) -> (h - 1 - y, x); the new width is h.
                let (nx, ny) = (h - 1 - y, x);
                let (s, d) = ((y * w + x) * 4, (ny * h + nx) * 4);
                out[d..d + 4].copy_from_slice(&current.rgba[s..s + 4]);
            }
        }
        current = CapturedImage::new(h as u32, w as u32, out).unwrap();
    }
    current
}

#[test]
fn rotated_text_is_recognized_by_the_accurate_level() {
    // Vision's accurate level is orientation aware: the fixture turned by 90, 180 and 270
    // degrees is still read, so a screenshot of rotated text (or a rotated display) is not
    // silently empty. Screen pixels are always top-down, so the adapter passes them upright.
    let upright = fixture();
    for turns in [1, 2, 3] {
        assert!(
            recognizes_hello(&rotate(&upright, turns)),
            "{turns} quarter turns"
        );
    }
}
