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
