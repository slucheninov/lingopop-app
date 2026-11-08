use crate::{clipboard_history::ClipboardHistory, state::lock};
use lingopop_core::{clipboard_history::ClipboardHistoryStore, ManualClock, Timestamp};
use std::sync::{atomic::Ordering, Arc};
fn rig() -> (tempfile::TempDir, ClipboardHistory) {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = ClipboardHistoryStore::open(
        dir.path().join("clipboard-history.json"),
        Arc::new(ManualClock::new(Timestamp::from_apple_seconds(1.0))),
    )
    .unwrap();
    (dir, ClipboardHistory::new(Some(store), 100))
}
#[test]
fn stale_read_and_pending_same_generation_do_not_resurrect_clear() {
    let (_dir, history) = rig();
    let revision = history.revision.load(Ordering::Acquire);
    history.observe(revision, 1, "old".into()).unwrap();
    history.destructive(None, || 1).unwrap();
    history.observe(revision, 1, "old".into()).unwrap();
    history
        .observe(history.revision.load(Ordering::Acquire), 1, "old".into())
        .unwrap();
    assert!(history.entries().unwrap().is_empty());
    history
        .observe(history.revision.load(Ordering::Acquire), 2, "new".into())
        .unwrap();
    assert_eq!(history.entries().unwrap()[0].text, "new");
}
#[test]
fn deletion_during_generation_sample_rejects_reader_and_retains_other_entries() {
    let (_dir, history) = rig();
    history.observe(0, 1, "one".into()).unwrap();
    history.observe(0, 2, "two".into()).unwrap();
    let id = history.entries().unwrap()[0].id;
    history
        .destructive(Some(id), || {
            let revision = history.revision.load(Ordering::Acquire);
            history.observe(revision, 2, "two".into()).unwrap();
            2
        })
        .unwrap();
    assert_eq!(history.entries().unwrap().len(), 1);
    assert_eq!(history.entries().unwrap()[0].text, "one");
    assert!(!lock(&history.inner).mutating);
}

#[test]
fn copy_promotion_deduplicates_and_failed_delete_never_publishes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (dir, history) = rig();
    history.observe(0, 1, "older".into()).unwrap();
    history.observe(0, 2, "newer".into()).unwrap();
    let before = history.entries().unwrap();
    let older_id = before[1].id;
    let events = Arc::new(AtomicUsize::new(0));
    let fired = events.clone();
    history.set_on_change(Arc::new(move || {
        fired.fetch_add(1, Ordering::AcqRel);
    }));
    history.observe(0, 3, "older".into()).unwrap();
    assert_eq!(history.entries().unwrap()[0].id, older_id);
    assert_eq!(history.entries().unwrap().len(), 2);
    history.observe(0, 3, "older".into()).unwrap();
    assert_eq!(events.load(Ordering::Acquire), 1);
    let path = dir.path().join("clipboard-history.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(history.destructive(Some(older_id), || 3).is_err());
    assert_eq!(events.load(Ordering::Acquire), 1);
    assert_eq!(history.entries().unwrap().len(), 2);
}
#[test]
fn retention_decrease_applies_to_next_observation_and_reloads() {
    let (dir, history) = rig();
    history.observe(0, 1, "one".into()).unwrap();
    history.observe(0, 2, "two".into()).unwrap();
    history.trim(1).unwrap();
    let revision = history.revision.load(Ordering::Acquire);
    assert!(!history.observe(0, 3, "three".into()).unwrap());
    assert!(history.observe(revision, 3, "three".into()).unwrap());
    let (reloaded, _) = ClipboardHistoryStore::open(
        dir.path().join("clipboard-history.json"),
        Arc::new(ManualClock::new(Timestamp::from_apple_seconds(1.0))),
    )
    .unwrap();
    assert_eq!(reloaded.entries().len(), 1);
    assert_eq!(reloaded.entries()[0].text, "three");
}

#[tokio::test]
async fn observer_never_waits_for_the_command_action_gate() {
    let (_dir, history) = rig();
    let _copy_in_progress = history.actions.lock().await;
    // Observer callbacks enter only the short store mutex, even while a clipboard command
    // waits for native main-thread work under the command action gate.
    assert!(history.observe(0, 1, "foreign copy".into()).unwrap());
    assert_eq!(history.entries().unwrap().len(), 1);
}
#[test]
fn clipboard_settings_projection_and_patch_do_not_change_translation_limits() {
    use crate::settings_cmds::{apply_patch, settings_view_parts, SettingsPatch};
    let dir = tempfile::tempdir().unwrap();
    let (config, _, ui) = crate::task4_tests::rig(dir.path());
    let before = settings_view_parts(&config, &ui, &[]);
    assert_eq!(before.max_clipboard_history_count, 100);
    assert_eq!(before.menu_clipboard_history_count, 10);
    config
        .update_settings(|settings| {
            apply_patch(settings, SettingsPatch::MaxClipboardHistoryCount(9999));
            apply_patch(settings, SettingsPatch::MenuClipboardHistoryCount(9999));
        })
        .unwrap();
    let after = settings_view_parts(&config, &ui, &[]);
    assert_eq!(after.max_clipboard_history_count, 1000);
    assert_eq!(after.menu_clipboard_history_count, 50);
    assert_eq!(after.max_history_count, before.max_history_count);
    assert_eq!(after.menu_history_count, before.menu_history_count);
}

#[test]
fn deleting_an_older_entry_does_not_suppress_a_pending_foreign_copy() {
    let (_dir, history) = rig();
    history.observe(0, 1, "old".into()).unwrap();
    let id = history.entries().unwrap()[0].id;
    // Generation 2 exists in the OS, but its worker callback has not committed yet.
    history.destructive(Some(id), || 2).unwrap();
    let revision = history.revision.load(Ordering::Acquire);
    assert!(history.observe(revision, 2, "foreign".into()).unwrap());
    assert_eq!(history.entries().unwrap()[0].text, "foreign");
    assert_eq!(history.entries().unwrap().len(), 1);
}

#[test]
fn successive_deletes_and_stale_ids_keep_the_current_generation_fence() {
    let (_dir, history) = rig();
    history.observe(0, 1, "older".into()).unwrap();
    history.observe(0, 2, "current".into()).unwrap();
    let entries = history.entries().unwrap();
    history.destructive(Some(entries[0].id), || 2).unwrap();
    history.destructive(Some(entries[1].id), || 2).unwrap();
    history
        .destructive(Some(uuid::Uuid::new_v4()), || 2)
        .unwrap();
    let revision = history.revision.load(Ordering::Acquire);
    history.observe(revision, 2, "current".into()).unwrap();
    assert!(history.entries().unwrap().is_empty());
    // A stale id must not fence an unrelated new generation.
    history
        .destructive(Some(uuid::Uuid::new_v4()), || 3)
        .unwrap();
    history
        .observe(
            history.revision.load(Ordering::Acquire),
            3,
            "foreign".into(),
        )
        .unwrap();
    assert_eq!(history.entries().unwrap()[0].text, "foreign");
}

#[test]
fn failed_observation_requests_retry_and_recovers() {
    use lingopop_platform::clipboard_observer::ClipboardObservation;
    let (dir, history) = rig();
    let path = dir.path().join("clipboard-history.json");
    std::fs::create_dir(&path).unwrap();
    let item = || ClipboardObservation {
        generation: 1,
        revision: 0,
        text: "copied".into(),
    };
    assert!(!crate::clipboard_history::accept_observation(
        &history,
        item()
    ));
    assert!(history.entries().unwrap().is_empty());
    std::fs::remove_dir(&path).unwrap();
    assert!(crate::clipboard_history::accept_observation(
        &history,
        item()
    ));
    assert_eq!(history.entries().unwrap()[0].text, "copied");
}
