//! Contract tests of the portable macOS capture core (`macos_screen`): plan execution, the
//! permission sequencing around every native call, row-stride and colour conversion. They run
//! on every OS over an injected `NativeScreen`; no real display is touched.

use lingopop_platform::fake::FakePermissions;
use lingopop_platform::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
use lingopop_platform::macos_screen::{
    GeometrySource, NativeBitmap, NativeScreen, PixelFormat, ScreenCaptureAdapter,
};
use lingopop_platform::{
    PermissionKind, PermissionState, Permissions, PlatformError, Rect, ScreenCapture,
};
use std::sync::{Arc, Mutex};

type Hook = Box<dyn Fn(usize) + Send + Sync>;

/// Records every request; answers with a solid bitmap of `colour` sized like the requested
/// rectangle times that display's backing scale.
struct FakeScreen {
    scales: Vec<(u32, f64)>,
    colours: Vec<(u32, [u8; 4])>,
    requests: Mutex<Vec<(u32, NativeRect)>>,
    /// Runs after the call counter was bumped, before the bitmap is returned.
    hook: Mutex<Option<Hook>>,
    fail: Mutex<bool>,
    override_bitmap: Mutex<Option<NativeBitmap>>,
}

impl FakeScreen {
    fn new(scales: &[(u32, f64)], colours: &[(u32, [u8; 4])]) -> Arc<Self> {
        Arc::new(Self {
            scales: scales.to_vec(),
            colours: colours.to_vec(),
            requests: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            fail: Mutex::new(false),
            override_bitmap: Mutex::new(None),
        })
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn requests(&self) -> Vec<(u32, NativeRect)> {
        self.requests.lock().unwrap().clone()
    }
    fn on_call(&self, f: impl Fn(usize) + Send + Sync + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(f));
    }
}

struct Shared(Arc<FakeScreen>);

impl NativeScreen for Shared {
    fn create_image(
        &self,
        display_id: u32,
        rect: NativeRect,
    ) -> Result<NativeBitmap, PlatformError> {
        let s = &self.0;
        let n = {
            let mut r = s.requests.lock().unwrap();
            r.push((display_id, rect));
            r.len()
        };
        if let Some(h) = s.hook.lock().unwrap().as_ref() {
            h(n);
        }
        if *s.fail.lock().unwrap() {
            return Err(PlatformError::Os {
                api: "CGDisplayCreateImageForRect",
                code: 0,
            });
        }
        if let Some(b) = s.override_bitmap.lock().unwrap().take() {
            return Ok(b);
        }
        let scale = s.scales.iter().find(|(d, _)| *d == display_id).unwrap().1;
        let colour = s.colours.iter().find(|(d, _)| *d == display_id).unwrap().1;
        let w = (rect.width * scale).round() as u32;
        let h = (rect.height * scale).round() as u32;
        let mut data = Vec::new();
        for _ in 0..w * h {
            data.extend_from_slice(&[colour[2], colour[1], colour[0], 0]); // BGRA, alpha 0 on purpose
        }
        Ok(NativeBitmap {
            width: w,
            height: h,
            bytes_per_row: w as usize * 4,
            format: PixelFormat::Bgra,
            data,
        })
    }
}

fn display(id: u32, x: f64, y: f64, w: f64, h: f64, scale: f64) -> MacDisplay {
    MacDisplay {
        id,
        desktop_bounds: NativeRect::new(x, y, w, h),
        work_area: NativeRect::new(x, y + 24.0, w, h - 74.0),
        backing_scale: scale,
        primary: id == 1,
    }
}

fn topology(displays: Vec<MacDisplay>) -> TopologyCache {
    let cache = TopologyCache::new(MacGeometry::empty());
    cache.publish(900.0, displays).unwrap();
    cache
}

fn adapter(
    screen: &Arc<FakeScreen>,
    cache: &TopologyCache,
    permissions: &Arc<FakePermissions>,
) -> ScreenCaptureAdapter<Shared> {
    let p: Arc<dyn Permissions> = permissions.clone();
    ScreenCaptureAdapter::new(Shared(screen.clone()), Arc::new(cache.clone()), p)
}

fn retina() -> (Arc<FakeScreen>, TopologyCache, Arc<FakePermissions>) {
    (
        FakeScreen::new(&[(1, 2.0)], &[(1, [10, 20, 30, 255])]),
        topology(vec![display(1, 0.0, 0.0, 1440.0, 900.0, 2.0)]),
        FakePermissions::new(),
    )
}

/// Regression anchor: a denied Screen Recording never reaches the native API, and the result
/// is an error, never a (wallpaper-only) image.
#[test]
fn screen_permission_denial_is_not_empty_success() {
    let (screen, cache, permissions) = retina();
    permissions.set_state(PermissionKind::ScreenRecording, PermissionState::Denied);
    let capture = adapter(&screen, &cache, &permissions);
    let result = capture.capture_region(Rect::new(10, 20, 100, 50));
    assert_eq!(
        result,
        Err(PlatformError::PermissionDenied(
            PermissionKind::ScreenRecording
        ))
    );
    assert_eq!(screen.calls(), 0);
    // The adapter never prompts: only the OS-level `request` may.
    assert_eq!(
        permissions.request_calls(PermissionKind::ScreenRecording),
        0
    );

    // 3x2 image with 16-byte padded rows (12 meaningful bytes + 4 padding).
    let (screen, _cache, permissions) = retina();
    let padded: Vec<u8> = (0..2u8)
        .flat_map(|row| {
            (0..3u8)
                .flat_map(move |col| [row * 10 + col, 100, 200, 7]) // B, G, R, A = 7
                .chain([0xEE; 4])
        })
        .collect();
    assert_eq!(padded.len(), 32);
    *screen.override_bitmap.lock().unwrap() = Some(NativeBitmap {
        width: 3,
        height: 2,
        bytes_per_row: 16,
        format: PixelFormat::Bgra,
        data: padded,
    });
    let cache1 = topology(vec![display(1, 0.0, 0.0, 200.0, 200.0, 1.0)]);
    let capture = adapter(&screen, &cache1, &permissions);
    let image = capture.capture_region(Rect::new(0, 0, 3, 2)).unwrap();
    let padded_rows_output = image.rgba.clone();
    let output_alpha_values: Vec<u8> = padded_rows_output.chunks(4).map(|px| px[3]).collect();
    assert_eq!(padded_rows_output.len(), 3 * 2 * 4);
    assert_eq!(output_alpha_values, vec![255; 6]);
    // BGRA -> RGBA, row by row, padding dropped.
    assert_eq!(&padded_rows_output[0..4], &[200, 100, 0, 255]);
    assert_eq!(&padded_rows_output[8..12], &[200, 100, 2, 255]);
    assert_eq!(&padded_rows_output[12..16], &[200, 100, 10, 255]);
    assert_eq!(&padded_rows_output[20..24], &[200, 100, 12, 255]);
}

#[test]
fn every_non_granted_state_is_denied_without_native_calls() {
    for state in [
        PermissionState::NotDetermined,
        PermissionState::Denied,
        PermissionState::Restricted,
    ] {
        let (screen, cache, permissions) = retina();
        permissions.set_state(PermissionKind::ScreenRecording, state);
        let r = adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50));
        assert_eq!(
            r,
            Err(PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording
            )),
            "{state:?}"
        );
        assert_eq!(screen.calls(), 0, "{state:?}");
    }
}

#[test]
fn permission_revoked_during_the_native_call_never_exposes_the_image() {
    let (screen, cache, permissions) = retina();
    let p = permissions.clone();
    screen.on_call(move |_| p.set_state(PermissionKind::ScreenRecording, PermissionState::Denied));
    let r = adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50));
    assert_eq!(
        r,
        Err(PlatformError::PermissionDenied(
            PermissionKind::ScreenRecording
        ))
    );
    assert_eq!(screen.calls(), 1);
}

#[test]
fn hundred_by_fifty_points_on_a_2x_screen_is_200_by_100_pixels() {
    let (screen, cache, permissions) = retina();
    let image = adapter(&screen, &cache, &permissions)
        .capture_region(Rect::new(10, 20, 100, 50))
        .unwrap();
    assert_eq!((image.width, image.height), (200, 100));
    assert_eq!(image.rgba.len(), 200 * 100 * 4);
    assert!(image.rgba.chunks(4).all(|p| p == [10, 20, 30, 255]));
    // The native call gets display-local POINTS (top-left origin), not pixels.
    assert_eq!(
        screen.requests(),
        vec![(1, NativeRect::new(10.0, 20.0, 100.0, 50.0))]
    );
}

#[test]
fn a_partly_outside_region_is_clipped_to_the_screen() {
    let (screen, cache, permissions) = retina();
    let image = adapter(&screen, &cache, &permissions)
        .capture_region(Rect::new(1400, 880, 100, 100))
        .unwrap();
    // 40x20 points remain on screen, at 2x.
    assert_eq!((image.width, image.height), (80, 40));
    assert_eq!(
        screen.requests(),
        vec![(1, NativeRect::new(1400.0, 880.0, 40.0, 20.0))]
    );
}

#[test]
fn fully_outside_or_empty_regions_are_invalid_and_capture_nothing() {
    let (screen, cache, permissions) = retina();
    let capture = adapter(&screen, &cache, &permissions);
    assert_eq!(
        capture.capture_region(Rect::new(5000, 5000, 10, 10)),
        Err(PlatformError::InvalidRegion)
    );
    assert_eq!(
        capture.capture_region(Rect::new(0, 0, 0, 10)),
        Err(PlatformError::InvalidRegion)
    );
    assert_eq!(screen.calls(), 0);
}

#[test]
fn multi_display_composition_follows_the_plan() {
    // A (2x) covers x 0..1440, B (1x) covers x 1440..2440, B is 100 points lower, so the strip
    // above B inside the region has no display: it stays opaque black.
    let screen = FakeScreen::new(
        &[(1, 2.0), (2, 1.0)],
        &[(1, [255, 0, 0, 255]), (2, [0, 0, 255, 255])],
    );
    let cache = topology(vec![
        display(1, 0.0, 0.0, 1440.0, 900.0, 2.0),
        display(2, 1440.0, 100.0, 1000.0, 800.0, 1.0),
    ]);
    let permissions = FakePermissions::new();
    let region = Rect::new(1400, 20, 100, 200);
    let plan = cache.snapshot().capture_plan(region).unwrap();
    let image = adapter(&screen, &cache, &permissions)
        .capture_region(region)
        .unwrap();
    assert_eq!(
        (image.width, image.height),
        (plan.output_size.0, plan.output_size.1)
    );
    assert_eq!((image.width, image.height), (200, 400));
    let px = |x: usize, y: usize| {
        let o = (y * 200 + x) * 4;
        [
            image.rgba[o],
            image.rgba[o + 1],
            image.rgba[o + 2],
            image.rgba[o + 3],
        ]
    };
    // Left 40 points (80 px) come from display A: red.
    assert_eq!(px(0, 0), [255, 0, 0, 255]);
    assert_eq!(px(79, 399), [255, 0, 0, 255]);
    // Right part from display B (upsampled to the output scale), from y = 100 points -> px 160.
    assert_eq!(px(80, 200), [0, 0, 255, 255]);
    assert_eq!(px(199, 399), [0, 0, 255, 255]);
    // No display above B: black, never another display's content.
    assert_eq!(px(150, 10), [0, 0, 0, 255]);
    assert_eq!(px(80, 159), [0, 0, 0, 255]);
    // Each native request is the display-local points rectangle of its tile.
    let reqs = screen.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0], (1, NativeRect::new(1400.0, 20.0, 40.0, 200.0)));
    assert_eq!(reqs[1], (2, NativeRect::new(0.0, 0.0, 60.0, 120.0)));
}

#[test]
fn a_topology_change_during_capture_fails_closed() {
    let (screen, cache, permissions) = retina();
    let c = cache.clone();
    screen.on_call(move |_| {
        // A display reconfiguration lands while the native call is running.
        c.publish(900.0, vec![display(1, 0.0, 0.0, 1440.0, 900.0, 1.0)])
            .unwrap();
    });
    let r = adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50));
    // Not `InvalidRegion` (the user would be told "selection too small"): a fixed-text `Os`
    // error that the shell shows as "Screen capture failed. Try again.". The capture plan's own
    // `InvalidRegion` contract (a region the topology cannot satisfy) is unchanged, see below.
    assert_eq!(
        r,
        Err(PlatformError::Os {
            api: "display topology changed",
            code: 0
        })
    );
}

#[test]
fn a_display_removed_during_capture_also_fails_with_the_os_error() {
    let (screen, cache, permissions) = retina();
    let adapter = adapter(&screen, &cache, &permissions);
    // The display of the planned tile disappears while the native call runs.
    let c = cache.clone();
    screen.on_call(move |_| {
        c.publish(900.0, vec![]).unwrap();
    });
    let r = adapter.capture_region(Rect::new(0, 0, 50, 50));
    assert_eq!(
        r,
        Err(PlatformError::Os {
            api: "display topology changed",
            code: 0
        })
    );
}

#[test]
fn an_empty_topology_is_an_invalid_region() {
    let screen = FakeScreen::new(&[], &[]);
    let cache = TopologyCache::new(MacGeometry::empty());
    let permissions = FakePermissions::new();
    let r = adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50));
    assert_eq!(r, Err(PlatformError::InvalidRegion));
    assert_eq!(screen.calls(), 0);
}

#[test]
fn a_native_failure_is_an_error_not_an_empty_image() {
    let (screen, cache, permissions) = retina();
    *screen.fail.lock().unwrap() = true;
    let r = adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50));
    assert_eq!(
        r,
        Err(PlatformError::Os {
            api: "CGDisplayCreateImageForRect",
            code: 0
        })
    );
}

#[test]
fn malformed_native_bitmaps_are_rejected_without_panicking() {
    let bad = |bitmap: NativeBitmap| {
        let (screen, cache, permissions) = retina();
        *screen.override_bitmap.lock().unwrap() = Some(bitmap);
        adapter(&screen, &cache, &permissions).capture_region(Rect::new(0, 0, 50, 50))
    };
    let os = Err(PlatformError::Os {
        api: "CGDisplayCreateImageForRect",
        code: 0,
    });
    // Too little data for the declared rows.
    assert_eq!(
        bad(NativeBitmap {
            width: 4,
            height: 4,
            bytes_per_row: 16,
            format: PixelFormat::Rgba,
            data: vec![0; 15],
        }),
        os
    );
    // Row stride smaller than one row of pixels.
    assert_eq!(
        bad(NativeBitmap {
            width: 4,
            height: 1,
            bytes_per_row: 8,
            format: PixelFormat::Rgba,
            data: vec![0; 64],
        }),
        os
    );
    // Zero size.
    assert_eq!(
        bad(NativeBitmap {
            width: 0,
            height: 4,
            bytes_per_row: 0,
            format: PixelFormat::Rgba,
            data: vec![],
        }),
        os
    );
}

#[test]
fn rgba_input_passes_through_and_alpha_is_forced_opaque() {
    let (screen, _cache, permissions) = retina();
    *screen.override_bitmap.lock().unwrap() = Some(NativeBitmap {
        width: 100,
        height: 50,
        bytes_per_row: 400,
        format: PixelFormat::Rgba,
        data: [1, 2, 3, 0].repeat(100 * 50),
    });
    let cache1 = topology(vec![display(1, 0.0, 0.0, 200.0, 200.0, 1.0)]);
    let image = adapter(&screen, &cache1, &permissions)
        .capture_region(Rect::new(0, 0, 100, 50))
        .unwrap();
    assert!(image.rgba.chunks(4).all(|p| p == [1, 2, 3, 255]));
}

/// A fractional crop (scale 1.5) asks the OS for the integral points that enclose it and is cut
/// back to the plan's exact pixel rectangle: no OS-side widening leaks into the image.
#[test]
fn a_fractional_crop_requests_integral_points_and_returns_the_exact_pixels() {
    let screen = FakeScreen::new(&[(1, 1.5)], &[(1, [0, 0, 0, 255])]);
    let cache = topology(vec![display(1, 0.0, 0.0, 1440.0, 900.0, 1.5)]);
    let permissions = FakePermissions::new();
    // Points (0,0,4,4) at scale 1.5 are 6x6 pixels; red = x, green = y of that bitmap.
    let mut data = Vec::new();
    for y in 0..6u8 {
        for x in 0..6u8 {
            data.extend_from_slice(&[x, y, 0, 0]);
        }
    }
    *screen.override_bitmap.lock().unwrap() = Some(NativeBitmap {
        width: 6,
        height: 6,
        bytes_per_row: 24,
        format: PixelFormat::Rgba,
        data,
    });
    let image = adapter(&screen, &cache, &permissions)
        .capture_region(Rect::new(1, 1, 3, 3))
        .unwrap();
    assert_eq!(
        screen.requests(),
        vec![(1, NativeRect::new(0.0, 0.0, 4.0, 4.0))]
    );
    assert_eq!((image.width, image.height), (5, 5));
    // Pixel (0,0) of the output is the bitmap's (1,1): the crop offset, not the widened origin.
    assert_eq!(&image.rgba[0..4], &[1, 1, 0, 255]);
    let last = (4 * 5 + 4) * 4;
    assert_eq!(&image.rgba[last..last + 4], &[5, 5, 0, 255]);
}

/// A region picked against an older topology revision is refused before any native call:
/// its coordinates belong to a layout that is gone. The same revision captures normally.
#[test]
fn a_region_picked_against_another_topology_revision_is_not_captured() {
    let (screen, cache, permissions) = retina();
    let live = cache.topology_revision();
    let capture = adapter(&screen, &cache, &permissions);
    let region = Rect::new(10, 20, 100, 50);
    assert!(capture.capture_region_at(region, Some(live)).is_ok());
    assert_eq!(screen.calls(), 1);

    cache
        .publish(900.0, vec![display(1, 0.0, 0.0, 1440.0, 900.0, 2.0)])
        .unwrap();
    assert_eq!(
        capture.capture_region_at(region, Some(live)),
        Err(PlatformError::Os {
            api: "display topology changed",
            code: 0
        })
    );
    assert_eq!(screen.calls(), 1, "the stale region never reached the OS");
    // No revision (plain `capture_region`, Windows) is not compared.
    assert!(capture.capture_region(region).is_ok());
}

/// Plays the topology observer: each `snapshot()` returns the next scripted layout (the last one
/// repeats), so a test fixes exactly which read sees which revision.
struct ScriptedGeometry {
    script: Mutex<Vec<MacGeometry>>,
    reads: Mutex<usize>,
}

impl GeometrySource for ScriptedGeometry {
    fn snapshot(&self) -> MacGeometry {
        let mut reads = self.reads.lock().unwrap();
        let script = self.script.lock().unwrap();
        let at = (*reads).min(script.len() - 1);
        *reads += 1;
        script[at].clone()
    }
}

/// The preliminary check (`ensure_topology`) sees revision R; by the time `capture_region_at`
/// reads its own snapshot to clip and plan, the layout is R+1 with geometry that still overlaps
/// the picked rect. The pick is stale: refused before any native call, never planned on R+1.
#[test]
fn a_topology_published_after_the_preliminary_check_is_still_refused_before_planning() {
    let layout = |revision| {
        MacGeometry::new(
            revision,
            900.0,
            vec![display(1, 0.0, 0.0, 1440.0, 900.0, 2.0)],
        )
        .unwrap()
    };
    let screen = FakeScreen::new(&[(1, 2.0)], &[(1, [10, 20, 30, 255])]);
    let permissions = FakePermissions::new();
    let source = Arc::new(ScriptedGeometry {
        script: Mutex::new(vec![layout(5), layout(6)]),
        reads: Mutex::new(0),
    });
    let p: Arc<dyn Permissions> = permissions.clone();
    let capture = ScreenCaptureAdapter::new(Shared(screen.clone()), source.clone(), p);
    let region = Rect::new(10, 20, 100, 50); // valid and overlapping in both layouts

    // First read (R = 5): the orchestrator's early check passes.
    assert_eq!(capture.ensure_topology(Some(5)), Ok(()));
    // Second read (R+1 = 6) is the one that would clip and plan: it must be compared too.
    assert_eq!(
        capture.capture_region_at(region, Some(5)),
        Err(PlatformError::Os {
            api: "display topology changed",
            code: 0
        })
    );
    assert_eq!(screen.calls(), 0, "no native call for a stale pick");
    assert_eq!(*source.reads.lock().unwrap(), 2, "one read per call");
}

/// One call, one snapshot: `capture_region_at` compares the pick's revision with the very
/// snapshot it plans from (read 0 = revision 5, a 2x layout). The observer then publishes
/// revision 6 (1x); the per-tile guard sees it and refuses before any native call. Before the
/// fix the revision was compared with a separate EARLIER read while the plan came from a later
/// one, so a pick made at 5 was planned on revision 6 and captured without any error.
#[test]
fn a_pick_is_compared_with_the_snapshot_it_is_planned_from() {
    let layout = |revision, scale| {
        MacGeometry::new(
            revision,
            900.0,
            vec![display(1, 0.0, 0.0, 1440.0, 900.0, scale)],
        )
        .unwrap()
    };
    let screen = FakeScreen::new(&[(1, 2.0)], &[(1, [10, 20, 30, 255])]);
    let permissions = FakePermissions::new();
    let source = Arc::new(ScriptedGeometry {
        script: Mutex::new(vec![layout(5, 2.0), layout(6, 1.0)]),
        reads: Mutex::new(0),
    });
    let p: Arc<dyn Permissions> = permissions.clone();
    let capture = ScreenCaptureAdapter::new(Shared(screen.clone()), source.clone(), p);
    let result = capture.capture_region_at(Rect::new(10, 20, 100, 50), Some(5));
    assert_eq!(
        result,
        Err(PlatformError::Os {
            api: "display topology changed",
            code: 0
        })
    );
    assert_eq!(
        screen.calls(),
        0,
        "refused by the guard before the native call"
    );
    assert!(*source.reads.lock().unwrap() >= 2);
}
