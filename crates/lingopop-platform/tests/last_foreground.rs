//! The pure tracker filter and the fake (every OS).

use lingopop_platform::fake::FakeLastForeground;
use lingopop_platform::last_foreground::{is_trackable, LastForeground};
use lingopop_platform::WindowHandle;

const OWN: u32 = 100;
const FOREIGN: u32 = 200;

#[test]
fn is_trackable_table() {
    assert!(!is_trackable("Notepad", OWN, OWN), "own pid");
    assert!(!is_trackable("Edit", OWN, OWN), "own pid");
    for class in [
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "XamlExplorerHostIslandWindow",
        "Progman",
        "WorkerW",
        "#32768",
    ] {
        assert!(!is_trackable(class, FOREIGN, OWN), "{class}");
    }
    assert!(is_trackable("Notepad", FOREIGN, OWN));
    assert!(is_trackable("Edit", FOREIGN, OWN));
    assert!(is_trackable("", FOREIGN, OWN));
}

#[test]
fn fake_returns_the_scripted_window_and_counts_calls() {
    let fake = FakeLastForeground::new(Some(WindowHandle(9)));
    assert_eq!(fake.calls(), 0);
    assert_eq!(fake.last_external(), Some(WindowHandle(9)));
    fake.set(None);
    assert_eq!(fake.last_external(), None);
    assert_eq!(fake.calls(), 2);
}

#[test]
fn macos_tray_tracker_remembers_the_external_app_through_the_trait() {
    // The same `LastForeground` seam the Windows hook tracker fills: our own windows never
    // become the remembered target, a vanished application is forgotten.
    use lingopop_platform::fake::FakeSleeper;
    use lingopop_platform::macos_targets::{
        AppIdentity, FakeApps, LastExternalTracker, TargetTracker,
    };
    use std::sync::Arc;

    let id = |pid: i32, bundle: &str| AppIdentity {
        pid,
        process_start: Some(i64::from(pid) * 1_000_000 + 1),
        bundle_id: Some(bundle.into()),
        launch: Some(1),
    };
    let apps = FakeApps::new();
    let tracker = Arc::new(TargetTracker::new(
        apps.clone(),
        Arc::new(FakeSleeper::new()),
    ));
    let last = LastExternalTracker::new(tracker.clone(), 1);
    let tray: &dyn LastForeground = &last;
    assert_eq!(tray.last_external(), None);
    apps.set_frontmost(id(300, "com.example.Editor"));
    let remembered = tray.last_external().unwrap();
    apps.set_frontmost(id(1, "app.lingopop.tauri.macos"));
    assert_eq!(tray.last_external(), Some(remembered));
    assert_eq!(tracker.restore(remembered), Ok(()));
    apps.quit(300);
    assert_eq!(tray.last_external(), None);
}
