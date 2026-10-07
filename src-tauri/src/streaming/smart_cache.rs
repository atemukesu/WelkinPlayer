// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Persistent "smart" cache: whole tracks kept for reuse, chosen by play rank.
//!
//! Unlike the transient read-ahead cache ([`crate::streaming::stream_cache`]), this keeps
//! complete files so replaying a frequent track needs no network. Candidates
//! come from the synced play counts (the play ranking): tracks played at least
//! five times, best-ranked first. Eviction is LFU — the least-played cached
//! track goes first — and the user can pin a track from the context menu, which
//! is downloaded first and never evicted.
//!
//! Downloads are suspended on metered connections; pinned tracks are explicit
//! user actions and are allowed through. A single background worker fills the
//! queue so we never saturate the link.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::media::{asset_hash, resolve_cache_dir};
use crate::data::sources::{find_source, load_sources};
use crate::streaming::stream_cache::{open_upstream, response_total};

const READ_CHUNK: usize = 64 * 1024;
const IDLE_WAIT: Duration = Duration::from_secs(15);
const INDEX_FILE: &str = "index.json";
/// Consecutive download failures before a track is abandoned (until re-synced).
const MAX_FAILURES: u32 = 3;

/// A track offered by the frontend for whole-file caching.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartCandidate {
    pub source_id: String,
    pub path: String,
    #[serde(default)]
    pub freq: u64,
}

/// Snapshot of the smart cache handed to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    pub cached: Vec<String>,
    pub pinned: Vec<String>,
}

struct Entry {
    key: String,
    source_id: String,
    path: String,
    file: PathBuf,
    freq: u64,
    pinned: bool,
    size: Option<u64>,
    written: u64,
    complete: bool,
    last_used: u64,
    /// Consecutive failed attempts; a track is dropped after a few.
    failures: u32,
}

struct State {
    entries: HashMap<String, Entry>,
    /// Keys to download, in priority order (pinned first).
    queue: Vec<String>,
    /// Key currently being downloaded, if any.
    active: Option<String>,
}

/// Persistent whole-track cache.
pub struct SmartCache {
    app: AppHandle,
    dir: PathBuf,
    client: Client,
    limit_bytes: AtomicU64,
    /// Whether automatic (non-pinned) caching is active. Manual pins ignore
    /// this: they are explicit user actions and keep downloading.
    enabled: AtomicBool,
    state: Mutex<State>,
    cv: Condvar,
}

impl SmartCache {
    /// Create the cache with the given persistent budget and start its worker.
    pub fn new(app: &AppHandle, limit_bytes: u64) -> Result<Arc<Self>, String> {
        let dir = resolve_cache_dir(app).join("smart");
        std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建智能缓存目录：{error}"))?;
        let client = Client::builder().build().map_err(|error| error.to_string())?;
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
            active: None,
        });

        let cache = Arc::new(Self {
            app: app.clone(),
            dir,
            client,
            limit_bytes: AtomicU64::new(limit_bytes),
            enabled: AtomicBool::new(true),
            state,
            cv: Condvar::new(),
        });
        // Resume any pinned tracks that did not finish last run.
        cache.requeue_pinned();
        let worker = cache.clone();
        std::thread::spawn(move || worker.run());
        Ok(cache)
    }

    /// Replace the automatic candidate set (already filtered/ranked upstream).
    pub fn sync(&self, candidates: Vec<SmartCandidate>) {
        let mut removed: Vec<PathBuf> = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            let mut desired: std::collections::HashSet<String> = std::collections::HashSet::new();
            // Only remote sources this device knows can be downloaded. Play
            // counts are synced across devices, so keys can reference a source
            // configured elsewhere (or local files, which are already on disk).
            let streamable: std::collections::HashSet<String> = load_sources(&self.app)
                .into_iter()
                .filter(|source| !source.is_local())
                .map(|source| source.id)
                .collect();
            for candidate in candidates {
                if !streamable.contains(&candidate.source_id) {
                    continue;
                }
                let key = asset_hash(&candidate.source_id, &candidate.path);
                desired.insert(key.clone());
                let entry = state.entries.entry(key.clone()).or_insert_with(|| Entry {
                    file: self.dir.join(format!("{key}.audio")),
                    key: key.clone(),
                    source_id: candidate.source_id.clone(),
                    path: candidate.path.clone(),
                    freq: 0,
                    pinned: false,
                    size: None,
                    written: 0,
                    complete: false,
                    last_used: 0,
                    failures: 0,
                });
                entry.freq = candidate.freq;
                if let Ok(meta) = std::fs::metadata(&entry.file) {
                    entry.written = entry.written.max(meta.len());
                }
            }

            // Drop automatic entries that are no longer candidates (stale source
            // ids, de-ranked tracks); keep everything pinned or already cached.
            state.entries.retain(|key, entry| {
                if entry.complete || entry.pinned || desired.contains(key) {
                    true
                } else {
                    removed.push(entry.file.clone());
                    false
                }
            });

            // Highest rank first; pinned incomplete tracks always lead.
            let mut queue: Vec<String> = state
                .entries
                .values()
                .filter(|entry| !entry.complete)
                .map(|entry| entry.key.clone())
                .collect();
            queue.sort_by(|a, b| {
                let left = state.entries.get(a).map(|entry| entry.freq).unwrap_or(0);
                let right = state.entries.get(b).map(|entry| entry.freq).unwrap_or(0);
                right.cmp(&left)
            });
            let mut pinned: Vec<String> = state
                .entries
                .values()
                .filter(|entry| entry.pinned && !entry.complete)
                .map(|entry| entry.key.clone())
                .collect();
            pinned.extend(queue.into_iter().filter(|key| {
                !state
                    .entries
                    .get(key)
                    .map(|entry| entry.pinned)
                    .unwrap_or(false)
            }));
            state.queue = pinned;
            self.cv.notify_all();
        }
        for file in removed {
            let _ = std::fs::remove_file(file);
        }
        self.persist();
        self.maybe_evict();
    }

    /// Pin a track: downloaded first, never evicted.
    pub fn pin(&self, source_id: &str, path: &str) {
        let key = asset_hash(source_id, path);
        {
            let mut state = self.state.lock().unwrap();
            let entry = state.entries.entry(key.clone()).or_insert_with(|| Entry {
                file: self.dir.join(format!("{key}.audio")),
                key: key.clone(),
                source_id: source_id.to_string(),
                path: path.to_string(),
                freq: u64::MAX,
                pinned: false,
                size: None,
                written: 0,
                complete: false,
                last_used: 0,
                failures: 0,
            });
            entry.pinned = true;
            entry.freq = u64::MAX;
            if !entry.complete {
                state.queue.retain(|item| item != &key);
                state.queue.insert(0, key.clone());
            }
            self.cv.notify_all();
        }
        self.persist();
    }

    /// Stop protecting a pinned track (it becomes evictable again).
    pub fn unpin(&self, key: &str) {
        {
            let mut state = self.state.lock().unwrap();
            if let Some(entry) = state.entries.get_mut(key) {
                entry.pinned = false;
            }
        }
        self.persist();
        self.maybe_evict();
    }

    /// Complete file for a key, when cached; marks it recently used.
    pub fn complete_file(&self, key: &str) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap();
        let entry = state.entries.get_mut(key)?;
        if entry.complete && entry.file.exists() {
            entry.last_used = now();
            Some(entry.file.clone())
        } else {
            None
        }
    }

    /// Bytes occupied by automatically cached (non-pinned) complete tracks.
    /// Only this counts toward the configured cache budget.
    pub fn automatic_bytes(&self) -> u64 {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .filter(|entry| entry.complete && !entry.pinned)
            .map(|entry| file_len(&entry.file))
            .sum()
    }

    /// Bytes occupied by manually pinned (user-cached) tracks. These are an
    /// explicit user choice: they never count toward the budget and are never
    /// evicted.
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

    /// Configured disk budget in bytes.
    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes.load(Ordering::Relaxed)
    }

    /// Turn automatic smart caching on or off. Manual pins ignore this and keep
    /// downloading; only the automatic candidate set is paused.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.cv.notify_all();
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
        for entry in state.entries.values() {
            let key = format!("{}::{}", entry.source_id, entry.path);
            if entry.complete {
                cached.push(key.clone());
            }
            if entry.pinned {
                pinned.push(key);
            }
        }
        CacheStatus { cached, pinned }
    }

    /// Evict non-pinned entries down to the disk budget.
    pub fn set_limit_bytes(&self, bytes: u64) {
        self.limit_bytes.store(bytes, Ordering::Relaxed);
        self.maybe_evict();
    }

    /// Remove every non-pinned entry.
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

    /// Background worker: download queued tracks one at a time.
    fn run(self: Arc<Self>) {
        loop {
            let Some((key, source_id, path, offset)) = self.next_task() else {
                continue;
            };
            let result = self.download(&key, &source_id, &path, offset);
            self.finish(&key, result);
        }
    }

    /// Block until a task is ready (and the network allows it).
    fn next_task(&self) -> Option<(String, String, String, u64)> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.active.is_some() {
                let (guard, _) = self.cv.wait_timeout(state, IDLE_WAIT).unwrap();
                state = guard;
                continue;
            }
            // Drop finished/missing heads so we always look at a real task.
            while let Some(front) = state.queue.first().cloned() {
                let done = state
                    .entries
                    .get(&front)
                    .map(|entry| entry.complete)
                    .unwrap_or(true);
                if done {
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
            let pinned = state
                .entries
                .get(&key)
                .map(|entry| entry.pinned)
                .unwrap_or(false);
            // Automatic caching is paused when disabled; manual pins continue.
            if !pinned && !self.enabled.load(Ordering::Relaxed) {
                let (guard, _) = self.cv.wait_timeout(state, IDLE_WAIT).unwrap();
                state = guard;
                continue;
            }
            if !crate::core::network::is_unmetered() && !pinned {
                let (guard, _) = self.cv.wait_timeout(state, IDLE_WAIT).unwrap();
                state = guard;
                continue;
            }
            // Budget / admission. The queue is rank-ordered, so the head is the
            // best remaining candidate. Admit it only if it actually fits:
            // evict strictly lower-ranked cached tracks to make room, otherwise
            // skip it. Using the known file size avoids downloading a track and
            // then immediately evicting it again (which made the cache size
            // oscillate up and down).
            let limit = self.limit_bytes.load(Ordering::Relaxed);
            if limit > 0 && !pinned {
                let want_freq = state.entries.get(&key).map(|entry| entry.freq).unwrap_or(0);
                let size = state.entries.get(&key).and_then(|entry| entry.size);
                match size {
                    Some(size) => {
                        let mut cached = Self::automatic_total(&state);
                        while cached.saturating_add(size) > limit {
                            let Some(victim) = Self::lowest_evictable(&state, want_freq) else { break };
                            Self::evict_entry(&mut state, &victim);
                            cached = Self::automatic_total(&state);
                        }
                        if cached.saturating_add(size) > limit {
                            // Cannot fit without displacing higher-ranked tracks.
                            state.queue.remove(0);
                            continue;
                        }
                    }
                    None => {
                        // Size unknown: make sure there is at least some room for
                        // a higher-ranked track, then download once to learn it.
                        if Self::automatic_total(&state) >= limit {
                            match Self::lowest_evictable(&state, want_freq) {
                                Some(victim) => {
                                    Self::evict_entry(&mut state, &victim);
                                    continue;
                                }
                                None => {
                                    state.queue.remove(0);
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
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
            state.active = Some(key.clone());
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

        let mut reader = response;
        let mut buffer = vec![0u8; READ_CHUNK];
        let mut offset = offset;
        loop {
            let read = reader.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                return Ok(());
            }
            file.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            offset += read as u64;
            let mut state = self.state.lock().unwrap();
            if let Some(entry) = state.entries.get_mut(key) {
                entry.written = offset;
            }
            if let Some(total) = total {
                if offset >= total {
                    return Ok(());
                }
            }
        }
    }

    /// Record the result of a download and advance the queue.
    fn finish(&self, key: &str, result: Result<(), String>) {
        {
            let mut state = self.state.lock().unwrap();
            state.active = None;
            if let Some(entry) = state.entries.get_mut(key) {
                entry.written = std::fs::metadata(&entry.file)
                    .map(|meta| meta.len())
                    .unwrap_or(entry.written);
                let caught_up = entry.size.map(|size| entry.written >= size).unwrap_or(false);
                if let Err(error) = &result {
                    log::warn!("smart cache download failed for {key}: {error}");
                } else if caught_up || entry.size.is_none() {
                    entry.complete = true;
                    entry.last_used = now();
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
                    log::warn!("smart cache giving up on {key} after {failures} attempts");
                }
            }
            self.cv.notify_all();
        }
        self.persist();
        self.maybe_evict();
    }

    /// Total bytes occupied by complete, non-pinned tracks (the budget usage).
    fn automatic_total(state: &State) -> u64 {
        state
            .entries
            .values()
            .filter(|entry| entry.complete && !entry.pinned)
            .map(|entry| file_len(&entry.file))
            .sum()
    }

    /// The least-played complete, non-pinned entry ranked below `want_freq`.
    fn lowest_evictable(state: &State, want_freq: u64) -> Option<String> {
        state
            .entries
            .values()
            .filter(|entry| entry.complete && !entry.pinned)
            .min_by(|a, b| {
                a.freq
                    .cmp(&b.freq)
                    .then_with(|| a.last_used.cmp(&b.last_used))
            })
            .filter(|entry| entry.freq < want_freq)
            .map(|entry| entry.key.clone())
    }

    /// Delete a non-pinned track's file and mark its entry incomplete, keeping
    /// the (now known) size so it is skipped instead of re-downloaded. This is
    /// what stops the cache size from oscillating: a track that cannot fit is
    /// not fetched again just to be evicted again.
    fn evict_entry(state: &mut State, key: &str) {
        let Some(entry) = state.entries.get_mut(key) else { return };
        if entry.pinned {
            return;
        }
        let size = entry.size.unwrap_or_else(|| file_len(&entry.file));
        entry.size = Some(size);
        entry.complete = false;
        entry.written = 0;
        entry.last_used = 0;
        let _ = std::fs::remove_file(&entry.file);
        log::debug!("smart cache evicted {key} ({size} bytes)");
    }

    /// Evict the least-played non-pinned cached tracks until under budget.
    fn maybe_evict(&self) {
        let limit = self.limit_bytes.load(Ordering::Relaxed);
        if limit == 0 {
            return;
        }
        let mut state = self.state.lock().unwrap();
        let mut total = Self::automatic_total(&state);
        while total > limit {
            let Some(victim) = Self::lowest_evictable_all(&state) else { break };
            Self::evict_entry(&mut state, &victim);
            total = Self::automatic_total(&state);
        }
    }

    /// The least-played complete, non-pinned entry, regardless of rank.
    fn lowest_evictable_all(state: &State) -> Option<String> {
        Self::lowest_evictable(state, u64::MAX)
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
                    freq: entry.freq,
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
                    log::warn!("failed to write smart cache index: {error}");
                }
            }
            Err(error) => log::warn!("failed to serialize smart cache index: {error}"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexRecord {
    key: String,
    source_id: String,
    path: String,
    freq: u64,
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
                freq: if record.pinned { u64::MAX } else { record.freq },
                pinned: record.pinned,
                size: record.size,
                written,
                complete,
                last_used: 0,
                failures: 0,
            },
        );
    }
    entries
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
