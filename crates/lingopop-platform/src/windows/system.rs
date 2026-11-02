use crate::{PlatformError, SystemInfo};
use std::time::Duration;
use windows::{
    core::{w, PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
};
pub struct WindowsSystem;
impl SystemInfo for WindowsSystem {
    fn ui_languages(&self) -> Vec<String> {
        let (mut count, mut len) = (0, 0);
        // SAFETY: count/len are writable; null buffer requests the required UTF-16 size.
        if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len) }
            .is_err()
        {
            return vec![];
        }
        let mut buffer = vec![0u16; len as usize];
        // SAFETY: buffer has the size requested above and remains alive for the call.
        if unsafe {
            GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut len,
            )
        }
        .is_err()
        {
            return vec![];
        }
        buffer
            .split(|c| *c == 0)
            .take_while(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and live through this synchronous call.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        }
        .0 as isize;
        if result <= 32 {
            Err(PlatformError::Os {
                api: "ShellExecuteW",
                code: result as i32,
            })
        } else {
            Ok(())
        }
    }
    fn wait_for_process_exit(&self, pid: u32, timeout: Duration) -> bool {
        // SAFETY: requests synchronization only; no borrowed pointers.
        let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(h) => h,
            Err(e) => {
                return e.code() == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0)
            }
        };
        // SAFETY: handle is owned and valid until CloseHandle below; timeout is finite.
        let result = unsafe {
            WaitForSingleObject(
                handle,
                timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
            )
        };
        // SAFETY: closes our owned handle exactly once, after the wait.
        let _ = unsafe { CloseHandle(handle) };
        result == WAIT_OBJECT_0
    }
}
