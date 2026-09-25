//! WebDAV credential storage.
//!
//! The server URL and username are persisted by the frontend through
//! `tauri-plugin-store` (`settings.json`). Passwords never touch that file:
//! they live in the OS keychain, indexed by `(SERVICE, username)` — Windows
//! Credential Manager, macOS Keychain, or Linux Secret Service.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::dav::{RemoteEntry, WebDavClient};
use crate::error::AppError;

/// Keychain service name; together with the username it forms the lookup key.
const SERVICE: &str = "com.example.app.webdav";

/// `tauri-plugin-store` file holding the non-secret WebDAV settings.
const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_URL_KEY: &str = "webdav.url";
const SETTINGS_USERNAME_KEY: &str = "webdav.username";

/// Outcome of a keychain read/write, returned to the frontend.
#[derive(Debug, Serialize)]
pub struct KeychainStatus {
    /// `false` when the OS keychain could not be reached (e.g. Linux headless).
    pub available: bool,
    /// The stored password, when one was found.
    pub password: Option<String>,
    /// Raw platform error, kept for diagnostics. The frontend shows a localized
    /// message whenever `available` is `false`.
    pub warning: Option<String>,
}

impl KeychainStatus {
    fn available(password: Option<String>) -> Self {
        Self {
            available: true,
            password,
            warning: None,
        }
    }

    fn unavailable(error: keyring::Error) -> Self {
        log::error!("system keychain unavailable: {error}");
        Self {
            available: false,
            password: None,
            warning: Some(error.to_string()),
        }
    }
}

fn new_entry(username: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, username)
}

/// Remove the credential for `username`, treating "not found" as success.
fn delete_credential(username: &str) -> Result<(), keyring::Error> {
    if username.is_empty() {
        return Ok(());
    }

    match new_entry(username)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Read the stored password for `username`.
#[tauri::command]
pub fn load_webdav_password(username: String) -> Result<KeychainStatus, AppError> {
    let username = username.trim();
    if username.is_empty() {
        return Ok(KeychainStatus::available(None));
    }

    let entry = match new_entry(username) {
        Ok(entry) => entry,
        Err(error) => return Ok(KeychainStatus::unavailable(error)),
    };

    match entry.get_password() {
        Ok(password) => Ok(KeychainStatus::available(Some(password))),
        Err(keyring::Error::NoEntry) => Ok(KeychainStatus::available(None)),
        Err(error) => Ok(KeychainStatus::unavailable(error)),
    }
}

/// Persist (or clear) the password for a username.
///
/// * `password` non-empty -> store it under `username`.
/// * `password` empty     -> remove the credential for `username`.
/// * `previous_username`  -> when the username changed, the old keychain entry
///   is removed first so stale credentials do not linger.
#[tauri::command]
pub fn save_webdav_password(
    username: String,
    previous_username: Option<String>,
    password: String,
) -> Result<KeychainStatus, AppError> {
    let username = username.trim().to_string();
    let previous = previous_username
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let clearing = password.is_empty();

    if !clearing && username.is_empty() {
        let error = AppError::invalid_argument("username", "is required to store a password");
        error.log();
        return Err(error);
    }

    if let Some(previous) = previous.as_deref() {
        if previous != username {
            if let Err(error) = delete_credential(previous) {
                return Ok(KeychainStatus::unavailable(error));
            }
        }
    }

    if clearing {
        return match delete_credential(&username) {
            Ok(()) => {
                log::info!("cleared stored WebDAV credential");
                Ok(KeychainStatus::available(None))
            }
            Err(error) => Ok(KeychainStatus::unavailable(error)),
        };
    }

    let entry = match new_entry(&username) {
        Ok(entry) => entry,
        Err(error) => return Ok(KeychainStatus::unavailable(error)),
    };

    match entry.set_password(&password) {
        Ok(()) => {
            log::info!("stored WebDAV credential");
            Ok(KeychainStatus::available(Some(password)))
        }
        Err(error) => Ok(KeychainStatus::unavailable(error)),
    }
}

/// Credentials used to talk to the configured WebDAV server.
pub(crate) struct SavedCredentials {
    pub(crate) url: String,
    pub(crate) username: String,
    pub(crate) password: String,
}

fn read_setting(app: &AppHandle, key: &str) -> Result<String, AppError> {
    let store = app.store(SETTINGS_FILE)?;
    Ok(store
        .get(key)
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default())
}

/// Load the saved URL + username (settings.json) and the password (keychain).
pub(crate) fn load_saved_credentials(
    app: &AppHandle,
) -> Result<Option<SavedCredentials>, AppError> {
    let url = read_setting(app, SETTINGS_URL_KEY)?;
    let username = read_setting(app, SETTINGS_USERNAME_KEY)?;

    if url.trim().is_empty() || username.trim().is_empty() {
        return Ok(None);
    }

    let password = match keyring::Entry::new(SERVICE, username.trim()) {
        Ok(entry) => match entry.get_password() {
            Ok(password) => password,
            Err(keyring::Error::NoEntry) => String::new(),
            Err(error) => return Err(AppError::Store(format!("系统钥匙串不可用：{error}"))),
        },
        Err(error) => return Err(AppError::Store(format!("系统钥匙串不可用：{error}"))),
    };

    Ok(Some(SavedCredentials {
        url,
        username,
        password,
    }))
}

pub(crate) fn require_credentials(app: &AppHandle) -> Result<SavedCredentials, AppError> {
    load_saved_credentials(app)?.ok_or_else(|| {
        AppError::MissingCredentials("请先在 WebDAV 设置中填写并保存服务器地址与用户名".to_string())
    })
}

/// Probe the saved server with a depth-0 PROPFIND (Stage 1.3).
#[tauri::command]
pub async fn test_webdav_connection(app: AppHandle) -> Result<String, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let xml = client.propfind("", 0).await?;

    if xml.trim().is_empty() {
        return Err(AppError::Xml("服务器返回了空的 PROPFIND 响应".to_string()));
    }

    log::info!("WebDAV connection test succeeded");
    Ok(credentials.url)
}

/// Recursively list audio files under the saved collection (Stage 1.4).
#[tauri::command]
pub async fn list_webdav_audio(
    app: AppHandle,
    path: Option<String>,
) -> Result<Vec<RemoteEntry>, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let root = path.unwrap_or_default();
    let files = client.list_audio_files(&root).await?;

    log::info!("listed {} audio files from WebDAV", files.len());
    Ok(files)
}
