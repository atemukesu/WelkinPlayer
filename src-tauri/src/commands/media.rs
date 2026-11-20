// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Audio metadata and cover-art cache commands.
//!
//! Metadata is parsed from a ranged read of the file head, retrying with larger
//! ranges when the initial read is truncated. Both the parsed metadata
//! (`meta/{hash}.json`) and the resized cover (`covers/{hash}.jpg`) are cached
//! under the configured cache directory, so revisiting the library needs no
//! source access at all for tracks that were read before.
//!
//! Every cache id is derived from the pair `(source id, path)`, so two sources
//! with identical paths never share a cache entry.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine;
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::core::error::AppError;
use crate::data::backend::{backend_for, Backend};
use crate::data::dav::RemoteEntry;
use crate::data::metadata::{self, ParsedMetadata, TrackMetadata};

const SETTINGS_FILE: &str = "settings.json";
const CACHE_DIR_KEY: &str = "cache.dir";
/// Per-source library cache file prefix (suffixed with the source hash).
const LIBRARY_CACHE_PREFIX: &str = "library-";
const LIBRARY_HASH_PREFIX: &str = "library-hash-";
/// Candidate listing kept while a suspicious shrink is confirmed by a second
/// independent scan.
const LIBRARY_PENDING_PREFIX: &str = "library-pending-";

/// A fresh listing may only replace the cache when it keeps at least this
/// percentage of the previous tracks. A sudden drop is far more likely to be a
/// partial/throttled PROPFIND response than a real mass deletion.
const SHRINK_GUARD_PERCENT: usize = 80;
/// Tiny libraries change all the time; only guard listings at least this large.
const SHRINK_GUARD_MIN_ENTRIES: usize = 20;
/// Ignore drops smaller than this even when the percentage threshold trips.
const SHRINK_GUARD_MIN_LOSS: usize = 5;
/// Share of paths two scans must agree on before a shrink is accepted.
const SHRINK_CONFIRM_PERCENT: usize = 98;

/// Progressive read sizes (bytes). A truncated head is retried with a larger
/// `Range` before falling back to downloading the complete file.
const HEAD_RANGES: [u64; 2] = [1_572_864, 6_291_456]; // 1.5 / 6 MiB

pub(crate) fn lyric_path(track_path: &str) -> Result<String, AppError> {
    let slash = track_path.rfind('/').unwrap_or(usize::MAX);
    // When the audio file sits at the source root there is no directory
    // separator; treat the whole path as the file name.
    let (dir, file_name) = if slash == usize::MAX {
        ("", track_path)
    } else {
        (&track_path[..slash + 1], &track_path[slash + 1..])
    };
    let dot = file_name.rfind('.').ok_or_else(|| {
        AppError::invalid_argument("path", "must include an audio file extension")
    })?;

    Ok(format!("{}{}.lrc", dir, &file_name[..dot]))
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

/// Grant the asset protocol read access to the cache directory.
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

/// Cache id for a `(source, path)` pair.
pub(crate) fn asset_hash(source_id: &str, path: &str) -> String {
    cover_hash(&format!("{source_id}\u{1f}{path}"))
}

/// Cache id for a source's library listing.
pub(crate) fn source_cache_hash(source_id: &str) -> String {
    cover_hash(&format!("source\u{1f}{source_id}"))
}

pub(crate) fn cover_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("covers").join(format!("{hash}.jpg"))
}

fn meta_file(dir: &Path, hash: &str) -> PathBuf {
    dir.join("meta").join(format!("{hash}.json"))
}

/// Read a track's cached parsed metadata (title/artist/album) from disk.
pub(crate) fn read_meta(dir: &Path, hash: &str) -> Option<TrackMetadata> {
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

/// Delete the cached assets (metadata, cover, lyrics) for one track.
///
/// Only called when the file itself changed (same path, new size/mtime). A file
/// vanishing from one listing is not a reason to drop its assets; see
/// [`changed_asset_paths`].
pub(crate) fn invalidate_track_assets(app: &AppHandle, source_id: &str, path: &str) {
    let dir = resolve_cache_dir(app);
    let hash = asset_hash(source_id, path);
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

fn library_cache_file(dir: &Path, source_id: &str) -> PathBuf {
    dir.join(format!(
        "{LIBRARY_CACHE_PREFIX}{}.json",
        source_cache_hash(source_id)
    ))
}

fn library_hash_file(dir: &Path, source_id: &str) -> PathBuf {
    dir.join(format!(
        "{LIBRARY_HASH_PREFIX}{}.txt",
        source_cache_hash(source_id)
    ))
}

fn library_pending_file(dir: &Path, source_id: &str) -> PathBuf {
    dir.join(format!(
        "{LIBRARY_PENDING_PREFIX}{}.json",
        source_cache_hash(source_id)
    ))
}

fn read_library_cache(dir: &Path, source_id: &str) -> Vec<RemoteEntry> {
    let file = library_cache_file(dir, source_id);
    match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Vec<RemoteEntry>>(&bytes) {
            Ok(entries) => dedupe_entries(entries),
            Err(error) => {
                // A truncated file must never be mistaken for an empty library.
                log::warn!(
                    "ignoring unreadable library cache {}: {error}",
                    file.display()
                );
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    }
}

/// Identity of a physical file inside a source, independent of the `href`
/// spelling a server happened to use (trailing slash, percent-encoding, ...).
fn object_key(entry: &RemoteEntry) -> String {
    let local = entry.local_path.trim_matches('/');
    if local.is_empty() {
        entry.path.trim_matches('/').to_string()
    } else {
        local.to_string()
    }
}

/// Drop repeated copies of the *same object*, keeping the first occurrence.
///
/// This never merges two distinct files that merely share a name; it only
/// removes rows that a server (or an older cache) reported twice.
fn dedupe_entries(entries: Vec<RemoteEntry>) -> Vec<RemoteEntry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|entry| seen.insert(object_key(entry)))
        .collect()
}

/// Write a cache file atomically so a concurrent reader can never observe a
/// truncated JSON document.
fn write_atomic(file: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = file.with_extension("tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, file)?;
    Ok(())
}

/// Whether a fresh listing is suspiciously smaller than the cached one.
fn is_suspicious_shrink(old_len: usize, new_len: usize) -> bool {
    old_len >= SHRINK_GUARD_MIN_ENTRIES
        && old_len.saturating_sub(new_len) >= SHRINK_GUARD_MIN_LOSS
        && new_len.saturating_mul(100) < old_len.saturating_mul(SHRINK_GUARD_PERCENT)
}

/// Whether a previously saved candidate and a new scan describe essentially
/// the same (smaller) library. Requiring two scans to agree stops a single
/// throttled/partial response from ever replacing the cache.
fn listings_match(pending: &[RemoteEntry], entries: &[RemoteEntry]) -> bool {
    if pending.len().abs_diff(entries.len()) > 2 {
        return false;
    }
    let pending_paths: std::collections::HashSet<String> = pending.iter().map(object_key).collect();
    let new_paths: std::collections::HashSet<String> = entries.iter().map(object_key).collect();
    if pending_paths.is_empty() && new_paths.is_empty() {
        return true;
    }
    let common = pending_paths.intersection(&new_paths).count();
    common.saturating_mul(100) >= pending_paths.len().max(new_paths.len()) * SHRINK_CONFIRM_PERCENT
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

/// Read the last successfully listed library for a source.
#[tauri::command]
pub fn load_library_cache(app: AppHandle, source_id: String) -> Result<Vec<RemoteEntry>, AppError> {
    Ok(read_library_cache(&resolve_cache_dir(&app), &source_id))
}

/// Cache the latest listing of a source so the library can load offline.
#[tauri::command]
pub fn save_library_cache(
    app: AppHandle,
    source_id: String,
    entries: Vec<RemoteEntry>,
) -> Result<(), AppError> {
    let dir = resolve_cache_dir(&app);
    let entries = dedupe_entries(entries);
    write_atomic(
        &library_cache_file(&dir, &source_id),
        &serde_json::to_vec(&entries)?,
    )?;
    write_atomic(
        &library_hash_file(&dir, &source_id),
        listing_hash(&entries).as_bytes(),
    )?;
    Ok(())
}

/// Result of a manual library refresh.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    pub entries: Vec<RemoteEntry>,
    pub changed: bool,
    /// Files whose size/mtime changed, so their cached assets were dropped.
    /// Songs that only disappeared from this scan are never listed here.
    pub changed_paths: Vec<String>,
    /// The fresh scan was not trusted as a complete snapshot, so the on-disk
    /// cache (and all cached metadata/covers/lyrics) was deliberately left
    /// untouched. A subsequent refresh that agrees with the smaller result will
    /// accept it.
    pub protected: bool,
    /// Directories that could not be enumerated during this scan.
    pub failed_paths: Vec<String>,
    /// Stable reason for `protected`: `"incomplete"` or `"shrink"`.
    pub warning: Option<String>,
}

/// One refresh at a time: concurrent refreshes of the same source would race on
/// the cache files and could invalidate assets a sibling scan is still using.
static REFRESH_LOCK: OnceLock<tauri::async_runtime::Mutex<()>> = OnceLock::new();

/// Append cached entries the fresh scan could not verify, so a transient
/// failure does not make existing songs vanish from the UI.
fn extend_with_cached(
    entries: &mut Vec<RemoteEntry>,
    cached: &[RemoteEntry],
    only_under: Option<&[String]>,
) {
    let mut known: std::collections::HashSet<String> = entries.iter().map(object_key).collect();
    for previous in cached {
        if let Some(directories) = only_under {
            let under_failed = directories.iter().any(|directory| {
                let directory = directory.trim_matches('/');
                previous.local_path == directory
                    || (previous.local_path.starts_with(directory)
                        && previous.local_path.as_bytes().get(directory.len()) == Some(&b'/'))
            });
            if !under_failed {
                continue;
            }
        }
        if known.insert(object_key(previous)) {
            entries.push(previous.clone());
        }
    }
    entries.sort_by(|left, right| {
        left.local_path
            .cmp(&right.local_path)
            .then_with(|| left.path.cmp(&right.path))
    });
}

/// Paths whose cached assets (metadata, cover, lyrics) are provably stale.
///
/// Only files that still exist in the new listing *and* whose size or
/// modification time changed are returned. A file that merely disappeared from
/// a listing is deliberately excluded: WebDAV servers can return a short but
/// syntactically valid listing, and treating "missing from this scan" as
/// "deleted" used to wipe `meta/` and `covers/` for songs that were still there.
/// The next launch then had to download every one of them again. Listing
/// membership is guarded by the library cache; assets are keyed by file
/// identity, not by whether a particular scan happened to see the file.
fn changed_asset_paths(entries: &[RemoteEntry], cached: &[RemoteEntry]) -> Vec<String> {
    let cached_meta: std::collections::HashMap<&str, (Option<u64>, Option<&str>)> = cached
        .iter()
        .map(|entry| (entry.path.as_str(), (entry.size, entry.modified.as_deref())))
        .collect();

    entries
        .iter()
        .filter(|entry| match cached_meta.get(entry.path.as_str()) {
            Some((size, modified)) => *size != entry.size || *modified != entry.modified.as_deref(),
            // Never seen before: there is no cached asset to invalidate.
            None => false,
        })
        .map(|entry| entry.path.clone())
        .collect()
}

/// Re-list a source immediately, comparing it with the cache by hash.
///
/// The cache is only replaced by a *complete* scan that does not look like a
/// partial response. When a scan is incomplete or shrinks suspiciously, the
/// fresh entries are still returned for display (plus anything the cache says
/// lives under a directory that failed), but no cache file or cached asset is
/// touched. A suspicious shrink is remembered and accepted once a second scan
/// agrees, so a real deletion still propagates.
#[tauri::command]
pub async fn refresh_source_library(
    app: AppHandle,
    source_id: String,
) -> Result<RefreshResult, AppError> {
    let _guard = REFRESH_LOCK
        .get_or_init(|| tauri::async_runtime::Mutex::new(()))
        .lock()
        .await;

    let backend = backend_for(&app, &source_id)?;
    let listing = backend.list_audio().await?;
    let incomplete = !listing.is_complete();
    let failed_paths = listing.failed_paths;
    let mut entries = dedupe_entries(listing.entries);

    let dir = resolve_cache_dir(&app);
    let new_hash = listing_hash(&entries);
    let old_hash = std::fs::read_to_string(library_hash_file(&dir, &source_id))
        .ok()
        .map(|value| value.trim().to_string());
    let old_entries = read_library_cache(&dir, &source_id);

    // Only files whose content actually changed lose their cached metadata,
    // cover, and lyrics. Songs that a short scan failed to report keep their
    // assets and are served from disk when they reappear.
    let changed_paths = changed_asset_paths(&entries, &old_entries);
    let changed = old_hash.as_deref() != Some(new_hash.as_str());

    let pending_file = library_pending_file(&dir, &source_id);
    let mut protected = false;
    let mut warning: Option<String> = None;

    if incomplete {
        protected = true;
        warning = Some("incomplete".to_string());
    } else if is_suspicious_shrink(old_entries.len(), entries.len()) {
        // A single short scan is not enough evidence to delete most of a
        // library. Only accept the smaller snapshot when an independent scan
        // reproduces it almost exactly.
        let pending_entries = std::fs::read(&pending_file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<RemoteEntry>>(&bytes).ok());
        let confirmed = pending_entries
            .as_deref()
            .map(|pending| listings_match(pending, &entries))
            .unwrap_or(false);

        if confirmed {
            log::warn!(
                "accepting confirmed smaller library for source {source_id}: \
                 {} -> {} files",
                old_entries.len(),
                entries.len()
            );
        } else {
            protected = true;
            warning = Some("shrink".to_string());
            write_atomic(&pending_file, &serde_json::to_vec(&entries)?)?;
        }
    }

    if protected {
        // Keep the songs the cache knows about under directories this scan
        // could not read (or the whole old list when the shrink is unconfirmed)
        // so the UI does not temporarily lose them.
        let only_under = (warning.as_deref() == Some("incomplete")).then_some(&failed_paths[..]);
        extend_with_cached(&mut entries, &old_entries, only_under);
        log::warn!(
            "keeping library cache for source {source_id}: {} files shown \
             (cache {}), failed={} warning={:?}",
            entries.len(),
            old_entries.len(),
            failed_paths.len(),
            warning
        );
        return Ok(RefreshResult {
            entries,
            changed,
            // Never invalidate cached assets from an untrusted scan.
            changed_paths: Vec::new(),
            protected,
            failed_paths,
            warning,
        });
    }

    let _ = std::fs::remove_file(&pending_file);

    if changed {
        for path in &changed_paths {
            invalidate_track_assets(&app, &source_id, path);
        }
        write_atomic(
            &library_cache_file(&dir, &source_id),
            &serde_json::to_vec(&entries)?,
        )?;
        write_atomic(&library_hash_file(&dir, &source_id), new_hash.as_bytes())?;
    }

    log::info!(
        "refreshed source {source_id}: {} files, changed={changed}",
        entries.len()
    );
    Ok(RefreshResult {
        entries,
        changed,
        changed_paths,
        protected: false,
        failed_paths: Vec::new(),
        warning: None,
    })
}

/// Download and parse the complete track.
async fn fetch_complete_metadata(
    backend: &Backend,
    path: &str,
) -> Result<ParsedMetadata, AppError> {
    let bytes = backend.read(path).await?;
    metadata::parse(&bytes)
}

/// Try increasing range sizes before downloading the complete track.
async fn fetch_metadata(backend: &Backend, path: &str) -> Result<Option<ParsedMetadata>, AppError> {
    let mut last_error: Option<AppError> = None;

    for size in HEAD_RANGES {
        match backend.read_range(path, size).await {
            Ok(bytes) if !bytes.is_empty() => match metadata::parse(&bytes) {
                Ok(parsed) => {
                    let metadata = &parsed.metadata;
                    // Duration comes from the container head (e.g. WAV `fmt`), so it
                    // is present even when the tag blocks live further in the file.
                    // It must NOT count as "useful", otherwise a ranged read that
                    // only reached the audio header would stop escalating and we'd
                    // return empty tags instead of widening the Range / reading the
                    // whole file (see the trailing ID3/RiffInfo in large WAVs).
                    let useful = metadata.title.is_some()
                        || metadata.artist.is_some()
                        || metadata.album.is_some()
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

    match fetch_complete_metadata(backend, path).await {
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

/// Read a track's metadata, serving it from the on-disk cache when available.
#[tauri::command]
pub async fn read_track_metadata(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<TrackMetadata, AppError> {
    let dir = resolve_cache_dir(&app);
    let hash = asset_hash(&source_id, &path);

    if let Some(cached) = read_meta(&dir, &hash) {
        return Ok(cached);
    }

    let backend = backend_for(&app, &source_id)?;
    let parsed = match fetch_metadata(&backend, &path).await? {
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

/// How many metadata reads may be in flight for one batch call.
const METADATA_CONCURRENCY: usize = 4;

/// Read metadata for many paths in a single IPC call.
///
/// Cached entries are returned as-is; the rest are fetched with bounded
/// concurrency. This backs the lazy, viewport-driven enrichment: the frontend
/// only asks for the tracks it is about to show, never the whole library.
#[tauri::command]
pub async fn read_track_metadata_batch(
    app: AppHandle,
    source_id: String,
    paths: Vec<String>,
    on_event: tauri::ipc::Channel<CachedTrack>,
) -> Result<(), AppError> {
    let backend = backend_for(&app, &source_id)?;
    let dir = resolve_cache_dir(&app);
    let source = source_id.clone();

    stream::iter(paths)
        .map(|path| {
            let backend = &backend;
            let dir = &dir;
            let source = &source;
            let on_event = &on_event;
            async move {
                let result = read_or_fetch_metadata(backend, dir, source, path).await;
                let _ = on_event.send(result);
            }
        })
        .buffer_unordered(METADATA_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    Ok(())
}

/// Serve one track's cached metadata, fetching and caching it when missing.
async fn read_or_fetch_metadata(
    backend: &Backend,
    dir: &Path,
    source_id: &str,
    path: String,
) -> CachedTrack {
    let hash = asset_hash(source_id, &path);
    if let Some(metadata) = read_meta(dir, &hash) {
        return CachedTrack {
            path,
            metadata: Some(metadata),
            cover_path: cover_path_for(dir, &hash),
        };
    }

    match fetch_metadata(backend, &path).await {
        Ok(Some(parsed)) => {
            let mut result = parsed.metadata;
            if let Some(cover) = parsed.cover {
                match metadata::encode_thumbnail(&cover) {
                    Ok(thumbnail) => match write_cover(dir, &hash, &thumbnail) {
                        Ok(()) => result.cover_hash = Some(hash.clone()),
                        Err(error) => log::warn!("failed to cache cover for {path}: {error}"),
                    },
                    Err(error) => log::warn!("failed to encode cover for {path}: {error}"),
                }
            }
            if let Err(error) = write_meta(dir, &hash, &result) {
                log::warn!("failed to cache metadata for {path}: {error}");
            }
            CachedTrack {
                cover_path: cover_path_for(dir, &hash),
                path,
                metadata: Some(result),
            }
        }
        Ok(None) => {
            let result = TrackMetadata::default();
            let _ = write_meta(dir, &hash, &result);
            CachedTrack {
                path,
                metadata: Some(result),
                cover_path: None,
            }
        }
        Err(error) => {
            log::warn!("metadata batch failed for {path}: {error}");
            CachedTrack {
                path,
                metadata: None,
                cover_path: None,
            }
        }
    }
}

/// Download a complete track before extracting its metadata.
#[tauri::command]
pub async fn download_track_metadata(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<TrackMetadata, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let parsed = fetch_complete_metadata(&backend, &path).await?;
    let dir = resolve_cache_dir(&app);
    let hash = asset_hash(&source_id, &path);
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

/// Ask the backend why a track cannot be streamed.
#[tauri::command]
pub async fn probe_track(app: AppHandle, source_id: String, path: String) -> Result<(), AppError> {
    let backend = backend_for(&app, &source_id)?;
    backend.probe(&path).await
}

/// Read the `.lrc` file beside an audio file, caching the result locally.
#[tauri::command]
pub async fn read_track_lyrics(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<String, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let remote = lyric_path(&path)?;
    let content = backend.read_text(&remote).await?;

    let dir = resolve_cache_dir(&app);
    let hash = asset_hash(&source_id, &path);
    if read_lyric(&dir, &hash).as_deref() != Some(content.as_str()) {
        if let Err(error) = write_lyric(&dir, &hash, &content) {
            log::warn!("failed to cache lyrics for {path}: {error}");
        }
    }
    Ok(content)
}

/// Cache-only lyrics lookup keyed by `(source, path)`.
#[tauri::command]
pub fn get_cached_lyrics(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<Option<String>, AppError> {
    let dir = resolve_cache_dir(&app);
    Ok(read_lyric(&dir, &asset_hash(&source_id, &path)))
}

/// Cache-only metadata lookup keyed by `(source, path)`.
#[tauri::command]
pub fn get_cached_metadata(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<Option<TrackMetadata>, AppError> {
    let dir = resolve_cache_dir(&app);
    Ok(read_meta(&dir, &asset_hash(&source_id, &path)))
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

/// Cache-first cover lookup keyed by `(source, path)`, with no source access.
#[tauri::command]
pub fn get_cached_cover(
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<Option<String>, AppError> {
    let dir = resolve_cache_dir(&app);
    read_cover_file(&cover_file(&dir, &asset_hash(&source_id, &path)))
}

pub(crate) fn cover_path_for(dir: &Path, hash: &str) -> Option<String> {
    let file = cover_file(dir, hash);
    file.is_file().then(|| file.to_string_lossy().to_string())
}

/// One track's on-disk cached assets, keyed by source path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedTrack {
    pub path: String,
    pub metadata: Option<TrackMetadata>,
    pub cover_path: Option<String>,
}

/// Batch cache-only lookup for one source's tracks in a single IPC call.
#[tauri::command]
pub async fn load_cached_tracks(
    app: AppHandle,
    source_id: String,
    paths: Vec<String>,
) -> Result<Vec<CachedTrack>, AppError> {
    let dir = resolve_cache_dir(&app);
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let hash = asset_hash(&source_id, &path);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(local: &str, path: &str, size: u64) -> RemoteEntry {
        RemoteEntry {
            name: local.rsplit('/').next().unwrap_or(local).to_string(),
            path: path.to_string(),
            local_path: local.to_string(),
            is_dir: false,
            size: Some(size),
            modified: None,
            content_type: None,
            type_unknown: false,
        }
    }

    #[test]
    fn dedupes_the_same_object_but_keeps_equal_looking_copies() {
        let entries = vec![
            entry("Album/Song.flac", "/webdav/Album/Song.flac", 10),
            // Same decoded path, alternate href spelling: one row.
            entry("Album/Song.flac", "/webdav/Album/Song%2Eflac", 10),
            // Same name and size in another folder: a real copy, kept.
            entry("Other/Song.flac", "/webdav/Other/Song.flac", 10),
        ];
        let deduped = dedupe_entries(entries);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].local_path, "Album/Song.flac");
        assert_eq!(deduped[1].local_path, "Other/Song.flac");
    }

    #[test]
    fn only_files_with_changed_content_lose_their_cached_assets() {
        let mut cached = vec![
            entry("a.flac", "/webdav/a.flac", 10),
            entry("b.flac", "/webdav/b.flac", 20),
            entry("gone.flac", "/webdav/gone.flac", 30),
        ];
        cached[0].modified = Some("t1".into());
        cached[1].modified = Some("t1".into());
        cached[2].modified = Some("t1".into());

        let mut fresh = vec![
            entry("a.flac", "/webdav/a.flac", 10),
            entry("b.flac", "/webdav/b.flac", 20),
            // A path the cache does not know yet has no asset to invalidate.
            entry("new.flac", "/webdav/new.flac", 40),
        ];
        fresh[0].modified = Some("t1".into());
        fresh[1].modified = Some("t2".into());
        fresh[2].modified = Some("t1".into());

        // `gone.flac` missing from the fresh scan must NOT be invalidated: a
        // short listing is not proof of deletion and used to force a re-download.
        assert_eq!(
            changed_asset_paths(&fresh, &cached),
            vec!["/webdav/b.flac".to_string()]
        );
    }

    #[test]
    fn shrink_guard_only_trips_for_meaningful_drops() {
        assert!(!is_suspicious_shrink(249, 249));
        assert!(!is_suspicious_shrink(249, 245));
        assert!(!is_suspicious_shrink(249, 210));
        assert!(is_suspicious_shrink(249, 120));
        // Tiny libraries are allowed to change freely.
        assert!(!is_suspicious_shrink(4, 1));
    }
}
