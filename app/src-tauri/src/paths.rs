//! Where the app keeps its files. The spec fixes `%APPDATA%\LingoPop` (NOT Tauri's
//! `app_data_dir()`, which would use the bundle identifier).

use std::ffi::OsString;
use std::path::PathBuf;

const FALLBACK_DIR: &str = "LingoPop-data";

/// The data directory for the running OS: `%APPDATA%\LingoPop` on Windows,
/// `~/Library/Application Support/LingoPop-dev` on macOS (dev only). When the environment
/// variable is missing the current directory's `LingoPop-data` is used (the caller logs it).
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA");
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME");
    data_dir_from(base)
}

/// Pure core of [`data_dir`]: `base` is `APPDATA` on Windows and `HOME` elsewhere.
pub fn data_dir_from(base: Option<OsString>) -> PathBuf {
    match base.filter(|b| !b.is_empty()) {
        #[cfg(windows)]
        Some(appdata) => PathBuf::from(appdata).join("LingoPop"),
        #[cfg(not(windows))]
        Some(home) => PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("LingoPop-dev"),
        None => PathBuf::from(FALLBACK_DIR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_ends_with_app_folder() {
        let dir = data_dir_from(Some("base".into()));
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if cfg!(windows) {
            assert_eq!(name, "LingoPop");
        } else {
            assert_eq!(name, "LingoPop-dev");
            assert!(dir.ends_with("Library/Application Support/LingoPop-dev"));
        }
    }

    #[test]
    fn data_dir_falls_back_to_current_dir_folder() {
        assert_eq!(data_dir_from(None), PathBuf::from("LingoPop-data"));
        assert_eq!(
            data_dir_from(Some(OsString::new())),
            PathBuf::from("LingoPop-data")
        );
    }
}
