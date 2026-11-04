//! TCC permissions on macOS: Accessibility and Screen Recording.
//!
//! macOS exposes only a boolean for each (`AXIsProcessTrusted`, `CGPreflightScreenCaptureAccess`),
//! so "never asked" and "refused" cannot be told apart by the OS. This adapter keeps that bit in
//! process memory: `true` is `Granted`; `false` is `NotDetermined` until `request` ran for the kind
//! in this process and `Denied` afterwards. The memory is dropped whenever `Granted` is observed,
//! so a grant followed by a revocation starts over and a grant is never shadowed by a cached
//! denial. `Restricted` is not reported: no API exposes it.
//!
//! `state` only reads. `request` is the one call that may show the system prompt
//! (`AXIsProcessTrustedWithOptions` with the prompt option, `CGRequestScreenCaptureAccess`); both
//! return at once and the answer arrives later. `open_settings` opens a FIXED
//! `x-apple.systempreferences:` URL chosen here per kind (the same ones as the frozen Swift app's
//! `PermissionsManager`), so no URL ever comes from the webview.
//!
//! API availability (checked against the macOS SDK headers): `AXIsProcessTrusted` and
//! `AXIsProcessTrustedWithOptions` / `kAXTrustedCheckOptionPrompt` (HIServices `AXUIElement.h`,
//! macOS 10.9), `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` (CoreGraphics
//! `CGWindow.h`, macOS 10.15), so the macOS 13 floor is covered.

use std::collections::HashSet;
use std::process::Command;
use std::sync::{Arc, Mutex};

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::access::ScreenCaptureAccess;

use crate::error::PlatformError;
use crate::permissions::{PermissionKind, PermissionState, Permissions};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// The three OS operations, behind a seam so the state mapping is tested without touching TCC.
pub(crate) trait Tcc: Send + Sync {
    /// The OS answer: is the permission currently granted to this process. Never prompts.
    fn granted(&self, kind: PermissionKind) -> bool;
    /// Triggers the system prompt (when the OS shows one); returns the granted bit right away.
    fn prompt(&self, kind: PermissionKind) -> bool;
    fn open(&self, url: &str) -> Result<(), PlatformError>;
}

struct NativeTcc;

impl Tcc for NativeTcc {
    fn granted(&self, kind: PermissionKind) -> bool {
        match kind {
            PermissionKind::Accessibility => {
                // SAFETY: AXIsProcessTrusted takes no arguments and only reads the process's
                // TCC state.
                unsafe { AXIsProcessTrusted() != 0 }
            }
            PermissionKind::ScreenRecording => ScreenCaptureAccess.preflight(),
        }
    }

    fn prompt(&self, kind: PermissionKind) -> bool {
        match kind {
            PermissionKind::Accessibility => {
                // SAFETY: `kAXTrustedCheckOptionPrompt` is a constant CFString exported by
                // HIServices and valid for the life of the process (get rule: it is retained here).
                let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
                let options = CFDictionary::from_CFType_pairs(&[(
                    key.as_CFType(),
                    CFBoolean::true_value().as_CFType(),
                )]);
                // SAFETY: `options` is a valid CFDictionary that outlives the call; the function
                // only reads it.
                unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
            }
            PermissionKind::ScreenRecording => ScreenCaptureAccess.request(),
        }
    }

    fn open(&self, url: &str) -> Result<(), PlatformError> {
        match Command::new("/usr/bin/open").arg(url).status() {
            Ok(status) if status.success() => Ok(()),
            _ => Err(PlatformError::Os {
                api: "open",
                code: 1,
            }),
        }
    }
}

/// The fixed System Settings pane of `kind` (macOS 13+ keeps the legacy `x-apple.systempreferences`
/// scheme). No caller supplies any part of it.
pub(crate) fn settings_url(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PermissionKind::ScreenRecording => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
    }
}

/// The OS bit plus "request already ran in this process".
pub(crate) fn map_state(granted: bool, requested: bool) -> PermissionState {
    match (granted, requested) {
        (true, _) => PermissionState::Granted,
        (false, false) => PermissionState::NotDetermined,
        (false, true) => PermissionState::Denied,
    }
}

pub struct MacPermissions {
    tcc: Arc<dyn Tcc>,
    /// Kinds `request` ran for in this process and that were not granted since.
    requested: Mutex<HashSet<PermissionKind>>,
}

impl MacPermissions {
    pub fn new() -> Self {
        Self {
            tcc: Arc::new(NativeTcc),
            requested: Mutex::new(HashSet::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_tcc(tcc: Arc<dyn Tcc>) -> Self {
        Self {
            tcc,
            requested: Mutex::new(HashSet::new()),
        }
    }

    fn requested(&self) -> std::sync::MutexGuard<'_, HashSet<PermissionKind>> {
        self.requested.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The state for an OS bit read just now; observing `Granted` forgets the request.
    fn resolve(&self, kind: PermissionKind, granted: bool) -> PermissionState {
        let mut requested = self.requested();
        if granted {
            requested.remove(&kind);
        }
        map_state(granted, requested.contains(&kind))
    }
}

impl Default for MacPermissions {
    fn default() -> Self {
        Self::new()
    }
}

impl Permissions for MacPermissions {
    fn state(&self, kind: PermissionKind) -> PermissionState {
        self.resolve(kind, self.tcc.granted(kind))
    }

    fn request(&self, kind: PermissionKind) -> Result<PermissionState, PlatformError> {
        if self.tcc.granted(kind) {
            return Ok(self.resolve(kind, true));
        }
        // Remember first: the prompt is asynchronous and a second operation must not ask again.
        self.requested().insert(kind);
        let granted = self.tcc.prompt(kind);
        Ok(self.resolve(kind, granted))
    }

    fn open_settings(&self, kind: PermissionKind) -> Result<(), PlatformError> {
        self.tcc.open(settings_url(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeTcc {
        trusted: Mutex<BTreeMap<PermissionKind, bool>>,
        prompts: Mutex<Vec<PermissionKind>>,
        opened: Mutex<Vec<String>>,
        fail_open: Mutex<bool>,
    }

    impl FakeTcc {
        fn set(&self, kind: PermissionKind, granted: bool) {
            self.trusted.lock().unwrap().insert(kind, granted);
        }
    }

    impl Tcc for FakeTcc {
        fn granted(&self, kind: PermissionKind) -> bool {
            self.trusted
                .lock()
                .unwrap()
                .get(&kind)
                .copied()
                .unwrap_or(false)
        }
        fn prompt(&self, kind: PermissionKind) -> bool {
            self.prompts.lock().unwrap().push(kind);
            self.granted(kind)
        }
        fn open(&self, url: &str) -> Result<(), PlatformError> {
            if *self.fail_open.lock().unwrap() {
                return Err(PlatformError::Os {
                    api: "open",
                    code: 1,
                });
            }
            self.opened.lock().unwrap().push(url.to_string());
            Ok(())
        }
    }

    const ACC: PermissionKind = PermissionKind::Accessibility;
    const SCR: PermissionKind = PermissionKind::ScreenRecording;

    fn rig() -> (Arc<FakeTcc>, MacPermissions) {
        let tcc = Arc::new(FakeTcc::default());
        (tcc.clone(), MacPermissions::with_tcc(tcc))
    }

    #[test]
    fn map_state_table() {
        assert_eq!(map_state(true, false), PermissionState::Granted);
        assert_eq!(map_state(true, true), PermissionState::Granted);
        assert_eq!(map_state(false, false), PermissionState::NotDetermined);
        assert_eq!(map_state(false, true), PermissionState::Denied);
    }

    #[test]
    fn state_never_prompts_and_is_not_determined_until_requested() {
        let (tcc, p) = rig();
        for _ in 0..3 {
            assert_eq!(p.state(ACC), PermissionState::NotDetermined);
            assert_eq!(p.state(SCR), PermissionState::NotDetermined);
        }
        assert!(tcc.prompts.lock().unwrap().is_empty());
    }

    #[test]
    fn request_prompts_once_and_a_still_missing_grant_reads_as_denied() {
        let (tcc, p) = rig();
        assert_eq!(p.request(SCR), Ok(PermissionState::Denied));
        assert_eq!(*tcc.prompts.lock().unwrap(), vec![SCR]);
        // Only the requested kind changed.
        assert_eq!(p.state(SCR), PermissionState::Denied);
        assert_eq!(p.state(ACC), PermissionState::NotDetermined);
    }

    #[test]
    fn request_when_already_trusted_does_not_prompt() {
        let (tcc, p) = rig();
        tcc.set(ACC, true);
        assert_eq!(p.request(ACC), Ok(PermissionState::Granted));
        assert!(tcc.prompts.lock().unwrap().is_empty());
    }

    #[test]
    fn a_grant_clears_the_denial_and_a_later_revocation_starts_over() {
        let (tcc, p) = rig();
        p.request(ACC).unwrap();
        assert_eq!(p.state(ACC), PermissionState::Denied);
        // The user ticks the box in System Settings: no permanent cached denial.
        tcc.set(ACC, true);
        assert_eq!(p.state(ACC), PermissionState::Granted);
        // And removes it again: the memory was cleared when Granted was observed.
        tcc.set(ACC, false);
        assert_eq!(p.state(ACC), PermissionState::NotDetermined);
    }

    #[test]
    fn open_settings_uses_a_fixed_url_per_kind() {
        let (tcc, p) = rig();
        p.open_settings(ACC).unwrap();
        p.open_settings(SCR).unwrap();
        assert_eq!(
            *tcc.opened.lock().unwrap(),
            vec![
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
                    .to_string(),
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
                    .to_string(),
            ]
        );
        assert!(tcc.prompts.lock().unwrap().is_empty());
        *tcc.fail_open.lock().unwrap() = true;
        assert!(p.open_settings(ACC).is_err());
    }

    /// Owner-run: reads the real TCC state of the running binary. Never prompts.
    #[test]
    #[ignore = "owner-run: touches TCC"]
    fn real_state_reads_do_not_prompt() {
        let p = MacPermissions::new();
        for kind in PermissionKind::ALL {
            let state = p.state(kind);
            assert!(matches!(
                state,
                PermissionState::Granted | PermissionState::NotDetermined
            ));
            println!("{}: {state:?}", kind.id());
        }
    }
}
