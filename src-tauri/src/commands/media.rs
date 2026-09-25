//! Audio metadata and cover-art cache commands (Stages 1.5 / 1.6).
//!
//! Metadata is parsed from a ranged GET of the file head, retrying with larger
//! ranges when the initial read is truncated. Both the parsed metadata
//! (`meta/{hash}.json`) and the resized cover (`covers/{hash}.jpg`) are cached
//! under the configured cache directory, so revisiting the library needs no
//! network requests at all for tracks that were read before.

use std::path::{Path, PathBuf};

use base64::Engine;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::commands::webdav::require_credentials;
use crate::dav::{RemoteEntry, WebDavClient};
use crate::error::AppError;
use crate::metadata::{self, ParsedMetadata, TrackMetadata};

const SETTINGS_FILE: &str = "settings.json";
const CACHE_DIR_KEY: &str = "cache.dir";
const LIBRARY_CACHE_FILE: &str = "library.json";

/// Progressive read sizes (bytes). A truncated head is retried with a larger
/// `Range` before falling back to downloading the complete file.
const HEAD_RANGES: [u64; 2] = [1_572_864, 6_291_456]; // 1.5 / 6 MiB

fn lyric_path(track_path: &str) -> Result<String, AppError> {
    let slash = track_path
        .rfind('/')
        .ok_or_else(|| AppError::invalid_argument("path", "must contain an audio file name"))?;
    let file_name = &track_path[slash + 1..];
    let dot = file_name.rfind('.').ok_or_else(|| {
        AppError::invalid_argument("path", "must include an audio file extension")
    })?;

    Ok(format!(
        "{}{}.lrc",
        &track_path[..slash + 1],
        &file_name[..dot]
    ))
}

fn default_cache_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_cache_dir()
        .map_err(|error| AppError::Store(format!("无法获取缓存目录：{error}")))
}

/// Configured cache directory, falling back to the app cache dir.
fn resolve_cache_dir(app: &AppHandle) -> PathBuf {
    let configured = app
        .store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(CACHE_DIR_KEY))
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty());

    match configured {
        Some(dir) => PathBuf::from(dir),
        None => default_cache_dir(app).unwrap_or_else(|_| PathBuf::from("welkin-cache")),
    }
}

/// Stable, path-derived cache id (16 hex chars).
fn cover_hash(path: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn cover_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("covers").join(format!("{hash}.jpg"))
}

fn meta_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("meta").join(format!("{hash}.json"))
}

fn read_meta(dir: &Path, hash: &str) -> Option<TrackMetadata> {
    let bytes = std::fs::read(meta_file(dir, hash)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_meta(dir: &Path, hash: &str, metadata: &TrackMetadata) -> Result<(), AppError> {
    let file = meta_file(dir, hash);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, serde_json::to_vec(metadata)?)?;
    Ok(())
}

fn write_cover(dir: &Path, hash: &str, bytes: &[u8]) -> Result<(), AppError> {
    let file = cover_file(dir, hash);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, bytes)?;
    Ok(())
}

fn read_cover_file(file: &Path) -> Result<Option<String>, AppError> {
    if !file.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(file)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(format!("data:image/jpeg;base64,{encoded}")))
}

/// Current cache directory (sub-directories are created lazily when writing).
#[tauri::command]
pub fn get_cache_dir(app: AppHandle) -> Result<String, AppError> {
    Ok(resolve_cache_dir(&app).to_string_lossy().to_string())
}

/// Persist a user-chosen cache directory; an empty value restores the default.
#[tauri::command]
pub fn set_cache_dir(app: AppHandle, dir: String) -> Result<String, AppError> {
    let store = app.store(SETTINGS_FILE)?;
    let value = dir.trim();

    if value.is_empty() {
        store.delete(CACHE_DIR_KEY);
    } else {
        store.set(CACHE_DIR_KEY, value);
    }
    store.save()?;

    Ok(resolve_cache_dir(&app).to_string_lossy().to_string())
}

/// Read the last successfully listed library (empty when none was cached).
#[tauri::command]
pub fn load_library_cache(app: AppHandle) -> Result<Vec<RemoteEntry>, AppError> {
    let file = resolve_cache_dir(&app).join(LIBRARY_CACHE_FILE);
    if !file.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(file)?;
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

/// Cache the latest WebDAV listing so the library can load offline next time.
#[tauri::command]
pub fn save_library_cache(app: AppHandle, entries: Vec<RemoteEntry>) -> Result<(), AppError> {
    let dir = resolve_cache_dir(&app);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(LIBRARY_CACHE_FILE), serde_json::to_vec(&entries)?)?;
    Ok(())
}

/// Download and parse the complete remote track.
async fn fetch_complete_metadata(
    client: &WebDavClient,
    path: &str,
) -> Result<ParsedMetadata, AppError> {
    let bytes = client.get(path).await?.bytes().await?;
    metadata::parse(&bytes)
}

/// Try increasing range sizes before downloading the complete track.
async fn fetch_metadata(
    client: &WebDavClient,
    path: &str,
) -> Result<Option<ParsedMetadata>, AppError> {
    let mut last_error: Option<AppError> = None;

    for size in HEAD_RANGES {
        match client.get_range(path, size).await {
            Ok(bytes) if !bytes.is_empty() => match metadata::parse(&bytes) {
                Ok(parsed) => {
                    let metadata = &parsed.metadata;
                    let useful = metadata.title.is_some()
                        || metadata.artist.is_some()
                        || metadata.album.is_some()
                        || metadata.duration_secs.is_some()
                        || parsed.cover.is_some();
                    if useful {
                        log::debug!("read metadata for {path} from {size} bytes");
                        return Ok(Some(parsed));
                    }
                }
                Err(error) => last_error = Some(error),
            },
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
    }

    match fetch_complete_metadata(client, path).await {
        Ok(parsed) => {
            log::debug!("read metadata for {path} from complete file");
            Ok(Some(parsed))
        }
        Err(error) => match last_error {
            Some(range_error) => Err(range_error),
            None => Err(error),
        },
    }
}

/// Read a remote track's metadata, serving it from the on-disk cache when
/// available (no network access in that case).
#[tauri::command]
pub async fn read_track_metadata(app: AppHandle, path: String) -> Result<TrackMetadata, AppError> {
    let dir = resolve_cache_dir(&app);
    let hash = cover_hash(&path);

    if let Some(cached) = read_meta(&dir, &hash) {
        return Ok(cached);
    }

    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;

    let parsed = match fetch_metadata(&client, &path).await? {
        Some(parsed) => parsed,
        None => return Ok(TrackMetadata::default()),
    };
    let mut result = parsed.metadata;

    if let Some(cover) = parsed.cover {
        match metadata::encode_thumbnail(&cover) {
            Ok(thumbnail) => match write_cover(&dir, &hash, &thumbnail) {
                Ok(()) => result.cover_hash = Some(hash.clone()),
                Err(error) => log::warn!("failed to cache cover for {path}: {error}"),
            },
            Err(error) => log::warn!("failed to encode cover for {path}: {error}"),
        }
    }

    write_meta(&dir, &hash, &result)?;
    Ok(result)
}

/// Download a complete remote track before extracting its metadata.
///
/// This is an explicit, user-triggered fallback for formats whose metadata is
/// not available in the file head. The result replaces any cached entry.
#[tauri::command]
pub async fn download_track_metadata(
    app: AppHandle,
    path: String,
) -> Result<TrackMetadata, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let parsed = fetch_complete_metadata(&client, &path).await?;
    let dir = resolve_cache_dir(&app);
    let hash = cover_hash(&path);
    let mut result = parsed.metadata;

    if let Some(cover) = parsed.cover {
        match metadata::encode_thumbnail(&cover) {
            Ok(thumbnail) => match write_cover(&dir, &hash, &thumbnail) {
                Ok(()) => result.cover_hash = Some(hash.clone()),
                Err(error) => log::warn!("failed to cache cover for {path}: {error}"),
            },
            Err(error) => log::warn!("failed to encode cover for {path}: {error}"),
        }
    }

    write_meta(&dir, &hash, &result)?;
    Ok(result)
}

/// Read the `.lrc` file beside a remote audio file. Parsing stays in the
/// frontend so the shared lyric library can handle all supported LRC variants.
#[tauri::command]
pub async fn read_track_lyrics(app: AppHandle, path: String) -> Result<String, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let path = lyric_path(&path)?;
    Ok(client.get(&path).await?.text().await?)
}

/// Cache-only metadata lookup keyed by remote path (no network access).
#[tauri::command]
pub fn get_cached_metadata(
    app: AppHandle,
    path: String,
) -> Result<Option<TrackMetadata>, AppError> {
    let dir = resolve_cache_dir(&app);
    Ok(read_meta(&dir, &cover_hash(&path)))
}

/// Return a cached cover as a data URL, or `None` when it is not cached.
#[tauri::command]
pub fn get_cover(app: AppHandle, hash: String) -> Result<Option<String>, AppError> {
    if hash.is_empty() || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(AppError::invalid_argument(
            "hash",
            "must be a hexadecimal cache id",
        ));
    }
    read_cover_file(&cover_file(&resolve_cache_dir(&app), &hash))
}

/// Cache-first cover lookup keyed by the remote path, with no network access.
#[tauri::command]
pub fn get_cached_cover(app: AppHandle, path: String) -> Result<Option<String>, AppError> {
    let dir = resolve_cache_dir(&app);
    read_cover_file(&cover_file(&dir, &cover_hash(&path)))
}
