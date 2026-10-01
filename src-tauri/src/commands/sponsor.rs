//! Sponsorship support: the per-install identifier.
//!
//! The sponsorship backend recognizes a specific installation across restarts
//! without collecting any personal data, so every install gets a random
//! identifier generated once with the OS CSPRNG.
//!
//! The identifier is treated as a secret. It lives only in the system keychain,
//! in the entry's password field, under its own service, and is deliberately
//! kept out of `settings.json` and the synchronized profile (both travel across
//! machines) so it cannot be copied to another install or leak through WebDAV.
//! The value is never written to the log.

use serde::Serialize;
use tauri::AppHandle;

use crate::commands::profile::local_nickname;
use crate::commands::webdav::keychain_init_error;
use crate::error::AppError;

/// Keychain service for the install identifier. Kept separate from the WebDAV
/// service (`com.atemukesu.welkinplayer.webdav`) so the install identifier and
/// the server password never share an entry.
const SERVICE: &str = "com.atemukesu.welkinplayer.instid";
/// Account name that holds the install identifier in the entry's password field.
const INSTALL_ID_ACCOUNT: &str = "install-id";

/// Result of resolving the install identifier.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallId {
    /// `false` when the OS keychain is unreachable, so nothing can be persisted.
    pub available: bool,
    /// The stable identifier; absent when the keychain is unavailable.
    pub install_id: Option<String>,
    /// Raw platform error, kept for diagnostics.
    pub warning: Option<String>,
}

fn entry() -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, INSTALL_ID_ACCOUNT)
}

/// Generate a fresh RFC 4122 version-4 UUID from the OS CSPRNG.
fn generate_install_id() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| AppError::Other(format!("无法生成安装标识：{error}")))?;
    // Version 4 (random) and RFC 4122 variant bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn unavailable(error: keyring::Error) -> InstallId {
    log::error!("system keychain unavailable for install id: {error}");
    InstallId {
        available: false,
        install_id: None,
        warning: Some(error.to_string()),
    }
}

/// Return the stable install identifier, creating and persisting it on first
/// use so it survives restarts and upgrades for the lifetime of the install.
///
/// The value only ever leaves the keychain through this command; callers must
/// not persist or log it elsewhere.
#[tauri::command]
pub fn get_install_id() -> Result<InstallId, AppError> {
    // Never read from keyring's non-persistent mock: fail loudly instead of
    // "generating" a new identifier on every launch.
    if let Some(reason) = keychain_init_error() {
        return Ok(InstallId {
            available: false,
            install_id: None,
            warning: Some(reason.to_string()),
        });
    }

    let entry = match entry() {
        Ok(entry) => entry,
        Err(error) => return Ok(unavailable(error)),
    };

    match entry.get_password() {
        // Reuse whatever was stored, even if another version wrote it.
        Ok(existing) if !existing.trim().is_empty() => Ok(InstallId {
            available: true,
            install_id: Some(existing),
            warning: None,
        }),
        // Empty or absent: mint one and persist it.
        Ok(_) | Err(keyring::Error::NoEntry) => {
            let install_id = generate_install_id()?;
            if let Err(error) = entry.set_password(&install_id) {
                return Ok(unavailable(error));
            }
            log::info!("generated a new install identifier");
            Ok(InstallId {
                available: true,
                install_id: Some(install_id),
                warning: None,
            })
        }
        Err(error) => Ok(unavailable(error)),
    }
}

/// The single-line JSON claim the user sends with their donation.
///
/// Field order matches the construction order for a stable, readable payload.
#[derive(Debug, Serialize)]
struct SponsorClaim {
    install_id: String,
    platform: String,
    username: String,
}

/// Build the single-line JSON claim the user copies and attaches to their
/// donation message, so the Pro activation code can be bound to this install.
///
/// Everything is assembled here in Rust — the frontend only displays the
/// returned string — and it is serialized with `serde_json` rather than
/// concatenated by hand. The nickname comes from the local profile cache and
/// the platform from the compile-time target OS.
#[tauri::command]
pub fn build_sponsor_claim(app: AppHandle) -> Result<String, AppError> {
    let status = get_install_id()?;
    let install_id = match status {
        InstallId {
            available: true,
            install_id: Some(install_id),
            ..
        } => install_id,
        status => {
            let reason = status.warning.unwrap_or_else(|| "未知错误".to_string());
            return Err(AppError::Store(format!("系统钥匙串不可用：{reason}")));
        }
    };

    let claim = SponsorClaim {
        install_id,
        platform: std::env::consts::OS.to_string(),
        username: local_nickname(&app).unwrap_or_default(),
    };

    Ok(serde_json::to_string(&claim)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_install_id_is_a_v4_uuid() {
        let id = generate_install_id().unwrap();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn generated_ids_are_unique() {
        assert_ne!(generate_install_id().unwrap(), generate_install_id().unwrap());
    }

    #[test]
    fn sponsor_claim_serializes_as_single_line_json() {
        let claim = SponsorClaim {
            install_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            platform: "windows".to_string(),
            username: "alice bob".to_string(),
        };
        let json = serde_json::to_string(&claim).unwrap();
        assert_eq!(
            json,
            r#"{"install_id":"550e8400-e29b-41d4-a716-446655440000","platform":"windows","username":"alice bob"}"#
        );
        assert!(!json.contains('\n'));
    }
}
