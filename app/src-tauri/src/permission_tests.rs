//! Permission IPC: statuses never prompt, requests are per kind, no URL comes from the webview,
//! and the retry-after-grant hint follows the brief.

use crate::permissions_cmds::*;
use crate::settings_cmds::settings_view_parts;
use lingopop_core::ui_prefs::Language;
use lingopop_platform::fake::{FakeCapture, FakePermissions};
use lingopop_platform::{
    PermissionKind, PermissionState, Permissions, PlatformError, Rect, ScreenCapture,
};
use std::sync::Arc;

const ACC: PermissionKind = PermissionKind::Accessibility;
const SCR: PermissionKind = PermissionKind::ScreenRecording;

fn status(list: &[PermissionStatus], kind: PermissionKind) -> &PermissionStatus {
    list.iter().find(|s| s.kind == kind.id()).unwrap()
}

/// Regression anchor: reading state at bootstrap never prompts, a revoked permission is reported
/// as such and the capture fails with `PermissionDenied`, and after the grant everything works
/// again with no cached denial.
#[test]
fn bootstrap_does_not_prompt_and_revocation_is_recoverable() {
    let fake = FakePermissions::new();
    fake.set_state(SCR, PermissionState::Denied);
    let capture = FakeCapture::new([1, 2, 3, 255]).with_permissions(fake.clone());
    let hints = PermissionHints::default();

    // Bootstrap: the settings view and the permission statuses are state reads only.
    let d = tempfile::tempdir().unwrap();
    let (config, _, ui) = crate::task4_tests::rig(d.path());
    let _view = settings_view_parts(&config, &ui, &[]);
    let at_bootstrap = permission_statuses(fake.as_ref(), &hints);
    let request_calls_after_bootstrap = fake.requests().len();
    assert_eq!(status(&at_bootstrap, SCR).state, "denied");
    assert!(fake.state_reads() > 0);

    let region = Rect::new(0, 0, 8, 8);
    let denied_capture = capture.capture_region(region).map(|_| ());
    fake.set_state(SCR, PermissionState::Granted);
    let capture_after_grant = capture.capture_region(region);

    assert_eq!(request_calls_after_bootstrap, 0);
    assert_eq!(
        denied_capture,
        Err(PlatformError::PermissionDenied(
            PermissionKind::ScreenRecording
        ))
    );
    assert!(capture_after_grant.is_ok());
    assert_eq!(
        status(&permission_statuses(fake.as_ref(), &hints), SCR).state,
        "granted"
    );
    assert_eq!(fake.requests().len(), 0);
}

#[test]
fn permission_statuses_json_shape_lists_both_kinds_in_order() {
    let fake = FakePermissions::new();
    fake.set_state(ACC, PermissionState::NotDetermined);
    fake.set_state(SCR, PermissionState::Restricted);
    let list = permission_statuses(fake.as_ref(), &PermissionHints::default());
    assert_eq!(
        serde_json::to_value(&list).unwrap(),
        serde_json::json!([
            {"kind": "accessibility", "state": "notDetermined", "relaunchHint": false},
            {"kind": "screenRecording", "state": "restricted", "relaunchHint": false},
        ])
    );
}

#[test]
fn permission_default_bundle_reports_everything_granted() {
    let list = permission_statuses(
        &lingopop_platform::AlwaysGranted,
        &PermissionHints::default(),
    );
    assert!(list
        .iter()
        .all(|s| s.state == "granted" && !s.relaunch_hint));
}

#[test]
fn permission_request_prompts_only_the_named_kind() {
    let fake = FakePermissions::new();
    fake.set_state(ACC, PermissionState::NotDetermined);
    fake.set_state(SCR, PermissionState::NotDetermined);
    fake.set_request_outcome(SCR, PermissionState::Denied);
    let hints = PermissionHints::default();
    let list = request_permission(fake.as_ref(), &hints, "screenRecording", Language::En).unwrap();
    assert_eq!(fake.requests(), vec![SCR]);
    assert_eq!(status(&list, SCR).state, "denied");
    assert_eq!(status(&list, ACC).state, "notDetermined");
}

#[test]
fn permission_request_rejects_unknown_kinds_without_calling_the_platform() {
    let fake = FakePermissions::new();
    let hints = PermissionHints::default();
    for kind in ["", "Accessibility", "https://example.com", "../x", "camera"] {
        assert!(request_permission(fake.as_ref(), &hints, kind, Language::En).is_err());
        assert!(open_permission_settings(fake.as_ref(), &hints, kind, Language::En).is_err());
    }
    assert!(fake.requests().is_empty());
    assert!(fake.opened().is_empty());
}

#[test]
fn permission_open_settings_passes_only_a_kind_never_a_url() {
    let fake = FakePermissions::new();
    let hints = PermissionHints::default();
    open_permission_settings(fake.as_ref(), &hints, "accessibility", Language::En).unwrap();
    assert_eq!(fake.opened(), vec![ACC]);
}

#[test]
fn permission_open_settings_failure_is_a_catalog_key_and_sets_no_hint() {
    let fake = FakePermissions::new();
    fake.set_state(SCR, PermissionState::Denied);
    fake.fail_open_settings();
    let hints = PermissionHints::default();
    let err = open_permission_settings(fake.as_ref(), &hints, "screenRecording", Language::En)
        .unwrap_err();
    assert_eq!(err, "Couldn't open System Settings.");
    let list = permission_statuses(fake.as_ref(), &hints);
    assert!(!status(&list, SCR).relaunch_hint);
}

/// Retry after the grant: the user opened System Settings, came back, and this process still
/// does not see Screen Recording (macOS 13 needs a relaunch) -> the relaunch hint.
#[test]
fn post_grant_preflight_false_shows_relaunch_hint() {
    let fake = FakePermissions::new();
    fake.set_state(SCR, PermissionState::Denied);
    fake.set_state(ACC, PermissionState::Denied);
    let hints = PermissionHints::default();

    // Nothing opened yet: no hint, even though the permission is missing.
    assert!(!status(&permission_statuses(fake.as_ref(), &hints), SCR).relaunch_hint);

    open_permission_settings(fake.as_ref(), &hints, "screenRecording", Language::En).unwrap();
    open_permission_settings(fake.as_ref(), &hints, "accessibility", Language::En).unwrap();
    // The user returns; the preflight in this process is still false.
    let list = refresh_permissions(fake.as_ref(), &hints);
    assert!(status(&list, SCR).relaunch_hint);
    // Accessibility is picked up live by the OS: it never needs a relaunch.
    assert!(!status(&list, ACC).relaunch_hint);
    assert_eq!(status(&list, SCR).state, "denied");

    // Once this process sees the grant, the hint and the memory are gone.
    fake.set_state(SCR, PermissionState::Granted);
    let list = refresh_permissions(fake.as_ref(), &hints);
    assert!(!status(&list, SCR).relaunch_hint);
    // A later revocation does not resurrect the hint until Settings is opened again.
    fake.set_state(SCR, PermissionState::Denied);
    assert!(!status(&refresh_permissions(fake.as_ref(), &hints), SCR).relaunch_hint);
}

#[test]
fn permission_denied_and_revoked_controls_stay_actionable() {
    // After a denial `request` is still allowed (it never throws), and a grant is picked up by
    // the next read without any reset.
    let fake = FakePermissions::new();
    fake.set_state(ACC, PermissionState::Denied);
    let hints = PermissionHints::default();
    let list = request_permission(fake.as_ref(), &hints, "accessibility", Language::En).unwrap();
    assert_eq!(status(&list, ACC).state, "denied");
    fake.set_state(ACC, PermissionState::Granted);
    assert_eq!(
        status(&refresh_permissions(fake.as_ref(), &hints), ACC).state,
        "granted"
    );
}

#[test]
fn permission_settings_view_names_the_platform() {
    let d = tempfile::tempdir().unwrap();
    let (config, _, ui) = crate::task4_tests::rig(d.path());
    let v = serde_json::to_value(settings_view_parts(&config, &ui, &[])).unwrap();
    let expected = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    };
    assert_eq!(v["platform"], expected);
}

#[test]
fn permission_status_has_no_text_fields_beyond_fixed_ids() {
    let fake: Arc<dyn Permissions> = FakePermissions::new();
    let json = serde_json::to_string(&permission_statuses(
        fake.as_ref(),
        &PermissionHints::default(),
    ))
    .unwrap();
    assert!(!json.contains("http") && !json.contains("://"));
}
