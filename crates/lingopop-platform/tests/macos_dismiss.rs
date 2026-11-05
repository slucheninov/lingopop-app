//! The portable macOS dismiss watcher on a fake monitor backend and a fake main thread
//! (every OS). Real NSEvent monitors are only exercised by the owner-run `macos_popup.rs`.
use lingopop_platform::fake::{FakeMainThread, FakeMonitorBackend};
use lingopop_platform::macos_dismiss::{
    MacDismissWatcher, MonitorSet, ObservedEvent, ESCAPE_KEY_CODE, OWN_EVENT_MARKER,
};
use lingopop_platform::{DismissEvent, DismissWatcher, PlatformError, Point, Rect};
use std::sync::Arc;

fn popup() -> Rect {
    Rect::new(100, 100, 360, 120)
}
fn rig(
    trusted: bool,
) -> (
    MacDismissWatcher<FakeMonitorBackend>,
    Arc<FakeMonitorBackend>,
    Arc<FakeMainThread>,
) {
    let backend = Arc::new(FakeMonitorBackend::new(trusted));
    let main = FakeMainThread::new();
    let watcher = MacDismissWatcher::new(backend.clone());
    watcher.bind_main_thread(main.clone());
    (watcher, backend, main)
}
fn click(x: i32, y: i32) -> ObservedEvent {
    ObservedEvent::MouseDown {
        location: Point { x, y },
        marker: 0,
    }
}
fn escape(modified: bool, marker: i64) -> ObservedEvent {
    ObservedEvent::KeyDown {
        key_code: ESCAPE_KEY_CODE,
        modified,
        marker,
    }
}

#[test]
fn start_is_nonblocking_and_installs_on_main() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    assert_eq!(backend.native_monitors(), 0, "start only posts");
    assert_eq!(main.pending(), 1);
    main.run_pending();
    assert_eq!(backend.native_monitors(), 4);
    assert_eq!(backend.installed(), Some(MonitorSet::for_trust(true)));
    assert!(backend.installed_on_main_only());
}

#[test]
fn click_inside_is_ignored_and_outside_delivers_once() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    let events = watcher.events();
    backend.fire(click(100, 100));
    backend.fire(click(459, 219));
    assert!(
        events.try_recv().is_err(),
        "half-open rect: both corners are inside"
    );
    backend.fire(click(460, 150));
    backend.fire(click(10, 10));
    assert_eq!(events.try_recv(), Ok(DismissEvent::ClickOutside));
    assert!(events.try_recv().is_err(), "one dismissal per start");
}

#[test]
fn physical_escape_delivers_once_and_modified_escape_is_ignored() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    let events = watcher.events();
    backend.fire(escape(true, 0));
    backend.fire(ObservedEvent::KeyDown {
        key_code: 0,
        modified: false,
        marker: 0,
    });
    assert!(events.try_recv().is_err());
    backend.fire(escape(false, 0));
    backend.fire(escape(false, 0));
    assert_eq!(events.try_recv(), Ok(DismissEvent::Escape));
    assert!(events.try_recv().is_err());
}

#[test]
fn own_injected_input_does_not_dismiss() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    let events = watcher.events();
    backend.fire(escape(false, OWN_EVENT_MARKER));
    backend.fire(ObservedEvent::MouseDown {
        location: Point { x: 1, y: 1 },
        marker: OWN_EVENT_MARKER,
    });
    assert!(events.try_recv().is_err());
    // Foreign injected input (another process's marker) still counts, like Windows.
    backend.fire(escape(false, 17));
    assert_eq!(events.try_recv(), Ok(DismissEvent::Escape));
}

#[test]
fn missing_accessibility_reports_escape_unavailable_and_keeps_click_outside() {
    let (watcher, backend, main) = rig(false);
    assert!(!watcher.escape_available());
    watcher.start(popup()).unwrap();
    main.run_pending();
    let set = backend.installed().unwrap();
    assert!(!set.global_key, "no global keyDown monitor without trust");
    assert!(set.global_mouse && set.local_mouse);
    let events = watcher.events();
    backend.fire(click(5, 5));
    assert_eq!(events.try_recv(), Ok(DismissEvent::ClickOutside));
    watcher.stop();
    main.run_pending();
    assert_eq!(backend.native_monitors(), 0, "no monitor leaks");
    assert_eq!(backend.max_native_monitors(), set.count());
}

#[test]
fn stop_removes_monitors_and_ignores_later_events() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    watcher.stop();
    let events = watcher.events();
    backend.fire(click(1, 1));
    assert!(
        events.try_recv().is_err(),
        "inactive at once, before main runs"
    );
    main.run_pending();
    assert_eq!(backend.native_monitors(), 0);
}

#[test]
fn restart_before_main_runs_keeps_one_installation() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    watcher.stop();
    watcher.start(popup()).unwrap();
    main.run_pending();
    assert_eq!(backend.native_monitors(), 4);
    assert_eq!(backend.max_native_monitors(), 4);
    watcher.stop();
    main.run_pending();
    assert_eq!(backend.native_monitors(), 0);
}

#[test]
fn start_drains_stale_events_and_update_rect_moves_the_hit_area() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    backend.fire(click(1, 1));
    watcher.stop();
    watcher.start(popup()).unwrap();
    let events = watcher.events();
    assert!(events.try_recv().is_err(), "stale event drained");
    watcher.update_rect(Rect::new(0, 0, 50, 50));
    backend.fire(click(1, 1));
    assert!(events.try_recv().is_err(), "inside the new rect");
    backend.fire(click(150, 150));
    assert_eq!(events.try_recv(), Ok(DismissEvent::ClickOutside));
}

#[test]
fn shutdown_removes_monitors_and_refuses_new_starts() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    let watcher = Arc::new(watcher);
    let (done_tx, done) = std::sync::mpsc::channel();
    let worker = {
        let watcher = watcher.clone();
        std::thread::spawn(move || {
            watcher.shutdown();
            done_tx.send(()).unwrap();
        })
    };
    // Off main, shutdown posts the removal and waits for it: a completion barrier.
    main.wait_for_pending(1);
    assert!(
        done.try_recv().is_err(),
        "shutdown returned before main ran"
    );
    assert_eq!(backend.native_monitors(), 4);
    main.run_pending();
    done.recv().unwrap();
    worker.join().unwrap();
    assert_eq!(
        backend.native_monitors(),
        0,
        "removed before shutdown returned"
    );
    assert!(watcher.start(popup()).is_err());
    main.run_pending();
    assert_eq!(backend.native_monitors(), 0);
}

#[test]
fn shutdown_on_main_with_a_pending_reconcile_removes_inline() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    // stop() off main queues a removal reconcile; the loop has not run it yet.
    watcher.stop();
    assert_eq!(main.pending(), 1);
    main.enter_main(|| watcher.shutdown());
    assert_eq!(
        backend.native_monitors(),
        0,
        "removed inline although a reconcile was queued"
    );
    // The loop then goes away without running the queued job: nothing is left behind.
    main.close();
    assert_eq!(backend.native_monitors(), 0);
    assert!(watcher.start(popup()).is_err());
}

#[test]
fn off_main_shutdown_returns_when_the_loop_drops_the_cleanup() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    let watcher = Arc::new(watcher);
    let worker = {
        let watcher = watcher.clone();
        std::thread::spawn(move || watcher.shutdown())
    };
    main.wait_for_pending(1);
    main.close();
    worker.join().unwrap();
    assert!(watcher.start(popup()).is_err());
    let _ = backend;
}

#[test]
fn escape_availability_follows_the_installed_set_across_a_trust_change() {
    let (watcher, backend, main) = rig(false);
    watcher.start(popup()).unwrap();
    main.run_pending();
    assert!(!watcher.escape_available());
    // Accessibility granted while the popup is open: the installed set has no global key
    // monitor yet, so Escape is still unavailable until the next start reinstalls.
    backend.set_trusted(true);
    assert!(
        !watcher.escape_available(),
        "reported from what is installed"
    );
    assert!(!backend.installed().unwrap().global_key);
    // A stop / start coalesced before main runs still reinstalls with the new trust.
    watcher.stop();
    watcher.start(popup()).unwrap();
    main.run_pending();
    assert_eq!(backend.installed(), Some(MonitorSet::for_trust(true)));
    assert!(watcher.escape_available());
    assert_eq!(
        backend.native_monitors(),
        4,
        "old set removed before reinstall"
    );
    // Revoked: the installed global key monitor still reports available until reinstall.
    backend.set_trusted(false);
    assert!(watcher.escape_available());
    watcher.stop();
    main.run_pending();
    assert!(
        !watcher.escape_available(),
        "nothing installed: the current trust"
    );
}

#[test]
fn shutdown_on_main_removes_inline() {
    let (watcher, backend, main) = rig(true);
    watcher.start(popup()).unwrap();
    main.run_pending();
    main.enter_main(|| watcher.shutdown());
    assert_eq!(backend.native_monitors(), 0);
}

#[test]
fn unbound_watcher_refuses_start_and_installs_nothing() {
    let backend = Arc::new(FakeMonitorBackend::new(true));
    let watcher = MacDismissWatcher::new(backend.clone());
    assert!(matches!(
        watcher.start(popup()),
        Err(PlatformError::Unsupported(_))
    ));
    assert_eq!(backend.native_monitors(), 0);
}

#[test]
fn failed_installation_leaks_nothing() {
    let (watcher, backend, main) = rig(true);
    backend.fail_install_after(2);
    watcher.start(popup()).unwrap();
    main.run_pending();
    assert_eq!(backend.native_monitors(), 0);
}
