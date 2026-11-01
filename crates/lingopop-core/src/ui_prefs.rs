//! Windows-only UI preferences, kept separate from the shared settings format.

use crate::{
    fsio::{atomic_write, is_blank, quarantine, read_optional, strip_bom},
    Clock, LoadOutcome, StoreError,
};
use serde::{Deserialize, Serialize};
use std::{ops::RangeInclusive, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Pl,
    Uk,
    De,
}

impl Language {
    pub const ALL: [Language; 4] = [Self::En, Self::Pl, Self::Uk, Self::De];

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
            Self::Uk => "uk",
            Self::De => "de",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Pl => "Polski",
            Self::Uk => "Українська",
            Self::De => "Deutsch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppLanguage {
    System,
    En,
    Pl,
    Uk,
    De,
}

impl AppLanguage {
    pub fn effective(self, system_preferences: &[String]) -> Language {
        match self {
            Self::System => resolve_system(system_preferences),
            Self::En => Language::En,
            Self::Pl => Language::Pl,
            Self::Uk => Language::Uk,
            Self::De => Language::De,
        }
    }

    pub fn needs_relaunch(self, launched: Language, system_preferences: &[String]) -> bool {
        self.effective(system_preferences) != launched
    }
}

pub fn resolve_system(preferences: &[String]) -> Language {
    preferences
        .iter()
        .find_map(|preference| {
            let primary = preference.split(['-', '_']).next().unwrap_or_default();
            Language::ALL
                .into_iter()
                .find(|language| primary.eq_ignore_ascii_case(language.code()))
        })
        .unwrap_or(Language::En)
}

pub const POPUP_FONT_RANGE: RangeInclusive<u32> = 11..=20;
pub const DEFAULT_POPUP_FONT_SIZE: u32 = 13;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPrefs {
    pub popup_font_size: u32,
    pub app_language: AppLanguage,
    pub latest_version: Option<String>,
    /// Unix seconds, unlike the Apple-reference timestamps in history and usage.
    pub last_update_check: Option<f64>,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            popup_font_size: DEFAULT_POPUP_FONT_SIZE,
            app_language: AppLanguage::System,
            latest_version: None,
            last_update_check: None,
        }
    }
}

impl UiPrefs {
    /// Missing or blank means defaults. Corrupt files are moved aside; never writes defaults.
    pub fn load(path: &Path, clock: &dyn Clock) -> Result<(UiPrefs, LoadOutcome), StoreError> {
        let Some(bytes) = read_optional(path)? else {
            return Ok((Self::default(), LoadOutcome::Missing));
        };
        if is_blank(&bytes) {
            return Ok((Self::default(), LoadOutcome::Missing));
        }
        if let Ok(serde_json::Value::Object(object)) = serde_json::from_slice(strip_bom(&bytes)) {
            fn field<T: serde::de::DeserializeOwned>(
                object: &serde_json::Map<String, serde_json::Value>,
                name: &str,
            ) -> Option<T> {
                object
                    .get(name)
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
            }
            let defaults = Self::default();
            let prefs = Self {
                popup_font_size: field::<i64>(&object, "popupFontSize").map_or(
                    defaults.popup_font_size,
                    |size| {
                        size.clamp(
                            i64::from(*POPUP_FONT_RANGE.start()),
                            i64::from(*POPUP_FONT_RANGE.end()),
                        ) as u32
                    },
                ),
                app_language: field(&object, "appLanguage").unwrap_or(defaults.app_language),
                latest_version: field(&object, "latestVersion"),
                last_update_check: field(&object, "lastUpdateCheck"),
            };
            Ok((prefs, LoadOutcome::Loaded))
        } else {
            let backup = quarantine(path, clock.now())?;
            Ok((Self::default(), LoadOutcome::CorruptBackedUp(backup)))
        }
    }

    /// Writes the clamped font size, so a file written by this build always loads back unchanged.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let stored = Self {
            popup_font_size: self
                .popup_font_size
                .clamp(*POPUP_FONT_RANGE.start(), *POPUP_FONT_RANGE.end()),
            ..self.clone()
        };
        atomic_write(path, &serde_json::to_vec_pretty(&stored)?, false)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataPaths, ManualClock, Timestamp};
    use std::fs;

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::from_unix_seconds(1_700_000_000.0))
    }
    fn read_json(text: &str) -> UiPrefs {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        fs::write(&path, text).unwrap();
        UiPrefs::load(&path, &clock()).unwrap().0
    }

    #[test]
    fn defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        assert_eq!(
            UiPrefs::load(&path, &clock()).unwrap(),
            (UiPrefs::default(), LoadOutcome::Missing)
        );
        assert!(!path.exists());
        for bytes in [b"  \n".as_slice(), b"\xef\xbb\xbf"] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(
                UiPrefs::load(&path, &clock()).unwrap(),
                (UiPrefs::default(), LoadOutcome::Missing)
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn clamps_font_size_and_defaults_unknown_language() {
        let prefs = read_json(r#"{"popupFontSize":40,"appLanguage":"fr"}"#);
        assert_eq!(prefs.popup_font_size, 20);
        assert_eq!(prefs.app_language, AppLanguage::System);
        assert_eq!(read_json(r#"{"popupFontSize":0}"#).popup_font_size, 11);
    }

    #[test]
    fn font_size_negative_and_huge_values_clamp() {
        assert_eq!(read_json(r#"{"popupFontSize":-1}"#).popup_font_size, 11);
        assert_eq!(
            read_json(r#"{"popupFontSize":5000000000}"#).popup_font_size,
            20
        );
        assert_eq!(read_json(r#"{"popupFontSize":"x"}"#).popup_font_size, 13);
        assert_eq!(read_json(r#"{"popupFontSize":1.5}"#).popup_font_size, 13);
    }

    #[test]
    fn lenient_fields_keep_the_rest() {
        let prefs = read_json(
            r#"{"popupFontSize":"big","latestVersion":"0.3.0","appLanguage":4,"lastUpdateCheck":"bad"}"#,
        );
        assert_eq!(
            prefs,
            UiPrefs {
                latest_version: Some("0.3.0".into()),
                ..UiPrefs::default()
            }
        );
    }

    #[test]
    fn corrupt_file_is_quarantined_and_not_written() {
        for text in ["[1,2", "[]", "null", "42"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ui.json");
            fs::write(&path, text).unwrap();
            let (prefs, outcome) = UiPrefs::load(&path, &clock()).unwrap();
            assert_eq!(prefs, UiPrefs::default());
            let LoadOutcome::CorruptBackedUp(backup) = outcome else {
                panic!("expected quarantine")
            };
            assert!(backup
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("ui.json.bak-"));
            assert_eq!(fs::read_to_string(backup).unwrap(), text);
            assert!(!path.exists());
        }
    }

    #[test]
    fn bom_is_accepted() {
        assert_eq!(
            read_json("\u{feff}{\"appLanguage\":\"uk\"}").app_language,
            AppLanguage::Uk
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        let prefs = UiPrefs {
            popup_font_size: 17,
            app_language: AppLanguage::Pl,
            latest_version: Some("0.3.0".into()),
            last_update_check: Some(1_700_000_000.5),
        };
        prefs.save(&path).unwrap();
        assert_eq!(
            UiPrefs::load(&path, &clock()).unwrap(),
            (prefs, LoadOutcome::Loaded)
        );
        let wire: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"popupFontSize":17,"appLanguage":"pl","latestVersion":"0.3.0","lastUpdateCheck":1_700_000_000.5})
        );
    }

    #[test]
    fn save_clamps_out_of_range_font_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        let prefs = UiPrefs {
            popup_font_size: 40,
            ..UiPrefs::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(prefs.popup_font_size, 40);
        assert_eq!(
            UiPrefs::load(&path, &clock()).unwrap().0.popup_font_size,
            20
        );
    }

    #[test]
    fn ui_prefs_live_in_ui_json_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::new(dir.path());
        assert_eq!(paths.ui_json(), dir.path().join("ui.json"));
        fs::write(paths.settings_json(), b"unchanged settings").unwrap();
        UiPrefs::default().save(&paths.ui_json()).unwrap();
        assert_eq!(
            fs::read(paths.settings_json()).unwrap(),
            b"unchanged settings"
        );
    }

    #[test]
    fn resolve_system_picks_first_supported() {
        for (input, want) in [
            (vec!["uk-UA", "en-US"], Language::Uk),
            (vec!["fr-FR"], Language::En),
            (vec![], Language::En),
            (vec!["PL"], Language::Pl),
            (vec!["de_DE"], Language::De),
            (vec!["fr", "DE-de", "uk"], Language::De),
        ] {
            assert_eq!(
                resolve_system(&input.into_iter().map(String::from).collect::<Vec<_>>()),
                want
            );
        }
        for (lang, code, name) in [
            (Language::En, "en", "English"),
            (Language::Pl, "pl", "Polski"),
            (Language::Uk, "uk", "Українська"),
            (Language::De, "de", "Deutsch"),
        ] {
            assert_eq!(lang.code(), code);
            assert_eq!(lang.native_name(), name);
            assert!(Language::ALL.contains(&lang));
            assert_eq!(serde_json::to_value(lang).unwrap(), code);
        }
    }

    #[test]
    fn needs_relaunch_compares_effective() {
        let preferences = vec!["de".into()];
        assert!(!AppLanguage::System.needs_relaunch(Language::De, &preferences));
        assert!(AppLanguage::En.needs_relaunch(Language::De, &preferences));
        for (app, lang) in [
            (AppLanguage::En, Language::En),
            (AppLanguage::Pl, Language::Pl),
            (AppLanguage::Uk, Language::Uk),
            (AppLanguage::De, Language::De),
        ] {
            assert_eq!(app.effective(&preferences), lang);
        }
    }
}
