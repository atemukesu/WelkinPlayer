//! Unified error type shared by every Tauri command.
//!
//! Commands return `Result<T, AppError>`; Tauri serializes the error as
//! `{ "code": "...", "message": "..." }` so the frontend can render a specific,
//! user-readable reason instead of an opaque `unknown error`.

use std::io;

use serde::ser::Serializer;
use serde::Serialize;

/// Errors surfaced from the Rust backend to the frontend.
///
/// Some variants are unused for now but are part of the reusable error surface
/// that upcoming lyrics / state-sync commands will return.
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A caller-supplied argument failed validation.
    #[error("invalid argument `{field}`: {reason}")]
    InvalidArgument { field: String, reason: String },

    /// The requested resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// Underlying I/O failure (filesystem, network streams, ...).
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),

    /// JSON (de)serialization failure.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Catch-all for anything that does not fit the variants above.
    #[error("{0}")]
    Other(String),

    /// The server rejected the supplied credentials (HTTP 401).
    #[error("认证失败：用户名或密码不正确（401）")]
    Unauthorized,

    /// The credentials are valid but lack permission (HTTP 403).
    #[error("没有访问权限（403）")]
    Forbidden,

    /// The remote path does not exist (HTTP 404).
    #[error("远端路径不存在：{0}")]
    WebdavNotFound(String),

    /// The request exceeded the configured timeout.
    #[error("连接超时：服务器在限定时间内没有响应")]
    Timeout,

    /// The host name could not be resolved.
    #[error("无法解析服务器地址（DNS 失败）：{0}")]
    Dns(String),

    /// The TLS handshake or certificate validation failed.
    #[error("证书或 TLS 错误：{0}")]
    Tls(String),

    /// A connection could not be established for another reason.
    #[error("无法连接到服务器：{0}")]
    Connection(String),

    /// The server answered with an unexpected HTTP status.
    #[error("服务器返回错误状态 {status}：{message}")]
    Http { status: u16, message: String },

    /// The server response could not be parsed as WebDAV XML.
    #[error("无法解析服务器响应：{0}")]
    Xml(String),

    /// A WebDAV operation was attempted without a complete set of credentials.
    #[error("缺少 WebDAV 凭据：{0}")]
    MissingCredentials(String),

    /// A plaintext HTTP connection to a public server was refused because the
    /// user has not explicitly allowed insecure connections.
    #[error("已阻止不安全的连接：{0}")]
    InsecureUrl(String),

    /// Reading or writing the local settings store failed.
    #[error("读取本地设置失败：{0}")]
    Store(String),

    /// The Pro activation code's signature did not verify against the
    /// developer's embedded Ed25519 public key.
    #[error("激活失败：这不是由开发者签发的 Key。")]
    LicenseSigner,

    /// The signed code is authentic but was issued for another installation.
    #[error("激活失败：此 Key 不适用于你的设备。")]
    LicenseDevice,

    /// The signed code is authentic but past its `expires_at`.
    #[error("激活失败：此 Key 已过期。")]
    LicenseExpired,

    /// The signed code is authentic but carries an unsupported tier.
    #[error("激活失败：此 Key 的层级无效。")]
    LicenseTier,
}

#[allow(dead_code)]
impl AppError {
    /// Stable, machine-readable code consumed by the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::InvalidArgument { .. } => "INVALID_ARGUMENT",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Io(_) => "IO",
            AppError::Serde(_) => "SERIALIZATION",
            AppError::Other(_) => "UNKNOWN",
            AppError::Unauthorized => "UNAUTHORIZED",
            AppError::Forbidden => "FORBIDDEN",
            AppError::WebdavNotFound(_) => "NOT_FOUND",
            AppError::Timeout => "TIMEOUT",
            AppError::Dns(_) => "DNS",
            AppError::Tls(_) => "TLS",
            AppError::Connection(_) => "CONNECTION",
            AppError::Http { .. } => "HTTP",
            AppError::Xml(_) => "XML",
            AppError::MissingCredentials(_) => "MISSING_CREDENTIALS",
            AppError::InsecureUrl(_) => "INSECURE_URL",
            AppError::Store(_) => "STORE",
            AppError::LicenseSigner => "LICENSE_SIGNER",
            AppError::LicenseDevice => "LICENSE_DEVICE",
            AppError::LicenseExpired => "LICENSE_EXPIRED",
            AppError::LicenseTier => "LICENSE_TIER",
        }
    }

    /// Build an [`AppError::Other`] from any string-like value.
    pub fn other(message: impl Into<String>) -> Self {
        AppError::Other(message.into())
    }

    /// Build an [`AppError::InvalidArgument`] for a named field.
    pub fn invalid_argument(field: impl Into<String>, reason: impl Into<String>) -> Self {
        AppError::InvalidArgument {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Emit this error to the application log with its stable code.
    pub fn log(&self) {
        log::error!("[{}] {}", self.code(), self);
    }
}

/// Classify a transport-level `reqwest` failure into a user-readable variant.
impl From<reqwest::Error> for AppError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            return AppError::Timeout;
        }

        let message = error.to_string();
        let lower = message.to_lowercase();
        let is_tls = lower.contains("certificate")
            || lower.contains("tls")
            || lower.contains("handshake")
            || lower.contains("invalid peer");

        if error.is_connect() {
            if is_tls {
                return AppError::Tls(message);
            }
            if lower.contains("dns")
                || lower.contains("name or service not known")
                || lower.contains("failed to lookup address")
                || lower.contains("nodename nor servname")
            {
                return AppError::Dns(message);
            }
            return AppError::Connection(message);
        }

        if is_tls {
            return AppError::Tls(message);
        }

        AppError::Connection(message)
    }
}

/// Reading/writing the settings store.
impl From<tauri_plugin_store::Error> for AppError {
    fn from(error: tauri_plugin_store::Error) -> Self {
        AppError::Store(error.to_string())
    }
}

/// Wire format sent to the frontend.
#[derive(Serialize)]
struct ErrorPayload<'a> {
    code: &'a str,
    message: String,
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}
