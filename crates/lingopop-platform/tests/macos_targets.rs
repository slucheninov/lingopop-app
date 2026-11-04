//! The macOS source-targeting logic (registry, tracker, last external) on a scripted fake.
//! Runs on every OS: it never touches AppKit or Accessibility.
//!
//! `is_current_target` is a look, not a lock (the user can switch apps right after it answers);
//! the tests named `*_is_not_atomic*` pin only what the check does catch.

use std::sync::Arc;
use std::time::Duration;

use lingopop_platform::fake::FakeSleeper;
use lingopop_platform::macos_targets::{
    AppIdentity, FakeApps, LastExternalTracker, MacApps, TargetRegistry, TargetTracker, CAPACITY,
    CONFIRM_WINDOW, ENTRY_LIFETIME,
};
use lingopop_platform::{
    bring_to_front, ForegroundWindow, PermissionKind, PlatformError, ReplaceOptions, WindowHandle,
};

const OWN: i32 = 1;
const SAFARI: i32 = 100;
const NOTES: i32 = 200;

/// A process with a readable start time (derived from pid and launch, so a "new launch" of the
/// same pid has a different one) and full LaunchServices metadata.
fn app(pid: i32, bundle: &str, launch: i64) -> AppIdentity {
    AppIdentity {
        pid,
        process_start: Some(i64::from(pid) * 1_000_000 + launch),
        bundle_id: Some(bundle.into()),
        launch: Some(launch),
    }
}

/// A process the kernel can time but LaunchServices did not launch: no launchDate, no bundle.
fn bare(pid: i32, start: Option<i64>) -> AppIdentity {
    AppIdentity {
        pid,
        process_start: start,
        bundle_id: None,
        launch: None,
    }
}

fn safari() -> AppIdentity {
    app(SAFARI, "com.apple.Safari", 1_000)
}

fn notes() -> AppIdentity {
    app(NOTES, "com.apple.Notes", 2_000)
}

struct Rig {
    apps: Arc<FakeApps>,
    clock: Arc<FakeSleeper>,
    tracker: Arc<TargetTracker<Arc<FakeApps>>>,
}

fn rig() -> Rig {
    let apps = FakeApps::new();
    let clock = Arc::new(FakeSleeper::new());
    let tracker = Arc::new(TargetTracker::new(apps.clone(), clock.clone()));
    Rig {
        apps,
        clock,
        tracker,
    }
}

// --- registry ---

#[test]
fn current_target_roundtrips_and_is_stable_between_polls() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    assert_eq!(
        r.tracker.current(),
        Some(h),
        "polling must not mint handles"
    );
    assert_eq!(r.tracker.registered(), 1);
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    assert_eq!(r.tracker.restore(h), Ok(()));
    assert_eq!(r.apps.activated(), vec![(SAFARI, None)]);
}

#[test]
fn different_frontmost_pid_never_receives_paste() {
    // The check says "no" once the frontmost process is another one; replace_selection then
    // sends nothing (pinned in tests/inject.rs against the fake foreground).
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    r.apps.set_frontmost(notes());
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    // The captured handle still names Safari: Notes gets its own handle.
    assert_ne!(r.tracker.current(), Some(h));
}

#[test]
fn process_exit_is_rejected() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    r.apps.quit(SAFARI);
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    assert!(!r.tracker.is_live(h));
}

#[test]
fn pid_reuse_is_rejected() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    // The same pid now belongs to a different launch (and another app).
    r.apps
        .set_frontmost(app(SAFARI, "com.example.Other", 9_999));
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    assert!(
        r.apps.activated().is_empty(),
        "never activates the new process"
    );
    // Same bundle, new launch date: also a different process.
    r.apps.set_frontmost(app(SAFARI, "com.apple.Safari", 5_000));
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
}

#[test]
fn evicted_registry_handle_is_rejected() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    // Fill the registry with other applications, one tick apart so Safari is the oldest.
    for i in 0..CAPACITY as i32 {
        r.clock.advance(Duration::from_secs(1));
        r.apps
            .set_frontmost(app(1_000 + i, &format!("com.example.App{i}"), i64::from(i)));
        r.tracker.current().unwrap();
    }
    assert_eq!(r.tracker.registered(), CAPACITY);
    // Safari is still running and frontmost again, but its old handle is gone.
    r.apps.set_frontmost(safari());
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    // Observing it again mints a fresh handle (a new generation), never the old one.
    assert_ne!(r.tracker.current(), Some(h));
}

#[test]
fn registry_is_bounded_and_evicts_the_entry_seen_longest_ago() {
    let mut reg = TargetRegistry::<u32>::new();
    let mut handles = Vec::new();
    for i in 0..(CAPACITY as i32 + 5) {
        let now = Duration::from_secs(i as u64);
        handles.push(reg.observe(now, app(10 + i, "x", 1), None));
    }
    assert_eq!(reg.len(), CAPACITY);
    let now = Duration::from_secs(100);
    for h in &handles[..5] {
        assert!(reg.get(now, *h).is_none(), "oldest five were evicted");
    }
    for h in &handles[5..] {
        assert!(reg.get(now, *h).is_some());
    }
}

#[test]
fn handles_are_generation_stamped_never_reused() {
    let mut reg = TargetRegistry::<u32>::new();
    let a = reg.observe(Duration::ZERO, app(1, "a", 1), None);
    // Expire it and observe another entry into the (now free) first slot.
    let later = ENTRY_LIFETIME + Duration::from_secs(1);
    let b = reg.observe(later, app(2, "b", 1), None);
    assert_ne!(a, b, "a reused slot gets a new generation");
    assert!(reg.get(later, a).is_none());
    assert!(reg.get(later, b).is_some());
    // Garbage handles never resolve.
    assert!(reg.get(later, WindowHandle(0)).is_none());
    assert!(reg.get(later, WindowHandle(u64::MAX)).is_none());
}

#[test]
fn entries_expire_after_ten_minutes_unless_seen_again() {
    let mut reg = TargetRegistry::<u32>::new();
    let h = reg.observe(Duration::ZERO, app(1, "a", 1), None);
    let just_before = ENTRY_LIFETIME - Duration::from_secs(1);
    assert!(reg.get(just_before, h).is_some());
    assert!(reg.get(ENTRY_LIFETIME, h).is_none());

    // An observation refreshes the lifetime and keeps the same handle.
    let mut reg = TargetRegistry::<u32>::new();
    let h = reg.observe(Duration::ZERO, app(1, "a", 1), None);
    let again = reg.observe(just_before, app(1, "a", 1), None);
    assert_eq!(h, again);
    assert!(reg.get(just_before + just_before, h).is_some());
    // A plain lookup does not extend it.
    assert!(reg.get(just_before + ENTRY_LIFETIME, h).is_none());
}

#[test]
fn expired_target_fails_safely_through_the_tracker() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    r.clock.advance(ENTRY_LIFETIME + Duration::from_secs(1));
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
}

// --- accessibility ---

#[test]
fn missing_ax_identity_still_allows_replace_at_pid_and_bundle_level() {
    let r = rig();
    r.apps.set_frontmost(safari());
    // No focused window known (Chromium without AX, a sandboxed helper, ...).
    let h = r.tracker.current().unwrap();
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    assert_eq!(r.tracker.restore(h), Ok(()));
    assert_eq!(r.apps.activated(), vec![(SAFARI, None)]);

    // Right after the restore the AX-less target is confirmed by process identity even when the
    // window turns readable: the very same handle.
    r.apps.set_focused_window(SAFARI, 7);
    assert_eq!(r.tracker.current(), Some(h));

    // Once the confirmation window is over, a readable window is a different entry (an unknown
    // window never stands in for a known one). The AX-less handle still works at process level.
    r.clock.advance(CONFIRM_WINDOW + Duration::from_secs(1));
    let with_window = r.tracker.current().unwrap();
    assert_ne!(with_window, h);
    assert_eq!(r.tracker.restore(with_window), Ok(()));
    assert_eq!(r.apps.activated().last(), Some(&(SAFARI, Some(7))));
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
}

#[test]
fn another_window_of_the_same_app_is_a_new_entry_but_not_a_refusal_reason() {
    let r = rig();
    r.apps.set_frontmost(safari());
    r.apps.set_focused_window(SAFARI, 1);
    let first = r.tracker.current().unwrap();
    r.apps.set_focused_window(SAFARI, 2);
    let second = r.tracker.current().unwrap();
    assert_ne!(
        first, second,
        "an AX-distinguishable window gets its own entry"
    );
    // Documented best-effort limit: the check is process level, so a paste target that is the
    // same app in another window is still `Ok(true)`.
    assert_eq!(r.tracker.is_current_target(first), Ok(true));
    // Restoring the first raises the first window.
    r.tracker.restore(first).unwrap();
    assert_eq!(r.apps.activated().last(), Some(&(SAFARI, Some(1))));
}

#[test]
fn revoked_accessibility_is_permission_denied_not_target_loss() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    r.apps.set_trusted(false);
    assert_eq!(
        r.tracker.is_current_target(h),
        Err(PlatformError::PermissionDenied(
            PermissionKind::Accessibility
        ))
    );
    // Restore only activates the application: no AX window is raised without trust.
    r.apps.set_trusted(true);
    r.apps.set_focused_window(SAFARI, 3);
    let h2 = r.tracker.current().unwrap();
    r.apps.set_trusted(false);
    r.tracker.restore(h2).unwrap();
    assert_eq!(r.apps.activated().last(), Some(&(SAFARI, None)));
}

#[test]
fn untrusted_observation_does_not_ask_for_ax_windows() {
    let r = rig();
    r.apps.set_trusted(false);
    r.apps.set_frontmost(safari());
    r.apps.set_focused_window(SAFARI, 9);
    let h = r.tracker.current().unwrap();
    r.apps.set_trusted(true);
    // Observed without AX: the entry has no window until a trusted observation sees one.
    r.tracker.restore(h).unwrap();
    assert_eq!(r.apps.activated(), vec![(SAFARI, None)]);
}

#[test]
fn activation_failure_is_an_error() {
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    r.apps.fail_activate(true);
    assert!(r.tracker.restore(h).is_err());
}

#[test]
fn errors_carry_no_application_data() {
    let r = rig();
    r.apps.set_frontmost(app(SAFARI, "com.secret.Bundle", 1));
    let h = r.tracker.current().unwrap();
    r.apps.quit(SAFARI);
    let err = r.tracker.restore(h).unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(!text.contains("secret"), "{text}");
}

#[test]
fn is_current_target_is_not_atomic_it_only_reports_the_instant_it_ran() {
    // The answer is a snapshot: a switch after it is invisible to the caller. This pins the
    // documented contract rather than pretending otherwise.
    let r = rig();
    r.apps.set_frontmost(safari());
    let h = r.tracker.current().unwrap();
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    r.apps.set_frontmost(notes());
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
}

// --- last external ---

fn last(r: &Rig) -> LastExternalTracker<Arc<FakeApps>> {
    LastExternalTracker::new(r.tracker.clone(), OWN)
}

#[test]
fn tray_remembers_the_last_external_target_and_ignores_own_windows() {
    let r = rig();
    let last = last(&r);
    r.apps.set_frontmost(safari());
    last.observe();
    // The user opens our settings window, then clicks the tray from there.
    r.apps
        .set_frontmost(app(OWN, "app.lingopop.tauri.macos", 1));
    last.observe();
    let remembered = last.last_external().expect("Safari is remembered");
    assert_eq!(r.tracker.restore(remembered), Ok(()));
    assert_eq!(r.apps.activated(), vec![(SAFARI, None)]);
    // The handle is the one a capture of Safari would have produced.
    r.apps.set_frontmost(safari());
    assert_eq!(r.tracker.current(), Some(remembered));
}

#[test]
fn last_external_follows_the_latest_external_app() {
    let r = rig();
    let last = last(&r);
    r.apps.set_frontmost(safari());
    last.observe();
    r.apps.set_frontmost(notes());
    // Sampled on demand too: no observe() call needed.
    let h = last.last_external().unwrap();
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    r.apps
        .set_frontmost(app(OWN, "app.lingopop.tauri.macos", 1));
    assert_eq!(last.last_external(), Some(h));
}

#[test]
fn last_external_is_none_when_nothing_was_seen_or_the_app_quit() {
    let r = rig();
    let last = last(&r);
    assert_eq!(last.last_external(), None);
    r.apps
        .set_frontmost(app(OWN, "app.lingopop.tauri.macos", 1));
    assert_eq!(last.last_external(), None, "only our own windows were seen");
    r.apps.set_frontmost(safari());
    assert!(last.last_external().is_some());
    r.apps.quit(SAFARI);
    assert_eq!(last.last_external(), None);
    // A reused pid is a different process.
    r.apps.set_frontmost(safari());
    let h = last.last_external().unwrap();
    r.apps
        .set_frontmost(app(OWN, "app.lingopop.tauri.macos", 1));
    r.apps.launch(app(SAFARI, "com.example.Impostor", 77));
    assert_eq!(last.last_external(), None);
    assert!(r.tracker.restore(h).is_err());
}

#[test]
fn fake_apps_implements_the_seam() {
    fn takes<P: MacApps>(_: &P) {}
    takes(&FakeApps::new());
}

#[test]
fn the_background_poller_samples_identity_only_and_never_asks_for_ax() {
    let r = rig();
    let tray = last(&r);
    r.apps.set_frontmost(safari());
    r.apps.set_focused_window(SAFARI, 4);
    tray.observe_light();
    let h = tray.last_external().unwrap(); // on demand: now with the window
    r.tracker.restore(h).unwrap();
    assert_eq!(r.apps.activated(), vec![(SAFARI, Some(4))]);

    // A light-only memory has no window to raise, but is still a valid, restorable target.
    let r = rig();
    r.apps.set_frontmost(safari());
    r.apps.set_focused_window(SAFARI, 4);
    let (h, _) = r.tracker.observe_frontmost_with(false).unwrap();
    r.tracker.restore(h).unwrap();
    assert_eq!(r.apps.activated(), vec![(SAFARI, None)]);
}

// --- process identity: the kernel start time ---

#[test]
fn pid_reuse_is_rejected_even_without_launch_date_or_bundle_id() {
    // Neither side has LaunchServices metadata (a binary started from a shell): only the
    // kernel start time can tell the two processes apart.
    let r = rig();
    r.apps.set_frontmost(bare(SAFARI, Some(111)));
    let h = r.tracker.current().unwrap();
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    r.apps.set_frontmost(bare(SAFARI, Some(222)));
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    assert!(!r.tracker.is_live(h));
    assert!(r.apps.activated().is_empty());
}

#[test]
fn start_time_mismatch_with_an_equal_pid_is_a_different_process() {
    let r = rig();
    let a = app(SAFARI, "com.apple.Safari", 1_000);
    r.apps.set_frontmost(a.clone());
    let h = r.tracker.current().unwrap();
    // Everything else identical, only the start time moved by one microsecond.
    let mut reused = a;
    reused.process_start = reused.process_start.map(|t| t + 1);
    r.apps.set_frontmost(reused);
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
}

#[test]
fn asymmetric_optional_metadata_is_not_a_mismatch_but_a_conflict_is() {
    let full = app(SAFARI, "com.apple.Safari", 1_000);
    let mut no_launch = full.clone();
    no_launch.launch = None;
    let mut no_bundle = full.clone();
    no_bundle.bundle_id = None;
    // One side lacks launchDate or bundle id: the start time decides.
    assert!(full.same_process(&no_launch) && no_launch.same_process(&full));
    assert!(full.same_process(&no_bundle) && no_bundle.same_process(&full));
    assert!(no_launch.same_process(&no_bundle));
    // Both sides know it and disagree: rejected although pid and start time are equal.
    let mut other_bundle = full.clone();
    other_bundle.bundle_id = Some("com.example.Other".into());
    assert!(!full.same_process(&other_bundle));
    let mut other_launch = full.clone();
    other_launch.launch = Some(2_000);
    assert!(!full.same_process(&other_launch));
    // Asymmetric through the tracker as well: captured with metadata, live without.
    let r = rig();
    r.apps.set_frontmost(full);
    let h = r.tracker.current().unwrap();
    let mut live = no_launch;
    live.bundle_id = None;
    r.apps.set_frontmost(live);
    assert_eq!(r.tracker.is_current_target(h), Ok(true));
    assert_eq!(r.tracker.restore(h), Ok(()));
}

#[test]
fn unreadable_start_time_fails_safe_on_either_side() {
    // Unreadable on both sides is NOT "equal": same_process is false.
    assert!(!bare(5, None).same_process(&bare(5, None)));
    assert!(!bare(5, None).same_process(&bare(5, Some(1))));
    assert!(!bare(5, Some(1)).same_process(&bare(5, None)));
    assert!(bare(5, Some(1)).same_process(&bare(5, Some(1))));

    // Captured while unreadable: the handle exists but never restores or pastes.
    let r = rig();
    r.apps.set_frontmost(bare(SAFARI, None));
    let h = r.tracker.current().unwrap();
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    assert!(r.apps.activated().is_empty());

    // Readable at capture, unreadable now (the kernel refused): same answer.
    let r = rig();
    r.apps.set_frontmost(bare(SAFARI, Some(10)));
    let h = r.tracker.current().unwrap();
    r.apps.set_frontmost(bare(SAFARI, None));
    assert_eq!(r.tracker.is_current_target(h), Ok(false));
    assert!(r.tracker.restore(h).is_err());
    assert!(!r.tracker.is_live(h));
}

// --- unknown AX window falls back to PROCESS identity, never to another entry ---

struct Fg(Arc<TargetTracker<Arc<FakeApps>>>);

impl ForegroundWindow for Fg {
    fn current(&self) -> Option<WindowHandle> {
        self.0.current()
    }
    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError> {
        self.0.restore(window)
    }
    fn is_elevated(&self, _: WindowHandle) -> bool {
        false
    }
    fn is_current_target(&self, window: WindowHandle) -> Result<bool, PlatformError> {
        self.0.is_current_target(window)
    }
}

/// Windows A and B of one process are registered and B is the captured target.
fn two_windows(r: &Rig) -> (WindowHandle, WindowHandle) {
    r.apps.set_frontmost(safari());
    r.apps.set_focused_window(SAFARI, 1);
    let a = r.tracker.current().unwrap();
    r.apps.set_focused_window(SAFARI, 2);
    let b = r.tracker.current().unwrap();
    assert_ne!(a, b);
    (a, b)
}

#[test]
fn unreadable_ax_window_after_restore_confirms_the_captured_entry_by_process() {
    // The review scenario: B is restored, the focused-window lookup then returns None. The
    // confirmation used to resolve to A (first compatible entry) and report ForegroundLost on
    // the correct process.
    let r = rig();
    let (a, b) = two_windows(&r);
    r.apps.set_frontmost(notes()); // the user left
    let fg = Fg(r.tracker.clone());
    r.apps.clear_focused_window(SAFARI); // AX cannot tell the window now
    let sleeper = FakeSleeper::new();
    assert!(bring_to_front(&fg, &sleeper, b, &ReplaceOptions::default()));
    assert_eq!(
        r.apps.activated().last(),
        Some(&(SAFARI, Some(2))),
        "B was raised"
    );
    assert_eq!(fg.current(), Some(b));
    assert_ne!(
        fg.current(),
        Some(a),
        "never resolves to the other window's entry"
    );
    assert_eq!(fg.is_current_target(b), Ok(true));
    // The paste check is process level too: unknown AX never refuses the right process.
    assert_eq!(fg.is_current_target(a), Ok(true));
}

#[test]
fn unreadable_ax_window_without_a_restore_never_picks_an_existing_entry() {
    let r = rig();
    let (a, b) = two_windows(&r);
    r.apps.clear_focused_window(SAFARI);
    let unknown = r.tracker.current().unwrap();
    assert_ne!(unknown, a);
    assert_ne!(unknown, b);
    assert_eq!(r.tracker.current(), Some(unknown), "stable between polls");
    // And restoring it raises nothing (no window is pinned).
    r.tracker.restore(unknown).unwrap();
    assert_eq!(r.apps.activated().last(), Some(&(SAFARI, None)));
}

#[test]
fn process_confirmation_expires_and_never_overrides_a_known_different_window() {
    let r = rig();
    let (a, b) = two_windows(&r);
    r.apps.set_frontmost(notes());
    r.tracker.restore(b).unwrap();
    // The raise did not take: AX says window 1 is focused. A known, different window is not
    // confirmed by process identity.
    r.apps.set_focused_window(SAFARI, 1);
    assert_eq!(r.tracker.current(), Some(a));
    let fg = Fg(r.tracker.clone());
    assert!(!bring_to_front(
        &fg,
        &FakeSleeper::new(),
        b,
        &ReplaceOptions::default()
    ));
    // After the confirmation window an unknown lookup is an ordinary AX-less observation again.
    r.apps.clear_focused_window(SAFARI);
    r.clock.advance(CONFIRM_WINDOW + Duration::from_secs(1));
    let later = r.tracker.current().unwrap();
    assert_ne!(later, b);
    assert_ne!(later, a);
}

#[test]
fn revoked_accessibility_during_restore_still_confirms_and_then_propagates() {
    let r = rig();
    let (_, b) = two_windows(&r);
    r.apps.set_frontmost(notes());
    r.apps.set_trusted(false);
    let fg = Fg(r.tracker.clone());
    assert!(bring_to_front(
        &fg,
        &FakeSleeper::new(),
        b,
        &ReplaceOptions::default()
    ));
    assert_eq!(
        fg.is_current_target(b),
        Err(PlatformError::PermissionDenied(
            PermissionKind::Accessibility
        ))
    );
}
