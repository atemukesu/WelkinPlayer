//! Audio metadata and cover-art cache commands (Stages 1.5 / 1.6).
//!
//! Metadata is parsed from a ranged GET of the file head, retrying with larger
//! ranges when the initial read is truncated. Both the parsed metadata
//! (`meta/{hash}.json`) and the resized cover (`covers/{hash}.jpg`) are cached
//! under the configured cache directory, so revisiting the library needs no
//! network requests at all for tracks that were read before.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::commands::webdav::require_credentials;
use crate::dav::{RemoteEntry, WebDavClient};
use crate::error::AppError;
use crate::metadata::{self, ParsedMetadata, TrackMetadata};

const SETTINGS_FILE: &str = "settings.json";
const CACHE_DIR_KEY: &str = "cache.dir";
const LIBRARY_CACHE_FILE: &str = "library.json";
const LIBRARY_HASH_FILE: &str = "library.hash";

/// Progressive read sizes (bytes). A truncated head is retried with a larger
/// `Range` before falling back to downloading the complete file.
const HEAD_RANGES: [u64; 2] = [1_572_864, 6_291_456]; // 1.5 / 6 MiB

pub(crate) fn lyric_path(track_path: &str) -> Result<String, AppError> {
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
pub(crate) fn resolve_cache_dir(app: &AppHandle) -> PathBuf {
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

/// Grant the asset protocol read access to the cache directory (covers are
/// served to the webview as plain URLs instead of base64 data URLs). Call this
/// at startup and whenever the cache directory changes.
pub(crate) fn allow_cache_dir(app: &AppHandle) {
    let dir = resolve_cache_dir(app);
    if let Err(error) = app.asset_protocol_scope().allow_directory(&dir, true) {
        log::warn!("failed to allow asset scope for {}: {error}", dir.display());
    }
}

/// Stable, path-derived cache id (16 hex chars).
pub(crate) fn cover_hash(path: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn cover_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("covers").join(format!("{hash}.jpg"))
}

fn meta_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("meta").join(format!("{hash}.json"))
}

fn read_meta(dir: &Path, hash: &str) -> Option<TrackMetadata> {
    let bytes = std::fs::read(meta_file(dir, hash)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn write_meta(dir: &Path, hash: &str, metadata: &TrackMetadata) -> Result<(), AppError> {
    let file = meta_file(dir, hash);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, serde_json::to_vec(metadata)?)?;
    Ok(())
}

pub(crate) fn write_cover(dir: &Path, hash: &str, bytes: &[u8]) -> Result<(), AppError> {
    let file = cover_file(dir, hash);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, bytes)?;
    Ok(())
}

fn lyric_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("lyrics").join(format!("{hash}.lrc"))
}

fn read_lyric(dir: &Path, hash: &str) -> Option<String> {
    std::fs::read_to_string(lyric_file(dir, hash)).ok()
}

pub(crate) fn write_lyric(dir: &Path, hash: &str, content: &str) -> Result<(), AppError> {
    let file = lyric_file(dir, hash);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, content)?;
    Ok(())
}

/// Delete the cached assets (metadata, cover, lyrics) for one remote track.
pub(crate) fn invalidate_track_assets(app: &AppHandle, path: &str) {
    let dir = resolve_cache_dir(app);
    let hash = cover_hash(path);
    let _ = std::fs::remove_file(meta_file(&dir, &hash));
    let _ = std::fs::remove_file(cover_file(&dir, &hash));
    let _ = std::fs::remove_file(lyric_file(&dir, &hash));
}

/// Stable hash over a listing's identity (path + size + modified time).
fn listing_hash(entries: &[RemoteEntry]) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in entries {
        entry.path.hash(&mut hasher);
        entry.size.hash(&mut hasher);
        entry.modified.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn read_library_cache(dir: &Path) -> Vec<RemoteEntry> {
    let file = dir.join(LIBRARY_CACHE_FILE);
    match std::fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
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

    allow_cache_dir(&app);
    Ok(resolve_cache_dir(&app).to_string_lossy().to_string())
}

/// Read the last successfully listed library (empty when none was cached).
#[tauri::command]
pub fn load_library_cache(app: AppHandle) -> Result<Vec<RemoteEntry>, AppError> {
    Ok(read_library_cache(&resolve_cache_dir(&app)))
}

/// Cache the latest WebDAV listing so the library can load offline next time.
#[tauri::command]
pub fn save_library_cache(app: AppHandle, entries: Vec<RemoteEntry>) -> Result<(), AppError> {
    let dir = resolve_cache_dir(&app);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(LIBRARY_CACHE_FILE), serde_json::to_vec(&entries)?)?;
    std::fs::write(dir.join(LIBRARY_HASH_FILE), listing_hash(&entries))?;
    Ok(())
}

/// Result of a manual library refresh.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    /// The freshly listed library.
    pub entries: Vec<RemoteEntry>,
    /// Whether the listing differs from the cached one.
    pub changed: bool,
    /// Paths whose assets must be re-read (new or modified).
    pub changed_paths: Vec<String>,
}

/// Re-list the WebDAV library immediately, comparing it with the cache by hash.
/// When the listing changed, changed/removed tracks have their cached assets
/// invalidated and the library cache is rewritten.
#[tauri::command]
pub async fn refresh_webdav_library(app: AppHandle) -> Result<RefreshResult, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let entries = client.list_audio_files("").await?;

    let dir = resolve_cache_dir(&app);
    let new_hash = listing_hash(&entries);
    let old_hash = std::fs::read_to_string(dir.join(LIBRARY_HASH_FILE))
        .ok()
        .map(|value| value.trim().to_string());
    let old_entries = read_library_cache(&dir);

    let old_meta: std::collections::HashMap<&str, (Option<u64>, Option<&str>)> = old_entries
        .iter()
        .map(|entry| (entry.path.as_str(), (entry.size, entry.modified.as_deref())))
        .collect();

    let mut changed_paths = Vec::new();
    for entry in &entries {
        let unchanged = old_meta
            .get(entry.path.as_str())
            .map(|(size, modified)| *size == entry.size && *modified == entry.modified.as_deref())
            .unwrap_or(false);
        if !unchanged {
            changed_paths.push(entry.path.clone());
        }
    }

    let changed = old_hash.as_deref() != Some(new_hash.as_str());
    if changed {
        for path in &changed_paths {
            invalidate_track_assets(&app, path);
        }
        let live: std::collections::HashSet<&str> =
            entries.iter().map(|entry| entry.path.as_str()).collect();
        for entry in &old_entries {
            if !live.contains(entry.path.as_str()) {
                invalidate_track_assets(&app, &entry.path);
            }
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(LIBRARY_CACHE_FILE), serde_json::to_vec(&entries)?)?;
        std::fs::write(dir.join(LIBRARY_HASH_FILE), &new_hash)?;
    }

    log::info!(
        "refreshed WebDAV library: {} files, changed={changed}",
        entries.len()
    );
    Ok(RefreshResult {
        entries,
        changed,
        changed_paths,
    })
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

/// Read the `.lrc` file beside a remote audio file, caching the result locally.
///
/// The cache is written only when the remote content differs from what is
/// stored, so unchanged lyrics are never rewritten. Parsing stays in the
/// frontend so the shared lyric library can handle all supported LRC variants.
#[tauri::command]
pub async fn read_track_lyrics(app: AppHandle, path: String) -> Result<String, AppError> {
    let credentials = require_credentials(&app)?;
    let client = WebDavClient::new(
        &credentials.url,
        &credentials.username,
        &credentials.password,
    )?;
    let remote = lyric_path(&path)?;
    let content = client.get(&remote).await?.text().await?;

    let dir = resolve_cache_dir(&app);
    let hash = cover_hash(&path);
    if read_lyric(&dir, &hash).as_deref() != Some(content.as_str()) {
        if let Err(error) = write_lyric(&dir, &hash, &content) {
            log::warn!("failed to cache lyrics for {path}: {error}");
        }
    }
    Ok(content)
}

/// Cache-only lyrics lookup keyed by the remote audio path (no network access).
#[tauri::command]
pub fn get_cached_lyrics(app: AppHandle, path: String) -> Result<Option<String>, AppError> {
    let dir = resolve_cache_dir(&app);
    Ok(read_lyric(&dir, &cover_hash(&path)))
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

pub(crate) fn cover_path_for(dir: &Path, hash: &str) -> Option<String> {
    let file = cover_file(dir, hash);
    file.is_file().then(|| file.to_string_lossy().to_string())
}

/// One track's on-disk cached assets, keyed by remote path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedTrack {
    /// Remote path the assets belong to.
    pub path: String,
    /// Cached text metadata, when present.
    pub metadata: Option<TrackMetadata>,
    /// Absolute path to the cached cover thumbnail, when present.
    pub cover_path: Option<String>,
}

/// Batch cache-only lookup: text metadata and cover path for many tracks in a
/// single IPC call, so the library can render fully from cache before any
/// network request is made.
#[tauri::command]
pub async fn load_cached_tracks(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<CachedTrack>, AppError> {
    let dir = resolve_cache_dir(&app);
    // Reading one JSON file per track must not run on the main thread once a
    // library holds thousands of entries.
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let hash = cover_hash(&path);
                CachedTrack {
                    metadata: read_meta(&dir, &hash),
                    cover_path: cover_path_for(&dir, &hash),
                    path,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| AppError::Other(format!("cached track task failed: {error}")))
}

/// Absolute path of a cached cover thumbnail, or `None` when not cached.
#[tauri::command]
pub fn cover_path(app: AppHandle, hash: String) -> Result<Option<String>, AppError> {
    if hash.is_empty() || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(AppError::invalid_argument(
            "hash",
            "must be a hexadecimal cache id",
        ));
    }
    Ok(cover_path_for(&resolve_cache_dir(&app), &hash))
}
