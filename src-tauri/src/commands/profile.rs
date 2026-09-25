//! Cross-device profile storage.
//!
//! The whole user profile (nickname, appearance, play counts, favorites and
//! playlists) lives in a single JSON document so reading it costs exactly one
//! `GET` instead of a request per concern. WebDAV is the authoritative copy;
//! a local copy (`profile.json`) is kept as a fallback for offline use and for
//! installs without a configured server.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::commands::webdav::{load_saved_credentials, require_credentials};
use crate::dav::WebDavClient;
use crate::error::AppError;

const LOCAL_FILE: &str = "profile.json";
const LOCAL_KEY: &str = "profile";
/// Profile document at the WebDAV collection root (ignored by the audio listing).
const REMOTE_PATH: &str = "welkin-profile.json";
/// Temporary object used to prove the server allows writes.
const WRITE_TEST_PATH: &str = "welkin-write-test.tmp";
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

/// Build a client from saved credentials, or `None` when the server is not
/// configured yet.
fn client(app: &AppHandle) -> Result<Option<WebDavClient>, AppError> {
    let Some(credentials) = load_saved_credentials(app)? else {
        return Ok(None);
    };
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    Ok(Some(client))
}

/// Load the profile, preferring the authoritative remote copy.
#[tauri::command]
pub async fn load_profile(app: AppHandle) -> Result<LoadedProfile, AppError> {
    let local = local_get(&app);

    let client = match client(&app) {
        Ok(client) => client,
        Err(error) => {
            // A broken keychain / invalid URL must not block local usage.
            return Ok(LoadedProfile {
                content: local,
                source: "local".to_string(),
                remote_error: Some(error.to_string()),
            });
        }
    };

    let Some(client) = client else {
        let source = if local.is_some() { "local" } else { "none" };
        return Ok(LoadedProfile {
            source: source.to_string(),
            content: local,
            remote_error: None,
        });
    };

    match client.get(REMOTE_PATH).await {
        Ok(response) => {
            let bytes = response.bytes().await?;
            if bytes.len() > MAX_PROFILE_BYTES {
                return Ok(LoadedProfile {
                    content: local,
                    source: "local".to_string(),
                    remote_error: Some("远程配置过大，已忽略".to_string()),
                });
            }
            let text = String::from_utf8(bytes.to_vec())
                .map_err(|error| AppError::Other(format!("远程配置不是有效的 UTF-8：{error}")))?;
            if text.trim().is_empty() {
                let source = if local.is_some() { "local" } else { "none" };
                return Ok(LoadedProfile {
                    source: source.to_string(),
                    content: local,
                    remote_error: None,
                });
            }
            // Keep the local fallback in sync with the authoritative copy.
            if let Err(error) = local_set(&app, &text) {
                log::warn!("failed to cache profile locally: {error}");
            }
            Ok(LoadedProfile {
                content: Some(text),
                source: "remote".to_string(),
                remote_error: None,
            })
        }
        Err(AppError::WebdavNotFound(_)) => {
            let source = if local.is_some() { "local" } else { "none" };
            Ok(LoadedProfile {
                source: source.to_string(),
                content: local,
                remote_error: None,
            })
        }
        Err(error) => {
            log::warn!("failed to read remote profile: {error}");
            Ok(LoadedProfile {
                content: local,
                source: "local".to_string(),
                remote_error: Some(error.to_string()),
            })
        }
    }
}

/// Persist the profile locally and, when configured, to WebDAV.
#[tauri::command]
pub async fn save_profile(app: AppHandle, content: String) -> Result<SaveProfileResult, AppError> {
    let local = local_set(&app, &content).is_ok();

    let Some(client) = client(&app)? else {
        return Ok(SaveProfileResult {
            remote: false,
            local,
            warning: None,
        });
    };

    match client
        .put(REMOTE_PATH, "application/json", content.into_bytes())
        .await
    {
        Ok(()) => Ok(SaveProfileResult {
            remote: true,
            local,
            warning: None,
        }),
        Err(error) => {
            log::warn!("failed to write remote profile: {error}");
            Ok(SaveProfileResult {
                remote: false,
                local,
                warning: Some(error.to_string()),
            })
        }
    }
}

/// Verify the configured server allows writes: PUT a throwaway object, read it
/// back and DELETE it. Returns the URL that was tested.
#[tauri::command]
pub async fn test_webdav_write(app: AppHandle) -> Result<String, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;

    let payload = b"welkin write probe".to_vec();
    client
        .put(WRITE_TEST_PATH, "text/plain", payload.clone())
        .await?;

    let echoed = client.get(WRITE_TEST_PATH).await?.bytes().await?;
    let delete_result = client.delete(WRITE_TEST_PATH).await;

    if echoed != payload {
        return Err(AppError::Other("写入校验失败：回读内容不一致".to_string()));
    }
    delete_result?;

    log::info!("WebDAV write test succeeded");
    Ok(credentials.url)
}
