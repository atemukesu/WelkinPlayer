//! Playback progress storage.
//!
//! Only the last played track and its position need to survive restarts, so it
//! lives in its own tiny document instead of the (potentially large) profile.
//! The local copy is written on every tick; the remote copy is written at low
//! frequency (pause / seek / exit) to keep WebDAV traffic minimal.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::backend::{sync_backend, Backend};
use crate::error::AppError;

const LOCAL_FILE: &str = "playback.json";
const LOCAL_KEY: &str = "playback";
/// Resume document at the WebDAV collection root (ignored by the audio listing).
const REMOTE_PATH: &str = "welkin-playback.json";
/// Maximum resume document size accepted from the server, as a sanity bound.
const MAX_PLAYBACK_BYTES: usize = 64 * 1024;

/// Persisted resume point.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    /// Remote path of the track that was playing last.
    pub path: String,
    /// Playback position (seconds) within `path`.
    pub position: f64,
    /// Last modification time (ms since epoch).
    #[serde(default)]
    pub updated_at: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn local_get(app: &AppHandle) -> Option<PlaybackState> {
    let raw = app
        .store(LOCAL_FILE)
        .ok()
        .and_then(|store| store.get(LOCAL_KEY))
        .and_then(|value| value.as_str().map(str::to_string))?;
    serde_json::from_str(&raw).ok()
}

fn local_set(app: &AppHandle, state: &PlaybackState) -> Result<(), AppError> {
    let store = app.store(LOCAL_FILE)?;
    store.set(LOCAL_KEY, serde_json::to_string(state)?);
    store.save()?;
    Ok(())
}

/// Build the sync source backend, or `None` when no sync source is configured.
fn client(app: &AppHandle) -> Result<Option<Backend>, AppError> {
    sync_backend(app)
}

/// Read the locally cached resume point instantly, without touching the
/// network. Used at startup so the UI can resume before the remote copy lands.
#[tauri::command]
pub fn load_local_playback(app: AppHandle) -> Result<Option<PlaybackState>, AppError> {
    Ok(local_get(&app))
}

/// Fetch the resume point from WebDAV, caching it locally. Falls back to the
/// local copy when the server is unreachable or has no document yet.
#[tauri::command]
pub async fn load_remote_playback(app: AppHandle) -> Result<Option<PlaybackState>, AppError> {
    let local = local_get(&app);

    let client = match client(&app) {
        Ok(client) => client,
        Err(error) => {
            // A broken keychain / invalid URL must not block local resuming.
            log::warn!("failed to build WebDAV client for playback: {error}");
            return Ok(local);
        }
    };
    let Some(client) = client else {
        return Ok(local);
    };

    match client.read_text(REMOTE_PATH).await {
        Ok(text) => {
            if text.len() > MAX_PLAYBACK_BYTES {
                log::warn!("remote playback state too large, ignoring");
                return Ok(local);
            }
            match serde_json::from_str::<PlaybackState>(&text) {
                Ok(state) => {
                    // Local progress is authoritative when it is at least as
                    // recent; only adopt (and cache) a genuinely newer remote.
                    if let Some(local) = local.filter(|local| local.updated_at >= state.updated_at)
                    {
                        return Ok(Some(local));
                    }
                    if let Err(error) = local_set(&app, &state) {
                        log::warn!("failed to cache playback state locally: {error}");
                    }
                    Ok(Some(state))
                }
                Err(error) => {
                    log::warn!("failed to parse remote playback state: {error}");
                    Ok(local)
                }
            }
        }
        Err(AppError::WebdavNotFound(_)) => Ok(local),
        Err(error) => {
            log::warn!("failed to read remote playback state: {error}");
            Ok(local)
        }
    }
}

/// Persist the resume point. The local copy is always written; the remote copy
/// only when `remote` is true. Remote failures are logged but never surfaced:
/// a lost progress tick must not interrupt playback.
#[tauri::command]
pub async fn save_playback(
    app: AppHandle,
    path: String,
    position: f64,
    remote: bool,
) -> Result<(), AppError> {
    let state = PlaybackState {
        path,
        position: position.max(0.0),
        updated_at: now_ms(),
    };

    local_set(&app, &state)?;

    if !remote {
        return Ok(());
    }

    let Some(client) = client(&app)? else {
        return Ok(());
    };

    let body = serde_json::to_vec(&state)?;
    if let Err(error) = client.put(REMOTE_PATH, "application/json", body).await {
        log::warn!("failed to write remote playback state: {error}");
    }
    Ok(())
}
