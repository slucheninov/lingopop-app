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
    ENTRY_LIFETIME,
};
use lingopop_platform::{PermissionKind, PlatformError, WindowHandle};

const OWN: i32 = 1;
const SAFARI: i32 = 100;
const NOTES: i32 = 200;

fn app(pid: i32, bundle: &str, launch: i64) -> AppIdentity {
    AppIdentity {
        pid,
        bundle_id: Some(bundle.into()),
        launch: Some(launch),
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

    // The window appears later: same handle, now with an AX window to raise.
    r.apps.set_focused_window(SAFARI, 7);
    assert_eq!(r.tracker.current(), Some(h));
    assert_eq!(r.tracker.restore(h), Ok(()));
    assert_eq!(r.apps.activated().last(), Some(&(SAFARI, Some(7))));
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
