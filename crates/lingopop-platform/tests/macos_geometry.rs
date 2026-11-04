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

#[cfg(target_os = "macos")]
#[test]
fn native_monitors_read_off_main_uses_cache_without_marker() {
    use lingopop_platform::{MacMonitors, Monitors};
    let monitors = MacMonitors::from_geometry(retina());
    let worker = monitors.clone();
    let got = std::thread::spawn(move || worker.monitors().unwrap())
        .join()
        .unwrap();
    assert_eq!(got[0].scale_factor, 1.0);
    assert_eq!(got[0].bounds, Rect::new(0, 0, 1440, 900));
    assert_eq!(monitors.topology_revision(), 1);
}

#[test]
fn topology_change_with_same_ids_invalidates_plan() {
    let cache = TopologyCache::new(retina());
    let plan = cache
        .snapshot()
        .capture_plan(Rect::new(10, 20, 100, 50))
        .unwrap();
    cache
        .publish(900.0, vec![display(1, 0.0, 0.0, 1440.0, 900.0, 1.0)])
        .unwrap();
    assert_eq!(
        cache.snapshot().ensure_current(&plan),
        Err(PlatformError::InvalidRegion)
    );
    assert!(MacGeometry::new(1, 900.0, vec![])
        .unwrap()
        .ensure_current(&plan)
        .is_err());
}
#[test]
fn above_left_displays_and_fractional_capture_are_local() {
    let g = MacGeometry::new(
        1,
        900.0,
        vec![display(2, -1000.25, -800.5, 1000.25, 800.5, 1.5)],
    )
    .unwrap();
    let p = g.capture_plan(Rect::new(-1000, -800, 100, 50)).unwrap();
    assert_eq!(p.output_size, (150, 75));
    assert_eq!(p.tiles[0].pixel_crop, Rect::new(0, 0, 151, 76));
    assert_eq!(g.monitors()[0].bounds, Rect::new(-1001, -801, 1001, 801));
}

#[test]
fn fractional_work_area_rounds_inward_while_display_bounds_expand() {
    let mut d = display(1, -10.25, -2.5, 100.5, 150.25, 2.0);
    d.work_area = NativeRect::new(-9.75, 21.25, 99.5, 76.5);
    let g = MacGeometry::new(1, 900.0, vec![d]).unwrap();
    assert_eq!(g.monitors()[0].bounds, Rect::new(-11, -3, 102, 151));
    assert_eq!(g.monitors()[0].work_area, Rect::new(-9, 22, 98, 75));
}

#[cfg(target_os = "macos")]
#[test]
fn initialization_off_main_is_nonfatal() {
    let monitors = lingopop_platform::MacMonitors::from_geometry(retina());
    let result = std::thread::spawn(move || monitors.initialize_on_main_thread())
        .join()
        .unwrap();
    assert_eq!(result, Ok(()));
}

#[test]
fn appkit_primary_visible_frame_excludes_menu_bar_and_dock() {
    assert_eq!(
        MacGeometry::appkit_frame_to_desktop(900.0, NativeRect::new(0.0, 50.0, 1440.0, 826.0)),
        NativeRect::new(0.0, 24.0, 1440.0, 826.0)
    );
}
#[test]
fn appkit_above_display_visible_frame_keeps_its_own_menu_bar() {
    assert_eq!(
        MacGeometry::appkit_frame_to_desktop(900.0, NativeRect::new(0.0, 900.0, 1920.0, 1056.0)),
        NativeRect::new(0.0, -1056.0, 1920.0, 1056.0)
    );
}
#[test]
fn appkit_left_display_frame_preserves_negative_fractional_x() {
    assert_eq!(
        MacGeometry::appkit_frame_to_desktop(
            900.0,
            NativeRect::new(-1920.25, 50.5, 1920.25, 825.25)
        ),
        NativeRect::new(-1920.25, 24.25, 1920.25, 825.25)
    );
}
#[test]
fn initial_topology_failure_registers_observer_first_and_notification_repairs_same_cache() {
    use std::cell::RefCell;
    for invalid_bounds in [false, true] {
        let cache = TopologyCache::new(retina());
        let shared = cache.clone();
        let old_plan = cache
            .snapshot()
            .capture_plan(Rect::new(10, 20, 100, 50))
            .unwrap();
        let events = RefCell::new(Vec::new());
        cache.initialize_with(
            || events.borrow_mut().push("observer registered".to_owned()),
            || {
                assert_eq!(*events.borrow(), vec!["observer registered"]);
                if invalid_bounds {
                    Ok((900.0, vec![display(1, f64::NAN, 0.0, 1440.0, 900.0, 2.0)]))
                } else {
                    // Missing NSScreenNumber or other enumeration failure.
                    Err(PlatformError::InvalidRegion)
                }
            },
            &|line| events.borrow_mut().push(line.to_owned()),
        );
        assert_eq!(
            *events.borrow(),
            vec!["observer registered", "display topology refresh failed"]
        );
        assert!(shared.snapshot().monitors().is_empty());
        assert_eq!(
            shared.snapshot().ensure_current(&old_plan),
            Err(PlatformError::InvalidRegion)
        );
        shared.refresh_with(
            || Ok((900.0, vec![display(1, 0.0, 0.0, 1440.0, 900.0, 2.0)])),
            &|_| panic!("successful refresh logged a failure"),
        );
        assert_eq!(
            cache.snapshot().monitors()[0].bounds,
            Rect::new(0, 0, 1440, 900)
        );
        assert_eq!(cache.topology_revision(), 3);
    }
}
