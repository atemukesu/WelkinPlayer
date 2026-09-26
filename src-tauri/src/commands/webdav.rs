//! WebDAV credential storage.
//!
//! The server URL and username are persisted by the frontend through
//! `tauri-plugin-store` (`settings.json`). Passwords never touch that file:
//! they live in the OS keychain, indexed by `(SERVICE, username)` — Windows
//! Credential Manager, macOS Keychain, or Linux Secret Service.

use std::net::IpAddr;

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
const SETTINGS_ALLOW_INSECURE_KEY: &str = "webdav.allowInsecure";

/// Outcome of a keychain read/write, returned to the frontend.
///
/// The secret itself is never returned: the frontend only needs to know whether
/// a credential exists, so the plaintext password never enters the webview.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeychainStatus {
    /// `false` when the OS keychain could not be reached (e.g. Linux headless).
    pub available: bool,
    /// Whether a password is currently stored for this username.
    pub has_password: bool,
    /// Raw platform error, kept for diagnostics. The frontend shows a localized
    /// message whenever `available` is `false`.
    pub warning: Option<String>,
}

impl KeychainStatus {
    fn available(has_password: bool) -> Self {
        Self {
            available: true,
            has_password,
            warning: None,
        }
    }

    fn unavailable(error: keyring::Error) -> Self {
        log::error!("system keychain unavailable: {error}");
        Self {
            available: false,
            has_password: false,
            warning: Some(error.to_string()),
        }
    }
}

/// How safe a configured WebDAV URL is for carrying Basic Auth credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UrlSafety {
    /// `https://` — credentials are protected in transit.
    Secure,
    /// `http://` on a local/private network host (intranet, LAN, loopback).
    InsecurePrivate,
    /// `http://` on a public host — refused unless the user opts in.
    InsecurePublic,
}

/// Risk report for a URL, so the frontend can warn the user.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlRisk {
    pub safety: UrlSafety,
    pub host: String,
    /// Whether the current policy refuses this URL.
    pub blocked: bool,
    /// Whether "allow insecure connections" is enabled.
    pub allow_insecure: bool,
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

/// Report whether a password is stored for `username` (never the secret).
#[tauri::command]
pub fn load_webdav_password(username: String) -> Result<KeychainStatus, AppError> {
    let username = username.trim();
    if username.is_empty() {
        return Ok(KeychainStatus::available(false));
    }

    let entry = match new_entry(username) {
        Ok(entry) => entry,
        Err(error) => return Ok(KeychainStatus::unavailable(error)),
    };

    match entry.get_password() {
        Ok(_) => Ok(KeychainStatus::available(true)),
        Err(keyring::Error::NoEntry) => Ok(KeychainStatus::available(false)),
        Err(error) => Ok(KeychainStatus::unavailable(error)),
    }
}

/// Read a stored secret, treating a missing entry as `None`.
fn read_password(username: &str) -> Result<Option<String>, keyring::Error> {
    if username.is_empty() {
        return Ok(None);
    }
    match new_entry(username)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Persist (or clear) the password for a username.
///
/// * `Some(non-empty)` -> store it under `username`.
/// * `Some(empty)`     -> remove the credential for `username`.
/// * `None`            -> keep whatever is stored, only re-indexing it when the
///   username changed (the keychain entry is keyed by username).
///
/// `previous_username` lets a username change move or remove the old entry
/// without ever sending the plaintext secret back to the frontend.
#[tauri::command]
pub fn save_webdav_password(
    username: String,
    previous_username: Option<String>,
    password: Option<String>,
) -> Result<KeychainStatus, AppError> {
    let username = username.trim().to_string();
    let previous = previous_username
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let renamed = previous
        .as_deref()
        .is_some_and(|previous| previous != username);

    // Read the current secret before mutating the keychain so a rename can move
    // it. When the username is unchanged the credential already lives under it.
    let existing = if renamed {
        match read_password(previous.as_deref().unwrap_or_default()) {
            Ok(secret) => secret,
            Err(error) => return Ok(KeychainStatus::unavailable(error)),
        }
    } else {
        match read_password(&username) {
            Ok(secret) => secret,
            Err(error) => return Ok(KeychainStatus::unavailable(error)),
        }
    };

    match password {
        None => {
            // Keep the credential, moving it when the username changed.
            if renamed {
                let Some(secret) = existing.as_deref() else {
                    return Ok(KeychainStatus::available(false));
                };
                if username.is_empty() {
                    if let Some(previous) = previous.as_deref() {
                        if let Err(error) = delete_credential(previous) {
                            return Ok(KeychainStatus::unavailable(error));
                        }
                    }
                    return Ok(KeychainStatus::available(false));
                }
                let entry = match new_entry(&username) {
                    Ok(entry) => entry,
                    Err(error) => return Ok(KeychainStatus::unavailable(error)),
                };
                if let Err(error) = entry.set_password(secret) {
                    return Ok(KeychainStatus::unavailable(error));
                }
                if let Some(previous) = previous.as_deref() {
                    let _ = delete_credential(previous);
                }
                log::info!("re-indexed stored WebDAV credential after username change");
            }
            Ok(KeychainStatus::available(existing.is_some()))
        }
        Some(secret) if secret.is_empty() => {
            // Explicitly clear the credential.
            if renamed {
                if let Some(previous) = previous.as_deref() {
                    if let Err(error) = delete_credential(previous) {
                        return Ok(KeychainStatus::unavailable(error));
                    }
                }
            }
            match delete_credential(&username) {
                Ok(()) => {
                    log::info!("cleared stored WebDAV credential");
                    Ok(KeychainStatus::available(false))
                }
                Err(error) => Ok(KeychainStatus::unavailable(error)),
            }
        }
        Some(secret) => {
            if username.is_empty() {
                let error =
                    AppError::invalid_argument("username", "is required to store a password");
                error.log();
                return Err(error);
            }
            if renamed {
                if let Some(previous) = previous.as_deref() {
                    if let Err(error) = delete_credential(previous) {
                        return Ok(KeychainStatus::unavailable(error));
                    }
                }
            }
            let entry = match new_entry(&username) {
                Ok(entry) => entry,
                Err(error) => return Ok(KeychainStatus::unavailable(error)),
            };
            match entry.set_password(&secret) {
                Ok(()) => {
                    log::info!("stored WebDAV credential");
                    Ok(KeychainStatus::available(true))
                }
                Err(error) => Ok(KeychainStatus::unavailable(error)),
            }
        }
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

/// Read a boolean setting, defaulting to `false` when absent or unreadable.
fn read_bool_setting(app: &AppHandle, key: &str) -> bool {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(key))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Whether a host belongs to the local machine, a LAN or an intranet.
///
/// Dotted hostnames we cannot resolve are treated as public (stricter): an
/// unknown public server must not silently receive credentials over plain HTTP.
fn is_local_or_private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                let octets = v6.octets();
                let unique_local = (octets[0] & 0xfe) == 0xfc;
                let link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
                v6.is_loopback() || v6.is_unspecified() || unique_local || link_local
            }
        };
    }

    // A bare hostname (no dot) is assumed to be an intranet name.
    !host.contains('.')
}

/// Classify a WebDAV URL by how safely it carries credentials.
pub fn classify_url(url: &str) -> Result<UrlSafety, AppError> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|error| AppError::invalid_argument("url", format!("无效的服务器地址：{error}")))?;

    match parsed.scheme() {
        "https" => Ok(UrlSafety::Secure),
        "http" => {
            let host = parsed.host_str().unwrap_or_default();
            if is_local_or_private_host(host) {
                Ok(UrlSafety::InsecurePrivate)
            } else {
                Ok(UrlSafety::InsecurePublic)
            }
        }
        other => Err(AppError::invalid_argument(
            "url",
            format!("不支持的协议：{other}"),
        )),
    }
}

/// Refuse plaintext HTTP to a public host unless the user opted in.
pub(crate) fn enforce_url_policy(app: &AppHandle, url: &str) -> Result<(), AppError> {
    match classify_url(url)? {
        UrlSafety::Secure | UrlSafety::InsecurePrivate => Ok(()),
        UrlSafety::InsecurePublic => {
            if read_bool_setting(app, SETTINGS_ALLOW_INSECURE_KEY) {
                Ok(())
            } else {
                Err(AppError::InsecureUrl(
                    "明文 HTTP 连接公网服务器已被阻止，请在设置中启用“允许不安全连接”或改用 HTTPS / VPN"
                        .to_string(),
                ))
            }
        }
    }
}

/// Risk report for a URL, so the frontend can warn before saving/connecting.
#[tauri::command]
pub fn webdav_url_risk(app: AppHandle, url: String) -> Result<UrlRisk, AppError> {
    let allow_insecure = read_bool_setting(&app, SETTINGS_ALLOW_INSECURE_KEY);
    let safety = classify_url(&url)?;
    let host = reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_default();
    let blocked = matches!(safety, UrlSafety::InsecurePublic) && !allow_insecure;
    Ok(UrlRisk {
        safety,
        host,
        blocked,
        allow_insecure,
    })
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

    // Refuse to send credentials over an unconfirmed plaintext public link.
    enforce_url_policy(app, &url)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_https_as_secure() {
        assert_eq!(
            classify_url("https://dav.example.com/music").unwrap(),
            UrlSafety::Secure
        );
    }

    #[test]
    fn classifies_private_http_as_insecure_but_allowed() {
        for url in [
            "http://localhost:8080/dav",
            "http://127.0.0.1/dav",
            "http://192.168.1.10/dav",
            "http://10.0.0.5/dav",
            "http://nas/dav",
            "http://[fe80::1]/dav",
            "http://[fc00::1]/dav",
        ] {
            assert_eq!(
                classify_url(url).unwrap(),
                UrlSafety::InsecurePrivate,
                "{url}"
            );
        }
    }

    #[test]
    fn classifies_public_http_as_insecure_public() {
        assert_eq!(
            classify_url("http://dav.example.com/music").unwrap(),
            UrlSafety::InsecurePublic
        );
    }

    #[test]
    fn rejects_unsupported_schemes() {
        assert!(classify_url("ftp://host/x").is_err());
        assert!(classify_url("not a url").is_err());
    }
}
