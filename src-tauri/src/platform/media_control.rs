// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

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
//! Commands arrive from `src/lib/mediaControl.ts`; desktop transport events are
//! applied straight to `PlaybackContext` (no webview round-trip) so playback
//! controls keep working when the webview is backgrounded or dead.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
// `Manager` (try_state / get_webview_window) is only reached from desktop-gated
// code; on Android it would be an unused import.
#[cfg(desktop)]
use tauri::Manager;

use crate::core::error::AppError;

#[cfg(desktop)]
use std::time::Duration;

#[cfg(desktop)]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

/// One metadata + playback snapshot pushed by the frontend.
#[derive(Debug, Clone, Deserialize, Serialize)]
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

/// A position delta larger than this (milliseconds) between two steady-state
/// snapshots is treated as a seek rather than normal playback advancement, so
/// the OS timeline gets re-anchored to the new position. The engine emits a
/// state snapshot roughly every `STATE_INTERVAL_MS` (250 ms), so normal
/// advancement per snapshot is ~250 ms; 500 ms leaves a comfortable margin for
/// tick jitter while still catching any user/programmatic seek. A false
/// re-anchor during steady play is harmless — it just re-pins the OS clock to
/// the already-correct current position at rate 1, with no visible jump.
#[cfg(desktop)]
const SEEK_ANCHOR_THRESHOLD_MS: f64 = 500.0;

/// The last payload actually handed to the OS controls, used to skip redundant
/// metadata / playback writes. See `apply_payload`.
///
/// Kept in its own lock (separate from `controls`) so the cheap dedup compare
/// never serialises with the heavier souvlaki platform calls, and so unrelated
/// commands are only blocked for the duration of the platform call itself.
#[cfg(desktop)]
#[derive(Clone)]
struct CachedSnapshot {
    title: String,
    artist: String,
    album: String,
    cover_hash: Option<String>,
    /// Track length in milliseconds. Part of the metadata dedup: the first
    /// snapshot of a fresh track often lands before the decoder has reported the
    /// real length, and MPRIS carries it as `mpris:length`. Without comparing it
    /// the correction would be dropped and clients show no/zero progress.
    duration_ms: f64,
    playing: bool,
    position_ms: f64,
    /// Output volume in the frontend's 0..100 convention, or `None` when it has
    /// not been reported yet. On Linux this is mirrored to MPRIS (see
    /// `apply_payload`), where every `set_volume` re-emits a
    /// `PropertiesChanged("Volume")` signal, so it is only written on change.
    volume: Option<f64>,
}

/// Managed state holding the desktop controls handle.
#[derive(Default)]
pub struct MediaControlState {
    #[cfg(desktop)]
    controls: std::sync::Mutex<Option<MediaControls>>,
    #[cfg(desktop)]
    last: std::sync::Mutex<Option<CachedSnapshot>>,
}

/// Build the OS media controls. Must run on the main thread (Windows needs its
/// message loop and macOS its `NSApplication`).
#[cfg(desktop)]
pub fn init(app: &AppHandle) {
    let config = PlatformConfig {
        // souvlaki registers `org.mpris.MediaPlayer2.<dbus_name>` on Linux, so
        // this must be the bare player suffix — passing a full
        // `org.mpris.MediaPlayer2.…` name registers the doubled
        // `org.mpris.MediaPlayer2.org.mpris.MediaPlayer2.welkin`, which GNOME
        // Shell's media controls and `playerctl` do not surface correctly.
        dbus_name: "welkin",
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
            // Drive the Rust playback context directly instead of round-tripping
            // through the webview: the OS control must work even when the webview
            // is backgrounded or dead.
            if let Some(context) = handle.try_state::<crate::playback::context::PlaybackContext>() {
                context.apply_control(&command.action, command.value);
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
    let window = app.get_webview_window("main")?;
    window.hwnd().ok().map(|hwnd| hwnd.0)
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
        // Both `Seek` and `SeekBy` map to the frontend's relative `"seekBy"`
        // action, whose `value` is interpreted in **seconds** by
        // `PlaybackContext::apply_control`. `SetPosition` maps to the absolute
        // `"seek"` action, whose `value` is interpreted in **milliseconds** there.
        // The two units are intentional and consistent end-to-end; do not "unify"
        // them without also changing `apply_control`.
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
    _state: tauri::State<MediaControlState>,
    payload: MediaControlPayload,
) -> Result<(), AppError> {
    apply_payload(&app, payload)
}

/// Shared implementation behind `media_control_update` and the Rust-driven
/// `push_update`. Resolves the OS transport UI (desktop souvlaki or Android
/// native MediaSession) from a fully-formed payload.
///
/// Desktop dedup / anchoring
/// -------------------------
/// This runs on every progress tick (≈4 Hz from the engine state loop, plus
/// every time the webview pushes a snapshot). souvlaki re-builds the entire
/// metadata record on *every* `set_metadata` call — Windows reloads the cover
/// through `StorageFile`, macOS re-decodes it into an `NSImage`, MPRIS re-emits
/// `PropertiesChanged` — and re-sets the timeline on every `set_playback`. To
/// avoid the resulting image-queue pile-up, CPU spin and D-Bus storms we:
///
/// * only call `set_metadata` when `title`/`artist`/`album`/`cover_hash` change
///   (or the track length materially changes, so the first tick's `0` duration
///   is corrected once the decoder reports the real one);
/// * re-anchor the timeline on a play/pause transition, a track change or an
///   explicit seek, and — because souvlaki transmits the playhead *only* through
///   `set_playback` and never sets a rate — re-push the current position on every
///   tick while playing so the OS clock tracks the engine instead of freezing;
/// * compute the dedup decision outside the `controls` lock, so the only work
///   done while holding it is the (potentially blocking) platform call.
pub(crate) fn apply_payload(
    app: &AppHandle,
    payload: MediaControlPayload,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let state = match app.try_state::<MediaControlState>() {
            Some(state) => state,
            None => return Ok(()),
        };

        // --- dedup / anchor decision (lock-free) -------------------------
        // Decide what changed *before* taking the controls lock: this keeps the
        // heavy souvlaki calls from serialising unrelated media-control commands
        // (#9), and avoids re-running cover loads that the OS just did.
        //
        // `last` is recovered from poisoning instead of failing forever: a panic
        // while an earlier update held it (e.g. inside a backend call) must not
        // permanently disable the OS controls for the rest of the process.
        let mut last = state
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let meta_changed = match &*last {
            None => true,
            Some(prev) => {
                prev.title != payload.title
                    || prev.artist != payload.artist
                    || prev.album != payload.album
                    || prev.cover_hash != payload.cover_hash
                    // Sub-second differences are rounding noise between the
                    // engine and the webview; only a materially different length
                    // (e.g. 0 -> real duration on the first tick) counts.
                    || (prev.duration_ms - payload.duration_ms).abs() > 999.0
            }
        };
        // Re-anchor the OS timeline only when the play state flips, the track
        // changes, or the position jumps by more than one tick's worth (a seek).
        let anchor_position = match &*last {
            None => true,
            Some(prev) => {
                prev.playing != payload.playing
                    || meta_changed
                    || (payload.playing
                        && (payload.position_ms - prev.position_ms).abs() > SEEK_ANCHOR_THRESHOLD_MS)
            }
        };
        let playing_changed = last.as_ref().map(|p| p.playing != payload.playing).unwrap_or(true);
        // Linux MPRIS re-emits `PropertiesChanged("Volume")` on *every*
        // `set_volume`, so only write it when the value actually moved — the
        // steady-state progress tick otherwise floods the bus at ~4 Hz.
        let volume_changed = last
            .as_ref()
            .map(|p| p.volume != Some(payload.volume))
            .unwrap_or(true);
        // Record the snapshot we are about to present; this is what the next
        // tick compares against even if the controls handle is missing.
        *last = Some(CachedSnapshot {
            title: payload.title.clone(),
            artist: payload.artist.clone(),
            album: payload.album.clone(),
            cover_hash: payload.cover_hash.clone(),
            duration_ms: payload.duration_ms,
            playing: payload.playing,
            position_ms: payload.position_ms,
            volume: Some(payload.volume),
        });
        drop(last);

        // --- platform writes (guarded by the controls lock) ---------------
        let mut guard = state
            .controls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(controls) = guard.as_mut() else {
            return Ok(());
        };

        // NOTE: do NOT early-return when `payload.title` is empty. A blank title
        // here means the parse cache has no tag for this track yet (the webview
        // only fetches metadata for visible / playing tracks, so a freshly
        // switched track is often uncached on desktop). Bailing out would leave
        // the OS control frozen on the *previous* track's title and cover while
        // also skipping the progress/play-state update — exactly the "stuck on
        // the first song" desktop bug. The "nothing is loaded" case is already
        // handled by `push_update` (empty source_id/path), so the only path that
        // reaches here is a real, playing track. `resolve_meta` falls back to the
        // file name when the tag is missing, so the OS control always gets a
        // distinct string to render; the real title is picked up once cached.

        if meta_changed {
            let cover = desktop_cover_url(app, &payload);
            // A cover load failure must not abort the whole update: log it and
            // keep going so the title/progress are still reported. On Windows the
            // artwork call can fail for an uncached/odd path without the rest
            // being broken.
            if let Err(error) = controls.set_metadata(MediaMetadata {
                title: Some(payload.title.as_str()),
                artist: Some(payload.artist.as_str()),
                album: Some(payload.album.as_str()),
                cover_url: cover.as_deref(),
                duration: Some(Duration::from_millis(payload.duration_ms.max(0.0) as u64)),
            }) {
                log::warn!("media control metadata update failed: {error}");
            }
        }

        // souvlaki transmits the playhead *only* through `set_playback` on every
        // backend, and never sets a playback rate. A stored position therefore
        // freezes unless it is re-sent. Re-push the current position on every
        // tick while playing so all desktop surfaces stay accurate — the
        // engine's position is authoritative and monotonic, so this corrects the
        // SMTC timeline, the macOS Now Playing elapsed time and the MPRIS
        // `Position` property alike, instead of relying on each OS to guess.
        // While paused there is nothing to advance, so keep the cheap dedup.
        let refresh_playhead = payload.playing;

        if playing_changed || meta_changed || anchor_position || refresh_playhead {
            let progress = if anchor_position || refresh_playhead {
                Some(MediaPosition(Duration::from_millis(
                    payload.position_ms.max(0.0) as u64,
                )))
            } else {
                None
            };
            let playback = if payload.playing {
                MediaPlayback::Playing { progress }
            } else {
                MediaPlayback::Paused { progress }
            };
            if let Err(error) = controls.set_playback(playback) {
                log::warn!("media control playback update failed: {error}");
            }
        }
        // `set_volume` only exists on the MPRIS (Linux) backend. Skip it unless
        // the value changed: on Linux every call emits a `Volume`
        // `PropertiesChanged`, and this path runs on the ~4 Hz progress tick.
        #[cfg(target_os = "linux")]
        if volume_changed {
            let _ = controls.set_volume((payload.volume / 100.0).clamp(0.0, 1.0));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = volume_changed;
    }

    #[cfg(target_os = "android")]
    {
        // A `mediaPlayback` foreground service may only be started while media
        // is actually playing; starting it for a paused/restored track can be
        // rejected (and take the lyric overlay down with it). Only start it on
        // play; otherwise just refresh an already-running session.
        if payload.playing {
            crate::platform::android::media_control_start().map_err(AppError::other)?;
        }
        let json = android_payload(app, &payload)?;
        crate::platform::android::media_control_update(&json).map_err(AppError::other)?;
    }

    Ok(())
}

/// Update only the playhead, leaving the metadata untouched.
///
/// On desktop this keeps the OS timeline anchored roughly once per second (on a
/// large jump) so the position shown stays correct without re-pinning every
/// tick — re-pushing the sampled position constantly would make the SMTC/MPRIS
/// bar stutter and look like the track restarts (#2). The dedup logic in
/// `apply_payload` already covers the high-frequency path; this command is the
/// webview's occasional playhead correction and is treated the same way.
#[tauri::command]
pub fn media_control_position(
    state: tauri::State<MediaControlState>,
    position_ms: f64,
    playing: bool,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let mut last = state
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let jumped = match &*last {
            Some(prev) => {
                playing
                    && (position_ms - prev.position_ms).abs() > SEEK_ANCHOR_THRESHOLD_MS
            }
            None => true,
        };
        let playing_changed = last.as_ref().map(|p| p.playing != playing).unwrap_or(true);
        if let Some(prev) = last.as_mut() {
            prev.playing = playing;
            prev.position_ms = position_ms;
        } else {
            *last = Some(CachedSnapshot {
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                cover_hash: None,
                duration_ms: 0.0,
                playing,
                position_ms,
                // Unknown until the next full snapshot arrives, so that snapshot
                // mirrors the real volume to MPRIS. This command is
                // desktop-unused (the engine reports position via the event
                // thread), but keeps the cache type coherent.
                volume: None,
            });
        }
        drop(last);

        // Only call into souvlaki when the anchor actually moved or the play
        // state flipped; otherwise leave the OS clock advancing on its own.
        if jumped || playing_changed {
            let mut guard = state
                .controls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(controls) = guard.as_mut() else {
                return Ok(());
            };
            let progress = Some(MediaPosition(Duration::from_millis(
                position_ms.max(0.0) as u64,
            )));
            let playback = if playing {
                MediaPlayback::Playing { progress }
            } else {
                MediaPlayback::Paused { progress }
            };
            let _ = controls.set_playback(playback);
        }
    }

    #[cfg(target_os = "android")]
    crate::platform::android::media_control_position(position_ms, playing)
        .map_err(AppError::other)?;

    let _ = &state;
    Ok(())
}

/// Clear the transport UI when playback stops entirely.
///
/// Desktop: reset both the playback state *and* the metadata — souvlaki has no
/// dedicated "clear" API, so pushing empty fields is how we wipe the previously
/// shown title/artist/cover (#8). The cached snapshot is also dropped so the
/// next track starts from a clean dedup baseline. This is idempotent: repeated
/// calls (the engine event thread keeps reporting "nothing loaded" every tick
/// while idle) do nothing once the UI is already clear, instead of re-emitting
/// metadata / playback `PropertiesChanged` on the bus forever.
#[tauri::command]
pub fn media_control_clear(
    app: AppHandle,
    state: tauri::State<MediaControlState>,
) -> Result<(), AppError> {
    #[cfg(desktop)]
    {
        let _ = &app;
        clear_controls(&state);
    }

    #[cfg(target_os = "android")]
    {
        let _ = &state;
        crate::platform::android::media_control_stop().map_err(AppError::other)?;
    }

    let _ = &app;
    Ok(())
}

/// Idempotently wipe the desktop transport UI and its dedup cache.
///
/// Returns without touching the platform layer when the snapshot is already
/// cleared, so a permanently-idle player (no track loaded) does not emit a
/// steady stream of MPRIS / SMTC updates on every engine tick.
#[cfg(desktop)]
fn clear_controls(state: &MediaControlState) {
    {
        let mut last = state
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_none() {
            return;
        }
        *last = None;
    }
    if let Some(controls) = state
        .controls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        let _ = controls.set_metadata(MediaMetadata {
            title: Some(""),
            artist: Some(""),
            album: Some(""),
            cover_url: Some(""),
            duration: None,
        });
        let _ = controls.set_playback(MediaPlayback::Stopped);
    }
}

/// Android only: drain a transport action queued by the native MediaSession.
#[tauri::command]
pub fn media_control_take_control() -> Result<Option<String>, AppError> {
    #[cfg(target_os = "android")]
    return crate::platform::android::media_control_take_control().map_err(AppError::other);
    #[cfg(not(target_os = "android"))]
    Ok(None)
}

/// Artwork URL for desktop players.
///
/// `souvlaki` loads `file://` artwork itself, so the cached cover JPEG can be
/// served straight from disk on every desktop platform — no loopback HTTP proxy
/// is needed.
///
/// The URL is built from a normalised, forward-slash path rather than a blind
/// `format!("file://{path}")`, which on Windows produced
/// `file://C:\Music\cover.jpg` — not a legal URI (drive letter + back-slashes),
/// so the cover silently never appeared (#4). We normalise separators to `/`
/// and drop any verbatim `\\?\` prefix so the result is a clean file URI.
///
/// Note on the current souvlaki (0.8.3) Windows backend: it does
/// `trim_start_matches("file://")` and feeds the remainder to
/// `StorageFile::GetFileFromPathAsync`. So a *correct* `file:///C:/...` would
/// arrive as `/C:/...` (leading slash) and fail, whereas our normalised
/// `file://C:/...` → `C:/...` round-trips correctly. We keep the form that works
/// with this backend; revisiting it if souvlaki ever parses the URI properly.
#[cfg(desktop)]
fn desktop_cover_url(app: &AppHandle, payload: &MediaControlPayload) -> Option<String> {
    let hash = valid_hash(payload.cover_hash.as_deref())?;
    let path = crate::commands::media::cover_path_for(
        &crate::commands::media::resolve_cache_dir(app),
        hash,
    )?;
    let pathbuf = std::path::PathBuf::from(&path);
    // Resolve to an absolute path so the URI is unambiguous.
    let absolute = std::path::absolute(&pathbuf).unwrap_or(pathbuf);
    let mut raw = absolute.to_string_lossy().into_owned();
    // Drop the Windows verbatim prefix (`\\?\C:\...`) if present.
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        raw = stripped.to_string();
    }
    // Normalise to forward slashes: `C:\a\b` -> `C:/a/b`, `/home/x` -> `/home/x`.
    let normalised = raw.replace('\\', "/");
    Some(format!("file://{normalised}"))
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

/// Resolve a track's display metadata (title / artist / album / cover hash) from
/// its `source_id::path`, reading the on-disk parse cache when available. The
/// backend already owns this data, so media-control reporting never needs the
/// webview to forward it.
fn resolve_meta(
    app: &AppHandle,
    source_id: &str,
    path: &str,
) -> (String, String, String, Option<String>) {
    let hash = crate::commands::media::asset_hash(source_id, path);
    let dir = crate::commands::media::resolve_cache_dir(app);
    let meta = crate::commands::media::read_meta(&dir, &hash);
    let cover = if valid_hash(Some(&hash)).is_some() {
        Some(hash)
    } else {
        None
    };
    let (title, artist, album) = match &meta {
        Some(meta) => (
            meta.title.clone().unwrap_or_default(),
            meta.artist.clone().unwrap_or_default(),
            meta.album.clone().unwrap_or_default(),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    // When the parse cache has no tag yet (a freshly switched track on desktop),
    // fall back to the file name so the OS control always gets a *distinct*
    // string instead of an empty title. An empty title would otherwise make the
    // desktop control render nothing / look identical to the previous track
    // until the real metadata is cached. `apply_payload` no longer early-returns
    // on an empty title, so this only needs to be a sane placeholder. Use
    // `Path::file_name` (not `rsplit('/')`) so it works on Windows paths too —
    // `C:\Music\xxx.flac` would otherwise be shown in full (#5).
    let title = if title.is_empty() {
        std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    } else {
        title
    };
    (title, artist, album, cover)
}

/// Push the current playback to the OS media controls, deriving the display
/// metadata from `source_id::path`. Called by the Rust playback event thread so
/// reporting no longer depends on the webview being alive.
///
/// An empty `source_id`/`path` means nothing is loaded: clear the OS UI.
pub(crate) fn push_update(
    app: &AppHandle,
    source_id: &str,
    path: &str,
    duration_ms: f64,
    position_ms: f64,
    playing: bool,
    volume: f64,
) {
    if source_id.is_empty() || path.is_empty() {
        #[cfg(desktop)]
        if let Some(state) = app.try_state::<MediaControlState>() {
            clear_controls(&state);
        }
        #[cfg(target_os = "android")]
        let _ = crate::platform::android::media_control_stop();
        return;
    }
    let (title, artist, album, cover_hash) = resolve_meta(app, source_id, path);
    let payload = MediaControlPayload {
        title,
        artist,
        album,
        duration_ms,
        position_ms,
        playing,
        volume,
        cover_hash,
    };
    let _ = apply_payload(app, payload);
}
