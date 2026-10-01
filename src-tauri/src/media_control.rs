//! System media controls.
//!
//! The frontend is the single source of truth for playback; this module mirrors
//! its snapshots onto the OS transport UI and forwards transport commands back:
//!
//! * desktop — the `souvlaki` crate drives Windows SMTC, the macOS Now Playing
//!   panel / Control Center and Linux MPRIS;
//! * Android — a native `MediaSession` hosted by a foreground service (see
//!   `android.rs` / `MediaControlBridge`) renders the notification and lock
//!   screen controls.
//!
//! Commands arrive from `src/lib/mediaControl.ts`; transport events are emitted
//! to the frontend as `media:control`.

use serde::Deserialize;
use tauri::AppHandle;

use crate::error::AppError;

#[cfg(desktop)]
use serde::Serialize;

#[cfg(desktop)]
use std::time::Duration;

#[cfg(desktop)]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

#[cfg(desktop)]
use tauri::Emitter;

/// One metadata + playback snapshot pushed by the frontend.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaControlPayload {
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_ms: f64,
    #[serde(default)]
    pub position_ms: f64,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub volume: f64,
    /// Hex cache id of the cover thumbnail, used to serve artwork.
    #[serde(default)]
    pub cover_hash: Option<String>,
}

/// Transport command emitted to the frontend on `media:control`.
#[cfg(desktop)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaControlCommand {
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
}

/// Managed state holding the desktop controls handle.
#[derive(Default)]
pub struct MediaControlState {
    #[cfg(desktop)]
    controls: std::sync::Mutex<Option<MediaControls>>,
}

/// Build the OS media controls. Must run on the main thread (Windows needs its
/// message loop and macOS its `NSApplication`).
#[cfg(desktop)]
pub fn init(app: &AppHandle) {
    use tauri::Manager;

    let config = PlatformConfig {
        dbus_name: "org.welkin.Welkin",
        display_name: "Welkin",
        hwnd: platform_hwnd(app),
    };

    let mut controls = match MediaControls::new(config) {
        Ok(controls) => controls,
        Err(error) => {
            log::warn!("system media controls unavailable: {error}");
            return;
        }
    };

    let handle = app.clone();
    if let Err(error) = controls.attach(move |event| {
        if let Some(command) = map_event(event) {
            if let Err(error) = handle.emit("media:control", command) {
                log::debug!("media control emit failed: {error}");
            }
        }
    }) {
        log::warn!("failed to attach media control handler: {error}");
        return;
    }

    if let Some(state) = app.try_state::<MediaControlState>() {
        if let Ok(mut guard) = state.controls.lock() {
            *guard = Some(controls);
        }
    }
}

/// Raw `HWND` for Windows SMTC; unused on macOS/Linux.
#[cfg(all(desktop, target_os = "windows"))]
fn platform_hwnd(app: &AppHandle) -> Option<*mut std::ffi::c_void> {
    use tauri::Manager;
    let window = app.get_webview_window("main")?;
    window
        .hwnd()
        .ok()
        .map(|hwnd| hwnd.0 as *mut std::ffi::c_void)
}

#[cfg(all(desktop, not(target_os = "windows")))]
fn platform_hwnd(_app: &AppHandle) -> Option<*mut std::ffi::c_void> {
    None
}

/// Map a `souvlaki` event onto the frontend's command shape.
#[cfg(desktop)]
fn map_event(event: MediaControlEvent) -> Option<MediaControlCommand> {
    let command = match event {
        MediaControlEvent::Play => MediaControlCommand {
            action: "play",
            value: None,
        },
        MediaControlEvent::Pause => MediaControlCommand {
            action: "pause",
            value: None,
        },
        MediaControlEvent::Toggle => MediaControlCommand {
            action: "toggle",
            value: None,
        },
        MediaControlEvent::Next => MediaControlCommand {
            action: "next",
            value: None,
        },
        MediaControlEvent::Previous => MediaControlCommand {
            action: "previous",
            value: None,
        },
        MediaControlEvent::Stop => MediaControlCommand {
            action: "stop",
            value: None,
        },
        MediaControlEvent::Seek(direction) => MediaControlCommand {
            action: "seekBy",
            value: Some(seek_sign(direction) * 10.0),
        },
        MediaControlEvent::SeekBy(direction, duration) => MediaControlCommand {
            action: "seekBy",
            value: Some(seek_sign(direction) * duration.as_secs_f64()),
        },
        MediaControlEvent::SetPosition(MediaPosition(position)) => MediaControlCommand {
            action: "seek",
            value: Some(position.as_millis() as f64),
        },
        MediaControlEvent::SetVolume(volume) => MediaControlCommand {
            action: "volume",
            value: Some(volume),
        },
        _ => return None,
    };
    Some(command)
}

#[cfg(desktop)]
fn seek_sign(direction: SeekDirection) -> f64 {
    match direction {
        SeekDirection::Forward => 1.0,
        SeekDirection::Backward => -1.0,
        #[allow(unreachable_patterns)]
        _ => 1.0,
    }
}

/// Push a snapshot to the OS transport UI (wired to `media_control_update`).
#[tauri::command]
pub fn media_control_update(
    app: AppHandle,
    state: tauri::State<MediaControlState>,
    payload: MediaControlPayload,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let mut guard = state
            .controls
            .lock()
            .map_err(|_| AppError::other("media control state poisoned"))?;
        let Some(controls) = guard.as_mut() else {
            return Ok(());
        };

        if payload.title.is_empty() {
            let _ = controls.set_playback(MediaPlayback::Stopped);
            return Ok(());
        }

        let cover = desktop_cover_url(&app, &payload);
        controls
            .set_metadata(MediaMetadata {
                title: Some(payload.title.as_str()),
                artist: Some(payload.artist.as_str()),
                album: Some(payload.album.as_str()),
                cover_url: cover.as_deref(),
                duration: Some(Duration::from_millis(payload.duration_ms.max(0.0) as u64)),
            })
            .map_err(|error| AppError::other(error.to_string()))?;

        let progress = Some(MediaPosition(Duration::from_millis(
            payload.position_ms.max(0.0) as u64,
        )));
        let playback = if payload.playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        controls
            .set_playback(playback)
            .map_err(|error| AppError::other(error.to_string()))?;
        // `set_volume` only exists on the MPRIS (Linux) backend.
        #[cfg(target_os = "linux")]
        let _ = controls.set_volume((payload.volume / 100.0).clamp(0.0, 1.0));
        #[cfg(not(target_os = "linux"))]
        let _ = payload.volume;
    }

    #[cfg(target_os = "android")]
    {
        crate::android::media_control_start().map_err(AppError::other)?;
        let json = android_payload(&app, &payload)?;
        crate::android::media_control_update(&json).map_err(AppError::other)?;
    }

    let _ = &state;
    Ok(())
}

/// Update only the playhead, leaving the metadata untouched.
#[tauri::command]
pub fn media_control_position(
    state: tauri::State<MediaControlState>,
    position_ms: f64,
    playing: bool,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let mut guard = state
            .controls
            .lock()
            .map_err(|_| AppError::other("media control state poisoned"))?;
        let Some(controls) = guard.as_mut() else {
            return Ok(());
        };
        let progress = Some(MediaPosition(Duration::from_millis(
            position_ms.max(0.0) as u64
        )));
        let playback = if playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        let _ = controls.set_playback(playback);
    }

    #[cfg(target_os = "android")]
    let _ = (position_ms, playing);

    let _ = &state;
    Ok(())
}

/// Clear the transport UI when playback stops entirely.
#[tauri::command]
pub fn media_control_clear(
    app: AppHandle,
    state: tauri::State<MediaControlState>,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let _ = &app;
        if let Ok(mut guard) = state.controls.lock() {
            if let Some(controls) = guard.as_mut() {
                let _ = controls.set_playback(MediaPlayback::Stopped);
            }
        }
    }

    #[cfg(target_os = "android")]
    {
        let _ = &state;
        crate::android::media_control_stop().map_err(AppError::other)?;
    }

    let _ = &app;
    Ok(())
}

/// Android only: drain a transport action queued by the native MediaSession.
#[tauri::command]
pub fn media_control_take_control() -> Result<Option<String>, AppError> {
    #[cfg(target_os = "android")]
    return crate::android::media_control_take_control().map_err(AppError::other);
    #[cfg(not(target_os = "android"))]
    Ok(None)
}

/// Artwork URL for desktop players.
#[cfg(desktop)]
fn desktop_cover_url(app: &AppHandle, payload: &MediaControlPayload) -> Option<String> {
    let hash = valid_hash(payload.cover_hash.as_deref())?;

    #[cfg(target_os = "windows")]
    {
        // SMTC cannot read arbitrary `file://` URIs from an unpackaged app, so
        // artwork is served through the loopback proxy that is already running.
        use tauri::Manager;
        let proxy = app.try_state::<crate::proxy::StreamProxy>()?;
        Some(format!(
            "http://127.0.0.1:{}/cover?token={}&hash={}",
            proxy.port, proxy.token, hash
        ))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let path = crate::commands::media::cover_path_for(
            &crate::commands::media::resolve_cache_dir(app),
            hash,
        )?;
        Some(format!("file://{path}"))
    }
}

/// Serialize the snapshot for the Android bridge, inlining the cover bytes.
#[cfg(target_os = "android")]
fn android_payload(app: &AppHandle, payload: &MediaControlPayload) -> Result<String, AppError> {
    use base64::Engine;

    let mut value = serde_json::to_value(payload)?;
    if let Some(hash) = valid_hash(payload.cover_hash.as_deref()) {
        let file = crate::commands::media::cover_file(
            &crate::commands::media::resolve_cache_dir(app),
            hash,
        );
        if let Ok(bytes) = std::fs::read(file) {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            value["coverBase64"] = serde_json::Value::String(encoded);
        }
    }
    Ok(value.to_string())
}

/// Validate a cache cover id (lowercase/upper hex, non-empty).
fn valid_hash(hash: Option<&str>) -> Option<&str> {
    hash.filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_hexdigit()))
}
