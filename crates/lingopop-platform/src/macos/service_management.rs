//! The native `SMAppService.mainApp` backend (ServiceManagement, macOS 13+) behind the portable
//! `MainAppService` seam of `login_service.rs`.
//!
//! API evidence: `objc2-service-management` 0.3.2 (`SMAppService`: `mainAppService`, `status`,
//! `registerAndReturnError:`, `unregisterAndReturnError:`, `openSystemSettingsLoginItems`),
//! checked against `ServiceManagement.framework/Headers/SMAppService.h`: every one of them is
//! `API_AVAILABLE(macos(13.0))`, the app floor. `NSBundle.mainBundle.bundlePath` is the bundle the
//! registration would name. The calls run on whatever thread the shell's blocking pool picks (no
//! main-thread requirement is documented), each inside its own autorelease pool.

use objc2::rc::autoreleasepool;
use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::macos_login::{is_installed_bundle_path, MainAppService, ServiceError, ServiceStatus};

/// `SMAppService.mainApp` of the running bundle.
pub struct SmMainApp;

fn status_of(raw: SMAppServiceStatus) -> ServiceStatus {
    match raw {
        SMAppServiceStatus::NotRegistered => ServiceStatus::NotRegistered,
        SMAppServiceStatus::Enabled => ServiceStatus::Enabled,
        SMAppServiceStatus::RequiresApproval => ServiceStatus::RequiresApproval,
        // `NotFound`, and any value a later macOS adds: no usable service.
        _ => ServiceStatus::NotFound,
    }
}

impl MainAppService for SmMainApp {
    fn is_installed_bundle(&self) -> bool {
        autoreleasepool(|pool| {
            let bundle = NSBundle::mainBundle();
            let path = bundle.bundlePath();
            // SAFETY: `to_str` borrows the string for the autorelease pool `pool`, which is still
            // open here; the text is copied out before the pool drains.
            let path = unsafe { path.to_str(pool) }.to_owned();
            is_installed_bundle_path(&path)
        })
    }

    fn status(&self) -> ServiceStatus {
        autoreleasepool(|_| {
            // SAFETY: `mainAppService` and `status` are plain getters on a class method of the
            // ServiceManagement framework (macOS 13+), no arguments.
            status_of(unsafe { SMAppService::mainAppService().status() })
        })
    }

    fn register(&self) -> Result<(), ServiceError> {
        autoreleasepool(|_| {
            // SAFETY: `registerAndReturnError:` takes an out-parameter that objc2 manages; the
            // returned NSError is retained and only its code is read.
            unsafe { SMAppService::mainAppService().registerAndReturnError() }.map_err(|error| {
                ServiceError {
                    code: error.code() as i64,
                }
            })
        })
    }

    fn unregister(&self) -> Result<(), ServiceError> {
        autoreleasepool(|_| {
            // SAFETY: as `register`, for `unregisterAndReturnError:`.
            unsafe { SMAppService::mainAppService().unregisterAndReturnError() }.map_err(|error| {
                ServiceError {
                    code: error.code() as i64,
                }
            })
        })
    }

    fn open_login_items_settings(&self) {
        autoreleasepool(|_| {
            // SAFETY: a class method with no arguments; it only asks System Settings to open the
            // Login Items pane (a fixed target chosen by the framework).
            unsafe { SMAppService::openSystemSettingsLoginItems() };
        });
    }
}
