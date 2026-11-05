//! Contract tests of the portable macOS capture core (`macos_screen`): plan execution, the
//! permission sequencing around every native call, row-stride and colour conversion. They run
//! on every OS over an injected `NativeScreen`; no real display is touched.

use lingopop_platform::fake::FakePermissions;
use lingopop_platform::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
use lingopop_platform::macos_screen::{
    NativeBitmap, NativeScreen, PixelFormat, ScreenCaptureAdapter,
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
    let (screen, cache, permissions) = retina();
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
    let _ = cache;
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
    assert_eq!(r, Err(PlatformError::InvalidRegion));
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
    let (screen, cache, permissions) = retina();
    *screen.override_bitmap.lock().unwrap() = Some(NativeBitmap {
        width: 100,
        height: 50,
        bytes_per_row: 400,
        format: PixelFormat::Rgba,
        data: [1, 2, 3, 0].repeat(100 * 50),
    });
    let cache1 = topology(vec![display(1, 0.0, 0.0, 200.0, 200.0, 1.0)]);
    let _ = cache;
    let image = adapter(&screen, &cache1, &permissions)
        .capture_region(Rect::new(0, 0, 100, 50))
        .unwrap();
    assert!(image.rgba.chunks(4).all(|p| p == [1, 2, 3, 255]));
}
