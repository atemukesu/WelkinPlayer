// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Persistent manual track cache: whole tracks the user explicitly pins.
//!
//! Pinning is a deliberate user action: the complete file is downloaded so
//! replaying the track needs no network, and it is never evicted. A single
//! background worker fills the queue so we never saturate the link.
//!
//! The earlier automatic, play-count-ranked "smart" cache has been removed.
//! The on-disk directory is kept for compatibility so existing pins survive the
//! upgrade; the settings "clear cache" action drops any leftover automatic
//! entries (everything not pinned), which are the only non-pinned entries left.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::commands::media::{asset_hash, resolve_cache_dir};
use crate::data::sources::{find_source, SETTINGS_FILE};
use crate::streaming::stream_cache::{open_upstream, response_total};

const READ_CHUNK: usize = 64 * 1024;
const IDLE_WAIT: Duration = Duration::from_secs(15);
const INDEX_FILE: &str = "index.json";
/// Consecutive download failures before a track is abandoned (until re-pinned).
const MAX_FAILURES: u32 = 3;
/// On-disk directory for whole-track caches. Retained from the removed
/// automatic "smart" cache so existing pins and downloads survive the upgrade.
const CACHE_DIR: &str = "smart";
/// Upper bound for the simultaneous-download setting, and the size of the
/// worker pool created at startup. Workers beyond the configured limit simply
/// wait, so changing the setting never needs to spawn threads.
pub const MAX_CONCURRENT_DOWNLOADS: usize = 8;
pub const MIN_CONCURRENT_DOWNLOADS: usize = 1;
/// Concurrent whole-track downloads by default.
pub const DEFAULT_CONCURRENT_DOWNLOADS: usize = 3;
const CONCURRENT_DOWNLOADS_KEY: &str = "cache.maxConcurrentDownloads";

/// One manual-cache entry, including live download progress.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheItem {
    /// `sourceId::path`, matching the frontend's `trackKey`.
    pub key: String,
    pub pinned: bool,
    pub complete: bool,
    /// Upstream size once the response headers are known.
    pub size: Option<u64>,
    /// Bytes currently written to disk.
    pub written: u64,
    /// Whether this entry is the one the worker is downloading right now.
    pub active: bool,
}

/// Snapshot of the manual cache handed to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    pub cached: Vec<String>,
    pub pinned: Vec<String>,
    /// Every cached/pinned entry with progress, for the download manager.
    pub items: Vec<CacheItem>,
}

struct Entry {
    key: String,
    source_id: String,
    path: String,
    file: PathBuf,
    pinned: bool,
    size: Option<u64>,
    written: u64,
    complete: bool,
    /// Consecutive failed attempts; a track is dropped after a few.
    failures: u32,
}

struct State {
    entries: HashMap<String, Entry>,
    /// Pinned keys still to download, in priority order.
    queue: Vec<String>,
    /// Keys currently being downloaded by a worker.
    active: HashSet<String>,
}

/// Persistent manual (pinned) whole-track cache.
pub struct ManualCache {
    app: AppHandle,
    dir: PathBuf,
    client: Client,
    state: Mutex<State>,
    cv: Condvar,
    /// Maximum simultaneous downloads; changing it wakes waiting workers.
    max_concurrent: AtomicUsize,
}

impl ManualCache {
    /// Create the cache and start its worker.
    pub fn new(app: &AppHandle) -> Result<Arc<Self>, String> {
        let dir = resolve_cache_dir(app).join(CACHE_DIR);
        std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建缓存目录：{error}"))?;
        let client = Client::builder()
            .build()
            .map_err(|error| error.to_string())?;
        let entries = load_index(&dir);
        // Remove orphaned files left by a crash mid-download (they are not in
        // the index and would otherwise never be cleaned).
        if let Ok(read) = std::fs::read_dir(&dir) {
            for item in read.flatten() {
                let path = item.path();
                if path.extension().and_then(|value| value.to_str()) != Some("audio") {
                    continue;
                }
                let known = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|stem| entries.contains_key(stem))
                    .unwrap_or(false);
                if !known {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        let state = Mutex::new(State {
            entries,
            queue: Vec::new(),
            active: HashSet::new(),
        });

        let cache = Arc::new(Self {
            app: app.clone(),
            dir,
            client,
            state,
            cv: Condvar::new(),
            max_concurrent: AtomicUsize::new(read_max_concurrent_downloads(app)),
        });
        // Resume any pinned tracks that did not finish last run.
        cache.requeue_pinned();
        // The pool is sized for the maximum setting; each worker checks the
        // live limit before claiming a task, so changing it needs no respawn.
        for index in 0..MAX_CONCURRENT_DOWNLOADS {
            let worker = cache.clone();
            std::thread::Builder::new()
                .name(format!("welkin-manual-cache-{index}"))
                .spawn(move || worker.run())
                .map_err(|error| format!("无法启动缓存线程：{error}"))?;
        }
        Ok(cache)
    }

    /// Current simultaneous-download limit.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
            .load(Ordering::Relaxed)
            .clamp(MIN_CONCURRENT_DOWNLOADS, MAX_CONCURRENT_DOWNLOADS)
    }

    /// Update the simultaneous-download limit, waking idle workers so extra
    /// capacity takes effect immediately.
    pub fn set_max_concurrent(&self, value: usize) {
        let value = value.clamp(MIN_CONCURRENT_DOWNLOADS, MAX_CONCURRENT_DOWNLOADS);
        self.max_concurrent.store(value, Ordering::Relaxed);
        self.cv.notify_all();
    }

    /// Pin a track: downloaded first, never evicted.
    pub fn pin(&self, source_id: &str, path: &str) {
        let key = asset_hash(source_id, path);
        {
            let mut state = self.state.lock().unwrap();
            let active = state.active.contains(&key);
            let entry = state.entries.entry(key.clone()).or_insert_with(|| Entry {
                file: self.dir.join(format!("{key}.audio")),
                key: key.clone(),
                source_id: source_id.to_string(),
                path: path.to_string(),
                pinned: false,
                size: None,
                written: 0,
                complete: false,
                failures: 0,
            });
            entry.pinned = true;
            if !entry.complete && !active {
                state.queue.retain(|item| item != &key);
                state.queue.insert(0, key.clone());
            }
            self.cv.notify_all();
        }
        self.persist();
    }

    /// Stop protecting a pinned track and drop its file: unpinning is an
    /// explicit "remove from cache" action, so the download is discarded.
    pub fn unpin(&self, key: &str) {
        let removed = {
            let mut state = self.state.lock().unwrap();
            state.queue.retain(|item| item != key);
            state.entries.remove(key).map(|entry| entry.file)
        };
        if let Some(file) = removed {
            let _ = std::fs::remove_file(file);
        }
        self.persist();
    }

    /// Complete file for a key, when cached.
    pub fn complete_file(&self, key: &str) -> Option<PathBuf> {
        let state = self.state.lock().unwrap();
        let entry = state.entries.get(key)?;
        if entry.complete && entry.file.exists() {
            Some(entry.file.clone())
        } else {
            None
        }
    }

    /// Bytes occupied by manually pinned (user-cached) tracks.
    pub fn pinned_bytes(&self) -> u64 {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .filter(|entry| entry.pinned)
            .map(|entry| file_len(&entry.file))
            .sum()
    }

    /// Whether a key's whole file is already cached (no side effects).
    pub fn is_complete(&self, key: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(key)
            .map(|entry| entry.complete && entry.file.exists())
            .unwrap_or(false)
    }

    /// Snapshot for the frontend badge state. Keys use the profile's
    /// `sourceId::path` format so they line up with the frontend's `trackKey`.
    pub fn status(&self) -> CacheStatus {
        let state = self.state.lock().unwrap();
        let mut cached = Vec::new();
        let mut pinned = Vec::new();
        let mut items = Vec::new();
        for entry in state.entries.values() {
            let key = format!("{}::{}", entry.source_id, entry.path);
            if entry.complete {
                cached.push(key.clone());
            }
            if entry.pinned {
                pinned.push(key.clone());
            }
            items.push(CacheItem {
                active: state.active.contains(&entry.key),
                key,
                pinned: entry.pinned,
                complete: entry.complete,
                size: entry.size,
                written: entry.written,
            });
        }
        CacheStatus {
            cached,
            pinned,
            items,
        }
    }

    /// Remove every non-pinned entry (leftover automatic cache). Pinned tracks
    /// are the user's explicit choice and are always kept.
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        let keep: Vec<String> = state
            .entries
            .values()
            .filter(|entry| entry.pinned)
            .map(|entry| entry.key.clone())
            .collect();
        let mut removed = Vec::new();
        state.entries.retain(|_key, entry| {
            if entry.pinned {
                true
            } else {
                removed.push(entry.file.clone());
                false
            }
        });
        state.queue.retain(|key| keep.contains(key));
        drop(state);
        for file in removed {
            let _ = std::fs::remove_file(file);
        }
        self.persist();
    }

    fn requeue_pinned(&self) {
        let mut state = self.state.lock().unwrap();
        let pinned: Vec<String> = state
            .entries
            .values()
            .filter(|entry| entry.pinned && !entry.complete)
            .map(|entry| entry.key.clone())
            .collect();
        state.queue = pinned;
        self.cv.notify_all();
    }

    /// Background worker: claim queued tracks up to the configured limit.
    fn run(self: Arc<Self>) {
        loop {
            let Some((key, source_id, path, offset)) = self.next_task() else {
                continue;
            };
            let result = self.download(&key, &source_id, &path, offset);
            self.finish(&key, result);
        }
    }

    /// Block until a pinned task is ready and a download slot is free.
    fn next_task(&self) -> Option<(String, String, String, u64)> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.active.len() >= self.max_concurrent() {
                let (guard, _) = self.cv.wait_timeout(state, IDLE_WAIT).unwrap();
                state = guard;
                continue;
            }
            // Drop finished/missing/active heads so we always look at a real task.
            while let Some(front) = state.queue.first().cloned() {
                let done = state
                    .entries
                    .get(&front)
                    .map(|entry| entry.complete)
                    .unwrap_or(true);
                if done || state.active.contains(&front) {
                    state.queue.remove(0);
                } else {
                    break;
                }
            }
            let Some(key) = state.queue.first().cloned() else {
                let (guard, _) = self.cv.wait_timeout(state, IDLE_WAIT).unwrap();
                state = guard;
                continue;
            };
            let Some(entry) = state.entries.get_mut(&key) else {
                state.queue.remove(0);
                continue;
            };
            entry.written = std::fs::metadata(&entry.file)
                .map(|meta| meta.len())
                .unwrap_or(entry.written);
            let offset = entry.written;
            let source_id = entry.source_id.clone();
            let path = entry.path.clone();
            // Claim the task immediately: with several workers the queue head
            // must not be handed to another thread before it is removed.
            state.queue.remove(0);
            state.active.insert(key.clone());
            return Some((key, source_id, path, offset));
        }
    }

    /// Download a whole file from `offset` to the end.
    fn download(&self, key: &str, source_id: &str, path: &str, offset: u64) -> Result<(), String> {
        let source = find_source(&self.app, source_id).map_err(|error| error.to_string())?;
        let file_path = self.dir.join(format!("{key}.audio"));
        let response = open_upstream(&self.app, &self.client, &source, path, offset)
            .map_err(|(status, message)| format!("上游返回 {status}：{message}"))?;
        if offset > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err("上游不支持分段传输".to_string());
        }
        let total = response_total(&response);
        if let Some(total) = total {
            let mut state = self.state.lock().unwrap();
            if let Some(entry) = state.entries.get_mut(key) {
                entry.size = Some(total);
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&file_path)
            .map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;

        // The user may have unpinned while we were setting up. Stop before
        // writing a file that is no longer wanted (and may have just been
        // recreated by `OpenOptions::create`).
        if !self.has_entry(key) {
            drop(file);
            let _ = std::fs::remove_file(&file_path);
            return Ok(());
        }

        let mut reader = response;
        let mut buffer = vec![0u8; READ_CHUNK];
        let mut offset = offset;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Ok(());
            }
            // Unpinning removes the entry; abandon the download and delete the
            // partial file instead of leaving orphaned bytes behind.
            if !self.has_entry(key) {
                drop(file);
                let _ = std::fs::remove_file(&file_path);
                return Ok(());
            }
            file.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            offset += read as u64;
            let mut state = self.state.lock().unwrap();
            if let Some(entry) = state.entries.get_mut(key) {
                entry.written = offset;
            } else {
                drop(state);
                drop(file);
                let _ = std::fs::remove_file(&file_path);
                return Ok(());
            }
            if let Some(total) = total {
                if offset >= total {
                    return Ok(());
                }
            }
        }
    }

    /// Whether a key still has an entry (unpinning removes it, which cancels
    /// an in-flight download).
    fn has_entry(&self, key: &str) -> bool {
        self.state.lock().unwrap().entries.contains_key(key)
    }

    /// Record the result of a download and advance the queue.
    fn finish(&self, key: &str, result: Result<(), String>) {
        let mut state = self.state.lock().unwrap();
        state.active.remove(key);
        if let Some(entry) = state.entries.get_mut(key) {
            entry.written = std::fs::metadata(&entry.file)
                .map(|meta| meta.len())
                .unwrap_or(entry.written);
            let caught_up = entry
                .size
                .map(|size| entry.written >= size)
                .unwrap_or(false);
            if let Err(error) = &result {
                log::warn!("manual cache download failed for {key}: {error}");
            } else if caught_up || entry.size.is_none() {
                entry.complete = true;
            }
        }
        let complete = state
            .entries
            .get(key)
            .map(|entry| entry.complete)
            .unwrap_or(false);
        if complete {
            if let Some(entry) = state.entries.get_mut(key) {
                entry.failures = 0;
            }
            state.queue.retain(|item| item != key);
        } else if result.is_err() {
            // Backoff: rotate the failing track to the back, drop it after a
            // few tries so a bad credential cannot spin the worker forever.
            let failures = match state.entries.get_mut(key) {
                Some(entry) => {
                    entry.failures += 1;
                    entry.failures
                }
                None => MAX_FAILURES,
            };
            state.queue.retain(|item| item != key);
            if failures < MAX_FAILURES {
                state.queue.push(key.to_string());
            } else {
                log::warn!("manual cache giving up on {key} after {failures} attempts");
            }
        }
        self.cv.notify_all();
        drop(state);
        self.persist();
    }

    /// Write the index so pins and cached keys survive a restart.
    fn persist(&self) {
        let records: Vec<IndexRecord> = {
            let state = self.state.lock().unwrap();
            state
                .entries
                .values()
                .filter(|entry| entry.complete || entry.pinned)
                .map(|entry| IndexRecord {
                    key: entry.key.clone(),
                    source_id: entry.source_id.clone(),
                    path: entry.path.clone(),
                    pinned: entry.pinned,
                    size: entry.size,
                    complete: entry.complete,
                })
                .collect()
        };
        let path = self.dir.join(INDEX_FILE);
        match serde_json::to_vec(&records) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(&path, bytes) {
                    log::warn!("failed to write manual cache index: {error}");
                }
            }
            Err(error) => log::warn!("failed to serialize manual cache index: {error}"),
        }
    }
}

/// Maximum simultaneous manual-cache downloads, clamped to its bounds.
pub fn read_max_concurrent_downloads(app: &AppHandle) -> usize {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(CONCURRENT_DOWNLOADS_KEY))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_CONCURRENT_DOWNLOADS)
        .clamp(MIN_CONCURRENT_DOWNLOADS, MAX_CONCURRENT_DOWNLOADS)
}

/// Persist the simultaneous-download limit, clamped to its bounds.
pub fn write_max_concurrent_downloads(app: &AppHandle, value: usize) -> Result<usize, String> {
    let value = value.clamp(MIN_CONCURRENT_DOWNLOADS, MAX_CONCURRENT_DOWNLOADS);
    let store = app
        .store(SETTINGS_FILE)
        .map_err(|error| error.to_string())?;
    store.set(CONCURRENT_DOWNLOADS_KEY, serde_json::json!(value as u64));
    store.save().map_err(|error| error.to_string())?;
    Ok(value)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexRecord {
    key: String,
    source_id: String,
    path: String,
    pinned: bool,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    complete: bool,
}

/// Load the persisted index, keeping only entries whose file still exists.
fn load_index(dir: &Path) -> HashMap<String, Entry> {
    let mut entries = HashMap::new();
    let Ok(bytes) = std::fs::read(dir.join(INDEX_FILE)) else {
        return entries;
    };
    let Ok(records) = serde_json::from_slice::<Vec<IndexRecord>>(&bytes) else {
        return entries;
    };
    for record in records {
        let file = dir.join(format!("{}.audio", record.key));
        if !file.exists() {
            continue;
        }
        let written = file_len(&file);
        let complete = record.complete || record.size.map(|size| written >= size).unwrap_or(false);
        entries.insert(
            record.key.clone(),
            Entry {
                key: record.key,
                source_id: record.source_id,
                path: record.path,
                file,
                pinned: record.pinned,
                size: record.size,
                written,
                complete,
                failures: 0,
            },
        );
    }
    entries
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}
