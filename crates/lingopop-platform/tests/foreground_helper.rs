//! Helper PROCESS of the desktop smoke tests (`harness = false`, see `Cargo.toml`).
//!
//! The last-foreground tracker hooks with `WINEVENT_SKIPOWNPROCESS`, so windows of the test
//! process itself are invisible to it; the "external" windows of the smoke tests live in this
//! process instead.
//!
//! `foreground_helper --window <title>` creates a topmost `EDIT` window with that title, prints
//! `READY <hwnd>` (decimal) on stdout, pumps messages and exits when stdin reaches EOF. Without
//! `--window` (plain `cargo test` runs every test target with no arguments, or with libtest-style
//! flags) it does nothing and exits 0. On other operating systems it is an empty program.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    helper::run();
}

#[cfg(windows)]
mod helper {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, TranslateMessage, MSG,
        PM_REMOVE, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };

    fn title_from_args() -> Option<String> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--window" {
                return args.next();
            }
        }
        None
    }

    pub fn run() {
        let Some(title) = title_from_args() else {
            return;
        };
        let class: Vec<u16> = "EDIT".encode_utf16().chain([0]).collect();
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();

        // SAFETY: the window is created, pumped and destroyed on this thread only; the UTF-16
        // buffers outlive the calls; `msg` is a valid MSG buffer.
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).expect("GetModuleHandleW");
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                100,
                100,
                300,
                60,
                None,
                None,
                Some(HINSTANCE::from(module)),
                None,
            )
            .expect("CreateWindowExW(EDIT)");

            // The parent closes our stdin (or dies) to end us.
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
                flag.store(true, Ordering::SeqCst);
            });

            let mut out = std::io::stdout();
            let _ = writeln!(out, "READY {}", hwnd.0 as usize);
            let _ = out.flush();

            let mut msg = MSG::default();
            while !stop.load(Ordering::SeqCst) {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = DestroyWindow(hwnd);
        }
    }
}
