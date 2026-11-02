use super::*;
use lingopop_core::hotkey::Modifier;
use lingopop_orchestrator::ConfigSource;
use lingopop_platform::fake::FakeHotkeyService;
use std::sync::atomic::{AtomicUsize, Ordering};

struct InlineMainThread;
impl MainThread for InlineMainThread {
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        job();
    }
}
fn rig() -> (HotkeyRegistry, Arc<AtomicUsize>) {
    install_service_for_tests(Box::new(FakeHotkeyService::new()));
    let changes = Arc::new(AtomicUsize::new(0));
    let count = changes.clone();
    let registry = HotkeyRegistry::new(
        Arc::new(InlineMainThread),
        Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }),
    );
    (registry, changes)
}
fn wanted() -> BTreeMap<AIOperation, Hotkey> {
    [
        (AIOperation::Translate, Hotkey::default_translate()),
        (
            AIOperation::ScreenTranslate,
            Hotkey::new("KeyS", [Modifier::Ctrl]),
        ),
    ]
    .into()
}
#[test]
fn register_all_includes_screen_translate() {
    let (registry, _) = rig();
    registry.apply(wanted());
    assert_eq!(registry.status().len(), 6);
    assert_eq!(
        registry.status()[&AIOperation::ScreenTranslate],
        HotkeyStatus::Registered
    );
}
#[test]
fn screen_translate_is_registered() {
    register_all_includes_screen_translate();
}
#[test]
fn apply_registers_the_rest_when_one_conflicts() {
    let service = FakeHotkeyService::new();
    let chord = Hotkey::new("KeyE", [Modifier::Ctrl, Modifier::Alt]);
    service.block(&chord).unwrap();
    let (registry, _) = rig();
    install_service_for_tests(Box::new(service));
    let mut map = wanted();
    map.insert(AIOperation::Explain, chord);
    registry.apply(map);
    assert_eq!(
        registry.status()[&AIOperation::Explain],
        HotkeyStatus::Conflict
    );
    assert_eq!(
        registry.status()[&AIOperation::Translate],
        HotkeyStatus::Registered
    );
}
#[test]
fn invalid_chord_is_reported_not_fatal() {
    let (registry, _) = rig();
    let mut map = wanted();
    map.insert(AIOperation::Explain, Hotkey::new("", []));
    registry.apply(map);
    assert_eq!(
        registry.status()[&AIOperation::Explain],
        HotkeyStatus::Invalid
    );
    assert_eq!(
        registry.status()[&AIOperation::Translate],
        HotkeyStatus::Registered
    );
}
#[test]
fn suspend_then_resume_restores_all_registrations() {
    let (registry, _) = rig();
    let fake = FakeHotkeyService::new();
    let events = fake.events();
    install_service_for_tests(Box::new(fake.clone()));
    registry.apply(wanted());
    let old = registry.inner.lock().unwrap().by_id.clone();
    registry.suspend(true);
    assert!(registry.inner.lock().unwrap().by_id.is_empty());
    assert_eq!(fake.registered_count(), 0);
    for id in old.keys() {
        fake.fire(*id);
    }
    assert!(events.try_recv().is_err());
    SERVICE.with(|s| {
        let s = s.borrow();
        let service = s.as_ref().unwrap();
        // Each old chord is now free, proving the service was unregistered.
        for chord in wanted().values() {
            let id = service.register(chord).unwrap();
            service.unregister(id).unwrap();
        }
    });
    registry.suspend(false);
    assert_eq!(registry.inner.lock().unwrap().by_id, old);
    assert_eq!(fake.registered_count(), wanted().len());
    for id in old.keys() {
        fake.fire(*id);
        assert_eq!(events.try_recv().unwrap().id, *id);
    }
    assert!(registry
        .status()
        .iter()
        .filter(|(op, _)| wanted().contains_key(op))
        .all(|(_, status)| *status == HotkeyStatus::Registered));
}
#[test]
fn suspend_is_idempotent() {
    let (registry, count) = rig();
    registry.apply(wanted());
    registry.suspend(false);
    registry.suspend(false);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    registry.suspend(true);
    registry.suspend(true);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}
#[test]
fn blur_then_destroyed_keep_everything_registered() {
    let (registry, count) = rig();
    registry.apply(wanted());
    registry.suspend(true);
    crate::windows::on_settings_window_event(&registry, &tauri::WindowEvent::Focused(false));
    crate::windows::on_settings_window_event(&registry, &tauri::WindowEvent::Destroyed);
    assert_eq!(registry.inner.lock().unwrap().by_id.len(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(
        registry.status()[&AIOperation::Translate],
        HotkeyStatus::Registered
    );
}
#[test]
fn settings_window_close_resumes_hotkeys() {
    let (registry, _) = rig();
    registry.apply(wanted());
    registry.suspend(true);
    crate::windows::on_settings_window_event(&registry, &tauri::WindowEvent::Destroyed);
    assert_eq!(registry.inner.lock().unwrap().by_id.len(), 2);
}
#[test]
fn apply_does_not_hold_lock_across_main_thread() {
    struct Reentrant(Arc<Mutex<RegistryState>>);
    impl MainThread for Reentrant {
        fn run(&self, job: Box<dyn FnOnce() + Send>) {
            assert!(self.0.try_lock().is_ok());
            job();
        }
    }
    let (mut registry, _) = rig();
    registry.main = Arc::new(Reentrant(registry.inner.clone()));
    let inner = registry.inner.clone();
    registry.on_change = Arc::new(move || {
        assert!(inner.try_lock().is_ok());
    });
    registry.apply(wanted());
    registry.suspend(true);
    registry.suspend(false);
}
#[test]
fn forwarder_uses_current_map_after_apply() {
    let (registry, _) = rig();
    registry.apply(wanted());
    let old = *registry
        .inner
        .lock()
        .unwrap()
        .by_id
        .iter()
        .find(|(_, op)| **op == AIOperation::Translate)
        .unwrap()
        .0;
    let mut map = wanted();
    map.insert(
        AIOperation::Translate,
        Hotkey::new("KeyQ", [Modifier::Ctrl]),
    );
    registry.apply(map);
    assert_eq!(registry.lookup(old), None);
    let new = *registry
        .inner
        .lock()
        .unwrap()
        .by_id
        .iter()
        .find(|(_, op)| **op == AIOperation::Translate)
        .unwrap()
        .0;
    assert_eq!(registry.lookup(new), Some(AIOperation::Translate));
}
#[test]
fn settings_update_hotkey_rejects_duplicate() {
    rejected_patch_keeps_settings(
        Hotkey::new("KeyT", [Modifier::Alt, Modifier::Ctrl, Modifier::Ctrl]),
        "Already used by %@.|Translate",
    );
}
#[test]
fn settings_update_hotkey_rejects_invalid() {
    rejected_patch_keeps_settings(
        Hotkey::new("KeyT", []),
        "This shortcut can't be registered.",
    );
}
fn rejected_patch_keeps_settings(hotkey: Hotkey, error: &str) {
    let dir = tempfile::tempdir().unwrap();
    let (config, _, _) = crate::task4_tests::rig(dir.path());
    config.update_settings(|_| {}).unwrap();
    let before = config.settings().to_json_bytes().unwrap();
    let bytes = std::fs::read(dir.path().join("settings.json")).unwrap();
    let patch = crate::settings_cmds::SettingsPatch::Hotkey {
        operation: AIOperation::Explain,
        hotkey: Some(hotkey),
    };
    assert_eq!(
        crate::settings_cmds::save_patch(&config, patch, lingopop_core::ui_prefs::Language::En),
        Err(error.into())
    );
    assert_eq!(config.settings().to_json_bytes().unwrap(), before);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        bytes
    );
}
#[test]
fn error_with_args_round_trip() {
    assert_eq!(
        crate::settings_cmds::error_with_args("Already used by %@.", &["Translate"]),
        "Already used by %@.|Translate"
    );
}

#[test]
fn queued_apply_uses_latest_wanted_and_suspension() {
    #[derive(Default)]
    struct Queued(Mutex<Vec<Box<dyn FnOnce() + Send>>>);
    impl MainThread for Queued {
        fn run(&self, job: Box<dyn FnOnce() + Send>) {
            self.0.lock().unwrap().push(job);
        }
    }
    let main = Arc::new(Queued::default());
    let registry = HotkeyRegistry::new(main.clone(), Arc::new(|| {}));
    install_service_for_tests(Box::new(FakeHotkeyService::new()));
    registry.apply(wanted());
    registry.suspend(true);
    let mut latest = wanted();
    latest.remove(&AIOperation::Translate);
    registry.apply(latest);
    for job in std::mem::take(&mut *main.0.lock().unwrap()) {
        job();
    }
    assert!(registry.inner.lock().unwrap().by_id.is_empty());
    registry.suspend(false);
    for job in std::mem::take(&mut *main.0.lock().unwrap()) {
        job();
    }
    assert_eq!(registry.inner.lock().unwrap().by_id.len(), 1);
    assert_eq!(
        registry.status()[&AIOperation::Translate],
        HotkeyStatus::None
    );
    assert_eq!(
        registry.status()[&AIOperation::ScreenTranslate],
        HotkeyStatus::Registered
    );
}
