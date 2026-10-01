//! Floating desktop-lyrics layer.
//!
//! The main window is the single source of truth; it ships a lyric snapshot,
//! a style document and a high-frequency playhead document here. This module
//! owns the platform-specific renderer:
//!
//! * desktop — a transparent, frameless, always-on-top [`tauri::WebviewWindow`]
//!   that loads `desktop-lyric-overlay.html` and receives the documents as
//!   Tauri events;
//! * Android — a native `WindowManager` overlay hosted by a foreground service
//!   (see `android.rs`), whose WebView is fed the exact same documents over JNI.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::AppError;

/// Last documents pushed by the frontend, replayed once the floating window's
/// webview has actually loaded (events emitted before then would be lost).
#[derive(Default)]
pub struct DesktopLyricCache {
    pub load: Mutex<Option<DesktopLyricLoadPayload>>,
    pub settings: Mutex<Option<serde_json::Value>>,
}

/// Label of the desktop floating window; shared with the frontend.
const WINDOW_LABEL: &str = "desktop-lyrics";

/// Overlay markup, shared by the desktop window and the Android WebView.
/// Bundled so Android needs no asset-server plumbing. (The desktop window loads
/// the file straight from the asset server, so this is Android-only.)
#[cfg(target_os = "android")]
const OVERLAY_HTML: &str = include_str!("../../../public/desktop-lyric-overlay.html");

/// Overlay renderer script. The desktop window loads it via the `<script src>`
/// tag (CSP forbids inline scripts); Android injects it directly because its
/// bare WebView cannot resolve the app asset path.
#[cfg(target_os = "android")]
const OVERLAY_JS: &str = include_str!("../../../public/desktop-lyric-overlay.js");

/// Marker removed before handing the markup to the Android WebView.
#[cfg(target_os = "android")]
const SCRIPT_TAG: &str = r#"<script src="/desktop-lyric-overlay.js"></script>"#;

/// One karaoke word.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopLyricWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// One flattened lyric line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopLyricLine {
    pub text: String,
    pub translation: String,
    pub roman: String,
    pub words: Vec<DesktopLyricWord>,
    #[serde(rename = "isBG")]
    pub is_bg: bool,
    pub is_duet: bool,
}

/// Full snapshot (track / lyrics / settings change).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopLyricLoadPayload {
    pub title: String,
    pub artist: String,
    pub lines: Vec<DesktopLyricLine>,
    pub settings: serde_json::Value,
}

/// Small playhead document pushed at ~30 Hz.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopLyricTickPayload {
    pub position_ms: f64,
    pub playing: bool,
    pub active_index: i64,
    pub active_indices: Vec<i64>,
    pub visible: bool,
}

/// Status returned to the settings UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopLyricStatus {
    pub supported: bool,
    pub permission_granted: bool,
    pub active: bool,
}

/// Current renderer status for this platform.
fn status(app: &AppHandle) -> DesktopLyricStatus {
    #[cfg(desktop)]
    return DesktopLyricStatus {
        supported: true,
        permission_granted: true,
        active: app.get_webview_window(WINDOW_LABEL).is_some(),
    };
    #[cfg(target_os = "android")]
    return DesktopLyricStatus {
        supported: crate::android::desktop_lyric_supported(),
        permission_granted: crate::android::desktop_lyric_has_permission(),
        active: crate::android::desktop_lyric_is_running(),
    };
}

/// Bring the floating renderer up (idempotent).
///
/// Must be `async`: synchronous commands run on the main thread, and
/// `WebviewWindowBuilder::build` blocks waiting for the main thread, which
/// would deadlock the whole app.
#[tauri::command]
pub async fn desktop_lyric_open(app: AppHandle) -> Result<DesktopLyricStatus, AppError> {
    #[cfg(desktop)]
    {
        if app.get_webview_window(WINDOW_LABEL).is_none() {
            let app_for_load = app.clone();
            let window = tauri::WebviewWindowBuilder::new(
                &app,
                WINDOW_LABEL,
                tauri::WebviewUrl::App("desktop-lyric-overlay.html".into()),
            )
            .title("Welkin Desktop Lyrics")
            .inner_size(920.0, 240.0)
            .min_inner_size(240.0, 72.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .on_page_load(move |window, payload| {
                if payload.event() != PageLoadEvent::Finished {
                    return;
                }
                let Some(cache) = app_for_load.try_state::<DesktopLyricCache>() else {
                    return;
                };
                let load = cache.load.lock().ok().and_then(|guard| guard.clone());
                let settings = cache.settings.lock().ok().and_then(|guard| guard.clone());
                if let Some(load) = load {
                    let _ = window.emit("desktop-lyric:load", &load);
                }
                if let Some(settings) = settings {
                    let _ = window.emit("desktop-lyric:settings", &settings);
                }
            })
            .build()
            .map_err(|error| AppError::other(format!("create desktop lyric window failed: {error}")))?;

            // Tell the main window when the layer goes away (closed from its own
            // button, the window manager, ...) so the enable toggle stays in sync.
            let app_for_close = app.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    if let Some(main) = app_for_close.get_webview_window("main") {
                        let _ = main.emit("desktop-lyric:closed", ());
                    }
                }
            });

            place_default(&window);
        }
    }
    #[cfg(target_os = "android")]
    {
        if crate::android::desktop_lyric_has_permission() {
            let html = OVERLAY_HTML.replace(SCRIPT_TAG, "");
            crate::android::desktop_lyric_start(&html, OVERLAY_JS).map_err(AppError::other)?;
        }
    }
    Ok(status(&app))
}

/// Tear the floating renderer down.
#[tauri::command]
pub fn desktop_lyric_close(app: AppHandle) -> Result<(), AppError> {
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.close();
    }
    #[cfg(target_os = "android")]
    {
        crate::android::desktop_lyric_stop().map_err(AppError::other)?;
    }
    Ok(())
}

/// Push a full lyric snapshot to the renderer.
#[tauri::command]
pub fn desktop_lyric_load(app: AppHandle, payload: DesktopLyricLoadPayload) -> Result<(), AppError> {
    if let Ok(mut cached) = app.state::<DesktopLyricCache>().load.lock() {
        *cached = Some(payload.clone());
    }
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.emit("desktop-lyric:load", &payload);
    }
    #[cfg(target_os = "android")]
    {
        let json = serde_json::to_string(&payload)?;
        crate::android::desktop_lyric_update("load", &json).map_err(AppError::other)?;
    }
    Ok(())
}

/// Push the high-frequency playhead document to the renderer.
#[tauri::command]
pub fn desktop_lyric_tick(app: AppHandle, payload: DesktopLyricTickPayload) -> Result<(), AppError> {
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.emit("desktop-lyric:tick", &payload);
    }
    #[cfg(target_os = "android")]
    {
        let json = serde_json::to_string(&payload)?;
        crate::android::desktop_lyric_update("tick", &json).map_err(AppError::other)?;
    }
    Ok(())
}

/// Push a live style change to the renderer (without reloading the lyrics).
#[tauri::command]
pub fn desktop_lyric_set_settings(app: AppHandle, settings: serde_json::Value) -> Result<(), AppError> {
    if let Ok(mut cached) = app.state::<DesktopLyricCache>().settings.lock() {
        *cached = Some(settings.clone());
    }
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.emit("desktop-lyric:settings", &settings);
    }
    #[cfg(target_os = "android")]
    {
        let json = serde_json::to_string(&settings)?;
        crate::android::desktop_lyric_update("settings", &json).map_err(AppError::other)?;
    }
    Ok(())
}

/// Report the current renderer status.
#[tauri::command]
pub fn desktop_lyric_status(app: AppHandle) -> Result<DesktopLyricStatus, AppError> {
    Ok(status(&app))
}

/// Android: drain a playback action requested from the overlay WebView.
///
/// The overlay cannot invoke commands, so it parks the action in the Kotlin
/// bridge; the main window polls this on a short interval.
#[tauri::command]
pub fn desktop_lyric_take_control() -> Result<Option<String>, AppError> {
    #[cfg(target_os = "android")]
    let control = crate::android::desktop_lyric_take_control().map_err(AppError::other)?;
    #[cfg(not(target_os = "android"))]
    let control: Option<String> = None;
    Ok(control)
}

/// Android only: open the system "display over other apps" settings screen.
#[tauri::command]
pub fn desktop_lyric_request_permission() -> Result<(), AppError> {
    #[cfg(target_os = "android")]
    {
        crate::android::desktop_lyric_open_settings().map_err(AppError::other)?;
    }
    Ok(())
}

/// Park a freshly created window near the bottom-centre of the primary monitor.
#[cfg(desktop)]
fn place_default(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let width = 920.0 * scale;
    let height = 240.0 * scale;
    let x = ((size.width as f64 - width) / 2.0).max(0.0) as i32;
    let y = (size.height as f64 - height - 120.0 * scale).max(0.0) as i32;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
