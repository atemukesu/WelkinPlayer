// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Track metadata editing.
//!
//! Editing rewrites the tags inside the audio container itself: the track is
//! read from its source, its tags changed in memory, then the modified bytes are
//! written back (uploaded for WebDAV, written in place for local files). The
//! local metadata/cover cache is refreshed in the same step.

use base64::Engine;
use lofty::picture::MimeType;
use tauri::AppHandle;

use super::media::{asset_hash, cover_file, resolve_cache_dir, write_cover, write_meta};
use crate::data::backend::backend_for;
use crate::core::error::AppError;
use crate::data::metadata::{self, TrackMetadata, TrackTags};

/// Hard cap on a newly embedded cover image, guarding against pathological uploads.
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;

/// Decode a `data:image/...;base64,...` URL into its MIME type and bytes.
fn decode_cover_data_url(data_url: &str) -> Result<(MimeType, Vec<u8>), AppError> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| AppError::invalid_argument("coverDataUrl", "must be a base64 data URL"))?;

    let mime = meta
        .strip_prefix("data:")
        .and_then(|value| value.split(';').next())
        .filter(|value| value.starts_with("image/"))
        .ok_or_else(|| AppError::invalid_argument("coverDataUrl", "must be an image data URL"))?;
    if !meta.contains(";base64") {
        return Err(AppError::invalid_argument(
            "coverDataUrl",
            "must be base64 encoded",
        ));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|error| AppError::Other(format!("无法解码封面图片：{error}")))?;

    if bytes.is_empty() {
        return Err(AppError::invalid_argument("coverDataUrl", "image is empty"));
    }
    if bytes.len() > MAX_COVER_BYTES {
        return Err(AppError::invalid_argument("coverDataUrl", "image is too large"));
    }

    let mime_type = MimeType::from_str(mime);
    Ok((mime_type, bytes))
}

/// Best-effort audio content type from the file extension.
fn content_type_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "wma" => "audio/x-ms-wma",
        _ => "application/octet-stream",
    }
}

/// Refresh the local metadata/cover cache after an edit and return the metadata.
fn cache_metadata(
    app: &AppHandle,
    source_id: &str,
    path: &str,
    mut metadata: TrackMetadata,
    cover: Option<Vec<u8>>,
) -> TrackMetadata {
    let dir = resolve_cache_dir(app);
    let hash = asset_hash(source_id, path);

    if let Some(cover) = cover {
        match metadata::encode_thumbnail(&cover) {
            Ok(thumbnail) => match write_cover(&dir, &hash, &thumbnail) {
                Ok(()) => metadata.cover_hash = Some(hash.clone()),
                Err(error) => log::warn!("failed to cache cover for {path}: {error}"),
            },
            Err(error) => log::warn!("failed to encode cover for {path}: {error}"),
        }
    } else {
        metadata.cover_hash = None;
        let _ = std::fs::remove_file(cover_file(&dir, &hash));
    }

    if let Err(error) = write_meta(&dir, &hash, &metadata) {
        log::warn!("failed to cache metadata for {path}: {error}");
    }
    metadata
}

/// Read every editable tag from a track (downloads the complete file).
#[tauri::command]
pub async fn read_track_tags(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<TrackTags, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let bytes = backend.read(&path).await?;
    metadata::read_tags(&bytes)
}

/// Rewrite every tag (and optionally the cover) and store the track back.
#[tauri::command]
pub async fn edit_track_metadata(
    app: AppHandle,
    source_id: String,
    path: String,
    tags: TrackTags,
    cover_data_url: Option<String>,
    remove_cover: bool,
) -> Result<TrackMetadata, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let original = backend.read(&path).await?;

    let cover = match cover_data_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(data_url) => Some(decode_cover_data_url(data_url)?),
        None => None,
    };

    let edited = metadata::write_tags(&original, &tags, cover, remove_cover)?;
    backend
        .put(&path, content_type_for(&path), edited.clone())
        .await?;
    log::info!(
        "edited metadata for {path} ({} -> {} bytes)",
        original.len(),
        edited.len()
    );

    let parsed = metadata::parse(&edited)?;
    Ok(cache_metadata(&app, &source_id, &path, parsed.metadata, parsed.cover))
}
