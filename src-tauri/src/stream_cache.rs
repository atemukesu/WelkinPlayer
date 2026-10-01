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
//! evicted least-recently-used once the user-configured disk budget is exceeded.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// `settings.json` key holding the stream cache budget in mebibytes.
pub const CACHE_LIMIT_KEY: &str = "stream.cacheLimitMb";
/// Default budget when the user has not chosen one.
pub const DEFAULT_CACHE_LIMIT_MB: u64 = 1024;
/// Fixed budget for the transient read-ahead tier (not user-facing).
const HOT_CACHE_LIMIT_MB: u64 = 256;

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
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    /// Least-recently-used key order (oldest first) for eviction.
    order: Mutex<Vec<String>>,
}

impl StreamCache {
    /// Create the cache rooted at `<cache_dir>/stream`.
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let dir = resolve_cache_dir(app).join("stream");
        std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建流缓存目录：{error}"))?;
        let client = Client::builder().build().map_err(|error| error.to_string())?;
        Ok(Self {
            dir,
            client,
            limit_bytes: AtomicU64::new(HOT_CACHE_LIMIT_MB.saturating_mul(1024 * 1024)),
            entries: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
        })
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
            if state.complete && state.total.map_or(false, |total| want_start < total) {
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
        let mut state = entry.state.lock().unwrap();
        if duration > 0.0 {
            state.duration = duration;
        }
        if position.is_finite() && position >= 0.0 {
            state.position = position;
        }
        entry.cv.notify_all();
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

    /// Drop every cached track.
    pub fn clear(&self) {
        let drained: Vec<Arc<Entry>> = self
            .entries
            .lock()
            .unwrap()
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        for entry in drained {
            {
                let mut state = entry.state.lock().unwrap();
                state.generation = state.generation.wrapping_add(1);
                entry.cv.notify_all();
            }
            let _ = std::fs::remove_file(&entry.path);
        }
        self.order.lock().unwrap().clear();
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
        let mut order = self.order.lock().unwrap();
        let candidates: Vec<String> = order.clone();
        for key in candidates {
            if total <= limit {
                break;
            }
            let busy = entries
                .get(&key)
                .map(|entry| entry.state.lock().unwrap().downloading)
                .unwrap_or(false);
            if busy {
                continue;
            }
            if let Some(entry) = entries.remove(&key) {
                let size = file_len(&entry.path);
                {
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
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "stream replaced",
                ));
            }
            if let Some((_, message)) = &state.failed {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, message.clone()));
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
            if let Some(total) = state.total {
                if cursor >= total {
                    state.complete = true;
                }
            }
            entry.cv.notify_all();
        }
    }
}

fn mark_complete(entry: &Arc<Entry>, generation: u64) {
    let mut state = entry.state.lock().unwrap();
    if state.generation == generation {
        state.complete = true;
        entry.cv.notify_all();
    }
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// Read the configured stream cache budget (MiB).
pub fn read_cache_limit_mb(app: &AppHandle) -> u64 {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(CACHE_LIMIT_KEY))
        .and_then(|value| value.as_u64())
        .unwrap_or(DEFAULT_CACHE_LIMIT_MB)
}

/// Persist the stream cache budget (MiB).
pub fn write_cache_limit_mb(app: &AppHandle, limit_mb: u64) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(CACHE_LIMIT_KEY, serde_json::json!(limit_mb));
    store.save().map_err(|error| error.to_string())?;
    Ok(())
}
