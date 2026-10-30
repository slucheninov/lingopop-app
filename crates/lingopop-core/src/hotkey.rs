//! Global hotkey model. `key` is a W3C `KeyboardEvent.code` value.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Win,
}

impl Modifier {
    fn label(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Win => "Win",
        }
    }
}

/// Supported keys that are not letters, digits or function keys.
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

/// `KeyA`-`KeyZ`, `Digit0`-`Digit9`, `F1`-`F24` and `NAMED_KEYS`.
fn is_supported_key(key: &str) -> bool {
    if let Some(letter) = key.strip_prefix("Key") {
        return matches!(letter.as_bytes(), [b'A'..=b'Z']);
    }
    if let Some(digit) = key.strip_prefix("Digit") {
        return matches!(digit.as_bytes(), [b'0'..=b'9']);
    }
    if let Some(number) = key.strip_prefix('F') {
        let plain = number.bytes().all(|b| b.is_ascii_digit()) && !number.starts_with('0');
        return plain && number.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n));
    }
    NAMED_KEYS.contains(&key)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub key: String,
    pub modifiers: Vec<Modifier>,
}

impl Hotkey {
    /// Builds a hotkey with modifiers deduplicated and ordered ctrl, alt, shift, win.
    pub fn new(key: impl Into<String>, modifiers: impl IntoIterator<Item = Modifier>) -> Self {
        let mut modifiers: Vec<Modifier> = modifiers.into_iter().collect();
        modifiers.sort_unstable();
        modifiers.dedup();
        Self {
            key: key.into(),
            modifiers,
        }
    }

    /// A supported key and at least one modifier.
    pub fn is_valid(&self) -> bool {
        is_supported_key(&self.key) && !self.modifiers.is_empty()
    }

    /// `Ctrl+Alt+T`: `KeyX` shows `X`, `DigitN` shows `N`, other keys as is.
    pub fn display_string(&self) -> String {
        let mut modifiers = self.modifiers.clone();
        modifiers.sort_unstable();
        modifiers.dedup();
        let key = self
            .key
            .strip_prefix("Key")
            .filter(|rest| rest.len() == 1)
            .or_else(|| {
                self.key
                    .strip_prefix("Digit")
                    .filter(|rest| rest.len() == 1)
            })
            .unwrap_or(&self.key);
        let mut parts: Vec<&str> = modifiers.into_iter().map(Modifier::label).collect();
        parts.push(key);
        parts.join("+")
    }

    /// Ctrl+Alt+T. Ctrl+Alt+letter is AltGr+letter on many layouts (Polish,
    /// German, ...), so defaults avoid letters AltGr maps to a character there
    /// (A C E L N O S X Z); T is not one of them.
    pub fn default_translate() -> Self {
        Self::new("KeyT", [Modifier::Ctrl, Modifier::Alt])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::AIOperation;
    use crate::settings::Settings;

    #[test]
    fn serde_shape() {
        let hotkey = Hotkey::new("KeyT", [Modifier::Ctrl, Modifier::Alt]);
        let json = serde_json::to_string(&hotkey).unwrap();
        assert_eq!(json, r#"{"key":"KeyT","modifiers":["ctrl","alt"]}"#);
        let back: Hotkey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hotkey);
        let all = serde_json::to_string(&[
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
            Modifier::Win,
        ])
        .unwrap();
        assert_eq!(all, r#"["ctrl","alt","shift","win"]"#);
    }

    #[test]
    fn new_normalizes_modifiers() {
        let hotkey = Hotkey::new("KeyT", [Modifier::Win, Modifier::Ctrl, Modifier::Ctrl]);
        assert_eq!(hotkey.modifiers, vec![Modifier::Ctrl, Modifier::Win]);
        let hotkey = Hotkey::new(
            "F5",
            [
                Modifier::Shift,
                Modifier::Alt,
                Modifier::Ctrl,
                Modifier::Win,
            ],
        );
        assert_eq!(
            hotkey.modifiers,
            vec![
                Modifier::Ctrl,
                Modifier::Alt,
                Modifier::Shift,
                Modifier::Win
            ]
        );
    }

    #[test]
    fn is_valid() {
        assert!(Hotkey::new("KeyT", [Modifier::Ctrl]).is_valid());
        assert!(!Hotkey::new("t", [Modifier::Ctrl]).is_valid());
        assert!(!Hotkey::new("", [Modifier::Ctrl]).is_valid());
        assert!(!Hotkey::new("KeyT", []).is_valid());
        assert!(Hotkey::new("F5", [Modifier::Shift]).is_valid());
    }

    #[test]
    fn supported_keys_are_exactly_the_documented_set() {
        let mut supported: Vec<String> = Vec::new();
        supported.extend(('A'..='Z').map(|c| format!("Key{c}")));
        supported.extend((0..=9).map(|d| format!("Digit{d}")));
        supported.extend((1..=24).map(|n| format!("F{n}")));
        supported.extend(
            [
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
            ]
            .map(String::from),
        );
        for key in &supported {
            assert!(
                Hotkey::new(key.clone(), [Modifier::Ctrl]).is_valid(),
                "{key} must be valid"
            );
        }
        for key in [
            "F0",
            "F25",
            "F01",
            "F+1",
            "KeyAA",
            "Keya",
            "Key",
            "Digit10",
            "Digit",
            "space",
            "Escape",
            "Numpad1",
            "ControlLeft",
            "Key1",
            " KeyT",
            "KeyT ",
        ] {
            assert!(
                !Hotkey::new(key, [Modifier::Ctrl]).is_valid(),
                "{key:?} must be invalid"
            );
        }
    }

    #[test]
    fn display_string() {
        assert_eq!(
            Hotkey::new("KeyT", [Modifier::Ctrl, Modifier::Alt]).display_string(),
            "Ctrl+Alt+T"
        );
        assert_eq!(
            Hotkey::new("Digit1", [Modifier::Shift]).display_string(),
            "Shift+1"
        );
        assert_eq!(
            Hotkey::new("Space", [Modifier::Ctrl, Modifier::Alt]).display_string(),
            "Ctrl+Alt+Space"
        );
        assert_eq!(
            Hotkey::new("F5", [Modifier::Win, Modifier::Shift]).display_string(),
            "Shift+Win+F5"
        );
        assert_eq!(
            Hotkey::new("ArrowUp", [Modifier::Ctrl]).display_string(),
            "Ctrl+ArrowUp"
        );
    }

    #[test]
    fn default_translate_is_ctrl_alt_t() {
        assert_eq!(
            Hotkey::default_translate(),
            Hotkey {
                key: "KeyT".to_string(),
                modifiers: vec![Modifier::Ctrl, Modifier::Alt],
            }
        );
        assert!(Hotkey::default_translate().is_valid());
    }

    #[test]
    fn default_hotkeys_avoid_altgr_letters() {
        const ALTGR_LETTERS: [&str; 9] = [
            "KeyA", "KeyC", "KeyE", "KeyL", "KeyN", "KeyO", "KeyS", "KeyX", "KeyZ",
        ];
        let hotkeys = Settings::default().hotkeys;
        assert!(!hotkeys.is_empty());
        assert_eq!(
            hotkeys,
            [(AIOperation::Translate, Hotkey::default_translate())]
                .into_iter()
                .collect()
        );
        let mut defaults: Vec<Hotkey> = hotkeys.into_values().collect();
        defaults.push(Hotkey::default_translate());
        for hotkey in defaults {
            let ctrl_alt = hotkey.modifiers.contains(&Modifier::Ctrl)
                && hotkey.modifiers.contains(&Modifier::Alt);
            if ctrl_alt {
                assert!(
                    !ALTGR_LETTERS.contains(&hotkey.key.as_str()),
                    "{} collides with AltGr",
                    hotkey.display_string()
                );
            }
        }
    }
}
