use std::sync::Arc;
use std::time::Duration;

use lingopop_platform::fake::{
    ClipboardCall, Event, FakeClipboard, FakeForeground, FakeKeys, FakeOp, FakeSleeper, KeyCall,
};
use lingopop_platform::{
    replace_selection, PlatformError, ReplaceOptions, ReplaceOutcome, Sleeper as _, WindowHandle,
};

const T: WindowHandle = WindowHandle(7);

struct Rig {
    clip: Arc<FakeClipboard>,
    keys: Arc<FakeKeys>,
    fg: Arc<FakeForeground>,
    sleeper: Arc<FakeSleeper>,
}

fn rig() -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone()).with_text("ORIGINAL");
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    let fg = FakeForeground::new(Some(WindowHandle(1)));
    fg.log_to(clip.log());
    Rig {
        clip,
        keys,
        fg,
        sleeper,
    }
}

fn replace(r: &Rig) -> Result<ReplaceOutcome, PlatformError> {
    replace_selection(
        &*r.fg,
        &*r.clip,
        &*r.keys,
        &*r.sleeper,
        T,
        "NEW",
        &ReplaceOptions::default(),
    )
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn replace_pastes_text_and_restores_snapshot() {
    let r = rig();
    assert_eq!(replace(&r), Ok(ReplaceOutcome::Pasted));
    assert_eq!(r.keys.pasted(), vec![some("NEW")]);
    assert_eq!(r.clip.text(), some("ORIGINAL"));
}

#[test]
fn replace_writes_transient_not_plain_text() {
    let r = rig();
    replace(&r).unwrap();
    let calls = r.clip.calls();
    assert!(calls.contains(&ClipboardCall::WriteTransient));
    assert!(!calls.contains(&ClipboardCall::Write));
}

#[test]
fn replace_restores_foreground_before_paste() {
    let r = rig();
    replace(&r).unwrap();
    assert_eq!(r.fg.restored(), vec![T]);
    let events: Vec<Event> = r.clip.log().events().into_iter().map(|e| e.event).collect();
    let fg_pos = events
        .iter()
        .position(|e| *e == Event::Foreground(T))
        .unwrap();
    let paste_pos = events
        .iter()
        .position(|e| *e == Event::Key(KeyCall::SendPaste))
        .unwrap();
    assert!(fg_pos < paste_pos);
}

#[test]
fn replace_confirms_foreground_by_polling() {
    let r = rig();
    r.fg.foreground_after_polls(3);
    assert_eq!(replace(&r), Ok(ReplaceOutcome::Pasted));
    let polls: Vec<_> = r
        .sleeper
        .slept()
        .into_iter()
        .filter(|d| *d == Duration::from_millis(20))
        .collect();
    assert_eq!(polls.len(), 2);
}

#[test]
fn replace_foreground_failure_leaves_plain_text_and_reports() {
    let r = rig();
    r.fg.fail_restore(true);
    assert_eq!(replace(&r), Ok(ReplaceOutcome::ForegroundLost));
    assert_eq!(r.clip.text(), some("NEW"));
    let calls = r.clip.calls();
    assert!(calls.contains(&ClipboardCall::Write));
    assert!(!calls.contains(&ClipboardCall::WriteTransient));
    assert!(r.keys.events().is_empty());
}

#[test]
fn replace_foreground_never_arriving_times_out_as_lost() {
    let r = rig();
    r.fg.foreground_after_polls(1000);
    assert_eq!(replace(&r), Ok(ReplaceOutcome::ForegroundLost));
    assert!(r.sleeper.now() <= Duration::from_millis(320));
    assert_eq!(r.clip.text(), some("NEW"));
}

#[test]
fn replace_elevated_target_is_input_blocked() {
    let r = rig();
    r.fg.set_elevated(T, true);
    assert_eq!(replace(&r), Err(PlatformError::InputBlocked));
    assert!(r.clip.calls().is_empty());
    assert_eq!(r.clip.text(), some("ORIGINAL"));
    assert!(r.fg.restored().is_empty());
}

#[test]
fn replace_paste_blocked_restores_snapshot() {
    let r = rig();
    r.keys.block_paste();
    assert_eq!(replace(&r), Err(PlatformError::InputBlocked));
    assert_eq!(r.clip.text(), some("ORIGINAL"));
}

#[test]
fn replace_does_not_clobber_newer_clipboard_write() {
    let r = rig();
    r.keys
        .on_paste(Box::new(|c| c.set_external_text("USER-COPIED")));
    assert_eq!(replace(&r), Ok(ReplaceOutcome::Pasted));
    assert!(!r.clip.calls().contains(&ClipboardCall::Restore));
    assert_eq!(r.clip.text(), some("USER-COPIED"));
}

#[test]
fn replace_waits_restore_delay_after_paste() {
    let r = rig();
    replace(&r).unwrap();
    assert!(r.sleeper.slept().contains(&Duration::from_millis(150)));
    assert_eq!(r.keys.pasted(), vec![some("NEW")]);
    let events = r.clip.log().events();
    let paste_at = events
        .iter()
        .find(|e| e.event == Event::Key(KeyCall::SendPaste))
        .unwrap()
        .at;
    let restore_at = events
        .iter()
        .find(|e| e.event == Event::Clipboard(ClipboardCall::Restore))
        .unwrap()
        .at;
    assert!(restore_at >= paste_at + Duration::from_millis(150));
}

#[test]
fn replace_transient_write_failure_restores_emptied_clipboard() {
    let r = rig();
    r.clip
        .fail_next(FakeOp::WriteTransient, PlatformError::ClipboardBusy);
    assert_eq!(replace(&r), Err(PlatformError::ClipboardBusy));
    assert!(r.clip.calls().contains(&ClipboardCall::Restore));
    assert_eq!(r.clip.text(), some("ORIGINAL"));
    assert!(r.keys.events().is_empty());
}

#[test]
fn replace_fallback_refuses_when_the_clipboard_is_not_preservable() {
    let r = rig();
    r.clip.set_unpreservable(true);
    r.fg.fail_restore(true);
    assert_eq!(
        replace(&r),
        Err(PlatformError::Unsupported("pasteboard snapshot"))
    );
    assert_eq!(r.clip.text(), some("ORIGINAL"));
    let calls = r.clip.calls();
    assert!(!calls.contains(&ClipboardCall::Write));
    assert!(!calls.contains(&ClipboardCall::WriteTransient));
    assert!(r.keys.events().is_empty());
}

#[test]
fn replace_default_clipboard_is_always_preservable_and_takes_no_extra_call() {
    // The default `check_preservable` is `Ok(())`: the Windows-like path is unchanged.
    let r = rig();
    r.fg.fail_restore(true);
    assert_eq!(replace(&r), Ok(ReplaceOutcome::ForegroundLost));
    assert_eq!(r.clip.calls(), vec![ClipboardCall::Write]);
}

// --- `is_current_target`: the last look before the keystroke (NOT atomic) ---

#[test]
fn replace_checks_the_target_once_right_before_the_paste() {
    let r = rig();
    assert_eq!(replace(&r), Ok(ReplaceOutcome::Pasted));
    assert_eq!(r.fg.target_checks(), vec![T]);
    let events: Vec<Event> = r.clip.log().events().into_iter().map(|e| e.event).collect();
    assert!(events.contains(&Event::Key(KeyCall::SendPaste)));
}

#[test]
fn foreground_change_after_restore_never_sends_to_the_new_app() {
    // The check is not atomic (the user can still switch between the answer and the
    // keystroke); this pins the case it does catch: the switch happened before the check.
    let r = rig();
    r.fg.switch_foreground_on_next_target_check(WindowHandle(55));
    assert_eq!(replace(&r), Ok(ReplaceOutcome::ForegroundLost));
    assert!(r.keys.pasted().is_empty());
    assert!(!r.keys.events().contains(&KeyCall::SendPaste));
    // The text is an ordinary clipboard entry for a manual paste.
    assert_eq!(r.clip.text(), some("NEW"));
    let calls = r.clip.calls();
    assert!(calls.contains(&ClipboardCall::Write));
}

#[test]
fn stale_target_is_a_lost_target_not_a_paste() {
    let r = rig();
    r.fg.retire_target(T);
    assert_eq!(replace(&r), Ok(ReplaceOutcome::ForegroundLost));
    assert!(r.keys.pasted().is_empty());
    assert_eq!(r.clip.text(), some("NEW"));
}

#[test]
fn revoked_accessibility_never_pastes() {
    use lingopop_platform::fake::FakePermissions;
    use lingopop_platform::{PermissionKind, PermissionState};
    let r = rig();
    let perms = FakePermissions::new();
    perms.set_state(PermissionKind::Accessibility, PermissionState::Denied);
    r.fg.require_permission(perms.clone());
    r.keys.require_permission(perms);
    assert_eq!(
        replace(&r),
        Err(PlatformError::PermissionDenied(
            PermissionKind::Accessibility
        ))
    );
    // `send_paste` was never even attempted, and the user's clipboard is back.
    let attempts = r
        .keys
        .events()
        .into_iter()
        .filter(|e| *e == KeyCall::SendPaste)
        .count();
    assert_eq!(attempts, 0);
    assert!(r.keys.pasted().is_empty());
    assert_eq!(r.clip.text(), some("ORIGINAL"));
}

#[test]
fn different_frontmost_after_pid_switch_is_not_the_target() {
    use lingopop_platform::ForegroundWindow as _;
    let r = rig();
    r.fg.set_current(Some(T));
    assert_eq!(r.fg.is_current_target(T), Ok(true));
    r.fg.set_current(Some(WindowHandle(99)));
    assert_eq!(r.fg.is_current_target(T), Ok(false));
}
