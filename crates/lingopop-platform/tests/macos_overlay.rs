//! OWNER-RUN real-AppKit region overlay check. Never part of the default suite and never run by
//! CI: this target has `harness = false` (AppKit windows and the event loop need the process
//! main thread) and without `--ignored` it prints a skip line and exits 0, creating nothing.
//!
//! With `--ignored` the overlay really covers every display and takes real input:
//!
//! * always: opens the overlay on all displays, then cancels it from the worker side (a sent
//!   cancel, then a dropped sender) and asserts `Ok(None)` and that no overlay window remains;
//! * with `LINGOPOP_OWNER_OVERLAY_DRAG=1`: waits up to 60 s for YOU to drag a region (try both
//!   displays, a reverse drag, a drag across the display edge, Esc, a click) and prints the
//!   resulting desktop rectangle and the displays involved. Nothing is captured or stored.
//!
//! Run it from a terminal with a desktop session; it activates this process while it runs:
//!
//! ```text
//! cargo test -p lingopop-platform --test macos_overlay --locked -- --ignored
//! LINGOPOP_OWNER_OVERLAY_DRAG=1 cargo test -p lingopop-platform --test macos_overlay --locked -- --ignored
//! ```

#[cfg(target_os = "macos")]
fn main() {
    if !std::env::args().any(|arg| arg == "--ignored") {
        println!("skipped: owner-run (pass --ignored; needs the macOS desktop)");
        return;
    }
    owner_run::run();
    println!("ok: macos_overlay");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod owner_run {
    use lingopop_platform::macos_geometry::MacGeometry;
    use lingopop_platform::main_thread::on_main_thread;
    use lingopop_platform::sleeper::SystemSleeper;
    use lingopop_platform::{MacMonitors, MacOverlay, MainThread, PlatformError, RegionOverlay};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{ns_string, NSDate};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    type Job = Box<dyn FnOnce() + Send>;

    /// The process main thread as a `MainThread`: jobs queue up and `pump` runs them between
    /// AppKit events.
    struct LoopMain(Mutex<Sender<Job>>);
    impl MainThread for LoopMain {
        fn is_main_thread(&self) -> bool {
            on_main_thread()
        }
        fn post(&self, job: Job) -> Result<(), PlatformError> {
            self.0
                .lock()
                .unwrap()
                .send(job)
                .map_err(|_| lingopop_platform::main_thread::dispatch_error())
        }
    }

    fn pump(app: &NSApplication, jobs: &Receiver<Job>, until: impl Fn() -> bool) {
        while !until() {
            while let Ok(job) = jobs.try_recv() {
                job();
            }
            let deadline = NSDate::dateWithTimeIntervalSinceNow(0.02);
            // NSDefaultRunLoopMode's value.
            let mode = ns_string!("kCFRunLoopDefaultMode");
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&deadline),
                mode,
                true,
            ) {
                app.sendEvent(&event);
            }
        }
    }

    fn worker(
        overlay: &Arc<MacOverlay>,
        cancel: crossbeam_channel::Receiver<()>,
    ) -> (
        JoinHandle<Result<Option<lingopop_platform::Rect>, PlatformError>>,
        Arc<AtomicBool>,
    ) {
        let done = Arc::new(AtomicBool::new(false));
        let (o, d) = (overlay.clone(), done.clone());
        let join = std::thread::spawn(move || {
            let r = o.pick(cancel);
            d.store(true, Ordering::SeqCst);
            r
        });
        (join, done)
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("the owner-run test must run on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let monitors = MacMonitors::new();
        monitors
            .initialize_on_main_thread()
            .expect("topology setup on the main thread");
        let geometry: MacGeometry = monitors.geometry();
        println!("displays: {}", geometry.displays().len());
        let overlay = Arc::new(MacOverlay::new(
            Arc::new(lingopop_platform::AppKitOverlay::new()),
            Arc::new(monitors),
            Arc::new(SystemSleeper::new()),
        ));
        let (tx, jobs) = channel::<Job>();
        overlay.bind_main_thread(Arc::new(LoopMain(Mutex::new(tx))));
        let windows_before = app.windows().len();

        // 1. A sent cancel ends the pick; the overlay is gone before `pick` returns.
        let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
        let (join, done) = worker(&overlay, cancel_rx);
        let started = Instant::now();
        pump(&app, &jobs, || {
            started.elapsed() > Duration::from_millis(800)
        });
        assert!(!done.load(Ordering::SeqCst), "the pick waits for input");
        assert!(
            app.windows().len() > windows_before,
            "overlay windows exist"
        );
        cancel_tx.send(()).unwrap();
        pump(&app, &jobs, || done.load(Ordering::SeqCst));
        assert_eq!(join.join().unwrap(), Ok(None));
        assert_eq!(
            app.windows().len(),
            windows_before,
            "no overlay window left"
        );

        // 2. A dropped sender does the same.
        let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
        let (join, done) = worker(&overlay, cancel_rx);
        let started = Instant::now();
        pump(&app, &jobs, || {
            started.elapsed() > Duration::from_millis(500)
        });
        drop(cancel_tx);
        pump(&app, &jobs, || done.load(Ordering::SeqCst));
        assert_eq!(join.join().unwrap(), Ok(None));
        assert_eq!(app.windows().len(), windows_before);

        // 3. Optional: a real drag.
        if std::env::var_os("LINGOPOP_OWNER_OVERLAY_DRAG").is_some() {
            println!("drag a region now (Esc or a click cancels; 60 s)");
            let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
            let (join, done) = worker(&overlay, cancel_rx);
            let started = Instant::now();
            pump(&app, &jobs, || {
                done.load(Ordering::SeqCst) || started.elapsed() > Duration::from_secs(60)
            });
            drop(cancel_tx);
            pump(&app, &jobs, || done.load(Ordering::SeqCst));
            match join.join().unwrap() {
                Ok(Some(r)) => {
                    let start = geometry.display_at(lingopop_platform::Point { x: r.x, y: r.y });
                    println!(
                        "selected {}x{} at ({}, {}); top-left on display {:?}",
                        r.width, r.height, r.x, r.y, start
                    );
                }
                other => println!("no selection: {other:?}"),
            }
            assert_eq!(app.windows().len(), windows_before);
        }
    }
}
