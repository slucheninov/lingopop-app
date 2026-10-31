//! A tiny file logger (`lingopop.log` in the data dir, one rotated generation `lingopop.log.1`
//! at 1 MB). `tauri-plugin-log` would add `fern` / `time` for what is 60 lines here.
//!
//! Callers never pass user text: platform and orchestrator messages are fixed text, counts and
//! format names. Third-party crates are limited to warnings so a debug-level HTTP client can
//! never write a request URL (which carries the text for the free Google endpoint).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

const MAX_BYTES: u64 = 1024 * 1024;
const FILE_NAME: &str = "lingopop.log";

struct FileLogger {
    path: PathBuf,
    state: Mutex<State>,
}

struct State {
    file: Option<File>,
    size: u64,
}

fn open(path: &Path) -> Option<(File, u64)> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let size = file.metadata().map_or(0, |m| m.len());
    Some((file, size))
}

impl FileLogger {
    fn new(dir: &Path) -> Self {
        let path = dir.join(FILE_NAME);
        let (file, size) = open(&path).map_or((None, 0), |(f, s)| (Some(f), s));
        Self {
            path,
            state: Mutex::new(State { file, size }),
        }
    }

    fn rotated_path(&self) -> PathBuf {
        self.path.with_file_name(format!("{FILE_NAME}.1"))
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if metadata.target().starts_with("lingopop") {
            true
        } else {
            metadata.level() <= Level::Warn
        }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let line = format!(
            "{secs} {:<5} {}: {}\n",
            record.level(),
            record.target(),
            record.args()
        );
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.size + line.len() as u64 > MAX_BYTES {
            state.file = None;
            let _ = fs::rename(&self.path, self.rotated_path());
            let (file, size) = open(&self.path).map_or((None, 0), |(f, s)| (Some(f), s));
            state.file = file;
            state.size = size;
        }
        if let Some(file) = state.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                state.size += line.len() as u64;
            }
        }
    }

    fn flush(&self) {
        if let Some(file) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .file
            .as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// Installs the logger once; a second call (or another logger) is ignored.
pub fn init(dir: &Path) {
    let level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    if log::set_boxed_logger(Box::new(FileLogger::new(dir))).is_ok() {
        log::set_max_level(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("lingopop-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let logger = FileLogger::new(&dir);
        logger.state.lock().unwrap().size = MAX_BYTES;
        logger.log(
            &Record::builder()
                .target("lingopop_test")
                .level(Level::Info)
                .args(format_args!("hello"))
                .build(),
        );
        assert!(dir.join("lingopop.log.1").exists());
        let current = fs::read_to_string(dir.join("lingopop.log")).unwrap();
        assert!(current.contains("lingopop_test: hello"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn third_party_info_is_dropped() {
        let dir = std::env::temp_dir().join(format!("lingopop-log-test2-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let logger = FileLogger::new(&dir);
        let meta = Metadata::builder()
            .target("reqwest::connect")
            .level(Level::Debug)
            .build();
        assert!(!logger.enabled(&meta));
        fs::remove_dir_all(&dir).unwrap();
    }
}
