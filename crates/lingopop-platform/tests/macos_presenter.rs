//! The portable macOS presentation seam on fakes (every OS): main-thread dispatch, the
//! on-main staleness check and completion-before-return. No AppKit, no clock.
use lingopop_platform::fake::{FakeMainThread, FakePopupWindow, PopupWindowCall};
use lingopop_platform::macos_presentation::{
    CurrentCheck, MacPresenter, MacWindowCommand, MacWindowOutcome,
};
use lingopop_platform::main_thread::{post_to_main, run_on_main, MainThread};
use lingopop_platform::{PlatformError, Rect, WindowHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const WINDOW: WindowHandle = WindowHandle(0x51);
fn rect() -> Rect {
    Rect::new(10, 20, 360, 100)
}
fn always() -> CurrentCheck {
    Arc::new(|| true)
}

#[test]
fn show_returns_only_after_the_main_thread_ran_it() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let presenter = MacPresenter::new(main.clone(), window.clone());
    let worker = std::thread::spawn(move || {
        presenter.execute(
            MacWindowCommand::Show {
                window: WINDOW,
                rect: rect(),
            },
            always(),
        )
    });
    main.wait_for_pending(1);
    assert!(window.calls().is_empty(), "nothing native before main runs");
    assert!(
        !worker.is_finished(),
        "the worker waits for native completion"
    );
    assert_eq!(main.run_pending(), 1);
    assert_eq!(worker.join().unwrap(), Ok(MacWindowOutcome::Applied));
    assert_eq!(
        window.calls(),
        vec![PopupWindowCall::ShowAt(WINDOW, rect())]
    );
    assert_eq!(window.main_thread_calls(), 1, "the native call ran on main");
}

#[test]
fn stale_on_main_skips_the_native_effect() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let presenter = MacPresenter::new(main.clone(), window.clone());
    let current = Arc::new(AtomicBool::new(true));
    let check = current.clone();
    let worker = std::thread::spawn(move || {
        presenter.execute(
            MacWindowCommand::Show {
                window: WINDOW,
                rect: rect(),
            },
            Arc::new(move || check.load(Ordering::SeqCst)),
        )
    });
    main.wait_for_pending(1);
    // A newer presentation (dismissal) is committed before main runs the queued Show.
    current.store(false, Ordering::SeqCst);
    main.run_pending();
    assert_eq!(worker.join().unwrap(), Ok(MacWindowOutcome::Stale));
    assert!(window.calls().is_empty());
}

#[test]
fn hide_runs_on_main_and_returns_its_result() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    window.fail_next(PlatformError::Os {
        api: "orderOut",
        code: 0,
    });
    let presenter = MacPresenter::new(main.clone(), window.clone());
    let worker = std::thread::spawn(move || {
        presenter.execute(MacWindowCommand::Hide { window: WINDOW }, always())
    });
    main.wait_for_pending(1);
    main.run_pending();
    assert_eq!(
        worker.join().unwrap(),
        Err(PlatformError::Os {
            api: "orderOut",
            code: 0
        })
    );
}

#[test]
fn on_main_runs_inline_without_posting() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let presenter = MacPresenter::new(main.clone(), window.clone());
    let outcome =
        main.enter_main(|| presenter.execute(MacWindowCommand::Hide { window: WINDOW }, always()));
    assert_eq!(outcome, Ok(MacWindowOutcome::Applied));
    assert_eq!(main.posted(), 0);
    assert_eq!(window.calls(), vec![PopupWindowCall::Hide(WINDOW)]);
}

#[test]
fn a_closed_main_loop_fails_instead_of_hanging() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let presenter = MacPresenter::new(main.clone(), window.clone());
    main.close();
    let result = presenter.execute(MacWindowCommand::Hide { window: WINDOW }, always());
    assert!(matches!(result, Err(PlatformError::Os { .. })));
    // A job queued before the loop went away is dropped, and its waiter is released.
    let main = FakeMainThread::new();
    let waiting = {
        let main = main.clone();
        std::thread::spawn(move || run_on_main(main.as_ref(), || 7))
    };
    main.wait_for_pending(1);
    main.close();
    assert!(matches!(
        waiting.join().unwrap(),
        Err(PlatformError::Os { .. })
    ));
    assert!(window.calls().is_empty());
}

#[test]
fn post_to_main_never_waits() {
    let main = FakeMainThread::new();
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    post_to_main(
        main.as_ref() as &dyn MainThread,
        Box::new(move || flag.store(true, Ordering::SeqCst)),
    )
    .unwrap();
    assert!(!ran.load(Ordering::SeqCst));
    main.run_pending();
    assert!(ran.load(Ordering::SeqCst));
}
