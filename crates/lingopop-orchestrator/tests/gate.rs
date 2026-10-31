use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use lingopop_orchestrator::{GateError, GateTimeouts, PlatformGate};
use lingopop_platform::PlatformError;

fn gate(acquire_ms: u64, run_ms: u64) -> PlatformGate {
    PlatformGate::new(GateTimeouts {
        acquire: Duration::from_millis(acquire_ms),
        run: Duration::from_millis(run_ms),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_runs_job_off_the_calling_thread() {
    let here = std::thread::current().id();
    let there = gate(50, 150)
        .run(|| Ok(std::thread::current().id()))
        .await
        .unwrap();
    assert_ne!(here, there);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_returns_job_value() {
    assert_eq!(gate(50, 150).run(|| Ok(42u32)).await, Ok(42));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_maps_platform_error() {
    let r: Result<(), _> = gate(50, 150)
        .run(|| Err(PlatformError::ClipboardBusy))
        .await;
    assert_eq!(r, Err(GateError::Platform(PlatformError::ClipboardBusy)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_serializes_jobs() {
    let g = gate(1000, 1000);
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max = Arc::new(AtomicUsize::new(0));
    let job = || {
        let (f, m) = (in_flight.clone(), max.clone());
        move || {
            let now = f.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            f.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
    };
    let (a, b) = tokio::join!(g.run(job()), g.run(job()));
    assert_eq!(a, Ok(()));
    assert_eq!(b, Ok(()));
    assert_eq!(max.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_busy_when_lock_held_past_acquire_timeout() {
    let g = gate(50, 1000);
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let g1 = g.clone();
    let first = tokio::spawn(async move {
        g1.run(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(7u8)
        })
        .await
    });
    started_rx.recv().unwrap();
    assert_eq!(g.run(|| Ok(())).await, Err(GateError::Busy));
    release_tx.send(()).unwrap();
    assert_eq!(first.await.unwrap(), Ok(7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_times_out_long_job_and_stays_locked_until_it_finishes() {
    let g = gate(50, 100);
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let r = g
        .run(move || {
            release_rx.recv().unwrap();
            Ok(())
        })
        .await;
    assert_eq!(r, Err(GateError::TimedOut));
    assert_eq!(g.run(|| Ok(())).await, Err(GateError::Busy));
    release_tx.send(()).unwrap();
    let mut ok = false;
    for _ in 0..40 {
        if g.run(|| Ok(())).await == Ok(()) {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(ok, "lock was not released after the job returned");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gate_panic_is_reported_and_releases_lock() {
    let g = gate(50, 150);
    let r: Result<(), _> = g.run(|| panic!("boom")).await;
    assert_eq!(r, Err(GateError::Panicked));
    assert_eq!(g.run(|| Ok(1)).await, Ok(1));
}

#[test]
fn gate_error_display_has_no_payload() {
    assert_eq!(GateError::Busy.to_string(), "platform is busy");
    assert_eq!(GateError::TimedOut.to_string(), "platform call timed out");
    assert_eq!(GateError::Panicked.to_string(), "platform call panicked");
    let e = PlatformError::ClipboardBusy;
    assert_eq!(GateError::Platform(e.clone()).to_string(), e.to_string());
}

#[test]
fn gate_types_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}
    assert_send_sync::<PlatformGate>();
    assert_clone::<PlatformGate>();
    assert_send_sync::<GateError>();
}
