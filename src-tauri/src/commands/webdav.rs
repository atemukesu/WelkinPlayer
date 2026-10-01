//! WebDAV credential storage and URL safety policy.
//!
//! Source URLs and usernames are persisted by the frontend through
//! `tauri-plugin-store` (`settings.json`, inside the `sources` array).
//! Passwords never touch that file: they live in the OS keychain, indexed by
//! the source id — Windows Credential Manager, macOS Keychain, or Linux Secret
//! Service.

use std::net::IpAddr;
use std::sync::OnceLock;

use serde::Serialize;
use tauri::AppHandle;

use crate::error::AppError;

/// Keychain service name; together with the account it forms the lookup key.
/// Prefixed with the app bundle identifier so entries are namespaced per app.
const SERVICE: &str = "com.atemukesu.welkinplayer.webdav";

/// Account prefix for source-scoped credentials (`source:<id>`).
const ACCOUNT_PREFIX: &str = "source:";

/// Why the platform keychain backend is unavailable, recorded once at startup.
static KEYCHAIN_INIT_ERROR: OnceLock<Option<String>> = OnceLock::new();

/// Record the one-time platform keychain setup result.
#[allow(dead_code)] // only invoked by the Android startup path
pub(crate) fn set_keychain_init_error(error: Option<String>) {
    let _ = KEYCHAIN_INIT_ERROR.set(error);
}

/// The startup keychain failure, when the backend is known to be unusable.
pub(crate) fn keychain_init_error() -> Option<&'static str> {
    KEYCHAIN_INIT_ERROR.get().and_then(|error| error.as_deref())
}

/// Outcome of a keychain read/write, returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeychainStatus {
    pub available: bool,
    pub has_password: bool,
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
        Self::unavailable_reason(error.to_string())
    }

    fn unavailable_reason(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            has_password: false,
            warning: Some(reason.into()),
        }
    }
}

/// How safe a configured WebDAV URL is for carrying Basic Auth credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UrlSafety {
    Secure,
    InsecurePrivate,
    InsecurePublic,
}

/// Risk report for a URL, so the frontend can warn the user.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlRisk {
    pub safety: UrlSafety,
    pub host: String,
    pub blocked: bool,
    pub allow_insecure: bool,
}

/// Keychain account for a source id.
fn account_for(source_id: &str) -> String {
    format!("{ACCOUNT_PREFIX}{source_id}")
}

fn new_entry(account: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, account)
}

/// Short-circuit a command when no persistent keychain backend was installed.
fn keychain_unavailable() -> Option<KeychainStatus> {
    keychain_init_error().map(KeychainStatus::unavailable_reason)
}

/// Remove the credential for `account`, treating "not found" as success.
fn delete_credential(account: &str) -> Result<(), keyring::Error> {
    if account.is_empty() {
        return Ok(());
    }
    match new_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Read a stored secret for `account`, treating a missing entry as `None`.
fn read_password(account: &str) -> Result<Option<String>, keyring::Error> {
    if account.is_empty() {
        return Ok(None);
    }
    match new_entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Read a source's password, defaulting to an empty string when none is stored.
///
/// Credentials are keyed by `source:<id>`, so any number of sources may share a
/// username without colliding: each has its own keychain entry.
pub(crate) fn source_password(_app: &AppHandle, source_id: &str) -> Result<String, AppError> {
    if let Some(reason) = keychain_init_error() {
        return Err(AppError::Store(format!("系统钥匙串不可用：{reason}")));
    }
    match read_password(&account_for(source_id)) {
        Ok(Some(secret)) => Ok(secret),
        Ok(None) => Ok(String::new()),
        Err(error) => Err(AppError::Store(format!("系统钥匙串不可用：{error}"))),
    }
}

/// Report whether a password is stored for a source (never the secret).
#[tauri::command]
pub fn load_source_password(app: AppHandle, source_id: String) -> Result<KeychainStatus, AppError> {
    let _ = app;
    if let Some(status) = keychain_unavailable() {
        return Ok(status);
    }
    if source_id.trim().is_empty() {
        return Ok(KeychainStatus::available(false));
    }
    match read_password(&account_for(source_id.trim())) {
        Ok(Some(_)) => Ok(KeychainStatus::available(true)),
        Ok(None) => Ok(KeychainStatus::available(false)),
        Err(error) => Ok(KeychainStatus::unavailable(error)),
    }
}

/// Persist (or clear) a source's password.
///
/// * `Some(non-empty)` -> store it under the source id.
/// * `Some(empty)`     -> remove the credential.
/// * `None`            -> keep whatever is stored.
#[tauri::command]
pub fn save_source_password(
    source_id: String,
    password: Option<String>,
) -> Result<KeychainStatus, AppError> {
    if let Some(status) = keychain_unavailable() {
        return Ok(status);
    }
    let source_id = source_id.trim();
    if source_id.is_empty() {
        return Err(AppError::invalid_argument("sourceId", "不能为空"));
    }
    let account = account_for(source_id);

    match password {
        None => match read_password(&account) {
            Ok(secret) => Ok(KeychainStatus::available(secret.is_some())),
            Err(error) => Ok(KeychainStatus::unavailable(error)),
        },
        Some(secret) if secret.is_empty() => match delete_credential(&account) {
            Ok(()) => Ok(KeychainStatus::available(false)),
            Err(error) => Ok(KeychainStatus::unavailable(error)),
        },
        Some(secret) => match new_entry(&account).and_then(|entry| entry.set_password(&secret)) {
            Ok(()) => Ok(KeychainStatus::available(true)),
            Err(error) => Ok(KeychainStatus::unavailable(error)),
        },
    }
}

/// Remove a source's stored credential entirely.
#[tauri::command]
pub fn delete_source_password(source_id: String) -> Result<(), AppError> {
    if keychain_unavailable().is_some() {
        return Ok(());
    }
    if source_id.trim().is_empty() {
        return Ok(());
    }
    delete_credential(&account_for(source_id.trim()))
        .map_err(|error| AppError::Store(format!("系统钥匙串不可用：{error}")))
}

/// Whether a host belongs to the local machine, a LAN or an intranet.
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
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                let octets = v6.octets();
                let unique_local = (octets[0] & 0xfe) == 0xfc;
                let link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
                v6.is_loopback() || v6.is_unspecified() || unique_local || link_local
            }
        };
    }

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

/// Refuse plaintext HTTP to a public host unless the source opted in.
pub(crate) fn enforce_url_policy(
    _app: &AppHandle,
    url: &str,
    allow_insecure: bool,
) -> Result<(), AppError> {
    match classify_url(url)? {
        UrlSafety::Secure | UrlSafety::InsecurePrivate => Ok(()),
        UrlSafety::InsecurePublic => {
            if allow_insecure {
                Ok(())
            } else {
                Err(AppError::InsecureUrl(
                    "明文 HTTP 连接公网服务器已被阻止，请为来源启用“允许不安全连接”或改用 HTTPS / VPN"
                        .to_string(),
                ))
            }
        }
    }
}

/// Risk report for a URL, so the frontend can warn before saving/connecting.
#[tauri::command]
pub fn webdav_url_risk(url: String, allow_insecure: Option<bool>) -> Result<UrlRisk, AppError> {
    let allow_insecure = allow_insecure.unwrap_or(false);
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
