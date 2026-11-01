use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lingopop_core::hotkey::Modifier;
use lingopop_platform::fake::{
    ClipboardCall, CopyBehavior, Event, FakeClipboard, FakeForeground, FakeKeys, FakeOp,
    FakeSleeper, KeyCall, Mutation,
};
use lingopop_platform::{
    capture_selection, CaptureOptions, Clipboard, ClipboardEntry, ClipboardFormat, LogFn,
    PlatformError, Sleeper as _, WindowHandle,
};

const W: WindowHandle = WindowHandle(1);

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
    let fg = FakeForeground::new(Some(W));
    Rig {
        clip,
        keys,
        fg,
        sleeper,
    }
}

fn capture(r: &Rig) -> Result<Option<String>, PlatformError> {
    capture_with(r, &CaptureOptions::default())
}

fn capture_with(r: &Rig, opts: &CaptureOptions) -> Result<Option<String>, PlatformError> {
    capture_selection(&*r.clip, &*r.keys, &*r.fg, &*r.sleeper, opts)
}

fn original() -> Option<String> {
    Some("ORIGINAL".to_owned())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn log_sink() -> (Arc<Mutex<Vec<String>>>, CaptureOptions) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let log: Arc<LogFn> = Arc::new(move |m: &str| sink.lock().unwrap().push(m.to_owned()));
    (
        lines,
        CaptureOptions {
            log: Some(log),
            ..CaptureOptions::default()
        },
    )
}

#[test]
fn capture_returns_selected_text_and_restores_original_clipboard() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_nothing_selected_returns_none_and_never_writes_clipboard() {
    let r = rig();
    let before = r.clip.entries();
    assert_eq!(capture(&r).unwrap(), None);
    let calls = r.clip.calls();
    assert!(calls.contains(&ClipboardCall::Snapshot));
    assert!(!calls.contains(&ClipboardCall::Restore));
    assert!(!calls.contains(&ClipboardCall::Write));
    assert!(!calls.contains(&ClipboardCall::WriteTransient));
    assert_eq!(r.clip.entries(), before);
}

#[test]
fn capture_identical_text_is_still_detected_via_sequence() {
    let r = rig();
    r.clip.set_external_text("same");
    r.keys.on_copy(CopyBehavior::Select("same".into()));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("same"));
}

#[test]
fn capture_waits_for_slow_copy_within_300ms() {
    let r = rig();
    r.keys
        .on_copy(CopyBehavior::SelectAfter(ms(250), "slow".into()));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("slow"));
}

#[test]
fn capture_gives_up_after_300ms() {
    let r = rig();
    r.keys
        .on_copy(CopyBehavior::SelectAfter(ms(400), "late".into()));
    assert_eq!(capture(&r).unwrap(), None);
    assert!(r.sleeper.now() <= ms(320), "{:?}", r.sleeper.now());
}

#[test]
fn capture_final_recheck_catches_copy_landing_at_deadline() {
    let r = rig();
    r.keys
        .on_copy(CopyBehavior::SelectAfter(ms(300), "edge".into()));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("edge"));
}

#[test]
fn capture_waits_for_double_sequence_bump() {
    // The set follows the clear by 30 ms, past the point where the sequence looks stable.
    let r = rig();
    r.keys.on_copy(CopyBehavior::ClearThenSetLate {
        first: ms(20),
        second: ms(50),
        text: "x".into(),
    });
    assert_eq!(capture(&r).unwrap().as_deref(), Some("x"));
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_empty_then_set_within_settle_budget_returns_text() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::ClearThenSetLate {
        first: ms(20),
        second: ms(60),
        text: "x".into(),
    });
    assert_eq!(capture(&r).unwrap().as_deref(), Some("x"));
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_empty_then_set_after_settle_budget_is_none() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::ClearThenSetLate {
        first: ms(20),
        second: ms(200),
        text: "x".into(),
    });
    assert_eq!(capture(&r).unwrap(), None);
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_non_text_copy_returns_none_and_restores() {
    let r = rig();
    let before = r.clip.entries();
    r.keys.on_copy(CopyBehavior::ImageOnly);
    assert_eq!(capture(&r).unwrap(), None);
    // Gave up only after the settle budget (100 ms) was spent.
    assert!(r.sleeper.now() >= ms(100), "{:?}", r.sleeper.now());
    assert_eq!(r.clip.entries(), before);
}

#[test]
fn capture_empty_or_whitespace_text_is_none() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("  \n\t".into()));
    assert_eq!(capture(&r).unwrap(), None);
}

#[test]
fn capture_waits_for_modifiers_release_without_synthesizing() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.keys
        .hold_modifiers(vec![Modifier::Ctrl, Modifier::Alt], Some(ms(80)));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    let events = r.clip.log().events();
    assert!(!r
        .keys
        .events()
        .iter()
        .any(|k| matches!(k, KeyCall::ReleaseModifiers(_))));
    let copy_at = events
        .iter()
        .find(|e| e.event == Event::Key(KeyCall::SendCopy))
        .unwrap()
        .at;
    assert!(copy_at >= ms(80), "{copy_at:?}");
}

#[test]
fn capture_releases_held_modifiers_after_timeout() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.keys
        .hold_modifiers(vec![Modifier::Ctrl, Modifier::Alt], None);
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    assert_eq!(
        r.keys.events(),
        vec![
            KeyCall::MaskMenuActivation,
            KeyCall::ReleaseModifiers(vec![Modifier::Ctrl, Modifier::Alt]),
            KeyCall::SendCopy
        ]
    );
    let release_at = r
        .clip
        .log()
        .events()
        .iter()
        .find(|e| matches!(e.event, Event::Key(KeyCall::ReleaseModifiers(_))))
        .unwrap()
        .at;
    assert_eq!(release_at, ms(300));
}

#[test]
fn capture_elevated_foreground_is_input_blocked_before_touching_clipboard() {
    let r = rig();
    r.fg.set_elevated(W, true);
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    assert_eq!(capture(&r), Err(PlatformError::InputBlocked));
    assert!(r.clip.calls().is_empty());
    assert_eq!(r.keys.events(), vec![KeyCall::MaskMenuActivation]);
}

#[test]
fn capture_send_copy_blocked_is_input_blocked_and_clipboard_intact() {
    let r = rig();
    let before = r.clip.entries();
    r.keys.on_copy(CopyBehavior::Blocked);
    assert_eq!(capture(&r), Err(PlatformError::InputBlocked));
    assert_eq!(r.clip.entries(), before);
}

#[test]
fn capture_restores_snapshot_when_read_text_fails() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.clip
        .fail_next(FakeOp::ReadText, PlatformError::ClipboardBusy);
    assert_eq!(capture(&r), Err(PlatformError::ClipboardBusy));
    assert!(r.clip.calls().contains(&ClipboardCall::Restore));
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_restores_snapshot_when_read_text_panics() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.clip.panic_on_read_text();
    let res = catch_unwind(AssertUnwindSafe(|| capture(&r)));
    assert!(res.is_err());
    assert_eq!(r.clip.text(), original());
}

#[test]
fn capture_restore_failure_keeps_text_and_logs() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.clip
        .fail_next(FakeOp::Restore, PlatformError::ClipboardBusy);
    let (lines, opts) = log_sink();
    assert_eq!(capture_with(&r, &opts).unwrap().as_deref(), Some("hello"));
    assert!(lines
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.contains("restore failed")));
}

#[test]
fn capture_does_not_clobber_a_foreign_write_after_copy() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    // Another app writes right after our copy settled, before we restore.
    r.clip
        .on_read_text(Box::new(|c| c.set_external_text("FOREIGN")));
    // The text read may be the foreign write, so it is not returned as the selection.
    assert_eq!(capture(&r), Ok(None));
    assert!(!r.clip.calls().contains(&ClipboardCall::Restore));
    assert_eq!(r.clip.text().as_deref(), Some("FOREIGN"));
}

#[test]
fn capture_foreign_write_during_modifier_wait_is_in_the_snapshot() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.keys.hold_modifiers(vec![Modifier::Ctrl], Some(ms(80)));
    r.clip.schedule(ms(40), Mutation::SetText("FOREIGN".into()));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    // Restore writes back the newer foreign value, not the pre-wait one.
    assert_eq!(r.clip.text().as_deref(), Some("FOREIGN"));
}

#[test]
fn restore_guard_does_not_clobber_a_write_between_check_and_restore() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    // Fires inside `restore_if`, i.e. after the guard's sequence check passed.
    r.clip
        .on_restore_if(Box::new(|c| c.set_external_text("LATE-FOREIGN")));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    assert_eq!(r.clip.text().as_deref(), Some("LATE-FOREIGN"));
}

#[test]
fn capture_snapshot_failure_aborts_before_copy() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.clip
        .fail_next(FakeOp::Snapshot, PlatformError::ClipboardBusy);
    assert_eq!(capture(&r), Err(PlatformError::ClipboardBusy));
    assert!(!r.keys.events().contains(&KeyCall::SendCopy));
}

#[test]
fn capture_logs_skipped_format_names_not_contents() {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone())
        .with_text("ORIGINAL")
        .with_skipped(vec!["format 49161".into()]);
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    let fg = FakeForeground::new(Some(W));
    let (lines, opts) = log_sink();
    capture_selection(&*clip, &*keys, &*fg, &*sleeper, &opts).unwrap();
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("49161")));
}

#[test]
fn capture_log_never_contains_selected_text() {
    let r = rig();
    r.keys
        .on_copy(CopyBehavior::Select("SECRET-SELECTION-42".into()));
    r.clip
        .fail_next(FakeOp::Restore, PlatformError::ClipboardBusy);
    let (lines, opts) = log_sink();
    let _ = capture_with(&r, &opts);
    assert!(!lines.lock().unwrap().is_empty());
    for l in lines.lock().unwrap().iter() {
        assert!(!l.contains("SECRET-SELECTION-42"), "{l}");
    }
    let snap = r.clip.snapshot().unwrap();
    assert!(!format!("{snap:?}").contains("SECRET-SELECTION-42"));
}

#[test]
fn clipboard_snapshot_debug_prints_counts_only() {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper).with_entries(vec![ClipboardEntry {
        format: ClipboardFormat::UnicodeText,
        data: b"SECRET".to_vec(),
    }]);
    let snap = clip.snapshot().unwrap();
    let shown = format!("{snap:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(shown.contains("entries: 1"), "{shown}");
    assert!(!format!("{:?}", snap.entries[0]).contains("SECRET"));
}

#[test]
fn capture_masks_menu_activation_before_wait_and_copy() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Select("hello".into()));
    r.keys
        .hold_modifiers(vec![Modifier::Ctrl, Modifier::Alt], Some(ms(80)));
    assert_eq!(capture(&r).unwrap().as_deref(), Some("hello"));
    let events = r.clip.log().events();
    let mask_at = events
        .iter()
        .position(|e| e.event == Event::Key(KeyCall::MaskMenuActivation))
        .expect("mask requested");
    let copy_at = events
        .iter()
        .position(|e| e.event == Event::Key(KeyCall::SendCopy))
        .unwrap();
    assert_eq!(mask_at, 0);
    assert!(mask_at < copy_at);
    assert_eq!(
        events[mask_at].at,
        Duration::ZERO,
        "before the modifier wait"
    );
}
