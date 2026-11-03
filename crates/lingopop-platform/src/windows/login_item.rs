use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

use super::util::{os_err, wide};
use crate::{run_value, LoginItem, PlatformError};

const VALUE_NAME: &str = "LingoPop";
const DEFAULT_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub struct WindowsLoginItem {
    subkey: String,
}

impl Default for WindowsLoginItem {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsLoginItem {
    pub fn new() -> Self {
        Self::with_subkey(DEFAULT_SUBKEY)
    }

    /// `subkey` is relative to `HKEY_CURRENT_USER`. Tests pass a `Software\LingoPopTest\...` key
    /// and never the real Run key.
    #[doc(hidden)]
    pub fn with_subkey(subkey: &str) -> Self {
        Self {
            subkey: subkey.to_string(),
        }
    }

    fn create_key(&self) -> Result<OpenKey, PlatformError> {
        let subkey = wide(&self.subkey);
        let mut key = HKEY::default();
        // SAFETY: `subkey` is NUL-terminated and outlives the call. On success `OpenKey` closes
        // the handle; a predefined key such as HKEY_CURRENT_USER is not closed here.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        };
        check("RegCreateKeyExW", status)?;
        Ok(OpenKey(key))
    }
}

struct OpenKey(HKEY);

impl Drop for OpenKey {
    fn drop(&mut self) {
        if self.0.is_invalid() {
            return;
        }
        // SAFETY: this handle was opened on this thread and is closed exactly once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn check(api: &'static str, status: WIN32_ERROR) -> Result<(), PlatformError> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_err(api, status.0 as i32))
    }
}

/// UTF-16 REG_SZ bytes, including the terminating NUL (cbData counts it).
fn reg_sz(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

impl LoginItem for WindowsLoginItem {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError> {
        let key = self.create_key()?;
        let name = wide(VALUE_NAME);
        if enabled {
            let data = reg_sz(&run_value(exe));
            // SAFETY: `name` and `data` outlive the call; `key` is an open HKCU key.
            let status = unsafe {
                RegSetValueExW(
                    key.0,
                    PCWSTR(name.as_ptr()),
                    Some(0),
                    REG_SZ,
                    Some(data.as_slice()),
                )
            };
            check("RegSetValueExW", status)
        } else {
            // SAFETY: `name` is NUL-terminated and outlives the call; `key` is open.
            let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
            if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(os_err("RegDeleteValueW", status.0 as i32))
            }
        }
    }
}
