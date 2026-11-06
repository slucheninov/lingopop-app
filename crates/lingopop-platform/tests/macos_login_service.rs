//! The SMAppService.mainApp decision logic on a fake backend (every OS). The native adapter is
//! covered by the owner-run `macos_login.rs`.

use std::path::Path;

use lingopop_platform::fake::{FakeMainAppService, ServiceCall};
use lingopop_platform::macos_login::{is_installed_bundle_path, ServiceLoginItem, ServiceStatus};
use lingopop_platform::{LoginItem, LoginItemStatus, PlatformError};

fn item(service: &FakeMainAppService) -> ServiceLoginItem<FakeMainAppService> {
    ServiceLoginItem::new(service.clone())
}

#[test]
fn login_enable_targets_main_app_and_ignores_exe() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    let item = item(&service);
    // An executable path that names neither the bundle nor anything that exists: ignored.
    let result = item.set_enabled(true, Path::new("/nonexistent/elsewhere/lingopop"));
    assert_eq!(result, Ok(()));
    assert_eq!(service.calls(), vec![ServiceCall::Register]);
    assert_eq!(service.status(), ServiceStatus::Enabled);
    // The same call with another exe does exactly the same thing.
    let other = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    assert_eq!(
        ServiceLoginItem::new(other.clone()).set_enabled(true, Path::new("")),
        Ok(())
    );
    assert_eq!(other.calls(), service.calls());
}

#[test]
fn enabling_an_enabled_item_does_not_register_again() {
    let service = FakeMainAppService::new(true, ServiceStatus::Enabled);
    assert_eq!(item(&service).set_enabled(true, Path::new("x")), Ok(()));
    assert_eq!(service.calls(), vec![]);
}

#[test]
fn requires_approval_after_register_is_not_silent_success() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.set_status_after_register(ServiceStatus::RequiresApproval);
    let result = item(&service).set_enabled(true, Path::new("x"));
    assert_eq!(result, Err(PlatformError::LoginItemRequiresApproval));
    assert_eq!(service.calls(), vec![ServiceCall::Register]);
}

#[test]
fn already_requiring_approval_does_not_register_again() {
    let service = FakeMainAppService::new(true, ServiceStatus::RequiresApproval);
    assert_eq!(
        item(&service).set_enabled(true, Path::new("x")),
        Err(PlatformError::LoginItemRequiresApproval)
    );
    assert_eq!(service.calls(), vec![]);
}

#[test]
fn a_register_error_that_left_the_service_requiring_approval_is_the_approval_error() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.fail_register(11, ServiceStatus::RequiresApproval);
    assert_eq!(
        item(&service).set_enabled(true, Path::new("x")),
        Err(PlatformError::LoginItemRequiresApproval)
    );
}

#[test]
fn a_register_error_that_left_the_service_enabled_is_success() {
    // kSMErrorAlreadyRegistered: the service is already registered.
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.fail_register(12, ServiceStatus::Enabled);
    assert_eq!(item(&service).set_enabled(true, Path::new("x")), Ok(()));
}

#[test]
fn any_other_register_error_is_an_os_error_with_its_code_only() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.fail_register(3, ServiceStatus::NotRegistered);
    assert_eq!(
        item(&service).set_enabled(true, Path::new("x")),
        Err(PlatformError::Os {
            api: "SMAppService register",
            code: 3
        })
    );
}

#[test]
fn a_service_that_is_still_not_registered_after_register_is_an_error() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.set_status_after_register(ServiceStatus::NotRegistered);
    assert_eq!(
        item(&service).set_enabled(true, Path::new("x")),
        Err(PlatformError::Os {
            api: "SMAppService status",
            code: 0
        })
    );
}

#[test]
fn unbundled_executable_returns_unsupported_and_touches_nothing() {
    let service = FakeMainAppService::new(false, ServiceStatus::NotRegistered);
    let result = item(&service).set_enabled(true, Path::new("/tmp/lingopop"));
    assert_eq!(
        result,
        Err(PlatformError::Unsupported(
            lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM
        ))
    );
    assert_eq!(service.calls(), vec![]);
    assert_eq!(service.status_reads(), 0);
}

#[test]
fn not_found_in_an_installed_bundle_is_its_own_unsupported_outcome() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotFound);
    assert_eq!(
        item(&service).set_enabled(true, Path::new("x")),
        Err(PlatformError::Unsupported(
            lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM_NOT_FOUND
        ))
    );
    assert_ne!(
        lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM_NOT_FOUND,
        lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM
    );
    assert_eq!(service.calls(), vec![]);
}

#[test]
fn disable_unregisters_an_enabled_or_pending_item() {
    for initial in [ServiceStatus::Enabled, ServiceStatus::RequiresApproval] {
        let service = FakeMainAppService::new(true, initial);
        assert_eq!(item(&service).set_enabled(false, Path::new("x")), Ok(()));
        assert_eq!(service.calls(), vec![ServiceCall::Unregister]);
        assert_eq!(service.status(), ServiceStatus::NotRegistered);
    }
}

#[test]
fn disable_of_an_unregistered_item_is_a_no_op() {
    for initial in [ServiceStatus::NotRegistered, ServiceStatus::NotFound] {
        let service = FakeMainAppService::new(true, initial);
        assert_eq!(item(&service).set_enabled(false, Path::new("x")), Ok(()));
        assert_eq!(service.calls(), vec![]);
    }
}

#[test]
fn disable_outside_a_bundle_is_a_no_op_not_an_error() {
    let service = FakeMainAppService::new(false, ServiceStatus::Enabled);
    assert_eq!(item(&service).set_enabled(false, Path::new("x")), Ok(()));
    assert_eq!(service.calls(), vec![]);
}

#[test]
fn a_failed_unregister_is_an_os_error_unless_the_item_is_already_gone() {
    let service = FakeMainAppService::new(true, ServiceStatus::Enabled);
    service.fail_unregister(6, ServiceStatus::Enabled);
    assert_eq!(
        item(&service).set_enabled(false, Path::new("x")),
        Err(PlatformError::Os {
            api: "SMAppService unregister",
            code: 6
        })
    );
    let service = FakeMainAppService::new(true, ServiceStatus::Enabled);
    service.fail_unregister(6, ServiceStatus::NotRegistered);
    assert_eq!(item(&service).set_enabled(false, Path::new("x")), Ok(()));
}

#[test]
fn open_settings_asks_the_backend_to_open_login_items() {
    let service = FakeMainAppService::new(true, ServiceStatus::RequiresApproval);
    assert_eq!(item(&service).open_settings(), Ok(()));
    assert_eq!(service.calls(), vec![ServiceCall::OpenLoginItemsSettings]);
}

#[test]
fn only_an_installed_app_bundle_path_counts() {
    assert!(is_installed_bundle_path("/Applications/LingoPop.app"));
    assert!(is_installed_bundle_path(
        "/Users/me/Applications/LingoPop.app"
    ));
    assert!(!is_installed_bundle_path("/Users/me/target/debug"));
    // Run straight from the mounted disk image: the copy disappears with the volume.
    assert!(!is_installed_bundle_path("/Volumes/LingoPop/LingoPop.app"));
    assert!(!is_installed_bundle_path(
        "/Volumes/Data/Applications/LingoPop.app"
    ));
    // The Data volume's firmlink path is not a mounted image.
    assert!(is_installed_bundle_path(
        "/System/Volumes/Data/Applications/LingoPop.app"
    ));
    assert!(!is_installed_bundle_path(""));
    // Gatekeeper's randomized read-only copy: the registration would point at a path that
    // disappears.
    assert!(!is_installed_bundle_path(
        "/private/var/folders/ab/cd/T/AppTranslocation/1A2B/d/LingoPop.app"
    ));
}

#[test]
fn status_is_read_only_and_maps_every_service_status() {
    for (raw, expected) in [
        (ServiceStatus::Enabled, LoginItemStatus::Enabled),
        (
            ServiceStatus::RequiresApproval,
            LoginItemStatus::RequiresApproval,
        ),
        (ServiceStatus::NotRegistered, LoginItemStatus::NotRegistered),
        (ServiceStatus::NotFound, LoginItemStatus::Unknown),
    ] {
        let service = FakeMainAppService::new(true, raw);
        assert_eq!(item(&service).status(), expected, "{raw:?}");
        // Reading registers and unregisters nothing, and opens nothing.
        assert_eq!(service.calls(), vec![], "{raw:?}");
    }
}

#[test]
fn status_outside_an_installed_bundle_is_unknown_and_reads_nothing() {
    let service = FakeMainAppService::new(false, ServiceStatus::Enabled);
    assert_eq!(item(&service).status(), LoginItemStatus::Unknown);
    assert_eq!(service.calls(), vec![]);
    assert_eq!(service.status_reads(), 0);
}
