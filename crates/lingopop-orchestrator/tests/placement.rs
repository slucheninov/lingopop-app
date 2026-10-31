//! Public-API check of `popup_origin`; the full matrix lives in `src/placement.rs`.
use lingopop_orchestrator::popup_origin;
use lingopop_platform::monitors::{MonitorInfo, Point, Rect};

#[test]
fn place_below_right_of_cursor() {
    let m = MonitorInfo {
        bounds: Rect::new(0, 0, 1920, 1080),
        work_area: Rect::new(0, 0, 1920, 1080),
        scale_factor: 1.0,
        primary: true,
    };
    assert_eq!(
        popup_origin((300.0, 200.0), Point { x: 100, y: 100 }, &[m]),
        Point { x: 112, y: 112 }
    );
}
