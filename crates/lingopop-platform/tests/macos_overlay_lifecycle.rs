//! Portable lifecycle tests of the macOS region overlay (`macos_overlay`): every OS, no AppKit.
//! The native overlay is a `FakeOverlayBackend` that records create / cleanup sequence numbers;
//! the main thread is a `FakeMainThread` (a pump thread, or driven by hand).
use crossbeam_channel::{bounded, Sender};
use lingopop_platform::fake::{FakeMainThread, FakeOverlayBackend, FakeSleeper};
use lingopop_platform::macos_dismiss::OverlayActive;
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
    active: OverlayActive,
    drags: std::sync::atomic::AtomicUsize,
}
impl Harness {
    fn with(displays: Vec<MacDisplay>, pump: bool) -> Self {
        Self::with_poll(displays, pump, Duration::from_millis(5))
    }
    fn with_poll(displays: Vec<MacDisplay>, pump: bool, poll: Duration) -> Self {
        let main = FakeMainThread::new();
        let backend = FakeOverlayBackend::new();
        let cache = TopologyCache::new(MacGeometry::new(1, 900.0, displays).unwrap());
        let sleeper = Arc::new(FakeSleeper::new());
        let active = OverlayActive::new();
        backend.watch_overlay_active(active.clone());
        let overlay = Arc::new(
            MacOverlay::new(backend.clone(), Arc::new(cache.clone()), sleeper.clone())
                .with_overlay_active(active.clone())
                .with_topology_poll(poll),
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
            active,
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

#[test]
fn a_topology_change_in_the_tick_of_a_finished_drag_discards_the_rect() {
    // A 60 s poll: only the finished drag can end this pick, the ticker never fires.
    let h = Harness::with_poll(one_display(1.0), true, Duration::from_secs(60));
    let (_tx, join) = h.pick_created(1);
    assert!(h.backend.fire(OverlayInput::Down(p(10, 10))));
    assert!(h.backend.fire(OverlayInput::Move(p(200, 200))));
    // The display observer publishes a new revision before the mouse-up arrives.
    h.cache.publish(900.0, one_display(2.0)).unwrap();
    assert!(h.backend.fire(OverlayInput::Up(p(200, 200))));
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!((h.backend.creates(), h.backend.cleanups()), (1, 1));
    assert_eq!(h.backend.native_active(), 0);
}

#[test]
fn an_unchanged_topology_still_returns_the_finished_rect() {
    let h = Harness::with_poll(one_display(1.0), true, Duration::from_secs(60));
    let result = h.drag(&[
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Up(p(200, 200)),
    ]);
    assert_eq!(result, Ok(Some(Rect::new(10, 10, 190, 190))));
}

#[test]
fn the_previous_app_is_restored_before_the_cleanup_acknowledgment_and_the_settle_is_150ms() {
    use lingopop_platform::fake::OverlayCall;
    let h = Harness::new(one_display(1.0));
    h.backend.hold_cleanup(true);
    let (_tx, join) = h.pick_created(1);
    assert!(h.backend.fire(OverlayInput::Escape));
    h.backend.wait_for_cleanups_started(1);
    assert_eq!(
        h.backend.restores(),
        0,
        "restore follows the window removal"
    );
    h.backend.hold_cleanup(false);
    assert_eq!(join.join().unwrap(), Ok(None));
    // pick returned, i.e. the acknowledgment happened: the restore is already recorded,
    // after the native removal and before the settle sleep.
    let calls = h.backend.calls();
    let cleanup = calls
        .iter()
        .position(|c| matches!(c, OverlayCall::Cleanup { .. }))
        .unwrap();
    let restore = calls
        .iter()
        .position(|c| matches!(c, OverlayCall::RestorePreviousApp { .. }))
        .unwrap();
    assert!(cleanup < restore);
    assert_eq!(h.backend.restores(), 1);
    assert_eq!(COMPOSITOR_SETTLE, Duration::from_millis(150));
    assert_eq!(h.sleeper.slept(), vec![Duration::from_millis(150)]);
}

#[test]
fn every_exit_path_restores_the_previous_app_once() {
    // selection
    let h = Harness::new(one_display(1.0));
    let _ = h.drag(&[
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Up(p(200, 200)),
    ]);
    assert_eq!(h.backend.restores(), 1, "selection");
    // escape
    let h = Harness::new(one_display(1.0));
    let _ = h.drag(&[OverlayInput::Escape]);
    assert_eq!(h.backend.restores(), 1, "escape");
    // dropped cancel sender
    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    drop(tx);
    join.join().unwrap().unwrap();
    assert_eq!(h.backend.restores(), 1, "cancel");
    // topology change
    let h = Harness::new(one_display(1.0));
    let (_tx, join) = h.pick_created(1);
    h.cache.publish(900.0, one_display(2.0)).unwrap();
    join.join().unwrap().unwrap();
    assert_eq!(h.backend.restores(), 1, "topology");
    // failed create (the create job already cleaned up)
    let h = Harness::new(one_display(1.0));
    h.backend.fail_next_create(PlatformError::Os {
        api: "create",
        code: 1,
    });
    let (_tx, rx) = bounded(1);
    assert!(h.overlay.pick(rx).is_err());
    assert_eq!(h.backend.restores(), 1, "failed create");
}

/// The flag the dismiss watcher reads: set before the native overlay exists, still set inside
/// `create`, `cleanup` and the focus restore, cleared once the pick returned.
fn assert_flag_bracketed_the_overlay(h: &Harness, what: &str) {
    assert_eq!(
        h.backend.flag_log(),
        vec![("create", true), ("cleanup", true), ("restore", true)],
        "{what}: the flag must be set from before create until after the restore"
    );
    assert!(
        !h.active.is_set(),
        "{what}: cleared after the pick returned"
    );
}

#[test]
fn the_overlay_flag_brackets_a_selection() {
    let h = Harness::new(one_display(1.0));
    assert!(!h.active.is_set());
    let result = h.drag(&[
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Up(p(200, 200)),
    ]);
    assert_eq!(result, Ok(Some(Rect::new(10, 10, 190, 190))));
    assert_flag_bracketed_the_overlay(&h, "selection");
}

#[test]
fn the_overlay_flag_brackets_escape_and_cancel() {
    let h = Harness::new(one_display(1.0));
    assert_eq!(h.drag(&[OverlayInput::Escape]), Ok(None));
    assert_flag_bracketed_the_overlay(&h, "escape");

    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    assert!(h.active.is_set(), "set while the overlay is up");
    drop(tx);
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_flag_bracketed_the_overlay(&h, "cancel");
}

#[test]
fn the_overlay_flag_stays_set_until_a_slow_cleanup_finished() {
    let h = Harness::new(one_display(1.0));
    h.backend.hold_cleanup(true);
    let (_tx, join) = h.pick_created(1);
    assert!(h.backend.fire(OverlayInput::Escape));
    h.backend.wait_for_cleanups_started(1);
    assert!(h.active.is_set(), "the windows are still being removed");
    h.backend.hold_cleanup(false);
    assert_eq!(join.join().unwrap(), Ok(None));
    assert!(!h.active.is_set());
}

#[test]
fn the_overlay_flag_is_cleared_after_a_failed_create_and_a_topology_change() {
    let h = Harness::new(one_display(1.0));
    h.backend.fail_next_create(PlatformError::Os {
        api: "create",
        code: 1,
    });
    let (_tx, rx) = bounded(1);
    assert!(h.overlay.pick(rx).is_err());
    assert_flag_bracketed_the_overlay(&h, "failed create");

    let h = Harness::new(one_display(1.0));
    let (_tx, join) = h.pick_created(1);
    h.cache.publish(900.0, one_display(2.0)).unwrap();
    join.join().unwrap().unwrap();
    assert_flag_bracketed_the_overlay(&h, "topology change");
}

#[test]
fn the_overlay_flag_is_never_set_when_no_overlay_is_created() {
    let h = Harness::new(vec![]);
    let (_tx, rx) = bounded::<()>(1);
    assert!(h.overlay.pick(rx).is_err());
    assert!(!h.active.is_set());
    let (tx, rx) = bounded::<()>(1);
    drop(tx);
    assert_eq!(h.overlay.pick(rx), Ok(None));
    assert!(h.backend.flag_log().is_empty());
    assert!(!h.active.is_set());
}

#[test]
fn the_overlay_flag_is_cleared_when_the_main_loop_dropped_the_cleanup() {
    let h = Harness::manual(one_display(1.0));
    let (tx, join) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // create
    assert!(h.active.is_set());
    drop(tx);
    h.main.wait_for_pending(1);
    h.main.close(); // drops the queued cleanup unrun
    assert!(join.join().unwrap().is_err());
    assert!(!h.active.is_set(), "no native overlay can exist any more");
}

#[test]
fn a_panic_in_the_pick_still_restores_the_previous_app_once_and_releases_the_lease() {
    let main = FakeMainThread::new();
    let pump = {
        let m = main.clone();
        std::thread::spawn(move || m.pump())
    };
    let backend = FakeOverlayBackend::new();
    let active = OverlayActive::new();
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
        .with_overlay_active(active.clone())
        .with_topology_poll(Duration::from_millis(5)),
    );
    overlay.bind_main_thread(main.clone());
    let (_tx, rx) = bounded::<()>(1);
    let o = overlay.clone();
    let join = std::thread::spawn(move || o.pick(rx));
    backend.wait_for_creates(1);
    // The unwinding pick (the guard's drop) is the only teardown path: nobody finalizes it.
    topology
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(join.join().is_err(), "the pick thread panicked");
    backend.wait_for_cleanups(1);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while active.is_set() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!active.is_set(), "the lease is released after the teardown");
    assert_eq!(backend.restores(), 1, "restored exactly once on unwind");
    assert!(
        backend.cleanup_seq(1).unwrap() < backend.restore_seq(1).unwrap(),
        "the restore follows the window removal"
    );
    main.close();
    pump.join().unwrap();
}

/// A main loop that survives a panicking job (like a run loop whose job is caught): each job
/// that unwinds is dropped and the next one still runs.
fn surviving_dispatcher(
    main: Arc<FakeMainThread>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    panics: Arc<std::sync::atomic::AtomicUsize>,
) -> JoinHandle<()> {
    use std::sync::atomic::Ordering::SeqCst;
    std::thread::spawn(move || {
        while !stop.load(SeqCst) {
            main.wait_for_pending(1);
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| main.run_pending()));
            if ran.is_err() {
                panics.fetch_add(1, SeqCst);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })
}

#[test]
fn a_backend_panic_in_the_middle_of_a_pick_still_restores_once_and_releases_the_lease() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    let h = Harness::manual(one_display(1.0));
    let (stop, panics) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let dispatcher = surviving_dispatcher(h.main.clone(), stop.clone(), panics.clone());
    let (tx, join) = h.pick_created(1);
    // The next draw (a job on the main thread) panics: the dispatcher drops that job and lives.
    h.backend.panic_next_draw();
    assert!(h.backend.fire(OverlayInput::Down(p(10, 10))));
    assert!(h.backend.fire(OverlayInput::Move(p(100, 100))));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while panics.load(SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        panics.load(SeqCst),
        1,
        "the injected backend panic ran once"
    );
    assert!(h.active.is_set(), "the pick is still in progress");
    // The user cancels: the normal teardown still runs on the surviving dispatcher.
    drop(tx);
    assert_eq!(join.join().unwrap(), Ok(None));
    assert_eq!(h.backend.cleanups(), 1);
    assert_eq!(h.backend.restores(), 1, "restored exactly once");
    assert!(
        h.backend.cleanup_seq(1).unwrap() < h.backend.restore_seq(1).unwrap(),
        "the restore follows the window removal"
    );
    assert!(!h.active.is_set(), "the lease is released");
    // The dispatcher survived the panic: a following pick works end to end.
    let (_tx2, join2) = h.pick_created(2);
    for e in [
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Up(p(200, 200)),
    ] {
        assert!(h.backend.fire(e));
    }
    assert_eq!(join2.join().unwrap(), Ok(Some(Rect::new(10, 10, 190, 190))));
    assert_eq!(h.backend.restores(), 2);
    assert!(!h.active.is_set());
    stop.store(true, SeqCst);
    h.main.close();
    dispatcher.join().unwrap();
}

#[test]
fn a_panic_in_the_pick_clears_the_overlay_flag_after_the_posted_cleanup() {
    let main = FakeMainThread::new();
    let pump = {
        let m = main.clone();
        std::thread::spawn(move || m.pump())
    };
    let backend = FakeOverlayBackend::new();
    let active = OverlayActive::new();
    backend.watch_overlay_active(active.clone());
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
        .with_overlay_active(active.clone())
        .with_topology_poll(Duration::from_millis(5)),
    );
    overlay.bind_main_thread(main.clone());
    let (_tx, rx) = bounded::<()>(1);
    let o = overlay.clone();
    let join = std::thread::spawn(move || o.pick(rx));
    backend.wait_for_creates(1);
    assert!(active.is_set());
    topology
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(join.join().is_err(), "the pick thread panicked");
    backend.wait_for_cleanups(1);
    // The flag is cleared by the posted teardown itself, after cleanup and restore.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while active.is_set() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!active.is_set());
    assert_eq!(
        backend.flag_log(),
        vec![("create", true), ("cleanup", true), ("restore", true)]
    );
    main.close();
    pump.join().unwrap();
}

#[test]
fn a_main_thread_pick_is_refused_before_it_waits_for_a_running_pick() {
    // `serial` is held for a whole pick: a main-thread caller must be refused first, never
    // blocked behind it (it would deadlock the main loop the running pick needs).
    let h = Harness::new(one_display(1.0));
    let (tx, join) = h.pick_created(1);
    let (done, refused) = bounded(1);
    let overlay = h.overlay.clone();
    let main = h.main.clone();
    std::thread::spawn(move || {
        let (_tx, rx) = bounded::<()>(1);
        let _ = done.send(main.enter_main(|| overlay.pick(rx)));
    });
    let outcome = refused
        .recv_timeout(Duration::from_secs(2))
        .expect("the main-thread pick blocked behind the running pick");
    assert_eq!(
        outcome,
        Err(PlatformError::Unsupported(
            "region overlay on the main thread"
        ))
    );
    drop(tx);
    assert_eq!(join.join().unwrap(), Ok(None));
}

#[test]
fn an_older_cleanup_that_runs_late_never_clears_the_flag_of_a_newer_pick() {
    // Pick 1's cleanup times out (the main loop is busy) and its `serial` lock is released; pick 2
    // starts and queues its create BEHIND the stale teardown. When main resumes, the old teardown
    // must not drop the protection of the overlay pick 2 creates next.
    let h = Harness::manual(one_display(1.0));
    let (tx1, join1) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // pick 1: create
    assert!(h.active.is_set());
    tx1.send(()).unwrap(); // pick 1 ends; its teardown is queued and main does not run it
    h.main.wait_for_pending(1);
    // Real-time wait of CLEANUP_TIMEOUT: the engine gives up on the queued cleanup.
    assert_eq!(
        join1.join().unwrap(),
        Err(PlatformError::Os {
            api: "region overlay cleanup",
            code: 0
        })
    );
    assert!(h.active.is_set(), "the stale cleanup has not run yet");

    let (tx2, join2) = h.pick();
    h.main.wait_for_pending(2); // old teardown, then pick 2's create
    h.main.run_pending(); // old teardown, then create 2
    assert_eq!(h.backend.creates(), 2);
    assert!(
        h.active.is_set(),
        "the older pick's teardown must not clear the newer pick's flag"
    );
    let create_flags: Vec<bool> = h
        .backend
        .flag_log()
        .into_iter()
        .filter(|(call, _)| *call == "create")
        .map(|(_, flag)| flag)
        .collect();
    assert_eq!(
        create_flags,
        vec![true, true],
        "set while pick 2 is created"
    );

    tx2.send(()).unwrap();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // pick 2's teardown
    assert_eq!(join2.join().unwrap(), Ok(None));
    assert!(!h.active.is_set(), "cleared after the newer pick's cleanup");
}

/// Starts a pick, lets it create, ends it, and waits out the cleanup timeout while the main
/// loop leaves the queued teardown unrun. Returns with the pick finished and its lease still
/// held by that queued teardown.
fn abandon_a_timed_out_pick(h: &Harness) {
    let (tx, join) = h.pick();
    h.main.wait_for_pending(1);
    h.main.run_pending(); // create
    tx.send(()).unwrap();
    h.main.wait_for_pending(1); // the teardown is queued and never runs
    assert_eq!(
        join.join().unwrap(),
        Err(PlatformError::Os {
            api: "region overlay cleanup",
            code: 0
        })
    );
    assert!(
        h.active.is_set(),
        "the queued teardown still owns the lease"
    );
}

#[test]
fn a_teardown_dropped_after_the_waiter_timed_out_gives_dismissal_back() {
    let h = Harness::manual(one_display(1.0));
    abandon_a_timed_out_pick(&h);
    h.main.close(); // the loop drops the queued teardown unrun
    assert!(
        !h.active.is_set(),
        "a dropped teardown must not leave dismissal suppressed for good"
    );
}

#[test]
fn a_teardown_that_unwinds_before_releasing_gives_dismissal_back() {
    let h = Harness::manual(one_display(1.0));
    abandon_a_timed_out_pick(&h);
    h.backend.panic_next_cleanup();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.main.run_pending()));
    assert!(unwound.is_err(), "the teardown panicked");
    assert!(
        !h.active.is_set(),
        "unwinding drops the job and its lease: dismissal is enabled again"
    );
}

#[test]
fn later_picks_after_an_abandoned_lease_leave_dismissal_enabled() {
    let h = Harness::manual(one_display(1.0));
    abandon_a_timed_out_pick(&h);
    h.main.close();
    assert!(!h.active.is_set());
    // The loop is gone, so a new pick fails at create; it must not re-leak either.
    let (_tx, rx) = bounded::<()>(1);
    assert!(h.overlay.pick(rx).is_err());
    assert!(!h.active.is_set());
}

#[test]
fn a_finished_pick_carries_the_topology_revision_it_was_made_against() {
    let h = Harness::new(one_display(1.0));
    let revision = h.cache.topology_revision();
    let (_tx_cancel, rx) = bounded::<()>(1);
    let overlay = h.overlay.clone();
    let join = std::thread::spawn(move || overlay.pick_with_revision(rx));
    h.backend.wait_for_creates(1);
    for e in [
        OverlayInput::Down(p(10, 10)),
        OverlayInput::Move(p(200, 200)),
        OverlayInput::Up(p(200, 200)),
    ] {
        assert!(h.backend.fire(e));
    }
    assert_eq!(
        join.join().unwrap(),
        Ok(Some((Rect::new(10, 10, 190, 190), Some(revision))))
    );
    // A cancelled pick has no rectangle and so no revision.
    let (tx, rx) = bounded::<()>(1);
    drop(tx);
    assert_eq!(h.overlay.pick_with_revision(rx), Ok(None));
}
