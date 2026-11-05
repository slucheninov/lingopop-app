//! The login-item outcome as the user sees it: an approval that is still pending is an error with
//! its own actionable message, never a silent success (fake `SMAppService`, every OS).

use std::path::Path;

use lingopop_orchestrator::messages::{
    platform_message, KEY_LOGIN_REQUIRES_APPROVAL, KEY_LOGIN_UNAVAILABLE,
};
use lingopop_platform::fake::{FakeLoginItem, FakeMainAppService, ServiceCall};
use lingopop_platform::macos_login::{ServiceLoginItem, ServiceStatus};
use lingopop_platform::{LoginItem, PlatformError};

#[test]
fn requires_approval_is_not_silent_success() {
    // The frozen Swift app's login data is a different seam that this item can never reach; this
    // fake stands for it and must stay untouched.
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
