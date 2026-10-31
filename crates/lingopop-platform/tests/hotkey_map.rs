//! Hotkey mapping tests: `to_chord` and the fake run on every OS, `to_global_hotkey` on windows and macos.

use std::str::FromStr;
use std::time::Duration;

use keyboard_types::{Code, Modifiers};
use lingopop_core::hotkey::{Hotkey, Modifier};
use lingopop_platform::fake::FakeHotkeyService;
use lingopop_platform::{to_chord, HotkeyEvent, HotkeyService, PlatformError};

const NAMED_KEYS: [&str; 25] = [
    "Space",
    "Enter",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Backquote",
    "Minus",
    "Equal",
    "BracketLeft",
    "BracketRight",
    "Backslash",
    "Semicolon",
    "Quote",
    "Comma",
    "Period",
    "Slash",
];

#[test]
fn to_chord_maps_default_translate_hotkey() {
    assert_eq!(
        to_chord(&Hotkey::default_translate()),
        Ok((Modifiers::CONTROL | Modifiers::ALT, Code::KeyT))
    );
}

#[test]
fn to_chord_maps_win_to_meta() {
    let hotkey = Hotkey::new("Digit1", [Modifier::Win, Modifier::Shift]);
    assert_eq!(
        to_chord(&hotkey),
        Ok((Modifiers::META | Modifiers::SHIFT, Code::Digit1))
    );
}

#[test]
fn to_chord_accepts_every_key_core_accepts() {
    let mut keys: Vec<String> = ('A'..='Z').map(|c| format!("Key{c}")).collect();
    keys.extend((0..=9).map(|d| format!("Digit{d}")));
    keys.extend((1..=24).map(|n| format!("F{n}")));
    keys.extend(NAMED_KEYS.iter().map(|k| k.to_string()));
    for key in keys {
        let hotkey = Hotkey::new(key.clone(), [Modifier::Ctrl]);
        assert!(hotkey.is_valid(), "core rejects {key}");
        match to_chord(&hotkey) {
            Ok((_, code)) => assert_eq!(Code::from_str(&key).ok(), Some(code), "{key}"),
            Err(e) => panic!("core accepts {key} but it has no Code: {e:?}"),
        }
    }
}

#[test]
fn to_chord_rejects_hotkey_without_modifier() {
    let hotkey = Hotkey::new("KeyT", []);
    assert_eq!(to_chord(&hotkey), Err(PlatformError::InvalidHotkey));
}

#[test]
fn to_chord_rejects_unknown_key() {
    for key in ["KeyÄ", "", "Numpad9"] {
        let hotkey = Hotkey::new(key, [Modifier::Ctrl]);
        assert_eq!(
            to_chord(&hotkey),
            Err(PlatformError::InvalidHotkey),
            "{key:?}"
        );
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn to_global_hotkey_id_is_stable_and_distinct() {
    use lingopop_platform::to_global_hotkey;
    let t = Hotkey::default_translate();
    let y = Hotkey::new("KeyY", [Modifier::Ctrl, Modifier::Alt]);
    assert_eq!(
        to_global_hotkey(&t).unwrap().id(),
        to_global_hotkey(&t).unwrap().id()
    );
    assert_ne!(
        to_global_hotkey(&t).unwrap().id(),
        to_global_hotkey(&y).unwrap().id()
    );
}

#[test]
fn fake_hotkey_service_register_conflict_and_unregister() {
    let service = FakeHotkeyService::new();
    let hotkey = Hotkey::default_translate();
    let id = service.register(&hotkey).expect("first register");
    assert_eq!(
        service.register(&hotkey),
        Err(PlatformError::HotkeyConflict)
    );
    service.unregister(id).expect("unregister");
    assert!(service.register(&hotkey).is_ok());
    let other = Hotkey::new("KeyY", [Modifier::Ctrl, Modifier::Alt]);
    service.block(&other).expect("block");
    assert_eq!(service.register(&other), Err(PlatformError::HotkeyConflict));
    assert_eq!(
        service.register(&Hotkey::new("KeyT", [])),
        Err(PlatformError::InvalidHotkey)
    );
}

#[test]
fn fake_hotkey_service_fire_delivers_event() {
    let service = FakeHotkeyService::new();
    let events = service.events();
    let id = service.register(&Hotkey::default_translate()).unwrap();
    service.fire(id);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)),
        Ok(HotkeyEvent { id })
    );
}
