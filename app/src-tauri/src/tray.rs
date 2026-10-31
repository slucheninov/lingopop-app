//! The tray icon with a single "Quit LingoPop" item (the full menu is Phase 4b).

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Manager};

const QUIT_ID: &str = "quit";

pub fn build(app: &App) -> tauri::Result<()> {
    // The catalog key "Quit LingoPop" of the mac app (English pass-through until Phase 4b).
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit LingoPop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("LingoPop")
        .menu(&menu)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == QUIT_ID {
                app.exit(0);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app.app_handle())?;
    Ok(())
}
