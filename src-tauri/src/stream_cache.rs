// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Aggressive read-ahead cache for the loopback streaming proxy.
//!
//! The browser's media element only buffers as far ahead as its internal
//! heuristic allows, and every `Range` request it issues would otherwise hit the
//! remote WebDAV server, so throughput stays low and the buffer stays short.
//! This module keeps a per-track background downloader that races ahead of the
//! playhead (a fixed lead window) and writes the bytes to disk. The proxy then
//! answers every `Range` request from that local cache — instant, so the media
//! element's bandwidth estimate climbs and it buffers far more.
//!
//! Playback still starts on the first byte: a request is served from a blocking
//! reader that yields bytes as the downloader produces them. A seek past the
//! cached region rebases the downloader onto the new offset. Cache entries are
//! evicted least-recently-used once the fixed read-ahead budget is exceeded.
//! The buffer can be disabled entirely, in which case the proxy streams straight
//! from the source.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::commands::media::{asset_hash, resolve_cache_dir};
use crate::commands::webdav::{enforce_url_policy, source_password};
use crate::sources::{SourceConfig, SETTINGS_FILE};

/// Seconds of audio kept downloaded ahead of the playhead.
pub const LEAD_SECONDS: f64 = 120.0;
/// Fallback bitrate when the track duration is still unknown (~320 kbps).
const DEFAULT_BYTES_PER_SECOND: f64 = 40_000.0;
const READ_CHUNK: usize = 64 * 1024;
/// How long the proxy waits for the first upstream response headers.
pub const TOTAL_WAIT: Duration = Duration::from_secs(20);

/// `settings.json` key holding the smart cache budget in mebibytes.
pub const SMART_LIMIT_KEY: &str = "cache.smartLimitMb";
/// `settings.json` key toggling the automatic (whole-track) cache.
pub const SMART_ENABLED_KEY: &str = "cache.smartEnabled";
/// `settings.json` key toggling the read-ahead buffer used while streaming.
pub const STREAM_ENABLED_KEY: &str = "cache.streamEnabled";
/// Legacy key from before the smart/stream split; read as a fallback only.
const LEGACY_LIMIT_KEY: &str = "stream.cacheLimitMb";
/// Default smart cache budget when the user has not chosen one.
pub const DEFAULT_SMART_LIMIT_MB: u64 = 1024;
/// Fixed read-ahead budget. This buffer is needed for smooth playback and seek,
/// so it is independent of the user's smart-cache limit and never spends the
/// smart-cache budget.
pub const STREAM_BUDGET_MB: u64 = 256;

/// Outcome of waiting for a track's total size.
pub enum TotalState {
    Known(u64),
    Failed(u16, String),
    /// No response in time; the caller should fall back to a direct proxy.
    Unknown,
}

/// Per-track read-ahead state.
struct EntryState {
    /// Start offset of the cached segment.
    seg_start: u64,
    /// End offset (exclusive) of the contiguously cached segment.
    seg_end: u64,
    /// Total resource size, once the first upstream response reported it.
    total: Option<u64>,
    /// Playhead position in seconds, reported by the frontend.
    position: f64,
    /// Track duration in seconds, reported by the frontend.
    duration: f64,
    /// `(http status, message)` from a failed downloader, if any.
    failed: Option<(u16, String)>,
    /// Whether a downloader thread is currently running for `generation`.
    downloading: bool,
    /// Whether the whole resource is cached.
    complete: bool,
    /// Bumped whenever the active segment is rebased; stale threads bail out.
    generation: u64,
    /// Last time the downloader made progress; detects hung upstream reads.
    last_advanced: Instant,
    /// Overrides [`LEAD_SECONDS`] for this track (e.g. a short preload). Cleared
    /// once the track is actually playing.
    lead_override: Option<f64>,
}

/// One cached track. Cheap to share across the request handler and downloader.
pub struct Entry {
    path: PathBuf,
    state: Mutex<EntryState>,
    cv: Condvar,
}

impl Entry {
    fn new(path: PathBuf) -> Self {
        // Create the file eagerly so a reader can open it before the
        // downloader writes its first byte.
        let _ = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path);
        Self {
            path,
            state: Mutex::new(EntryState {
                seg_start: 0,
                seg_end: 0,
                total: None,
                position: 0.0,
                duration: 0.0,
                failed: None,
                downloading: false,
                complete: false,
                generation: 0,
                last_advanced: Instant::now(),
                lead_override: None,
            }),
            cv: Condvar::new(),
        }
    }
}

/// Shared cache state, owned by the streaming proxy.
pub struct StreamCache {
    dir: PathBuf,
    client: Client,
    limit_bytes: AtomicU64,
    /// Whether the read-ahead buffer is in use. When off, the proxy streams
    /// straight from the source instead of caching bytes locally.
    enabled: AtomicBool,
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    /// Least-recently-used key order (oldest first) for eviction.
    order: Mutex<Vec<String>>,
    /// Track currently being played; never evicted mid-stream.
    active: Mutex<Option<String>>,
}

impl StreamCache {
    /// Create the cache rooted at `<cache_dir>/stream` with the given budget.
    pub fn new(app: &AppHandle, limit_bytes: u64) -> Result<Self, String> {
        let dir = resolve_cache_dir(app).join("stream");
        std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建流缓存目录：{error}"))?;
        // The read-ahead tier is transient: drop any leftovers from last run.
        if let Ok(read) = std::fs::read_dir(&dir) {
            for item in read.flatten() {
                let _ = std::fs::remove_file(item.path());
            }
        }
        let client = Client::builder().build().map_err(|error| error.to_string())?;
        Ok(Self {
            dir,
            client,
            limit_bytes: AtomicU64::new(limit_bytes),
            enabled: AtomicBool::new(true),
            entries: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            active: Mutex::new(None),
        })
    }

    /// Whether the read-ahead buffer is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Turn the read-ahead buffer on or off. Cached bytes are left on disk;
    /// [`StreamCache::clear`] reclaims them.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Mark the track currently being played so it resists eviction.
    pub fn set_active(&self, key: &str) {
        *self.active.lock().unwrap() = Some(key.to_string());
    }

    /// Bytes currently occupied on disk by the read-ahead cache.
    pub fn used_bytes(&self) -> u64 {
        self.entries
            .lock()
            .unwrap()
            .values()
            .map(|entry| file_len(&entry.path))
            .sum()
    }

    /// Configured budget in bytes.
    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes.load(Ordering::Relaxed)
    }

    /// Look up or create the entry for a `(source, path)` pair.
    pub fn ensure(&self, key: &str) -> Arc<Entry> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get(key) {
            return entry.clone();
        }
        let entry = Arc::new(Entry::new(self.dir.join(format!("{key}.audio"))));
        entries.insert(key.to_string(), entry.clone());
        drop(entries);
        self.order.lock().unwrap().push(key.to_string());
        entry
    }

    /// Mark an entry as most-recently-used and evict if over budget.
    pub fn touch(&self, key: &str) {
        {
            let mut order = self.order.lock().unwrap();
            order.retain(|item| item != key);
            order.push(key.to_string());
        }
        self.maybe_evict();
    }

    /// Start (or resume/rebase) the background downloader so bytes from
    /// `want_start` onward are being fetched.
    pub fn ensure_stream(
        self: &Arc<Self>,
        app: &AppHandle,
        source: &SourceConfig,
        remote_path: &str,
        entry: &Arc<Entry>,
        want_start: u64,
        lead: Option<f64>,
    ) {
        let generation;
        {
            let mut state = entry.state.lock().unwrap();
            // A real request clears the short preload override; a preload sets it.
            state.lead_override = lead;
            entry.cv.notify_all();
            if state.complete && state.total.is_some_and(|total| want_start < total) {
                return;
            }
            // A downloader that stopped advancing is presumed hung; abandon it
            // (a new generation makes the stale thread bail) and fetch afresh.
            let stalled = state.downloading
                && state.last_advanced.elapsed() > Duration::from_secs(10);
            if stalled {
                state.generation = state.generation.wrapping_add(1);
                state.downloading = true;
                state.failed = None;
                state.last_advanced = Instant::now();
                generation = state.generation;
                entry.cv.notify_all();
            } else {
                // `seg_end` is exclusive, so `want_start == seg_end` is the
                // current head: continue the same segment instead of rebasing.
                let covered = want_start >= state.seg_start && want_start <= state.seg_end;
                if covered && state.downloading {
                    return;
                }
                if covered {
                    state.downloading = true;
                    state.failed = None;
                    state.last_advanced = Instant::now();
                } else {
                    let bytes_per_second = bytes_per_second(&state);
                    state.seg_start = want_start;
                    state.seg_end = want_start;
                    state.position = want_start as f64 / bytes_per_second;
                    state.generation = state.generation.wrapping_add(1);
                    state.complete = false;
                    state.failed = None;
                    state.downloading = true;
                    state.last_advanced = Instant::now();
                }
                generation = state.generation;
                entry.cv.notify_all();
            }
        }
        spawn_downloader(
            self.clone(),
            app.clone(),
            source.clone(),
            remote_path.to_string(),
            entry.clone(),
            generation,
        );
    }

    /// Begin prefetching a track without an active request (used for the next
    /// queue entry). `lead` overrides [`LEAD_SECONDS`] for a short preload.
    pub fn prefetch(
        self: &Arc<Self>,
        app: &AppHandle,
        source: SourceConfig,
        path: &str,
        lead: Option<f64>,
    ) {
        let key = asset_hash(&source.id, path);
        let entry = self.ensure(&key);
        self.touch(&key);
        self.ensure_stream(app, &source, path, &entry, 0, lead);
    }

    /// Update the reported playhead, letting the downloader advance its lead.
    pub fn report(&self, source_id: &str, path: &str, position: f64, duration: f64) {
        let key = asset_hash(source_id, path);
        let entry = { self.entries.lock().unwrap().get(&key).cloned() };
        let Some(entry) = entry else { return };
        {
            let mut state = entry.state.lock().unwrap();
            if duration > 0.0 {
                state.duration = duration;
            }
            if position.is_finite() && position >= 0.0 {
                state.position = position;
            }
            entry.cv.notify_all();
        }
        // The playing track is the most-recently-used and protected from
        // eviction; this is also a cheap moment to bound the cache.
        self.set_active(&key);
        self.touch(&key);
    }

    /// Percentage (0-100) of the track the read-ahead tier has cached, if the
    /// total size is known. Drives the player progress bar's prefill.
    pub fn coverage_percent(&self, key: &str) -> Option<f64> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(key)?;
        let state = entry.state.lock().unwrap();
        let total = state.total?;
        if total == 0 {
            return Some(0.0);
        }
        Some(((state.seg_end as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
    }

    /// Wait until the track size is known, the downloader failed, or timeout.
    pub fn wait_total(&self, entry: &Arc<Entry>, timeout: Duration) -> TotalState {
        let deadline = Instant::now() + timeout;
        let mut state = entry.state.lock().unwrap();
        loop {
            if let Some((status, message)) = &state.failed {
                return TotalState::Failed(*status, message.clone());
            }
            if let Some(total) = state.total {
                return TotalState::Known(total);
            }
            let now = Instant::now();
            if now >= deadline {
                return TotalState::Unknown;
            }
            let (guard, _) = entry.cv.wait_timeout(state, deadline - now).unwrap();
            state = guard;
        }
    }

    /// A blocking reader that yields `[start, end]` as the downloader writes it.
    pub fn reader(&self, entry: &Arc<Entry>, start: u64, end: u64) -> CacheReader {
        let generation = entry.state.lock().unwrap().generation;
        CacheReader {
            file: File::open(&entry.path).ok(),
            entry: entry.clone(),
            offset: start,
            end,
            generation,
        }
    }

    /// Drop every cached track except the one playing right now, so a clear
    /// during playback does not cut the active stream.
    pub fn clear(&self) {
        let active = self.active.lock().unwrap().clone();
        let drained: Vec<(String, Arc<Entry>)> = {
            let mut entries = self.entries.lock().unwrap();
            let keys: Vec<String> = entries.keys().cloned().collect();
            let mut drained = Vec::new();
            for key in keys {
                if active.as_deref() == Some(key.as_str()) {
                    continue;
                }
                if let Some(entry) = entries.remove(&key) {
                    drained.push((key, entry));
                }
            }
            drained
        };
        for (_, entry) in &drained {
            {
                let mut state = entry.state.lock().unwrap();
                state.generation = state.generation.wrapping_add(1);
                entry.cv.notify_all();
            }
            let _ = std::fs::remove_file(&entry.path);
        }
        self.order
            .lock()
            .unwrap()
            .retain(|key| active.as_deref() == Some(key.as_str()));
    }

    /// Evict least-recently-used, non-downloading entries until under budget.
    fn maybe_evict(&self) {
        let limit = self.limit_bytes.load(Ordering::Relaxed);
        if limit == 0 {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        let mut total: u64 = entries.values().map(|entry| file_len(&entry.path)).sum();
        if total <= limit {
            return;
        }
        let active = self.active.lock().unwrap().clone();
        let mut order = self.order.lock().unwrap();
        let candidates: Vec<String> = order.clone();
        for key in candidates {
            if total <= limit {
                break;
            }
            // Never evict the track that is playing right now.
            if active.as_deref() == Some(key.as_str()) {
                continue;
            }
            if let Some(entry) = entries.remove(&key) {
                let size = file_len(&entry.path);
                {
                    // Bumping the generation stops a parked downloader too.
                    let mut state = entry.state.lock().unwrap();
                    state.generation = state.generation.wrapping_add(1);
                    entry.cv.notify_all();
                }
                let _ = std::fs::remove_file(&entry.path);
                total = total.saturating_sub(size);
            }
            order.retain(|item| item != &key);
        }
    }
}

/// Blocking reader over a growing cache file.
pub struct CacheReader {
    file: Option<File>,
    entry: Arc<Entry>,
    offset: u64,
    end: u64,
    generation: u64,
}

impl Read for CacheReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.offset > self.end {
            return Ok(0);
        }
        loop {
            let state = self.entry.state.lock().unwrap();
            if state.generation != self.generation {
                return Err(std::io::Error::other("stream replaced"));
            }
            if let Some((_, message)) = &state.failed {
                return Err(std::io::Error::other(message.clone()));
            }
            let available = state.seg_end;
            if self.offset < available {
                let want = (self.end - self.offset + 1)
                    .min(available - self.offset)
                    .min(buf.len() as u64) as usize;
                drop(state);
                let file = self.file.as_mut().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "cache file missing")
                })?;
                file.seek(SeekFrom::Start(self.offset))?;
                let read = file.read(&mut buf[..want])?;
                if read == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "cache read returned no data",
                    ));
                }
                self.offset += read as u64;
                return Ok(read);
            }
            if state.complete {
                return Ok(0);
            }
            // No data yet: wait for the downloader to advance or fail.
            let (guard, _) = self
                .entry
                .cv
                .wait_timeout(state, Duration::from_secs(30))
                .unwrap();
            drop(guard);
        }
    }
}

/// Fetch a bounded range beginning at `start` (open-ended upstream request).
pub(crate) fn open_upstream(
    app: &AppHandle,
    client: &Client,
    source: &SourceConfig,
    remote_path: &str,
    start: u64,
) -> Result<Response, (u16, String)> {
    let url = source.url.clone().unwrap_or_default();
    let username = source.username.clone().unwrap_or_default();
    enforce_url_policy(app, &url, source.allow_insecure)
        .map_err(|error| (403, error.to_string()))?;
    let password = source_password(app, &source.id).map_err(|error| (401, error.to_string()))?;
    let url = crate::proxy::build_url(&url, remote_path).map_err(|error| (400, error))?;

    let response = client
        .get(url)
        .basic_auth(&username, Some(&password))
        .header(reqwest::header::RANGE, format!("bytes={start}-"))
        .send()
        .map_err(|error| (502, error.to_string()))?;

    let status = response.status();
    if !(status.is_success() || status == StatusCode::PARTIAL_CONTENT) {
        return Err((status.as_u16(), format!("上游返回 {status}")));
    }
    Ok(response)
}

/// Total resource size from a response, honouring `Content-Range` for partials.
pub(crate) fn response_total(response: &Response) -> Option<u64> {
    if response.status() == StatusCode::PARTIAL_CONTENT {
        // A partial response's `Content-Length` is the chunk size, so the total
        // must come from `Content-Range` (`bytes start-end/total`).
        let value = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)?
            .to_str()
            .ok()?;
        let total = value.rsplit('/').next()?;
        return if total == "*" { None } else { total.parse().ok() };
    }
    response.content_length()
}

fn bytes_per_second(state: &EntryState) -> f64 {
    match state.total {
        Some(total) if state.duration > 0.0 => total as f64 / state.duration,
        _ => DEFAULT_BYTES_PER_SECOND,
    }
}

/// Effective lead window for a track (short override, else the default).
fn lead_seconds(state: &EntryState) -> f64 {
    state.lead_override.unwrap_or(LEAD_SECONDS)
}

fn spawn_downloader(
    cache: Arc<StreamCache>,
    app: AppHandle,
    source: SourceConfig,
    remote_path: String,
    entry: Arc<Entry>,
    generation: u64,
) {
    std::thread::spawn(move || {
        let result = run_downloader(&cache, &app, &source, &remote_path, &entry, generation);
        let mut state = entry.state.lock().unwrap();
        if state.generation == generation {
            if let Err((status, message)) = result {
                state.failed = Some((status, message));
            }
            state.downloading = false;
            entry.cv.notify_all();
        }
    });
}

fn run_downloader(
    cache: &StreamCache,
    app: &AppHandle,
    source: &SourceConfig,
    remote_path: &str,
    entry: &Arc<Entry>,
    generation: u64,
) -> Result<(), (u16, String)> {
    loop {
        // Bound the cache as bytes accumulate, even without new requests.
        cache.maybe_evict();
        let (cursor, target, total, complete) = {
            let state = entry.state.lock().unwrap();
            if state.generation != generation {
                return Ok(());
            }
            if state.failed.is_some() {
                return Ok(());
            }
            let bytes_per_second = bytes_per_second(&state);
            let lead = (lead_seconds(&state) * bytes_per_second) as u64;
            let position = (state.position * bytes_per_second) as u64;
            let desired = position.saturating_add(lead);
            let target = state.total.map(|total| desired.min(total)).unwrap_or(desired);
            (state.seg_end, target, state.total, state.complete)
        };

        if complete {
            return Ok(());
        }
        if let Some(total) = total {
            if cursor >= total {
                mark_complete(entry, generation);
                return Ok(());
            }
        }
        if cursor >= target {
            // Caught up to the lead window: sleep until the playhead advances.
            let state = entry.state.lock().unwrap();
            if state.generation != generation {
                return Ok(());
            }
            let _ = entry.cv.wait_timeout(state, Duration::from_secs(5)).unwrap();
            continue;
        }

        let response = open_upstream(app, &cache.client, source, remote_path, cursor)?;
        let status = response.status();
        if let Some(total) = response_total(&response) {
            let mut state = entry.state.lock().unwrap();
            if state.generation == generation && state.total.is_none() {
                state.total = Some(total);
                entry.cv.notify_all();
            }
        }
        if cursor > 0 && status != StatusCode::PARTIAL_CONTENT {
            return Err((502, "上游不支持分段传输".to_string()));
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&entry.path)
            .map_err(|error| (500, error.to_string()))?;
        file.seek(SeekFrom::Start(cursor))
            .map_err(|error| (500, error.to_string()))?;

        let mut reader = response;
        let mut buffer = vec![0u8; READ_CHUNK];
        let mut cursor = cursor;
        loop {
            let target = {
                let state = entry.state.lock().unwrap();
                if state.generation != generation {
                    return Ok(());
                }
                let bytes_per_second = bytes_per_second(&state);
                let desired = ((state.position * bytes_per_second) as u64)
                    .saturating_add((lead_seconds(&state) * bytes_per_second) as u64);
                state.total.map(|total| desired.min(total)).unwrap_or(desired)
            };
            if cursor >= target {
                break;
            }
            let read = reader.read(&mut buffer).map_err(|error| (502, error.to_string()))?;
            if read == 0 {
                let total = {
                    let state = entry.state.lock().unwrap();
                    if state.generation != generation {
                        return Ok(());
                    }
                    state.total
                };
                if let Some(total) = total {
                    if cursor < total {
                        return Err((502, "上游音频流提前结束".to_string()));
                    }
                }
                mark_complete(entry, generation);
                return Ok(());
            }
            file.write_all(&buffer[..read])
                .map_err(|error| (500, error.to_string()))?;
            cursor += read as u64;
            let mut state = entry.state.lock().unwrap();
            if state.generation != generation {
                return Ok(());
            }
            state.seg_end = cursor;
            state.last_advanced = Instant::now();
            // Only a segment that starts at the file's beginning covers a whole
            // file; a forward seek rebases `seg_start` and leaves an unwritten
            // hole below it, so marking it complete would let a later read from
            // offset 0 stream those zero bytes.
            if let Some(total) = state.total {
                if cursor >= total && state.seg_start == 0 {
                    state.complete = true;
                }
            }
            entry.cv.notify_all();
        }
    }
}

/// Mark the entry complete, but only when its cached segment spans the whole
/// file from offset 0. A segment rebased forward (after a seek) only covers
/// `[seg_start, total)` and leaves a hole below `seg_start`; treating it as
/// complete would let subsequent readers of the leading range serve zeros.
fn mark_complete(entry: &Arc<Entry>, generation: u64) {
    let mut state = entry.state.lock().unwrap();
    if state.generation == generation && state.seg_start == 0 {
        state.complete = true;
    }
    entry.cv.notify_all();
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// Read the configured smart cache budget (MiB), honouring the legacy key from
/// before the smart/stream split.
pub fn read_smart_limit_mb(app: &AppHandle) -> u64 {
    let Ok(store) = app.store(SETTINGS_FILE) else {
        return DEFAULT_SMART_LIMIT_MB;
    };
    store
        .get(SMART_LIMIT_KEY)
        .and_then(|value| value.as_u64())
        .or_else(|| store.get(LEGACY_LIMIT_KEY).and_then(|value| value.as_u64()))
        .unwrap_or(DEFAULT_SMART_LIMIT_MB)
}

/// Persist the smart cache budget (MiB).
pub fn write_smart_limit_mb(app: &AppHandle, limit_mb: u64) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(SMART_LIMIT_KEY, serde_json::json!(limit_mb));
    store.save().map_err(|error| error.to_string())?;
    Ok(())
}

/// Whether the automatic smart cache is enabled (defaults to on).
pub fn read_smart_enabled(app: &AppHandle) -> bool {
    read_bool(app, SMART_ENABLED_KEY, true)
}

/// Persist the smart cache enabled flag.
pub fn write_smart_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    write_bool(app, SMART_ENABLED_KEY, enabled)
}

/// Whether the read-ahead stream buffer is enabled (defaults to on).
pub fn read_stream_enabled(app: &AppHandle) -> bool {
    read_bool(app, STREAM_ENABLED_KEY, true)
}

/// Persist the stream buffer enabled flag.
pub fn write_stream_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    write_bool(app, STREAM_ENABLED_KEY, enabled)
}

fn read_bool(app: &AppHandle, key: &str, fallback: bool) -> bool {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(key))
        .and_then(|value| value.as_bool())
        .unwrap_or(fallback)
}

fn write_bool(app: &AppHandle, key: &str, value: bool) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(key, serde_json::json!(value));
    store.save().map_err(|error| error.to_string())?;
    Ok(())
}
