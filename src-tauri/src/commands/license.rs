// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Pro activation.
//!
//! The Pro activation code is a JWT (`header.payload.signature`) signed by the
//! developer with an Ed25519 private key. Everything security-relevant — the
//! signature check, the device binding, the expiry and the tier — is verified
//! here in Rust, never in the webview, so the check cannot be patched away by
//! editing frontend code.
//!
//! The signed payload carries the exact fields of the claim the user sent
//! (`install_id`, `username`, `platform`) plus `expires_at`, `tier` and
//! `signer`. Activation succeeds only when the signature verifies against the
//! embedded developer public key *and* the three bound fields match this
//! installation exactly.
//!
//! Only the raw code is persisted, in the OS keychain — never the verification
//! result. Every `get_pro_status` call re-derives the status from the code
//! (signature + device binding + expiry + tier), so a stored flag can neither be
//! forged nor go stale.

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::profile::local_nickname;
use crate::commands::sponsor::{get_install_id, InstallId};
use crate::commands::webdav::keychain_init_error;
use crate::error::AppError;

/// Keychain service/account holding the activation code. Kept separate from the
/// install-id (`...instid`) and WebDAV (`...webdav`) entries.
const SERVICE: &str = "com.atemukesu.welkinplayer.license";
const LICENSE_ACCOUNT: &str = "activation-code";

/// Developer's Ed25519 public key, base64-encoded (32 bytes).
///
/// This is the
/// counterpart of the private key the developer signs activation codes with;
/// it is safe to ship inside the app.
const PUBLIC_KEY_B64: &str = "jYKyuCr2qCcbjRVgGbA126XDxm9pNWFva/nIaCpCAAE=";

/// Threshold above which an `expires_at` value is treated as milliseconds
/// rather than seconds (roughly the year 5138, so no real timestamp is close).
const MILLIS_THRESHOLD: i64 = 100_000_000_000;

/// Claims embedded in a developer-signed activation code.
#[derive(Debug, Deserialize, Serialize)]
struct LicenseClaims {
    install_id: String,
    username: String,
    platform: String,
    expires_at: i64,
    tier: u32,
    signer: String,
}

/// The locally-bound values a signed code must reproduce exactly.
struct Device {
    install_id: String,
    username: String,
    platform: String,
}

/// Activation state returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProStatus {
    /// Whether a valid, unexpired, device-bound code is stored.
    pub active: bool,
    /// Developer identity from the signed payload.
    pub signer: Option<String>,
    /// Edition tier from the signed payload.
    pub tier: Option<u32>,
    /// Bound nickname from the signed payload.
    pub username: Option<String>,
    /// Bound platform from the signed payload.
    pub platform: Option<String>,
    /// Expiry as a Unix timestamp in seconds.
    pub expires_at: Option<i64>,
    /// Reason a stored code was rejected, when `active` is false.
    pub error: Option<String>,
}

impl ProStatus {
    fn inactive() -> Self {
        Self {
            active: false,
            signer: None,
            tier: None,
            username: None,
            platform: None,
            expires_at: None,
            error: None,
        }
    }

    fn inactive_with(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::inactive()
        }
    }

    fn from_claims(claims: &LicenseClaims) -> Self {
        Self {
            active: true,
            signer: Some(claims.signer.clone()),
            tier: Some(claims.tier),
            username: Some(claims.username.clone()),
            platform: Some(claims.platform.clone()),
            expires_at: Some(normalize_expiry(claims.expires_at)),
            error: None,
        }
    }
}

/// Decode a base64url JWT segment, tolerating optional padding.
fn decode_segment(segment: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(segment))
        .ok()
}

/// Count an `expires_at` value as seconds, accepting millisecond timestamps too.
fn normalize_expiry(value: i64) -> i64 {
    if value > MILLIS_THRESHOLD {
        value / 1000
    } else {
        value
    }
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolve the developer public key, or fail as "not developer-issued".
fn developer_key() -> Result<VerifyingKey, AppError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(PUBLIC_KEY_B64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(PUBLIC_KEY_B64))
        .map_err(|_| AppError::LicenseSigner)?;
    let array: [u8; 32] = bytes.as_slice().try_into().map_err(|_| AppError::LicenseSigner)?;
    VerifyingKey::from_bytes(&array).map_err(|_| AppError::LicenseSigner)
}

/// Verify a code end-to-end: signature, device binding, tier and expiry.
///
/// Kept free of Tauri state so it can be unit-tested with an arbitrary key.
fn verify_token(
    code: &str,
    public_key: &VerifyingKey,
    device: &Device,
    now: i64,
) -> Result<LicenseClaims, AppError> {
    let mut parts = code.trim().split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(AppError::LicenseSigner);
    };

    // The header must declare EdDSA; anything else was not issued by us.
    let header_bytes = decode_segment(header_b64).ok_or(AppError::LicenseSigner)?;
    let header: serde_json::Value =
        serde_json::from_slice(&header_bytes).map_err(|_| AppError::LicenseSigner)?;
    if header.get("alg").and_then(|value| value.as_str()) != Some("EdDSA") {
        return Err(AppError::LicenseSigner);
    }

    let signature_bytes = decode_segment(signature_b64).ok_or(AppError::LicenseSigner)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| AppError::LicenseSigner)?;

    let signing_input = format!("{header_b64}.{payload_b64}");
    public_key
        .verify_strict(signing_input.as_bytes(), &signature)
        .map_err(|_| AppError::LicenseSigner)?;

    let payload_bytes = decode_segment(payload_b64).ok_or(AppError::LicenseSigner)?;
    let claims: LicenseClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| AppError::LicenseSigner)?;

    // The signature is authentic; now require it to be bound to this install.
    if claims.install_id != device.install_id
        || claims.username != device.username
        || claims.platform != device.platform
    {
        return Err(AppError::LicenseDevice);
    }

    if claims.tier < 1 {
        return Err(AppError::LicenseTier);
    }

    if now >= normalize_expiry(claims.expires_at) {
        return Err(AppError::LicenseExpired);
    }

    Ok(claims)
}

/// Gather the values the code must match: install id, nickname and platform.
fn current_device(app: &AppHandle) -> Result<Device, AppError> {
    let install_id = match get_install_id()? {
        InstallId {
            available: true,
            install_id: Some(install_id),
            ..
        } => install_id,
        status => {
            log::warn!(
                "install identifier unavailable for license check: {}",
                status.warning.as_deref().unwrap_or("unknown keychain error")
            );
            return Err(AppError::LicenseDevice);
        }
    };

    Ok(Device {
        install_id,
        username: local_nickname(app).unwrap_or_default(),
        platform: std::env::consts::OS.to_string(),
    })
}

fn entry() -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, LICENSE_ACCOUNT)
}

/// Read the stored activation code from the OS keychain, if any.
fn read_stored_code() -> Option<String> {
    if keychain_init_error().is_some() {
        return None;
    }
    match entry().ok()?.get_password() {
        Ok(code) if !code.trim().is_empty() => Some(code),
        _ => None,
    }
}

/// Persist the activation code in the OS keychain.
fn store_code(code: &str) -> Result<(), AppError> {
    if let Some(reason) = keychain_init_error() {
        return Err(AppError::Store(format!("系统钥匙串不可用：{reason}")));
    }
    let entry = entry().map_err(|error| AppError::Store(format!("系统钥匙串不可用：{error}")))?;
    entry
        .set_password(code)
        .map_err(|error| AppError::Store(format!("无法保存激活码：{error}")))
}

/// How long a verification is held before the result is returned.
///
/// Keeps the frontend's verification animation honest and slows down repeated
/// guessing. The wait runs on the blocking pool, never on the UI thread.
const VERIFY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// Verify a code and, when valid, persist it. Pure blocking work.
fn verify_and_store(app: &AppHandle, code: &str) -> Result<ProStatus, AppError> {
    let key = developer_key()?;
    let device = current_device(app)?;
    let claims = verify_token(code, &key, &device, now_seconds())?;

    store_code(code)?;

    log::info!(
        "pro activation succeeded (tier {}, signer {})",
        claims.tier,
        claims.signer
    );
    Ok(ProStatus::from_claims(&claims))
}

/// Verify and persist a Pro activation code.
///
/// Returns the resulting status on success; on failure returns a structured
/// [`AppError`] whose code lets the frontend localize the exact reason. Either
/// way the result is held for [`VERIFY_DELAY`] so the animation can play out.
#[tauri::command]
pub async fn activate_pro(app: AppHandle, code: String) -> Result<ProStatus, AppError> {
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err(AppError::invalid_argument("code", "must not be empty"));
    }

    let status = tauri::async_runtime::spawn_blocking(move || {
        let result = verify_and_store(&app, &code);
        std::thread::sleep(VERIFY_DELAY);
        result
    })
    .await
    .map_err(|error| AppError::Other(format!("激活任务失败：{error}")))??;

    Ok(status)
}

/// Report the current Pro status, re-verifying any stored code from scratch.
#[tauri::command]
pub fn get_pro_status(app: AppHandle) -> Result<ProStatus, AppError> {
    let Some(code) = read_stored_code() else {
        return Ok(ProStatus::inactive());
    };

    let result = (|| {
        let key = developer_key()?;
        let device = current_device(&app)?;
        verify_token(&code, &key, &device, now_seconds())
    })();

    match result {
        Ok(claims) => Ok(ProStatus::from_claims(&claims)),
        Err(error) => {
            log::warn!("stored pro activation code is no longer valid: {error}");
            Ok(ProStatus::inactive_with(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ed25519_dalek::{Signer, SigningKey};

    /// Deterministic keypair for tests (a fixed 32-byte seed, no RNG needed).
    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn claims() -> LicenseClaims {
        LicenseClaims {
            install_id: "install-123".to_string(),
            username: "Alice".to_string(),
            platform: "windows".to_string(),
            expires_at: 4_102_444_800, // 2100-01-01
            tier: 1,
            signer: "Atemukesu".to_string(),
        }
    }

    fn token(signing_key: &SigningKey, claims: &LicenseClaims) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
        let signing_input = format!("{header}.{payload}");
        let signature = signing_key.sign(signing_input.as_bytes());
        format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        )
    }

    fn device() -> Device {
        Device {
            install_id: "install-123".to_string(),
            username: "Alice".to_string(),
            platform: "windows".to_string(),
        }
    }

    #[test]
    fn accepts_a_correctly_signed_bound_code() {
        let key = test_key();
        let code = token(&key, &claims());
        let parsed = verify_token(&code, &key.verifying_key(), &device(), 1_700_000_000).unwrap();
        assert_eq!(parsed.signer, "Atemukesu");
        assert_eq!(parsed.tier, 1);
    }

    #[test]
    fn rejects_a_code_signed_by_another_key() {
        let code = token(&test_key(), &claims());
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let error =
            verify_token(&code, &other.verifying_key(), &device(), 1_700_000_000).unwrap_err();
        assert_eq!(error.code(), "LICENSE_SIGNER");
    }

    #[test]
    fn rejects_a_tampered_payload() {
        let key = test_key();
        let code = token(&key, &claims());
        let mut parts: Vec<&str> = code.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA"}"#);
        parts[0] = &forged;
        let tampered = parts.join(".");
        let error =
            verify_token(&tampered, &key.verifying_key(), &device(), 1_700_000_000).unwrap_err();
        assert_eq!(error.code(), "LICENSE_SIGNER");
    }

    #[test]
    fn rejects_a_code_bound_to_another_device() {
        let key = test_key();
        let code = token(&key, &claims());
        let other = Device {
            install_id: "install-999".to_string(),
            username: "Alice".to_string(),
            platform: "windows".to_string(),
        };
        let error =
            verify_token(&code, &key.verifying_key(), &other, 1_700_000_000).unwrap_err();
        assert_eq!(error.code(), "LICENSE_DEVICE");
    }

    #[test]
    fn rejects_an_expired_code() {
        let key = test_key();
        let code = token(&key, &claims());
        let error =
            verify_token(&code, &key.verifying_key(), &device(), 4_102_444_800).unwrap_err();
        assert_eq!(error.code(), "LICENSE_EXPIRED");
    }

    #[test]
    fn accepts_millisecond_expiry() {
        let key = test_key();
        let mut value = claims();
        value.expires_at = 4_102_444_800_000; // same instant, in milliseconds
        let code = token(&key, &value);
        assert!(verify_token(&code, &key.verifying_key(), &device(), 1_700_000_000).is_ok());
    }

    #[test]
    fn rejects_an_unsupported_tier() {
        let key = test_key();
        let mut value = claims();
        value.tier = 0;
        let code = token(&key, &value);
        let error =
            verify_token(&code, &key.verifying_key(), &device(), 1_700_000_000).unwrap_err();
        assert_eq!(error.code(), "LICENSE_TIER");
    }

    #[test]
    fn rejects_malformed_tokens() {
        assert_eq!(
            verify_token("not-a-jwt", &test_key().verifying_key(), &device(), 0)
                .unwrap_err()
                .code(),
            "LICENSE_SIGNER"
        );
        assert_eq!(
            verify_token("a.b", &test_key().verifying_key(), &device(), 0)
                .unwrap_err()
                .code(),
            "LICENSE_SIGNER"
        );
    }
}
