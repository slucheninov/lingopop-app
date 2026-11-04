//! The permission seam: pure kinds / states, the always-granted default, the fake and the
//! permission-aware fakes of the capture and key services.

use std::sync::Arc;

use lingopop_platform::fake::{FakeCapture, FakeClipboard, FakeKeys, FakePermissions, FakeSleeper};
use lingopop_platform::permissions::{
    ensure_permission, AlwaysGranted, PermissionKind, PermissionState, Permissions,
};
use lingopop_platform::{KeySynth, PlatformError, Rect, ScreenCapture};

const KINDS: [PermissionKind; 2] = [
    PermissionKind::Accessibility,
    PermissionKind::ScreenRecording,
];

#[test]
fn always_granted_never_prompts_and_opens_nothing() {
    let p = AlwaysGranted;
    for kind in KINDS {
        assert_eq!(p.state(kind), PermissionState::Granted);
        assert_eq!(p.request(kind), Ok(PermissionState::Granted));
        assert_eq!(p.open_settings(kind), Ok(()));
    }
}

#[test]
fn fake_defaults_to_granted_and_counts_requests() {
    let p = FakePermissions::new();
    for kind in KINDS {
        assert_eq!(p.state(kind), PermissionState::Granted);
    }
    assert_eq!(p.request_calls(PermissionKind::Accessibility), 0);
    p.request(PermissionKind::Accessibility).unwrap();
    assert_eq!(p.request_calls(PermissionKind::Accessibility), 1);
    assert_eq!(p.request_calls(PermissionKind::ScreenRecording), 0);
}

#[test]
fn fake_request_applies_the_scripted_outcome() {
    let p = FakePermissions::new();
    p.set_state(
        PermissionKind::ScreenRecording,
        PermissionState::NotDetermined,
    );
    p.set_request_outcome(PermissionKind::ScreenRecording, PermissionState::Denied);
    assert_eq!(
        p.request(PermissionKind::ScreenRecording),
        Ok(PermissionState::Denied)
    );
    assert_eq!(
        p.state(PermissionKind::ScreenRecording),
        PermissionState::Denied
    );
}

#[test]
fn fake_records_open_settings_per_kind() {
    let p = FakePermissions::new();
    p.open_settings(PermissionKind::ScreenRecording).unwrap();
    assert_eq!(p.opened(), vec![PermissionKind::ScreenRecording]);
}

#[test]
fn ensure_permission_requests_only_when_not_determined() {
    for (state, outcome, expect_requests, expect_ok) in [
        (PermissionState::Granted, PermissionState::Granted, 0, true),
        (PermissionState::Denied, PermissionState::Granted, 0, false),
        (
            PermissionState::Restricted,
            PermissionState::Granted,
            0,
            false,
        ),
        (
            PermissionState::NotDetermined,
            PermissionState::Granted,
            1,
            true,
        ),
        (
            PermissionState::NotDetermined,
            PermissionState::Denied,
            1,
            false,
        ),
    ] {
        let p = FakePermissions::new();
        p.set_state(PermissionKind::Accessibility, state);
        p.set_request_outcome(PermissionKind::Accessibility, outcome);
        let result = ensure_permission(&*p, PermissionKind::Accessibility, &|_| {});
        assert_eq!(
            p.request_calls(PermissionKind::Accessibility),
            expect_requests
        );
        assert_eq!(p.request_calls(PermissionKind::ScreenRecording), 0);
        assert_eq!(result.is_ok(), expect_ok, "{state:?} -> {outcome:?}");
        if !expect_ok {
            assert_eq!(
                result,
                Err(PlatformError::PermissionDenied(
                    PermissionKind::Accessibility
                ))
            );
        }
    }
}

#[test]
fn permission_denied_display_names_the_kind_without_payload() {
    assert_eq!(
        PlatformError::PermissionDenied(PermissionKind::Accessibility).to_string(),
        "Accessibility permission was denied"
    );
    assert_eq!(
        PlatformError::PermissionDenied(PermissionKind::ScreenRecording).to_string(),
        "Screen Recording permission was denied"
    );
}

#[test]
fn kind_and_state_wire_ids_are_stable() {
    assert_eq!(PermissionKind::Accessibility.id(), "accessibility");
    assert_eq!(PermissionKind::ScreenRecording.id(), "screenRecording");
    assert_eq!(
        PermissionKind::from_id("screenRecording"),
        Some(PermissionKind::ScreenRecording)
    );
    assert_eq!(PermissionKind::from_id("file:///etc/passwd"), None);
    assert_eq!(PermissionState::NotDetermined.id(), "notDetermined");
    assert_eq!(PermissionState::Restricted.id(), "restricted");
}

#[test]
fn fake_capture_follows_the_screen_recording_state() {
    let p = FakePermissions::new();
    p.set_state(PermissionKind::ScreenRecording, PermissionState::Denied);
    let capture = FakeCapture::new([1, 2, 3, 255]).with_permissions(p.clone());
    let region = Rect::new(0, 0, 4, 4);
    assert_eq!(
        capture.capture_region(region).unwrap_err(),
        PlatformError::PermissionDenied(PermissionKind::ScreenRecording)
    );
    p.set_state(PermissionKind::ScreenRecording, PermissionState::Granted);
    assert!(capture.capture_region(region).is_ok());
}

#[test]
fn fake_keys_follow_the_accessibility_state() {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(clip, sleeper);
    let p = FakePermissions::new();
    p.set_state(PermissionKind::Accessibility, PermissionState::Denied);
    keys.require_permission(p.clone());
    assert_eq!(
        keys.send_copy().unwrap_err(),
        PlatformError::PermissionDenied(PermissionKind::Accessibility)
    );
    assert_eq!(
        keys.send_paste().unwrap_err(),
        PlatformError::PermissionDenied(PermissionKind::Accessibility)
    );
    p.set_state(PermissionKind::Accessibility, PermissionState::Granted);
    assert!(keys.send_copy().is_ok());
}

#[test]
fn ensure_permission_logs_fixed_lines_only() {
    let lines = std::sync::Mutex::new(Vec::<String>::new());
    let log = |l: &str| lines.lock().unwrap().push(l.to_string());
    let p = FakePermissions::new();
    p.set_state(
        PermissionKind::ScreenRecording,
        PermissionState::NotDetermined,
    );
    p.set_request_outcome(PermissionKind::ScreenRecording, PermissionState::Denied);
    let _ = ensure_permission(&*p, PermissionKind::ScreenRecording, &log);
    assert_eq!(
        lines.into_inner().unwrap(),
        vec![
            "permission requested: screenRecording".to_string(),
            "permission missing: screenRecording".to_string()
        ]
    );
}
