use std::path::{Path, PathBuf};

/// The files that live in the per-user data directory. The core never decides
/// where that directory is (`%APPDATA%\LingoPop` on Windows); callers inject it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    dir: PathBuf,
}

impl DataPaths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings_json(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    pub fn ui_json(&self) -> PathBuf {
        self.dir.join("ui.json")
    }

    pub fn history_json(&self) -> PathBuf {
        self.dir.join("history.json")
    }

    pub fn usage_json(&self) -> PathBuf {
        self.dir.join("usage.json")
    }

    pub fn secrets_dat(&self) -> PathBuf {
        self.dir.join("secrets.dat")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_paths_file_names() {
        let paths = DataPaths::new("/data/LingoPop");
        let dir = std::path::Path::new("/data/LingoPop");
        assert_eq!(paths.settings_json(), dir.join("settings.json"));
        assert_eq!(paths.history_json(), dir.join("history.json"));
        assert_eq!(paths.usage_json(), dir.join("usage.json"));
        assert_eq!(paths.secrets_dat(), dir.join("secrets.dat"));
    }
}
