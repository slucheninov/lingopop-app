// Tray-only app: no console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "macos")]
    if lingopop_platform::clipboard_observer::dispatch_macos_clipboard_helper() {
        return;
    }
    lingopop_app::run();
}
