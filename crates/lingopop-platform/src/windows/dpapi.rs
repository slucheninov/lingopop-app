//! `SecretProtector` over DPAPI (`CryptProtectData`, CurrentUser scope, no extra entropy).

use std::ptr;

use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::{LocalFree, HLOCAL};
use ::windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use lingopop_core::secrets::{ProtectError, SecretProtector};

use super::util::win32_code;

/// Seals `secrets.dat` to the current Windows user (and machine profile). An empty payload is
/// passed to DPAPI like any other (through a one-byte dummy buffer so the input pointer is never
/// null) and round-trips to an empty payload.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsProtector;

impl WindowsProtector {
    pub fn new() -> Self {
        Self
    }
}

/// A DPAPI output blob, released with `LocalFree` on drop (so on every path).
struct OutputBlob(CRYPT_INTEGER_BLOB);

impl OutputBlob {
    fn new() -> Self {
        Self(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        })
    }

    fn to_vec(&self) -> Vec<u8> {
        if self.0.pbData.is_null() || self.0.cbData == 0 {
            return Vec::new();
        }
        // SAFETY: DPAPI succeeded, so `pbData` points to `cbData` readable bytes it allocated; they
        // stay alive until Drop frees them.
        unsafe { std::slice::from_raw_parts(self.0.pbData, self.0.cbData as usize) }.to_vec()
    }
}

impl Drop for OutputBlob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: a non-null `pbData` was allocated by DPAPI with LocalAlloc and is freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0.pbData.cast()))) };
        }
    }
}

/// Runs one DPAPI call over `input`. The message names the API and the error code only, never data.
fn run(
    api: &'static str,
    input: &[u8],
    call: impl FnOnce(*const CRYPT_INTEGER_BLOB, *mut CRYPT_INTEGER_BLOB) -> ::windows::core::Result<()>,
) -> Result<Vec<u8>, ProtectError> {
    let len = u32::try_from(input.len())
        .map_err(|_| ProtectError::Failed(format!("{api} failed: input too large")))?;
    let dummy = 0u8;
    let data = if input.is_empty() {
        &dummy as *const u8
    } else {
        input.as_ptr()
    };
    // DPAPI only reads the input blob; the `*mut` is required by the struct definition.
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: data as *mut u8,
    };
    let mut out = OutputBlob::new();
    call(&blob_in, &mut out.0)
        .map_err(|e| ProtectError::Failed(format!("{api} failed (code {})", win32_code(&e))))?;
    Ok(out.to_vec())
}

impl SecretProtector for WindowsProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        run("CryptProtectData", plain, |input, output| {
            // SAFETY: `input` points to a blob whose buffer outlives the call; `output` is a
            // zeroed blob DPAPI fills (freed by `OutputBlob`); optional pointers are null.
            unsafe {
                CryptProtectData(
                    input,
                    PCWSTR::null(),
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    output,
                )
            }
        })
    }

    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
        run("CryptUnprotectData", data, |input, output| {
            // SAFETY: as in `protect`; no description is requested, so nothing else is allocated.
            unsafe {
                CryptUnprotectData(
                    input,
                    None,
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    output,
                )
            }
        })
    }
}
