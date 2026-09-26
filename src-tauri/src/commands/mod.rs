//! Tauri command handlers exposed to the frontend.
//!
//! Every command returns `Result<T, AppError>` so failures reach the frontend
//! as structured `{ code, message }` payloads. Domain-specific commands live in
//! submodules (e.g. [`webdav`]) and are registered in `run()` via
//! `tauri::generate_handler!`.

pub mod covers;
pub mod editor;
pub mod fonts;
pub mod lyrics;
pub mod media;
pub mod playback;
pub mod profile;
pub mod webdav;

use crate::error::AppError;

/// Connectivity probe for the frontend <-> Rust IPC channel.
///
/// Returns `"pong"`; used to verify that `invoke("ping")` reaches Rust and
/// that the response travels back to the frontend.
#[tauri::command]
pub fn ping() -> Result<String, AppError> {
    log::debug!("ping received");
    Ok("pong".to_string())
}

/// Echoes `message` back after validating it.
///
/// Exists to exercise the structured error path: an empty or overly long
/// message produces an [`AppError::InvalidArgument`] with a clear reason.
#[tauri::command]
pub fn echo(message: String) -> Result<String, AppError> {
    let message = message.trim();

    if message.is_empty() {
        let error = AppError::invalid_argument("message", "must not be empty");
        error.log();
        return Err(error);
    }

    if message.chars().count() > 64 {
        let error = AppError::invalid_argument("message", "must be 64 characters or fewer");
        error.log();
        return Err(error);
    }

    log::debug!("echo: {message}");
    Ok(message.to_string())
}

/// Bridges frontend diagnostics into the Rust log.
///
/// The webview console is easy to miss when the app runs in a window, so the
/// frontend routes its pipeline traces through here to land on the same stderr
/// stream as the backend under `tauri dev`.
#[tauri::command]
pub fn log_message(level: String, message: String) -> Result<(), AppError> {
    let message = message.trim();

    if message.is_empty() {
        return Ok(());
    }

    match level.as_str() {
        "warn" => log::warn!("{message}"),
        "error" => log::error!("{message}"),
        _ => log::info!("{message}"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping().unwrap(), "pong");
    }

    #[test]
    fn echo_rejects_empty_message() {
        let error = echo("   ".to_string()).unwrap_err();
        assert_eq!(error.code(), "INVALID_ARGUMENT");
        assert_eq!(
            error.to_string(),
            "invalid argument `message`: must not be empty"
        );
    }

    #[test]
    fn echo_rejects_overlong_message() {
        let error = echo("a".repeat(65)).unwrap_err();
        assert_eq!(error.code(), "INVALID_ARGUMENT");
        assert_eq!(
            error.to_string(),
            "invalid argument `message`: must be 64 characters or fewer"
        );
    }

    #[test]
    fn error_serializes_to_structured_payload() {
        let error = AppError::invalid_argument("message", "must not be empty");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "INVALID_ARGUMENT");
        assert_eq!(
            json["message"],
            "invalid argument `message`: must not be empty"
        );
    }
}
