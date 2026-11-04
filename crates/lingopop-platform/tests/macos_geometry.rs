use lingopop_platform::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
use lingopop_platform::{PlatformError, Rect};

fn display(id: u32, x: f64, y: f64, w: f64, h: f64, scale: f64) -> MacDisplay {
    MacDisplay {
        id,
        desktop_bounds: NativeRect::new(x, y, w, h),
        work_area: NativeRect::new(x, y + 24.0, w, h - 74.0),
        backing_scale: scale,
        primary: id == 1,
    }
}
fn retina() -> MacGeometry {
    MacGeometry::new(1, 900.0, vec![display(1, 0.0, 0.0, 1440.0, 900.0, 2.0)]).unwrap()
}
#[test]
fn retina_capture_scales_pixels_once() {
    let g = retina();
    let p = g.capture_plan(Rect::new(10, 20, 100, 50)).unwrap();
    assert_eq!(p.output_size, (200, 100));
    assert_eq!(p.tiles[0].pixel_crop, Rect::new(20, 40, 200, 100));
    assert_eq!(g.monitors()[0].scale_factor, 1.0);
}
#[test]
fn adjacent_mixed_scale_tiles_have_deterministic_offsets() {
    let g = MacGeometry::new(
        2,
        900.0,
        vec![
            display(1, 0.0, 0.0, 1440.0, 900.0, 2.0),
            display(2, 1440.0, 0.0, 1000.0, 900.0, 1.0),
        ],
    )
    .unwrap();
    let p = g.capture_plan(Rect::new(1400, 20, 100, 50)).unwrap();
    assert_eq!(p.output_size, (200, 100));
    assert_eq!(p.tiles[0].destination, Rect::new(0, 0, 80, 100));
    assert_eq!(p.tiles[1].destination, Rect::new(80, 0, 120, 100));
    assert_eq!(p.tiles[1].pixel_crop, Rect::new(0, 20, 60, 50));
    assert_eq!(g.monitors()[1].bounds.x, 1440);
}
#[test]
fn negative_origins_appkit_flip_roundtrip() {
    let g = retina();
    let r = Rect::new(-500, -300, 100, 50);
    let n = g.desktop_to_appkit(r).unwrap();
    assert_eq!(n, NativeRect::new(-500.0, 1150.0, 100.0, 50.0));
    assert_eq!(g.appkit_to_desktop(n).unwrap(), r);
}
#[test]
fn work_area_excludes_menu_bar_and_dock() {
    assert_eq!(
        retina().monitors()[0].work_area,
        Rect::new(0, 24, 1440, 826)
    );
}
#[test]
fn fractional_bounds_expand_half_open_edges() {
    let r = NativeRect::new(-10.25, -2.5, 100.5, 50.25);
    assert_eq!(r.enclosing_rect().unwrap(), Rect::new(-11, -3, 102, 51));
}
#[test]
fn display_removal_invalidates_capture_plan() {
    let cache = TopologyCache::new(retina());
    let plan = cache
        .snapshot()
        .capture_plan(Rect::new(10, 20, 100, 50))
        .unwrap();
    cache.publish(900.0, vec![]).unwrap();
    assert_eq!(
        cache.snapshot().ensure_current(&plan),
        Err(PlatformError::InvalidRegion)
    );
    assert_eq!(cache.topology_revision(), 2);
}
#[test]
fn monitors_read_off_main_uses_cache_without_marker() {
    let cache = TopologyCache::new(retina());
    let got = std::thread::spawn(move || cache.snapshot().monitors())
        .join()
        .unwrap();
    assert_eq!(got[0].bounds, Rect::new(0, 0, 1440, 900));
}
#[test]
fn overlay_threshold_uses_starting_display_backing_pixels() {
    let g = retina();
    assert!(g.selection_accepted(1, Rect::new(0, 0, 5, 5)).unwrap());
    assert!(!g.selection_accepted(1, Rect::new(0, 0, 4, 5)).unwrap());
}
#[test]
fn gaps_stay_black_and_offscreen_is_rejected() {
    let g = MacGeometry::new(
        1,
        900.0,
        vec![
            display(1, 0.0, 0.0, 100.0, 100.0, 1.0),
            display(2, 200.0, 0.0, 100.0, 100.0, 2.0),
        ],
    )
    .unwrap();
    let p = g.capture_plan(Rect::new(50, 10, 200, 20)).unwrap();
    assert_eq!(p.output_size, (400, 40));
    assert_eq!(p.background_rgba, [0, 0, 0, 255]);
    assert_eq!(p.tiles[1].destination.x, 300);
    assert!(g.capture_plan(Rect::new(120, 10, 20, 20)).is_err());
}
#[test]
fn invalid_geometry_is_rejected() {
    assert!(NativeRect::new(f64::NAN, 0.0, 1.0, 1.0)
        .enclosing_rect()
        .is_err());
    assert!(retina().capture_plan(Rect::new(0, 0, 0, 10)).is_err());
    assert!(MacGeometry::new(1, 900.0, vec![display(1, 0.0, 0.0, 10.0, 10.0, 0.0)]).is_err());
}
