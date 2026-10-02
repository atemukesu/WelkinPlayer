//! System tray icon and window-close behaviour (desktop only).
//!
//! A left click restores the main window; a right click opens a small menu with
//! "打开" / "退出". Whether closing the main window quits the app or hides it to
//! the tray is chosen by the user in settings (see [`set_close_to_tray`]) and
//! persisted on the frontend; this module only keeps the latest value.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

/// Whether a main-window close should hide to the tray (true) or quit (false).
#[derive(Default)]
pub struct CloseToTray(AtomicBool);

impl CloseToTray {
    #[cfg_attr(not(desktop), allow(dead_code))]
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Record the user's close-window preference. Called by the frontend on boot
/// and whenever the setting changes.
#[tauri::command]
pub fn set_close_to_tray(app: AppHandle, close_to_tray: bool) {
    if let Some(state) = app.try_state::<CloseToTray>() {
        state.0.store(close_to_tray, Ordering::Relaxed);
    }
}

/// Restore, un-minimise and focus the main window.
#[cfg(desktop)]
fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Tear the floating lyrics down and exit the process.
#[cfg(desktop)]
fn quit_app(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window("desktop-lyrics") {
        let _ = overlay.close();
    }
    app.exit(0);
}

/// Create the tray icon and its context menu.
#[cfg(desktop)]
pub fn init(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "tray-open", "打开", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "退出", true, false, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("Welkin")
        // Left click restores the window; the menu only opens on right click.
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-open" => show_main(app),
            "tray-quit" => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    // Keep the handle alive for the whole app lifetime; dropping it removes the
    // icon. Managed state also lets later code find it if needed.
    let tray = builder.build(app)?;
    app.manage(tray);
    Ok(())
}
