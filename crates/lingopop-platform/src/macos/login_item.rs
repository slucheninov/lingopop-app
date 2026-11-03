use crate::{LoginItem, PlatformError};
use std::path::Path;

/// Dev-only no-op. The macOS app is not shipped; launch-at-login is a Windows Run key.
pub struct MacLoginItem;

impl LoginItem for MacLoginItem {
    fn set_enabled(&self, _enabled: bool, _exe: &Path) -> Result<(), PlatformError> {
        Ok(())
    }
}
