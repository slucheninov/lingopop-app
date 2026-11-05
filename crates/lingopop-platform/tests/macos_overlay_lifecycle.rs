//! Portable lifecycle tests of the macOS region overlay (`macos_overlay`): every OS, no AppKit.
//! The native overlay is a `FakeOverlayBackend` that records create / cleanup sequence numbers;
//! the main thread is a `FakeMainThread` (a pump thread, or driven by hand).
use crossbeam_channel::{bounded, Sender};
use lingopop_platform::fake::{FakeMainThread, FakeOverlayBackend, FakeSleeper};
use lingopop_platform::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
use lingopop_platform::macos_overlay::{
    selection_in_display, MacOverlay, TopologySource, COMPOSITOR_SETTLE,
};
use lingopop_platform::overlay::OverlayInput;
use lingopop_platform::{PlatformError, Point, Rect, RegionOverlay, Sleeper};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

type PickResult = Result<Option<Rect>, PlatformError>;

fn display(id: u32, x: f64, y: f64, w: f64, h: f64, scale: f64, primary: bool) -> MacDisplay {
    MacDisplay {
        id,
        desktop_bounds: NativeRect::new(x, y, w, h),
        work_area: NativeRect::new(x, y, w, h),
        backing_scale: scale,
        primary,
    }
}
fn one_display(scale: f64) -> Vec<MacDisplay> {
    vec![display(1, 0.0, 0.0, 1440.0, 900.0, scale, true)]
}
fn p(x: i32, y: i32) -> Point {
    Point { x, y }
}

struct Harness {
    main: Arc<FakeMainThread>,
    backend: Arc<FakeOverlayBackend>,
    cache: TopologyCache,
    sleeper: Arc<FakeSleeper>,
    overlay: Arc<MacOverlay<FakeOverlayBackend>>,
    pump: Option<JoinHandle<()>>,
    drags: std::sync::atomic::AtomicUsize,
}
impl Harness {
    fn with(displays: Vec<MacDisplay>, pump: bool) -> Self {
        let main = FakeMainThread::new();
        let backend = FakeOverlayBackend::new();
        let cache = TopologyCache::new(MacGeometry::new(1, 900.0, displays).unwrap());
        let sleeper = Arc::new(FakeSleeper::new());
        let overlay = Arc::new(
            MacOverlay::new(backend.clone(), Arc::new(cache.clone()), sleeper.clone())
                .with_topology_poll(Duration::from_millis(5)),
        );
        overlay.bind_main_thread(main.clone());
        let pump = pump.then(|| {
            let m = main.clone();
            std::thread::spawn(move || m.pump())
        });
        Self {
            main,
            backend,
            cache,
            sleeper,
            overlay,
            pump,
            drags: 0.into(),
        }
    }
    /// A pump thread plays the main loop.
    fn new(displays: Vec<MacDisplay>) -> Self {
        Self::with(displays, true)
    }
    /// The test runs main-thread jobs by hand (`run_pending`).
    fn manual(displays: Vec<MacDisplay>) -> Self {
        Self::with(displays, false)
    }
    fn pick(&self) -> (Sender<()>, JoinHandle<PickResult>) {
        let (tx, rx) = bounded(1);
        let overlay = self.overlay.clone();
        (tx, std::thread::spawn(move || overlay.pick(rx)))
    }
    /// Starts a pick and waits until its native overlay exists.
    fn pick_created(&self, n: usize) -> (Sender<()>, JoinHandle<PickResult>) {
        let started = self.pick();
        self.backend.wait_for_creates(n);
        started
    }
    /// A full drag through the pump-driven harness (the nth drag waits for the nth overlay).
    fn drag(&self, events: &[OverlayInput]) -> PickResult {
        let n = self.drags.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let (_tx, join) = self.pick_created(n);
        for e in events {
            assert!(self.backend.fire(*e), "overlay must consume {e:?}");
        }
        join.join().unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.main.close();
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

#[test]
fn escape_cancels_with_cleanup() {
    let h = Harness::new(one_display(1.0));
    let result = h.drag(&[
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Escape,
    ]);
    assert_eq!(result, Ok(None));
    assert_eq!((h.backend.creates(), h.backend.cleanups()), (1, 1));
    assert_eq!(h.backend.native_active(), 0);
    assert_eq!(h.sleeper.slept(), vec![COMPOSITOR_SETTLE]);
}

#[test]
fn dropping_the_cancel_sender_cancels_with_cleanup() {
    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    drop(tx);
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!((h.backend.creates(), h.backend.cleanups()), (1, 1));
}

#[test]
fn sending_on_the_cancel_channel_cancels_with_cleanup() {
    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    tx.send(()).unwrap();
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!(h.backend.cleanups(), 1);
}

#[test]
fn a_sender_dropped_before_the_pick_creates_nothing() {
    let h = Harness::new(one_display(1.0));
    let (tx, rx) = bounded::<()>(1);
    drop(tx);
    assert_eq!(h.overlay.pick(rx), Ok(None));
    assert_eq!((h.backend.creates(), h.backend.cleanups()), (0, 0));
}

#[test]
fn threshold_is_ten_backing_pixels_at_1x() {
    let h = Harness::new(one_display(1.0));
    let small = h.drag(&[
        OverlayInput::Down(p(100, 100)),
        OverlayInput::Up(p(109, 200)),
    ]);
    assert_eq!(small, Ok(None));
    let exact = h.drag(&[
        OverlayInput::Down(p(100, 100)),
        OverlayInput::Up(p(110, 110)),
    ]);
    assert_eq!(exact, Ok(Some(Rect::new(100, 100, 10, 10))));
}

#[test]
fn threshold_is_ten_backing_pixels_at_2x() {
    let h = Harness::new(one_display(2.0));
    let small = h.drag(&[
        OverlayInput::Down(p(100, 100)),
        OverlayInput::Up(p(104, 110)),
    ]);
    assert_eq!(small, Ok(None), "4 points = 8 backing pixels");
    let exact = h.drag(&[
        OverlayInput::Down(p(100, 100)),
        OverlayInput::Up(p(105, 105)),
    ]);
    assert_eq!(exact, Ok(Some(Rect::new(100, 100, 5, 5))), "5 points = 10");
}

#[test]
fn threshold_uses_the_display_where_the_drag_started() {
    let two = vec![
        display(1, 0.0, 0.0, 1000.0, 800.0, 1.0, true),
        display(2, 1000.0, 0.0, 1000.0, 800.0, 2.0, false),
    ];
    let h = Harness::new(two);
    // Starts on the 1x display, ends over the 2x one: 5 points are only 5 pixels.
    let from_1x = h.drag(&[
        OverlayInput::Down(p(998, 100)),
        OverlayInput::Up(p(1003, 200)),
    ]);
    assert_eq!(from_1x, Ok(None));
    // Starts on the 2x display: 5 points are 10 backing pixels.
    let from_2x = h.drag(&[
        OverlayInput::Down(p(1002, 100)),
        OverlayInput::Up(p(997, 200)),
    ]);
    assert_eq!(from_2x, Ok(Some(Rect::new(997, 100, 5, 100))));
}

#[test]
fn negative_origins_and_reverse_drags_are_normalized() {
    let two = vec![
        display(1, 0.0, 0.0, 1440.0, 900.0, 2.0, true),
        display(2, -1920.0, -100.0, 1920.0, 1080.0, 1.0, false),
    ];
    let h = Harness::new(two);
    let reverse = h.drag(&[
        OverlayInput::Down(p(-1000, 500)),
        OverlayInput::Move(p(-1050, 450)),
        OverlayInput::Up(p(-1100, 400)),
    ]);
    assert_eq!(reverse, Ok(Some(Rect::new(-1100, 400, 100, 100))));
    // Exact threshold on the 1x display with a negative origin: 10 accepted, 9 not.
    assert_eq!(
        h.drag(&[
            OverlayInput::Down(p(-500, -50)),
            OverlayInput::Up(p(-510, -40))
        ]),
        Ok(Some(Rect::new(-510, -50, 10, 10)))
    );
    assert_eq!(
        h.drag(&[
            OverlayInput::Down(p(-500, -50)),
            OverlayInput::Up(p(-509, -40))
        ]),
        Ok(None)
    );
}

#[test]
fn a_click_without_a_drag_is_cancelled() {
    let h = Harness::new(one_display(1.0));
    let result = h.drag(&[
        OverlayInput::Down(p(300, 300)),
        OverlayInput::Up(p(300, 300)),
    ]);
    assert_eq!(result, Ok(None));
    assert_eq!(h.backend.cleanups(), 1);
}

#[test]
fn one_native_window_set_covers_every_display() {
    let two = vec![
        display(1, 0.0, 0.0, 1000.0, 800.0, 1.0, true),
        display(2, 1000.0, 0.0, 1000.0, 800.0, 2.0, false),
    ];
    let h = Harness::new(two);
    let (tx, join) = h.pick_created(1);
    assert_eq!(h.backend.last_display_count(), 2);
    drop(tx);
    join.join().unwrap().unwrap();
}

/// Down, Move and Up in ONE batch keep their order (the Up finishes, a later Move cannot change
/// it) and nothing is uploaded for a finished selection.
#[test]
fn coalesced_events_keep_down_up_order() {
    let h = Harness::manual(one_display(1.0));
    let (_tx, join) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // create
    for e in [
        OverlayInput::Down(p(0, 0)),
        OverlayInput::Move(p(50, 50)),
        OverlayInput::Up(p(60, 60)),
        OverlayInput::Move(p(500, 500)),
    ] {
        h.backend.fire(e);
    }
    assert_eq!(h.main.pending(), 1, "one drain job for the whole backlog");
    h.main.run_pending();
    assert_eq!(h.backend.draws(), Vec::<Option<Rect>>::new());
    h.main.wait_for_pending(1);
    h.main.run_pending(); // cleanup
    assert_eq!(join.join().unwrap(), Ok(Some(Rect::new(0, 0, 60, 60))));
}

#[test]
fn a_move_backlog_is_one_redraw_and_identical_moves_do_not_repaint() {
    let h = Harness::manual(one_display(1.0));
    let (tx, join) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending();
    h.backend.fire(OverlayInput::Down(p(10, 10)));
    for n in 1..=50 {
        h.backend.fire(OverlayInput::Move(p(10 + n, 10 + n)));
    }
    assert_eq!(h.main.pending(), 1);
    h.main.run_pending();
    assert_eq!(h.backend.draws(), vec![Some(Rect::new(10, 10, 50, 50))]);
    h.backend.fire(OverlayInput::Move(p(60, 60)));
    h.main.run_pending();
    assert_eq!(h.backend.draws().len(), 1, "same rect is not repainted");
    drop(tx);
    h.main.wait_for_pending(1);
    h.main.run_pending();
    assert_eq!(join.join().unwrap(), Ok(None));
}

/// REGRESSION ANCHOR: the second overlay is created only after the first one's native cleanup.
#[test]
fn cancel_restart_waits_for_overlay_cleanup() {
    let h = Harness::new(one_display(1.0));
    let (first_tx, first) = h.pick_created(1);
    drop(first_tx); // cancel by dropping the sender ...
    let (_second_tx, second) = h.pick(); // ... and ask for the next pick at once
    let first_result = first.join().unwrap();
    h.backend.wait_for_creates(2);
    let max_native_overlays = h.backend.max_native();
    let first_cleanup_sequence = h.backend.cleanup_seq(1).unwrap();
    let second_create_sequence = h.backend.create_seq(2).unwrap();
    assert_eq!(first_result, Ok(None));
    assert_eq!(max_native_overlays, 1);
    assert!(first_cleanup_sequence < second_create_sequence);
    drop(_second_tx);
    assert_eq!(second.join().unwrap(), Ok(None));
}

#[test]
fn a_restart_requested_during_a_slow_cleanup_does_not_create_early() {
    let h = Harness::new(one_display(1.0));
    h.backend.hold_cleanup(true);
    let (first_tx, first) = h.pick_created(1);
    drop(first_tx);
    h.backend.wait_for_cleanups_started(1);
    let (second_tx, second) = h.pick();
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(
        h.backend.creates(),
        1,
        "no second overlay while cleaning up"
    );
    h.backend.hold_cleanup(false);
    assert_eq!(first.join().unwrap(), Ok(None));
    h.backend.wait_for_creates(2);
    assert_eq!(h.backend.max_native(), 1);
    drop(second_tx);
    assert_eq!(second.join().unwrap(), Ok(None));
}

#[test]
fn a_display_change_cancels_with_cleanup() {
    let h = Harness::new(one_display(1.0));
    let (_tx, join) = h.pick_created(1);
    // The scale of the display changed (same id, new revision).
    h.cache.publish(900.0, one_display(2.0)).unwrap();
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!((h.backend.creates(), h.backend.cleanups()), (1, 1));
    assert_eq!(h.backend.native_active(), 0);
}

#[test]
fn a_removed_display_cancels_with_cleanup() {
    let two = vec![
        display(1, 0.0, 0.0, 1000.0, 800.0, 1.0, true),
        display(2, 1000.0, 0.0, 1000.0, 800.0, 2.0, false),
    ];
    let h = Harness::new(two);
    let (_tx, join) = h.pick_created(1);
    h.cache
        .publish(800.0, vec![display(1, 0.0, 0.0, 1000.0, 800.0, 1.0, true)])
        .unwrap();
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!(h.backend.cleanups(), 1);
    // A failed refresh publishes an empty topology: also a cancel, never a stale selection.
    let (_tx2, join2) = h.pick_created(2);
    h.cache.publish(0.0, vec![]).unwrap();
    assert_eq!(join2.join().unwrap(), Ok(None));
}

/// The overlay is gone (and the compositor given time) before `pick` returns, so the capture
/// that follows cannot contain the tint.
struct SettleProbe {
    backend: Arc<FakeOverlayBackend>,
    inner: FakeSleeper,
    active_at_sleep: Mutex<Vec<usize>>,
}
impl Sleeper for SettleProbe {
    fn now(&self) -> Duration {
        self.inner.now()
    }
    fn sleep(&self, d: Duration) {
        self.active_at_sleep
            .lock()
            .unwrap()
            .push(self.backend.native_active());
        self.inner.sleep(d);
    }
}
#[test]
fn cleanup_and_the_compositor_settle_precede_the_return() {
    let main = FakeMainThread::new();
    let backend = FakeOverlayBackend::new();
    let probe = Arc::new(SettleProbe {
        backend: backend.clone(),
        inner: FakeSleeper::new(),
        active_at_sleep: Mutex::new(vec![]),
    });
    let cache = TopologyCache::new(MacGeometry::new(1, 900.0, one_display(1.0)).unwrap());
    let overlay = Arc::new(MacOverlay::new(
        backend.clone(),
        Arc::new(cache),
        probe.clone(),
    ));
    overlay.bind_main_thread(main.clone());
    let pump = {
        let m = main.clone();
        std::thread::spawn(move || m.pump())
    };
    let (tx, rx) = bounded(1);
    let o = overlay.clone();
    let join = std::thread::spawn(move || o.pick(rx));
    backend.wait_for_creates(1);
    backend.fire(OverlayInput::Down(p(0, 0)));
    backend.fire(OverlayInput::Up(p(100, 100)));
    assert_eq!(join.join().unwrap(), Ok(Some(Rect::new(0, 0, 100, 100))));
    drop(tx);
    assert_eq!(backend.native_active(), 0);
    assert_eq!(*probe.active_at_sleep.lock().unwrap(), vec![0]);
    assert_eq!(probe.inner.slept(), vec![COMPOSITOR_SETTLE]);
    main.close();
    pump.join().unwrap();
}

#[test]
fn input_after_the_overlay_is_gone_is_not_consumed() {
    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    drop(tx);
    join.join().unwrap().unwrap();
    assert!(!h.backend.fire(OverlayInput::Down(p(1, 1))));
}

#[test]
fn a_failed_create_cleans_up_and_returns_the_error() {
    let h = Harness::new(one_display(1.0));
    let error = PlatformError::Os {
        api: "overlay window",
        code: 7,
    };
    h.backend.fail_next_create(error.clone());
    let (_tx, rx) = bounded::<()>(1);
    assert_eq!(h.overlay.pick(rx), Err(error));
    assert_eq!(h.backend.cleanups(), 1);
    assert_eq!(h.backend.native_active(), 0);
}

#[test]
fn no_displays_is_an_invalid_region_and_creates_nothing() {
    let h = Harness::new(vec![]);
    let (_tx, rx) = bounded::<()>(1);
    assert_eq!(h.overlay.pick(rx), Err(PlatformError::InvalidRegion));
    assert_eq!(h.backend.creates(), 0);
}

#[test]
fn pick_needs_a_bound_main_thread_and_never_runs_on_it() {
    let backend = FakeOverlayBackend::new();
    let cache = TopologyCache::new(MacGeometry::new(1, 900.0, one_display(1.0)).unwrap());
    let overlay = MacOverlay::new(
        backend.clone(),
        Arc::new(cache),
        Arc::new(FakeSleeper::new()),
    );
    let (_tx, rx) = bounded::<()>(1);
    assert_eq!(
        overlay.pick(rx),
        Err(PlatformError::Unsupported(
            "region overlay without main thread"
        ))
    );
    let h = Harness::new(one_display(1.0));
    let (_tx, rx) = bounded::<()>(1);
    // Blocking the main thread would deadlock the very work it must run.
    let on_main = h.main.enter_main(|| h.overlay.pick(rx));
    assert_eq!(
        on_main,
        Err(PlatformError::Unsupported(
            "region overlay on the main thread"
        ))
    );
    assert_eq!(h.backend.creates(), 0);
}

#[test]
fn a_closed_main_loop_is_an_error_not_a_hang() {
    let h = Harness::manual(one_display(1.0));
    h.main.close();
    let (_tx, rx) = bounded::<()>(1);
    assert!(h.overlay.pick(rx).is_err());
    assert_eq!(h.backend.creates(), 0);
}

#[test]
fn a_cleanup_the_main_loop_drops_is_an_error() {
    let h = Harness::manual(one_display(1.0));
    let (tx, join) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // create
    h.main.close(); // the loop goes away before the cleanup runs
    drop(tx);
    assert!(join.join().unwrap().is_err());
}

struct PanickingTopology {
    cache: TopologyCache,
    armed: std::sync::atomic::AtomicBool,
}
impl TopologySource for PanickingTopology {
    fn geometry(&self) -> MacGeometry {
        self.cache.snapshot()
    }
    fn topology_revision(&self) -> u64 {
        if self.armed.load(std::sync::atomic::Ordering::SeqCst) {
            panic!("test topology panic");
        }
        self.cache.topology_revision()
    }
}
#[test]
fn a_panic_during_the_pick_still_cleans_up_and_keeps_later_picks_working() {
    let main = FakeMainThread::new();
    let pump = {
        let m = main.clone();
        std::thread::spawn(move || m.pump())
    };
    let backend = FakeOverlayBackend::new();
    let topology = Arc::new(PanickingTopology {
        cache: TopologyCache::new(MacGeometry::new(1, 900.0, one_display(1.0)).unwrap()),
        armed: false.into(),
    });
    let overlay = Arc::new(
        MacOverlay::new(
            backend.clone(),
            topology.clone(),
            Arc::new(FakeSleeper::new()),
        )
        .with_topology_poll(Duration::from_millis(5)),
    );
    overlay.bind_main_thread(main.clone());
    let (_tx, rx) = bounded::<()>(1);
    let o = overlay.clone();
    let join = std::thread::spawn(move || o.pick(rx));
    backend.wait_for_creates(1);
    topology
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(join.join().is_err(), "the pick thread panicked");
    backend.wait_for_cleanups(1);
    assert_eq!(backend.native_active(), 0);
    topology
        .armed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let (tx, rx) = bounded::<()>(1);
    drop(tx);
    assert_eq!(overlay.pick(rx), Ok(None));
    main.close();
    pump.join().unwrap();
}

#[test]
fn selection_is_clipped_into_display_local_top_left_coordinates() {
    let d = Rect::new(-1920, -100, 1920, 1080);
    // Fully inside.
    assert_eq!(
        selection_in_display(d, Rect::new(-1000, 0, 200, 100)),
        Some(Rect::new(920, 100, 200, 100))
    );
    // Straddling the right edge of this display: clipped.
    assert_eq!(
        selection_in_display(d, Rect::new(-50, 0, 200, 100)),
        Some(Rect::new(1870, 100, 50, 100))
    );
    // On the neighbouring display only.
    assert_eq!(selection_in_display(d, Rect::new(10, 0, 100, 100)), None);
    // A bare click has no area.
    assert_eq!(selection_in_display(d, Rect::new(-10, 0, 0, 0)), None);
}
