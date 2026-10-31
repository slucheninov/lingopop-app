use std::time::Duration;

use lingopop_platform::fake::FakeSleeper;
use lingopop_platform::{retry_busy, PlatformError};

const D: Duration = Duration::from_millis(10);

#[test]
fn retry_busy_returns_first_success_without_sleeping() {
    let s = FakeSleeper::new();
    assert_eq!(retry_busy(10, D, &s, || Ok::<_, PlatformError>(5)), Ok(5));
    assert!(s.slept().is_empty());
}

#[test]
fn retry_busy_retries_until_success() {
    let s = FakeSleeper::new();
    let mut n = 0;
    let r = retry_busy(10, D, &s, || {
        n += 1;
        if n <= 3 {
            Err(PlatformError::ClipboardBusy)
        } else {
            Ok(n)
        }
    });
    assert_eq!(r, Ok(4));
    assert_eq!(s.slept(), vec![D; 3]);
}

#[test]
fn retry_busy_gives_up_after_attempts_with_clipboard_busy() {
    let s = FakeSleeper::new();
    let mut calls = 0;
    let r: Result<(), _> = retry_busy(10, D, &s, || {
        calls += 1;
        Err(PlatformError::ClipboardBusy)
    });
    assert_eq!(r, Err(PlatformError::ClipboardBusy));
    assert_eq!(calls, 10);
    assert_eq!(s.slept().len(), 9);
}

#[test]
fn retry_busy_does_not_retry_other_errors() {
    let s = FakeSleeper::new();
    let mut calls = 0;
    let err = PlatformError::Os {
        api: "OpenClipboard",
        code: 5,
    };
    let r: Result<(), _> = retry_busy(10, D, &s, || {
        calls += 1;
        Err(err.clone())
    });
    assert_eq!(r, Err(err));
    assert_eq!(calls, 1);
    assert!(s.slept().is_empty());
}
