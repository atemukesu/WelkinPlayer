// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Cross-device profile storage.
//!
//! The whole user profile (nickname, play counts, favorites and playlists)
//! lives in a single JSON document so reading it costs exactly one `GET`
//! instead of a request per concern. Client-specific preferences (theme,
//! accent, locale, lyric typography, …) are intentionally not part of this
//! document. WebDAV is the authoritative copy; a local copy (`profile.json`)
//! is kept as a fallback for offline use and for installs without a
//! configured server.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::core::error::AppError;
use crate::data::backend::{sync_backend, Backend};

const LOCAL_FILE: &str = "profile.json";
const LOCAL_KEY: &str = "profile";
/// Profile document at the source root (ignored by the audio listing).
const REMOTE_PATH: &str = "welkin-profile.json";
/// Maximum profile size (1 MiB) accepted from the server, as a sanity bound.
const MAX_PROFILE_BYTES: usize = 1024 * 1024;

/// Result of loading a profile.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedProfile {
    /// Raw JSON document, or `None` when nothing was stored yet.
    pub content: Option<String>,
    /// Where the document came from: `remote`, `local` or `none`.
    pub source: String,
    /// Non-fatal reason the remote copy could not be used, if any.
    pub remote_error: Option<String>,
    /// Whether the server was reachable but has no profile document yet.
    pub remote_missing: bool,
    /// Opaque revision token (`etag:…` or `hash:…`) for the remote document.
    pub revision: Option<String>,
}

/// Result of persisting a profile.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileResult {
    /// Whether the remote (authoritative) copy was written successfully.
    pub remote: bool,
    /// Local write is always attempted; false when it failed too.
    pub local: bool,
    /// Human-readable reason the remote write failed.
    pub warning: Option<String>,
    /// Whether the remote copy changed underneath us (HTTP 412).
    pub conflict: bool,
    /// Revision token for the document we just wrote.
    pub revision: Option<String>,
}

/// Read the nickname from the cached local profile document, if any.
///
/// The authoritative copy may live on WebDAV; this only reads the local cache
/// that [`load_remote_profile`] / [`save_profile`] keep in sync, so callers can
/// stay synchronous and offline.
pub(crate) fn local_nickname(app: &AppHandle) -> Option<String> {
    let raw = local_get(app)?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("nickname")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn local_get(app: &AppHandle) -> Option<String> {
    app.store(LOCAL_FILE)
        .ok()
        .and_then(|store| store.get(LOCAL_KEY))
        .and_then(|value| value.as_str().map(str::to_string))
}

fn local_set(app: &AppHandle, value: &str) -> Result<(), AppError> {
    let store = app.store(LOCAL_FILE)?;
    store.set(LOCAL_KEY, value);
    store.save()?;
    Ok(())
}

/// Build the sync source backend, or `None` when no sync source is configured.
fn client(app: &AppHandle) -> Result<Option<Backend>, AppError> {
    sync_backend(app)
}

/// Read the local cache instantly, without touching the network. Used at
/// startup so the UI can render before the authoritative remote copy arrives.
#[tauri::command]
pub fn load_local_profile(app: AppHandle) -> Result<LoadedProfile, AppError> {
    let local = local_get(&app);
    let source = if local.is_some() { "local" } else { "none" };
    Ok(LoadedProfile {
        content: local,
        source: source.to_string(),
        remote_error: None,
        remote_missing: false,
        revision: None,
    })
}

/// Fetch the authoritative remote profile, falling back to the local cache when
/// the server is unreachable or has no document yet.
#[tauri::command]
pub async fn load_remote_profile(app: AppHandle) -> Result<LoadedProfile, AppError> {
    let local = local_get(&app);

    let client = match client(&app) {
        Ok(client) => client,
        Err(error) => {
            // A broken keychain / invalid URL must not block local usage.
            return Ok(LoadedProfile {
                content: local,
                source: "local".to_string(),
                remote_error: Some(error.to_string()),
                remote_missing: false,
                revision: None,
            });
        }
    };

    let Some(client) = client else {
        let source = if local.is_some() { "local" } else { "none" };
        return Ok(LoadedProfile {
            source: source.to_string(),
            content: local,
            remote_error: None,
            remote_missing: false,
            revision: None,
        });
    };

    match client.read_text_with_revision(REMOTE_PATH).await {
        Ok((text, revision)) => {
            if text.len() > MAX_PROFILE_BYTES {
                return Ok(LoadedProfile {
                    content: local,
                    source: "local".to_string(),
                    remote_error: Some("远程配置过大，已忽略".to_string()),
                    remote_missing: false,
                    revision: None,
                });
            }
            if text.trim().is_empty() {
                let source = if local.is_some() { "local" } else { "none" };
                return Ok(LoadedProfile {
                    source: source.to_string(),
                    content: local,
                    remote_error: None,
                    remote_missing: true,
                    revision: None,
                });
            }
            // Keep the local cache in sync with the authoritative copy.
            if let Err(error) = local_set(&app, &text) {
                log::warn!("failed to cache profile locally: {error}");
            }
            Ok(LoadedProfile {
                content: Some(text),
                source: "remote".to_string(),
                remote_error: None,
                remote_missing: false,
                revision: Some(revision),
            })
        }
        Err(AppError::WebdavNotFound(_)) => {
            let source = if local.is_some() { "local" } else { "none" };
            Ok(LoadedProfile {
                source: source.to_string(),
                content: local,
                remote_error: None,
                remote_missing: true,
                revision: None,
            })
        }
        Err(error) => {
            log::warn!("failed to read remote profile: {error}");
            Ok(LoadedProfile {
                content: local,
                source: "local".to_string(),
                remote_error: Some(error.to_string()),
                remote_missing: false,
                revision: None,
            })
        }
    }
}

/// Persist the profile locally and, when configured, to WebDAV.
///
/// `expected_revision` is the opaque token returned by
/// [`load_remote_profile`]; a stale token makes the write fail with
/// `conflict: true` instead of overwriting another device's changes. `force`
/// skips the precondition, used only after the user explicitly chooses the
/// local version in the conflict dialog.
#[tauri::command]
pub async fn save_profile(
    app: AppHandle,
    content: String,
    expected_revision: Option<String>,
    force: bool,
) -> Result<SaveProfileResult, AppError> {
    let local = local_set(&app, &content).is_ok();

    let Some(client) = client(&app)? else {
        return Ok(SaveProfileResult {
            remote: false,
            local,
            warning: None,
            conflict: false,
            revision: None,
        });
    };

    let expected = if force {
        None
    } else {
        expected_revision.as_deref()
    };
    match client
        .put_if_match(
            REMOTE_PATH,
            "application/json",
            content.into_bytes(),
            expected,
        )
        .await
    {
        Ok((false, _)) => Ok(SaveProfileResult {
            remote: false,
            local,
            warning: Some("conflict".to_string()),
            conflict: true,
            revision: None,
        }),
        Ok((true, revision)) => Ok(SaveProfileResult {
            remote: true,
            local,
            warning: None,
            conflict: false,
            revision,
        }),
        Err(error) => {
            log::warn!("failed to write remote profile: {error}");
            Ok(SaveProfileResult {
                remote: false,
                local,
                warning: Some(error.to_string()),
                conflict: false,
                revision: None,
            })
        }
    }
}
