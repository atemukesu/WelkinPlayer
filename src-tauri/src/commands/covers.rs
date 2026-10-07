// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Standalone playlist cover files.
//!
//! User-uploaded playlist covers are *not* embedded in the profile JSON as data
//! URLs. Instead the (already downscaled) image is uploaded to WebDAV as its own
//! object named from the SHA-256 of its bytes, so the name can never collide
//! with another cover, another user file or the profile/playback documents. The
//! profile only stores the file name; the bytes are cached on disk exactly like
//! track cover thumbnails and served to the webview through the asset protocol.

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::data::backend::{sync_backend, Backend};
use crate::commands::media::{cover_file, cover_path_for, resolve_cache_dir, write_cover};
use crate::core::error::AppError;
use crate::data::metadata;

/// Prefix that scopes cover objects away from audio files and profile documents.
const COVER_PREFIX: &str = "welkin-cover-";
/// Hard cap on the decoded image, guarding against pathological uploads.
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;

/// Result of uploading a playlist cover.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedCover {
    /// Remote file name to store in the profile (e.g. `welkin-cover-<sha>.jpg`).
    pub file: String,
    /// Content hash, also the local cache id.
    pub hash: String,
    /// Absolute path of the freshly cached cover, when it could be written.
    pub path: Option<String>,
}

/// One playlist cover resolved to a local asset-protocol path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedCover {
    pub file: String,
    pub path: Option<String>,
}

/// The backend of the configured sync source, or `None` when none is set.
fn client(app: &AppHandle) -> Result<Option<Backend>, AppError> {
    sync_backend(app)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn file_for_hash(hash: &str) -> String {
    format!("{COVER_PREFIX}{hash}.jpg")
}

/// Extract the content hash from a cover file name, rejecting anything that does
/// not match the naming scheme this module produces.
fn hash_from_file(file: &str) -> Option<String> {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let hash = name.strip_prefix(COVER_PREFIX)?.strip_suffix(".jpg")?;
    (hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
}

/// Strip a `data:image/...;base64,` prefix and decode the payload.
fn decode_data_url(data_url: &str) -> Result<Vec<u8>, AppError> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| AppError::invalid_argument("dataUrl", "must be a base64 data URL"))?;

    if !meta.starts_with("data:image/") || !meta.contains(";base64") {
        return Err(AppError::invalid_argument(
            "dataUrl",
            "must be a base64 image data URL",
        ));
    }

    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload.trim())
        .map_err(|error| AppError::Other(format!("无法解码封面图片：{error}")))?;

    if bytes.is_empty() {
        return Err(AppError::invalid_argument("dataUrl", "image is empty"));
    }
    if bytes.len() > MAX_COVER_BYTES {
        return Err(AppError::invalid_argument("dataUrl", "image is too large"));
    }
    Ok(bytes)
}

/// Decode, normalize and store a playlist cover.
///
/// The image is re-encoded as a JPEG thumbnail (also bounding its size), named
/// after the SHA-256 of the encoded bytes and cached locally. Uploading the same
/// picture twice reuses the same object, so the operation is idempotent.
#[tauri::command]
pub async fn upload_playlist_cover(
    app: AppHandle,
    data_url: String,
) -> Result<UploadedCover, AppError> {
    let raw = decode_data_url(&data_url)?;
    let jpeg = metadata::encode_thumbnail(&raw)?;
    let hash = sha256_hex(&jpeg);
    let file = file_for_hash(&hash);

    let dir = resolve_cache_dir(&app);
    write_cover(&dir, &hash, &jpeg)?;
    let path = cover_path_for(&dir, &hash);

    // A failed upload (offline, keychain) must not block local editing: the
    // cover already renders from cache and the profile still references it.
    match client(&app) {
        Ok(Some(client)) => {
            if let Err(error) = client.put(&file, "image/jpeg", jpeg).await {
                log::warn!("failed to upload playlist cover {file}: {error}");
            }
        }
        Ok(None) => log::info!("no sync source; playlist cover cached locally only"),
        Err(error) => log::warn!("skipping playlist cover upload: {error}"),
    }

    Ok(UploadedCover { file, hash, path })
}

/// Resolve many cover file names to local asset paths in one round trip.
///
/// Cached covers are returned immediately; the rest are downloaded (when the
/// server is reachable) and cached. Missing/offline covers resolve to `null`
/// so the UI can fall back to a track icon.
#[tauri::command]
pub async fn resolve_playlist_covers(
    app: AppHandle,
    files: Vec<String>,
) -> Result<Vec<ResolvedCover>, AppError> {
    let dir = resolve_cache_dir(&app);
    let mut resolved = Vec::new();
    let mut pending = Vec::new();

    for file in files {
        if let Some(hash) = hash_from_file(&file) {
            if let Some(path) = cover_path_for(&dir, &hash) {
                resolved.push(ResolvedCover {
                    file,
                    path: Some(path),
                });
                continue;
            }
        }
        pending.push(file);
    }

    if pending.is_empty() {
        return Ok(resolved);
    }

    let client = match client(&app) {
        Ok(Some(client)) => client,
        Ok(None) => {
            resolved.extend(
                pending
                    .into_iter()
                    .map(|file| ResolvedCover { file, path: None }),
            );
            return Ok(resolved);
        }
        Err(error) => {
            log::warn!("cannot resolve playlist covers: {error}");
            resolved.extend(
                pending
                    .into_iter()
                    .map(|file| ResolvedCover { file, path: None }),
            );
            return Ok(resolved);
        }
    };

    for file in pending {
        let Some(hash) = hash_from_file(&file) else {
            resolved.push(ResolvedCover { file, path: None });
            continue;
        };
        let path = match client.read(&file).await {
            Ok(bytes) => match write_cover(&dir, &hash, &bytes) {
                Ok(()) => cover_path_for(&dir, &hash),
                Err(error) => {
                    log::warn!("failed to cache playlist cover {file}: {error}");
                    None
                }
            },
            Err(error) => {
                log::warn!("failed to download playlist cover {file}: {error}");
                None
            }
        };
        resolved.push(ResolvedCover { file, path });
    }

    Ok(resolved)
}

/// Delete a cover object and its local cache entry.
///
/// The caller is responsible for ensuring no other playlist still references the
/// same content hash (identical images share one object).
#[tauri::command]
pub async fn delete_playlist_cover(app: AppHandle, file: String) -> Result<(), AppError> {
    if let Some(hash) = hash_from_file(&file) {
        let dir = resolve_cache_dir(&app);
        let _ = std::fs::remove_file(cover_file(&dir, &hash));
    }

    match client(&app) {
        Ok(Some(client)) => {
            if let Err(error) = client.delete(&file).await {
                log::warn!("failed to delete playlist cover {file}: {error}");
            }
        }
        Ok(None) => log::info!("no sync source; playlist cover cache cleared only"),
        Err(error) => log::warn!("skipping playlist cover delete: {error}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_parses_cover_file_names() {
        let hash = sha256_hex(b"hello");
        let file = file_for_hash(&hash);
        assert!(file.starts_with(COVER_PREFIX));
        assert_eq!(hash_from_file(&file).as_deref(), Some(hash.as_str()));
        assert_eq!(hash_from_file("random.jpg"), None);
        assert_eq!(hash_from_file(&format!("{COVER_PREFIX}abc.jpg")), None);
    }
}
