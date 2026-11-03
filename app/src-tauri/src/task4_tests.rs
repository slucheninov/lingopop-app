use crate::{settings_cmds::*, state::*};
use lingopop_core::{
    paths::DataPaths,
    provider::{default_providers, ProviderType},
    secrets::encode_payload,
    time::SystemClock,
    ui_prefs::Language,
};
use lingopop_orchestrator::{ConfigSource, Persist};
use lingopop_platform::fake::FakeSystem;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
struct Protector;
impl lingopop_core::secrets::SecretProtector for Protector {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
        Ok(data.to_vec())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
        Ok(data.to_vec())
    }
}
pub(crate) fn platform() -> lingopop_platform::Platform {
    use lingopop_platform::{fake::*, Platform, PlatformError, Point};
    let sleeper = Arc::new(FakeSleeper::new());
    let clipboard = FakeClipboard::new(sleeper.clone());
    Platform {
        overlay: Arc::new(lingopop_platform::fake::FakeOverlay::new(Ok(None))),
        system: Arc::new(FakeSystem::new(vec!["uk-UA".into()])),
        login_item: Arc::new(FakeLoginItem::new()),
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: Arc::new(FakeDismissWatcher::default()),
        keys: FakeKeys::new(clipboard.clone(), sleeper.clone()),
        clipboard,
        foreground: FakeForeground::new(None),
        last_foreground: lingopop_platform::fake::FakeLastForeground::new(None),
        monitors: Arc::new(FakeMonitors::new(Point { x: 0, y: 0 }, vec![])),
        capture: Arc::new(FakeCapture::new([0; 4])),
        ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
        protector: Arc::new(Protector),
        sleeper,
    }
}
pub(crate) fn rig(dir: &std::path::Path) -> (Arc<ShellConfig>, Arc<ShellStores>, Arc<UiState>) {
    let mut platform = platform();
    platform.system = Arc::new(FakeSystem::new(vec!["uk-UA".into()]));
    load(&DataPaths::new(dir), &platform, Arc::new(SystemClock))
}
#[test]
fn settings_patch_json_shape() {
    for json in [
        r#"{"field":"fallback","value":{"provider":"Claude","fallback":null}}"#,
        r#"{"field":"hotkey","value":{"operation":"translate","hotkey":{"key":"KeyT","modifiers":["ctrl","alt"]}}}"#,
        r#"{"field":"autoCopyOCR","value":false}"#,
    ] {
        serde_json::from_str::<SettingsPatch>(json).unwrap();
    }
}
#[test]
fn ui_patch_json_shape() {
    assert!(matches!(
        serde_json::from_str::<UiPatch>(r#"{"field":"popupFontSize","value":16}"#).unwrap(),
        UiPatch::PopupFontSize(16)
    ));
    for lang in ["uk", "system"] {
        serde_json::from_value::<UiPatch>(serde_json::json!({"field":"appLanguage","value":lang}))
            .unwrap();
    }
}
#[test]
fn settings_patch_debug_hides_user_text() {
    assert_eq!(
        format!("{:?}", SettingsPatch::MagicPrompt("secret words".into())),
        "MagicPrompt(12 chars)"
    );
    assert_eq!(
        format!("{:?}", SettingsPatch::LanguagePairs(Default::default())),
        "LanguagePairs(0 pairs)"
    );
}
#[test]
fn settings_update_round_trips_the_12_fields_only() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let before = c.settings();
    c.update_settings(|s| apply_patch(s, SettingsPatch::TargetLanguage("German".into())))
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(d.path().join("settings.json")).unwrap()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&before.to_json_bytes().unwrap()).unwrap();
    assert_eq!(
        v.as_object().unwrap().keys().collect::<Vec<_>>(),
        expected.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    assert_eq!(v["targetLanguage"], "German");
    assert_eq!(v["magicPrompt"], expected["magicPrompt"]);
}
#[test]
fn update_settings_restores_previous_on_save_error() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let before = c.settings();
    std::fs::create_dir(d.path().join("settings.json")).unwrap();
    assert!(c
        .update_settings(|s| s.target_language = "German".into())
        .is_err());
    assert_eq!(c.settings(), before);
}
#[test]
fn set_connection_refuses_when_corrupt() {
    let d = tempfile::tempdir().unwrap();
    // A fake identity protector isolates the payload contract from the OS.
    let mut entries = std::collections::BTreeMap::new();
    entries.insert("providerSettings.v1".into(), "garbage".into());
    let platform = platform();
    let bytes = platform
        .protector
        .protect(&encode_payload(&entries).unwrap())
        .unwrap();
    std::fs::write(d.path().join("secrets.dat"), &bytes).unwrap();
    let (c, _, _) = rig(d.path());
    assert!(matches!(
        c.set_connection(
            ProviderType::Claude,
            default_providers()[&ProviderType::Claude].clone()
        ),
        Err(SecretsWriteError::Corrupt)
    ));
    assert_eq!(std::fs::read(d.path().join("secrets.dat")).unwrap(), bytes);
}
#[test]
fn set_connection_keeps_other_providers() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let mut a = default_providers()[&ProviderType::Claude].clone();
    a.base_url = "https://c".into();
    c.set_connection(ProviderType::Claude, a).unwrap();
    let mut b = default_providers()[&ProviderType::OpenAI].clone();
    b.model = "m".into();
    c.set_connection(ProviderType::OpenAI, b).unwrap();
    let (c, _, _) = rig(d.path());
    assert_eq!(c.connection(ProviderType::Claude).base_url, "https://c");
    assert_eq!(c.connection(ProviderType::OpenAI).model, "m");
}
#[test]
fn secrets_write_error_display_is_fixed_text() {
    assert_eq!(
        SecretsWriteError::Store(SecretsErrorKind::Encode).to_string(),
        "secrets could not be encoded"
    );
}
#[test]
fn settings_view_json_has_no_key_material() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, u) = rig(d.path());
    c.set_api_key(ProviderType::Claude, "sk-SECRET").unwrap();
    let v = serde_json::to_value(settings_view_parts(&c, &u, &[])).unwrap();
    assert!(!v.to_string().contains("SECRET"));
    assert_eq!(
        v["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "Claude")
            .unwrap()["hasKey"],
        true
    );
}
#[test]
fn url_allowed_rules() {
    assert!(url_allowed(lingopop_core::updates::RELEASES_PAGE_URL));
    assert!(url_allowed(
        "https://github.com/slucheninov/lingopop-app/blob/main/docs/api-keys.md#claude"
    ));
    for u in [
        "https://evil.com",
        "https://github.com/slucheninov/lingopop-app/blob/main/../x",
        "file:///C:/x",
        "https://github.com/slucheninov/lingopop-app/blob/main/%2e%2e/x",
        "https://github.com/slucheninov/lingopop-app/blob/main/a b",
    ] {
        assert!(!url_allowed(u));
    }
}
fn entry() -> lingopop_core::records::HistoryEntry {
    serde_json::from_value(serde_json::json!({"id":"00000000-0000-0000-0000-000000000001","timestamp":0,"original":"a","translated":"b","operation":"translate","provider":"Claude","targetLanguage":"German"})).unwrap()
}
#[test]
fn stores_change_hook_fires_on_every_mutation() {
    let d = tempfile::tempdir().unwrap();
    let (_, s, _) = rig(d.path());
    let h = Arc::new(AtomicUsize::new(0));
    let u = Arc::new(AtomicUsize::new(0));
    let (hh, uu) = (h.clone(), u.clone());
    s.set_on_change(Arc::new(move |kind| {
        match kind {
            StoreKind::History => &hh,
            StoreKind::Usage => &uu,
        }
        .fetch_add(1, Ordering::SeqCst);
    }));
    let e = entry();
    let id = e.id;
    s.add_history(e, 10);
    s.remove_history(id).unwrap();
    s.trim_history(2);
    s.clear_history().unwrap();
    let record=serde_json::from_value(serde_json::json!({"id":"00000000-0000-0000-0000-000000000002","timestamp":0,"provider":"Claude","model":"m","operation":"translate","inputTokens":1,"outputTokens":1})).unwrap();
    s.append_usage(record);
    s.reset_usage().unwrap();
    assert_eq!(h.load(Ordering::SeqCst), 4);
    assert_eq!(u.load(Ordering::SeqCst), 2);
}
#[test]
fn stores_change_hook_may_read_the_store() {
    let d = tempfile::tempdir().unwrap();
    let (_, s, _) = rig(d.path());
    let weak = Arc::downgrade(&s);
    s.set_on_change(Arc::new(move |_| {
        assert_eq!(weak.upgrade().unwrap().history_entries().len(), 1)
    }));
    s.add_history(entry(), 10);
}
#[test]
fn load_computes_launched_language_from_system() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(rig(d.path()).2.launched, Language::Uk);
    std::fs::write(d.path().join("ui.json"), r#"{"appLanguage":"de"}"#).unwrap();
    assert_eq!(rig(d.path()).2.launched, Language::De);
}
#[test]
fn pending_tab_is_taken_once() {
    let d = tempfile::tempdir().unwrap();
    let (_, _, u) = rig(d.path());
    u.set_pending_tab(Some("about".into()));
    assert_eq!(u.take_pending_tab().as_deref(), Some("about"));
    assert_eq!(u.take_pending_tab(), None);
}

#[test]
fn settings_effects_trim_uses_committed_history_limit() {
    let d = tempfile::tempdir().unwrap();
    let (config, stores, _) = rig(d.path());
    for _ in 0..5 {
        let mut e = entry();
        e.id = uuid::Uuid::new_v4();
        stores.add_history(e, 10);
    }
    let earlier = config.update_settings(|s| s.max_history_count = 2).unwrap();
    config
        .update_settings(|s| s.max_history_count = 10)
        .unwrap();
    assert_eq!(earlier.max_history_count, 2);
    // Side effects must read committed settings rather than use the earlier save result.
    // This also proves no config guard is held across a reentrant history callback.
    let c = config.clone();
    stores.set_on_change(Arc::new(move |_| {
        assert_eq!(c.settings().max_history_count, 10)
    }));
    apply_settings_effects(&config, &stores, &NoSettingsHooks, true, false, false);
    assert_eq!(stores.history_entries().len(), 5);
    assert_eq!(config.settings().max_history_count, 10);
}

#[cfg(unix)]
#[test]
fn reset_usage_returns_the_store_error() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let (_, s, _) = rig(d.path());
    assert!(s.usage_available());
    let mode = std::fs::metadata(d.path()).unwrap().permissions().mode();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = s.reset_usage();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(mode)).unwrap();
    assert!(result.is_err());
}
