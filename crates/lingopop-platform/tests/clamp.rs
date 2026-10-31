//! Pure geometry: popup clamping, region validation, the captured image and the fakes.

use lingopop_platform::fake::{FakeCapture, FakeMonitors};
use lingopop_platform::{
    clamp_to_work_area, monitor_at, physical_size, validate_region, virtual_screen, CapturedImage,
    MonitorInfo, Monitors, PlatformError, Point, Rect, ScreenCapture, Size,
};

fn mon(bounds: Rect, work: Rect, scale: f64, primary: bool) -> MonitorInfo {
    MonitorInfo {
        bounds,
        work_area: work,
        scale_factor: scale,
        primary,
    }
}

/// 1920x1080 at the origin, taskbar at the bottom (work area 1920x1040).
fn a() -> MonitorInfo {
    mon(
        Rect::new(0, 0, 1920, 1080),
        Rect::new(0, 0, 1920, 1040),
        1.0,
        true,
    )
}

const POPUP: Size = Size {
    width: 400,
    height: 300,
};

fn p(x: i32, y: i32) -> Point {
    Point { x, y }
}

#[test]
fn clamp_keeps_anchor_when_popup_fits() {
    assert_eq!(clamp_to_work_area(POPUP, p(100, 100), &[a()]), p(100, 100));
}

#[test]
fn clamp_shifts_left_near_right_edge() {
    assert_eq!(
        clamp_to_work_area(POPUP, p(1900, 100), &[a()]),
        p(1520, 100)
    );
}

#[test]
fn clamp_shifts_up_near_bottom_edge_using_work_area() {
    assert_eq!(clamp_to_work_area(POPUP, p(100, 1000), &[a()]), p(100, 740));
}

#[test]
fn clamp_taskbar_on_left_keeps_left_inset() {
    let m = mon(
        Rect::new(0, 0, 1920, 1080),
        Rect::new(48, 0, 1872, 1080),
        1.0,
        true,
    );
    assert_eq!(clamp_to_work_area(POPUP, p(10, 10), &[m]), p(48, 10));
}

#[test]
fn clamp_left_secondary_monitor_negative_coords() {
    let b = mon(
        Rect::new(-1920, 0, 1920, 1080),
        Rect::new(-1920, 0, 1920, 1080),
        1.0,
        false,
    );
    let all = [a(), b];
    assert_eq!(clamp_to_work_area(POPUP, p(-10, 500), &all), p(-400, 500));
    assert_eq!(clamp_to_work_area(POPUP, p(-1919, 5), &all), p(-1919, 5));
}

#[test]
fn clamp_monitor_above_negative_y() {
    let c = mon(
        Rect::new(0, -1080, 1920, 1080),
        Rect::new(0, -1080, 1920, 1080),
        1.0,
        false,
    );
    assert_eq!(
        clamp_to_work_area(POPUP, p(100, -20), &[a(), c]),
        p(100, -300)
    );
}

#[test]
fn clamp_anchor_picks_monitor_containing_it() {
    let second = mon(
        Rect::new(1920, 0, 1280, 1024),
        Rect::new(1920, 0, 1280, 984),
        1.0,
        false,
    );
    let all = [a(), second];
    assert_eq!(clamp_to_work_area(POPUP, p(3190, 990), &all), p(2800, 684));
    assert_eq!(clamp_to_work_area(POPUP, p(1900, 100), &all), p(1520, 100));
}

#[test]
fn clamp_anchor_in_gap_uses_nearest_monitor() {
    let left = mon(
        Rect::new(0, 0, 1000, 800),
        Rect::new(0, 0, 1000, 800),
        1.0,
        true,
    );
    let right = mon(
        Rect::new(1100, 0, 1000, 800),
        Rect::new(1100, 0, 1000, 800),
        1.0,
        false,
    );
    let all = [left, right];
    // 1020 is 21 px from the left monitor, 80 px from the right one.
    assert_eq!(clamp_to_work_area(POPUP, p(1020, 100), &all), p(600, 100));
    // 1090 is nearer to the right monitor and gets pulled onto it.
    assert_eq!(clamp_to_work_area(POPUP, p(1090, 100), &all), p(1100, 100));
}

#[test]
fn monitor_at_is_half_open() {
    let first = mon(
        Rect::new(0, 0, 1000, 800),
        Rect::new(0, 0, 1000, 800),
        1.0,
        true,
    );
    let second = mon(
        Rect::new(1000, 0, 1000, 800),
        Rect::new(1000, 0, 1000, 800),
        1.0,
        false,
    );
    let all = [first, second];
    assert_eq!(monitor_at(p(999, 0), &all), Some(&first));
    assert_eq!(monitor_at(p(1000, 0), &all), Some(&second));
    assert_eq!(monitor_at(p(0, 800), &all).map(|m| m.primary), Some(true));
}

#[test]
fn clamp_mixed_dpi_uses_scaled_size_of_target_monitor() {
    let d = mon(
        Rect::new(1920, 0, 2560, 1440),
        Rect::new(1920, 0, 2560, 1440),
        1.5,
        false,
    );
    let size = physical_size(400.0, 300.0, 1.5);
    assert_eq!(
        size,
        Size {
            width: 600,
            height: 450
        }
    );
    assert_eq!(
        clamp_to_work_area(size, p(4470, 100), &[a(), d]),
        p(3880, 100)
    );
}

#[test]
fn physical_size_rounds_up() {
    assert_eq!(physical_size(100.5, 80.0, 1.25).width, 126);
    assert_eq!(physical_size(100.5, 80.0, 1.25).height, 100);
}

#[test]
fn clamp_popup_larger_than_work_area_pins_to_top_left() {
    let big = Size {
        width: 3000,
        height: 2000,
    };
    assert_eq!(clamp_to_work_area(big, p(500, 500), &[a()]), p(0, 0));
}

#[test]
fn clamp_no_monitors_returns_anchor() {
    assert_eq!(clamp_to_work_area(POPUP, p(7, 9), &[]), p(7, 9));
}

#[test]
fn virtual_screen_is_union_of_bounds_including_negative_origin() {
    let b = mon(
        Rect::new(-1920, 100, 1920, 1080),
        Rect::new(-1920, 100, 1920, 1080),
        1.0,
        false,
    );
    assert_eq!(virtual_screen(&[a(), b]), Rect::new(-1920, 0, 3840, 1180));
    assert_eq!(virtual_screen(&[]).width, 0);
}

#[test]
fn rect_intersect_and_contains_are_half_open() {
    let r = Rect::new(0, 0, 10, 10);
    assert!(r.contains(p(0, 0)));
    assert!(!r.contains(p(10, 5)));
    assert_eq!(r.intersect(Rect::new(10, 0, 5, 5)), None);
    assert_eq!(
        r.intersect(Rect::new(5, 5, 10, 10)),
        Some(Rect::new(5, 5, 5, 5))
    );
}

#[test]
fn validate_region_rejects_empty_and_zero_size() {
    let all = [a()];
    for r in [
        Rect::new(10, 10, 0, 50),
        Rect::new(10, 10, 50, 0),
        Rect::new(10, 10, -5, 50),
        Rect::new(10, 10, 50, -5),
    ] {
        assert_eq!(validate_region(r, &all), Err(PlatformError::InvalidRegion));
    }
    assert_eq!(
        validate_region(Rect::new(0, 0, 10, 10), &[]),
        Err(PlatformError::InvalidRegion)
    );
}

#[test]
fn validate_region_rejects_fully_off_screen() {
    let all = [a()];
    for r in [
        Rect::new(5000, 5000, 50, 50),
        Rect::new(1920, 0, 50, 50),
        Rect::new(-50, 0, 50, 50),
    ] {
        assert_eq!(validate_region(r, &all), Err(PlatformError::InvalidRegion));
    }
}

#[test]
fn validate_region_clips_partially_outside_region() {
    let b = mon(
        Rect::new(-1920, 0, 1920, 1080),
        Rect::new(-1920, 0, 1920, 1080),
        1.0,
        false,
    );
    assert_eq!(
        validate_region(Rect::new(-1950, 10, 100, 20), &[a(), b]),
        Ok(Rect::new(-1920, 10, 70, 20))
    );
    assert_eq!(
        validate_region(Rect::new(10, 10, 20, 20), &[a()]),
        Ok(Rect::new(10, 10, 20, 20))
    );
}

#[test]
fn captured_image_new_rejects_wrong_length() {
    assert!(CapturedImage::new(2, 2, vec![0; 16]).is_ok());
    assert_eq!(
        CapturedImage::new(2, 2, vec![0; 15]).unwrap_err(),
        PlatformError::InvalidRegion
    );
    assert_eq!(
        CapturedImage::new(u32::MAX, u32::MAX, vec![0; 16]).unwrap_err(),
        PlatformError::InvalidRegion
    );
}

#[test]
fn captured_image_to_png_roundtrips_pixels() {
    let rgba = vec![
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 1, 2, 3, 128,
    ];
    let png_bytes = CapturedImage::new(2, 2, rgba.clone())
        .unwrap()
        .to_png()
        .unwrap();
    assert_eq!(&png_bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (2, 2));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
}

#[test]
fn captured_image_to_png_of_empty_image_is_unsupported() {
    assert_eq!(
        CapturedImage::new(0, 0, Vec::new()).unwrap().to_png(),
        Err(PlatformError::Unsupported("png encoding failed"))
    );
}

#[test]
fn captured_image_debug_prints_dimensions_only() {
    let img = CapturedImage::new(2, 2, vec![77; 16]).unwrap();
    let shown = format!("{img:?}");
    assert!(shown.contains("2x2"), "{shown}");
    assert!(!shown.contains("77"), "{shown}");
}

#[test]
fn fake_capture_records_regions() {
    let cap = FakeCapture::new([1, 2, 3, 255]);
    let img = cap.capture_region(Rect::new(5, 6, 3, 2)).unwrap();
    assert_eq!((img.width, img.height), (3, 2));
    assert_eq!(img.rgba.len(), 24);
    assert!(img.rgba.chunks(4).all(|px| px == [1, 2, 3, 255]));
    assert_eq!(
        cap.capture_region(Rect::new(0, 0, 0, 4)),
        Err(PlatformError::InvalidRegion)
    );
    assert_eq!(
        cap.regions(),
        vec![Rect::new(5, 6, 3, 2), Rect::new(0, 0, 0, 4)]
    );
}

#[test]
fn fake_monitors_report_what_they_were_given() {
    let m = FakeMonitors::new(p(3, 4), vec![a()]);
    assert_eq!(m.cursor_position(), Ok(p(3, 4)));
    assert_eq!(m.monitors().unwrap(), vec![a()]);
}
