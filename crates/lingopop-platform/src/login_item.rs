//! Launch at login. Windows stores a quoted executable path under HKCU Run.
use crate::PlatformError;
use std::path::Path;

pub trait LoginItem: Send + Sync {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError>;
}

/// Registry Run value: the executable path wrapped in double quotes.
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}
