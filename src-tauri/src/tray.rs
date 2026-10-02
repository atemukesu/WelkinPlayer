//! System tray icon and window-close behaviour (desktop only).
//!
//! The tray has no context menu: a left click restores the main window, which
//! matches the "minimise to tray" workflow without a right-click menu to
//! maintain. Whether closing the main window quits the app or hides it to the
//! tray is chosen by the user in settings (see [`set_close_to_tray`]) and
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

/// Create the tray icon. Left click restores and focuses the main window; no
/// menu is set, so right-click does nothing.
#[cfg(desktop)]
pub fn init(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let mut builder = TrayIconBuilder::new()
        .tooltip("Welkin")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
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
