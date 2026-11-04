use lingopop_core::{
    host::HostKind,
    hotkey::{Hotkey, Modifier},
    operation::AIOperation,
    settings::Settings,
    time::{ManualClock, Timestamp},
};
#[test]
fn mac_translate_default_and_windows_labels_are_distinct() {
    assert_eq!(
        Hotkey::default_translate_for(HostKind::Unix),
        Hotkey::new("KeyT", vec![Modifier::Alt])
    );
    assert_eq!(Hotkey::default_translate().display_string(), "Ctrl+Alt+T");
    let command = Hotkey::new("KeyT", [Modifier::Win]).display_for(HostKind::Unix);
    assert!(command.contains('⌘'));
    assert!(!command.contains("Win"));
}
#[test]
fn old_settings_without_hotkeys_gets_platform_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, br#"{"activeProvider":"Claude"}"#).unwrap();
    let clock = ManualClock::new(Timestamp::from_apple_seconds(0.0));
    for host in [HostKind::Unix, HostKind::Windows] {
        let (s, _) = Settings::load(&path, &clock, host).unwrap();
        assert_eq!(
            s.hotkey(AIOperation::Translate),
            Some(&Hotkey::default_translate_for(host))
        );
        assert_eq!(s.hotkeys.len(), 1);
        std::fs::write(&path, br#"{"hotkeys":{}}"#).unwrap();
        assert!(Settings::load(&path, &clock, host)
            .unwrap()
            .0
            .hotkeys
            .is_empty());
        std::fs::write(&path, br#"{"activeProvider":"Claude"}"#).unwrap();
    }
}
