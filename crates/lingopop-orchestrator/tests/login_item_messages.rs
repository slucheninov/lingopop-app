//! The login-item outcome as the user sees it: an approval that is still pending is an error with
//! its own actionable message, never a silent success (fake `SMAppService`, every OS).

use std::path::Path;

use lingopop_orchestrator::messages::{
    platform_message, KEY_LOGIN_REQUIRES_APPROVAL, KEY_LOGIN_SERVICE_NOT_FOUND,
    KEY_LOGIN_UNAVAILABLE,
};
use lingopop_platform::fake::{FakeLoginItem, FakeMainAppService, ServiceCall};
use lingopop_platform::macos_login::{ServiceLoginItem, ServiceStatus};
use lingopop_platform::{LoginItem, PlatformError};

#[test]
fn requires_approval_is_not_silent_success() {
    // This counter only documents the seam: `MainAppService` can name the main app of the running
    // bundle and nothing else, so the item has no way to reach the frozen Swift app's login item.
    // It is not a behavioural check; the type system is what keeps the two apart.
    let swift_login_item = FakeLoginItem::new();
    let service = FakeMainAppService::new(true, ServiceStatus::NotRegistered);
    service.set_status_after_register(ServiceStatus::RequiresApproval);
    let item = ServiceLoginItem::new(service.clone());

    let result = item.set_enabled(true, Path::new("/ignored/lingopop"));

    assert_eq!(result, Err(PlatformError::LoginItemRequiresApproval));
    assert_eq!(
        platform_message(&result.unwrap_err()).key,
        KEY_LOGIN_REQUIRES_APPROVAL
    );
    assert_eq!(service.calls(), vec![ServiceCall::Register]);
    assert_eq!(swift_login_item.calls().len(), 0, "swift login item writes");
}

#[test]
fn an_unbundled_or_relocated_executable_shows_the_fixed_actionable_message() {
    let service = FakeMainAppService::new(false, ServiceStatus::NotRegistered);
    let error = ServiceLoginItem::new(service.clone())
        .set_enabled(true, Path::new("/tmp/lingopop"))
        .unwrap_err();
    assert_eq!(platform_message(&error).key, KEY_LOGIN_UNAVAILABLE);
    assert!(platform_message(&error).args.is_empty());
    assert_eq!(service.calls(), vec![]);
}

#[test]
fn a_missing_service_in_an_installed_bundle_does_not_tell_the_user_to_move_the_app() {
    let service = FakeMainAppService::new(true, ServiceStatus::NotFound);
    let error = ServiceLoginItem::new(service.clone())
        .set_enabled(true, Path::new("/ignored/lingopop"))
        .unwrap_err();
    assert_eq!(platform_message(&error).key, KEY_LOGIN_SERVICE_NOT_FOUND);
    assert_ne!(platform_message(&error).key, KEY_LOGIN_UNAVAILABLE);
    assert_eq!(service.calls(), vec![]);
}
