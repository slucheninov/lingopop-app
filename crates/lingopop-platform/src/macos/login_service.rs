//! Launch at login on macOS through `SMAppService.mainApp` (ServiceManagement, macOS 13+): the
//! decision logic over an injected [`MainAppService`] backend, compiled on every OS so it runs
//! on fakes. The native backend is `macos/service_management.rs`.
//!
//! No LaunchAgent plist is written and no other login data (the frozen Swift app's login item
//! included) is read or changed: the backend trait can only name the main app of the running
//! bundle. The `exe` argument of [`LoginItem::set_enabled`] is ignored.
//!
//! Status mapping (`SMAppServiceStatus`): `Enabled` after `register` = success;
//! `RequiresApproval` = [`PlatformError::LoginItemRequiresApproval`] (the user must allow the item
//! in System Settings, never a silent success); an executable that is not an installed `.app`
//! bundle (a plain binary, a Gatekeeper-translocated copy, an app run from a mounted disk
//! image) = `Unsupported(UNSUPPORTED_LOGIN_ITEM)` ("move it to Applications"); `NotFound` for a
//! bundle that is installed = `Unsupported(UNSUPPORTED_LOGIN_ITEM_NOT_FOUND)` (another message:
//! moving the app would not help); `NotRegistered` after a disable = success.

use std::path::Path;

use crate::login_item::{
    LoginItemStatus, UNSUPPORTED_LOGIN_ITEM, UNSUPPORTED_LOGIN_ITEM_NOT_FOUND,
};
use crate::{LoginItem, PlatformError};

/// `SMAppServiceStatus` (`SMAppService.h`: notRegistered 0, enabled 1, requiresApproval 2,
/// notFound 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

/// A failed `register` / `unregister`: the `NSError` code only (no domain, no description, no
/// user data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceError {
    pub code: i64,
}

/// `SMAppService.mainApp` of the running bundle.
pub trait MainAppService: Send + Sync {
    /// The running executable lives in an installed `.app` bundle that SMAppService can name.
    fn is_installed_bundle(&self) -> bool;
    fn status(&self) -> ServiceStatus;
    fn register(&self) -> Result<(), ServiceError>;
    fn unregister(&self) -> Result<(), ServiceError>;
    /// `SMAppService.openSystemSettingsLoginItems`.
    fn open_login_items_settings(&self);
}

/// True for the path of an installed app bundle: it ends in `.app`, is not one of Gatekeeper's
/// randomized read-only copies (`.../AppTranslocation/...`) and does not run from a mounted
/// volume (`/Volumes/...`, e.g. straight from the downloaded disk image): all of those vanish
/// and cannot stay registered.
pub fn is_installed_bundle_path(bundle_path: &str) -> bool {
    bundle_path.ends_with(".app")
        && !bundle_path.contains("/AppTranslocation/")
        && !bundle_path.starts_with("/Volumes/")
}

fn os_error(api: &'static str, error: ServiceError) -> PlatformError {
    PlatformError::Os {
        api,
        code: i32::try_from(error.code).unwrap_or(i32::MAX),
    }
}

/// The macOS [`LoginItem`] over a [`MainAppService`].
pub struct ServiceLoginItem<S> {
    service: S,
}

impl<S: MainAppService> ServiceLoginItem<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    fn enable(&self) -> Result<(), PlatformError> {
        if !self.service.is_installed_bundle() {
            return Err(PlatformError::Unsupported(UNSUPPORTED_LOGIN_ITEM));
        }
        match self.service.status() {
            ServiceStatus::Enabled => return Ok(()),
            // Registered already; only the user can allow it, and registering again does not.
            ServiceStatus::RequiresApproval => {
                return Err(PlatformError::LoginItemRequiresApproval)
            }
            ServiceStatus::NotFound => {
                return Err(PlatformError::Unsupported(UNSUPPORTED_LOGIN_ITEM_NOT_FOUND))
            }
            ServiceStatus::NotRegistered => {}
        }
        if let Err(error) = self.service.register() {
            // `kSMErrorAlreadyRegistered` and `kSMErrorLaunchDeniedByUser` are errors of a service
            // that is in a well-defined state: trust the status, not the error.
            return match self.service.status() {
                ServiceStatus::Enabled => Ok(()),
                ServiceStatus::RequiresApproval => Err(PlatformError::LoginItemRequiresApproval),
                _ => Err(os_error("SMAppService register", error)),
            };
        }
        match self.service.status() {
            ServiceStatus::Enabled => Ok(()),
            ServiceStatus::RequiresApproval => Err(PlatformError::LoginItemRequiresApproval),
            ServiceStatus::NotFound => {
                Err(PlatformError::Unsupported(UNSUPPORTED_LOGIN_ITEM_NOT_FOUND))
            }
            ServiceStatus::NotRegistered => Err(PlatformError::Os {
                api: "SMAppService status",
                code: 0,
            }),
        }
    }

    fn disable(&self) -> Result<(), PlatformError> {
        // Outside an installed bundle nothing of ours can be registered, and turning the switch
        // off must not report an error.
        if !self.service.is_installed_bundle() {
            return Ok(());
        }
        match self.service.status() {
            ServiceStatus::NotRegistered | ServiceStatus::NotFound => Ok(()),
            ServiceStatus::Enabled | ServiceStatus::RequiresApproval => {
                match self.service.unregister() {
                    Ok(()) => Ok(()),
                    // Already gone (`kSMErrorJobNotFound`) is what was asked for.
                    Err(_) if self.service.status() == ServiceStatus::NotRegistered => Ok(()),
                    Err(error) => Err(os_error("SMAppService unregister", error)),
                }
            }
        }
    }
}

impl<S: MainAppService> LoginItem for ServiceLoginItem<S> {
    fn set_enabled(&self, enabled: bool, _exe: &Path) -> Result<(), PlatformError> {
        if enabled {
            self.enable()
        } else {
            self.disable()
        }
    }

    /// Reads `SMAppService.mainApp.status` only: nothing is registered or unregistered.
    fn status(&self) -> LoginItemStatus {
        if !self.service.is_installed_bundle() {
            return LoginItemStatus::Unknown;
        }
        match self.service.status() {
            ServiceStatus::Enabled => LoginItemStatus::Enabled,
            ServiceStatus::RequiresApproval => LoginItemStatus::RequiresApproval,
            ServiceStatus::NotRegistered => LoginItemStatus::NotRegistered,
            ServiceStatus::NotFound => LoginItemStatus::Unknown,
        }
    }

    fn open_settings(&self) -> Result<(), PlatformError> {
        self.service.open_login_items_settings();
        Ok(())
    }
}
