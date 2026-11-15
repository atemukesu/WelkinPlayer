// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Smart transition analysis.
//!
//! Rather than hand-writing DSP, this module reuses two existing crates:
//!
//! * [`automix`] — BPM / key / energy / loudness / beat analysis (pure DSP, no
//!   C dependency). Its loudness-only `TransitionPointFinder` is *not* used:
//!   mix points come from the beat/phrase grid below instead;
//! * [`held_karp_core`] — an exact Held-Karp TSP solver, used to order a
//!   playlist so every adjacent pair has a good transition.
//!
//! Only the first and last [`WINDOW_SECS`] seconds of each track are decoded:
//! the middle of the file is never analysed. That is enough for both the
//! outgoing (tail) and incoming (head) halves of every transition. Remote
//! tracks are first fetched sequentially into a scratch file (concurrently, a
//! few at a time) so the decoder never has to probe random byte ranges over the
//! network; local files and cached tracks are read in place.
//!
//! The resulting document is small, plain JSON and is stored per playlist, both
//! in the local cache and (when a sync source exists) on WebDAV.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use automix::{Analyzer, AudioBuffer, TrackFeatures};
use rodio::{Decoder, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::commands::media::{asset_hash, resolve_cache_dir};
use crate::core::error::AppError;
use crate::data::backend::safe_join;
use crate::data::sources::{find_source, SourceConfig};
use crate::playback::source::ReadSeek;
use crate::streaming::manual_cache::ManualCache;
use crate::streaming::stream_cache::{open_upstream, response_total, StreamCache};

/// Fallback edge window when the container cannot report a duration. Real
/// tracks scale their windows with [`head_window_secs`] / [`tail_window_secs`].
pub const WINDOW_SECS: f32 = 60.0;
/// Bump when the planning model changes. Stored per-track features are kept
/// across this version and only the transitions are recomputed; bump
/// [`SCHEMA_VERSION`] instead when the extracted features change.
pub const ALGORITHM_VERSION: u32 = 7;
/// On-disk document schema (also invalidates cached per-track features when the
/// feature set or DSP pipeline changes).
pub const SCHEMA_VERSION: u32 = 2;
/// Prefix that scopes transition documents away from audio and other objects.
pub const FILE_PREFIX: &str = "welkin-transition-";

/// The head only needs to cover the intro, so it is a smaller share of the
/// track than the tail. Both scale with duration: a two-minute track should not
/// spend two minutes decoding, and a ten-minute track needs a longer outro
/// search than a fixed sixty seconds.
const HEAD_FRACTION: f32 = 0.20;
const HEAD_MIN_SECS: f32 = 20.0;
const HEAD_MAX_SECS: f32 = 45.0;
const TAIL_FRACTION: f32 = 0.30;
const TAIL_MIN_SECS: f32 = 20.0;
const TAIL_MAX_SECS: f32 = 90.0;

/// Seconds to decode from the start of `duration`.
fn head_window_secs(duration: f32) -> f32 {
    if duration > 1.0 {
        (duration * HEAD_FRACTION).clamp(HEAD_MIN_SECS, HEAD_MAX_SECS)
    } else {
        WINDOW_SECS
    }
}

/// Seconds to decode from the end of `duration`.
fn tail_window_secs(duration: f32) -> f32 {
    if duration > 1.0 {
        (duration * TAIL_FRACTION).clamp(TAIL_MIN_SECS, TAIL_MAX_SECS)
    } else {
        WINDOW_SECS
    }
}

/// One track to analyse, as handed over by the webview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInput {
    /// `sourceId::path`.
    pub key: String,
    pub title: String,
}

/// The edge features `automix` needs, kept in a serializable form.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeFeatures {
    pub bpm: f32,
    pub key: String,
    #[serde(default)]
    pub beats: Vec<f32>,
    #[serde(default)]
    pub mfcc: Vec<f32>,
    #[serde(default)]
    pub chroma: Vec<f32>,
    #[serde(default)]
    pub energy_curve: Vec<f32>,
    #[serde(default)]
    pub loudness_curve_db: Vec<f32>,
    #[serde(default)]
    pub loudness_curve_resolution_seconds: f32,
    /// Phase of the first downbeat inside [`Self::beats`], in beats (`0..4`).
    ///
    /// `automix` only returns onset peaks, so `beats[0]` may be any beat of the
    /// bar. A DJ grid has to start on the downbeat; this offset is the index of
    /// the first beat whose low-band energy puts it at the top of a bar.
    #[serde(default)]
    pub downbeat_offset: u32,
}

impl EdgeFeatures {
    fn from_features(features: &TrackFeatures, downbeat_offset: u32) -> Self {
        Self {
            bpm: round(features.bpm),
            key: features.key.clone(),
            beats: features.beats.iter().map(|v| round(*v)).collect(),
            mfcc: features.mfcc.iter().map(|v| round(*v)).collect(),
            chroma: features.chroma.iter().map(|v| round(*v)).collect(),
            energy_curve: features.energy_curve.iter().map(|v| round(*v)).collect(),
            loudness_curve_db: features
                .loudness_curve_db
                .iter()
                .map(|v| round(*v))
                .collect(),
            loudness_curve_resolution_seconds: features.loudness_curve_resolution_seconds,
            downbeat_offset,
        }
    }

    fn empty(&self) -> bool {
        self.loudness_curve_db.is_empty()
    }

    /// Seconds covered by the stored curves. Used to recover the window length
    /// of documents written before it was recorded explicitly.
    fn span_secs(&self) -> f32 {
        let loud = self.loudness_curve_db.len() as f32 * self.loudness_curve_resolution_seconds;
        if loud > 0.0 {
            loud
        } else {
            self.energy_curve.len() as f32 * ENERGY_RESOLUTION_SECS
        }
    }

    /// Mean RMS energy across the window (used for continuity costs).
    fn mean_energy(&self) -> f32 {
        if self.energy_curve.is_empty() {
            return 0.0;
        }
        self.energy_curve.iter().copied().sum::<f32>() / self.energy_curve.len() as f32
    }

    /// Mean loudness of the whole window in dB. Silence (`-120 dB`) is kept
    /// because it is a real level and the mean is only used as a fallback.
    fn mean_loudness_db(&self) -> Option<f32> {
        let values: Vec<f32> = self
            .loudness_curve_db
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f32>() / values.len() as f32)
        }
    }

    /// Mean energy over `span` seconds starting at `offset` (relative to the
    /// decoded window). Falls back to the whole-window mean when the region is
    /// outside the analysed data.
    fn energy_between(&self, offset: f32, span: f32) -> Option<f32> {
        mean_between(&self.energy_curve, ENERGY_RESOLUTION_SECS, offset, span).or_else(|| {
            if self.energy_curve.is_empty() {
                None
            } else {
                Some(self.mean_energy())
            }
        })
    }

    /// Mean loudness in dB over `span` seconds starting at `offset` (relative
    /// to the decoded window). Falls back to the whole-window mean.
    fn loudness_between(&self, offset: f32, span: f32) -> Option<f32> {
        mean_between(
            &self.loudness_curve_db,
            self.loudness_curve_resolution_seconds,
            offset,
            span,
        )
        .or_else(|| self.mean_loudness_db())
    }
}

/// Resolution of `automix`'s energy curve. The crate does not expose it in
/// `TrackFeatures`, so it is mirrored here (see `Analyzer::compute_energy_curve`).
const ENERGY_RESOLUTION_SECS: f32 = 0.5;

/// Mean of a curve over a time region, mapped through its sample resolution.
fn mean_between(values: &[f32], resolution: f32, offset: f32, span: f32) -> Option<f32> {
    if values.is_empty() || !(resolution.is_finite() && resolution > 0.0) {
        return None;
    }
    let resolution = resolution.max(0.01);
    let start = (offset.max(0.0) / resolution).floor() as usize;
    if start >= values.len() {
        return None;
    }
    let count = ((span.max(0.0) / resolution).ceil() as usize).max(1);
    let end = (start + count).min(values.len());
    let slice = &values[start..end];
    let mut sum = 0.0f32;
    let mut used = 0usize;
    for value in slice {
        if value.is_finite() {
            sum += *value;
            used += 1;
        }
    }
    if used == 0 {
        None
    } else {
        Some(sum / used as f32)
    }
}

/// Per-track analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackAnalysis {
    pub key: String,
    pub title: String,
    pub duration: f32,
    /// Whether the container reported a real total duration. Some remote or
    /// header-less streams cannot, and guessing from the head window would put
    /// a Mix Out point 30 s into a five-minute track. Such tracks can still be
    /// analysed and ordered, but never receive a timed transition plan.
    #[serde(default)]
    pub duration_known: bool,
    pub bpm: f32,
    pub key_camelot: String,
    pub energy: f32,
    pub head: EdgeFeatures,
    pub tail: EdgeFeatures,
    /// Absolute second where the tail window starts. Zero when the tail was
    /// taken from the head (short track or a failed seek), so beat times and
    /// energy curves can be mapped back onto the real timeline.
    #[serde(default)]
    pub tail_offset: f32,
    /// Seconds decoded from the head, recorded so the planner knows how far its
    /// cue search may reach. Older documents omit it; [`Self::head_window`]
    /// then recovers the length from the stored curves.
    #[serde(default)]
    pub head_secs: f32,
    /// Seconds decoded from the tail (see [`Self::tail_window`]).
    #[serde(default)]
    pub tail_secs: f32,
    /// `ok` or `skipped`; skipped tracks never take part in ordering.
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl TrackAnalysis {
    /// Length of the decoded head window in seconds.
    fn head_window(&self) -> f32 {
        if self.head_secs > 0.0 {
            return self.head_secs;
        }
        let span = self.head.span_secs();
        if span > 0.0 && self.duration > 0.0 {
            span.min(self.duration)
        } else if span > 0.0 {
            span
        } else {
            self.duration.max(0.0)
        }
    }

    /// Length of the decoded tail window in seconds.
    fn tail_window(&self) -> f32 {
        if self.tail_secs > 0.0 {
            return self.tail_secs;
        }
        if self.tail_offset > 0.0 && self.duration > 0.0 {
            return (self.duration - self.tail_offset).max(0.0);
        }
        let span = self.tail.span_secs();
        if span > 0.0 && self.duration > 0.0 {
            span.min(self.duration)
        } else if span > 0.0 {
            span
        } else {
            self.duration.max(0.0)
        }
    }

    fn usable(&self) -> bool {
        // `low` still carries usable features; only hard failures are excluded.
        (self.status == "ok" || self.status == "low")
            && !self.head.empty()
            && !self.tail.empty()
            // An unknown duration is fine for ordering (which never places an
            // absolute cue), but a known one must be a real track length.
            && (!self.duration_known || self.duration > 1.0)
    }

    /// Whether a time-based transition plan can be trusted for this track.
    fn plan_ready(&self) -> bool {
        self.usable() && self.duration_known && self.duration > 1.0
    }
}

/// One planned transition between two adjacent tracks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPlan {
    pub from: String,
    pub to: String,
    /// Absolute position in the outgoing track where the fade starts (seconds).
    pub out_at: f32,
    /// Position in the incoming track where it starts (seconds).
    pub in_at: f32,
    /// How long the fade runs (seconds).
    pub blend_secs: f32,
    pub fade_out_curve: String,
    pub fade_in_curve: String,
    /// `strict`, `relaxed` or `fallback`.
    pub mode: String,
    /// Playback-rate multiplier the incoming deck should use so its beats line
    /// up with the outgoing track (octave-folded, `1.0` when unknown or when
    /// the difference is too large to correct musically).
    #[serde(default = "unity")]
    pub tempo_ratio: f32,
    /// Linear gain applied to the incoming deck during the overlap so the two
    /// tracks meet at a similar loudness (`1.0` when unknown).
    #[serde(default = "unity")]
    pub in_gain: f32,
    /// Lower is better: energy/timbre continuity plus a grid-confidence penalty.
    pub score: f32,
}

/// A lightweight transition view for previews and playback scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPreview {
    pub from: String,
    pub to: String,
    pub from_title: String,
    pub to_title: String,
    pub out_at: f32,
    pub in_at: f32,
    pub blend_secs: f32,
    pub mode: String,
    pub score: f32,
}

/// Aggregate counters kept in the document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisStats {
    pub analyzed: usize,
    pub skipped: usize,
    pub reused: usize,
    pub low_confidence: usize,
}

/// The full per-playlist analysis document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDocument {
    pub schema: u32,
    pub algorithm_version: u32,
    pub playlist_id: String,
    pub playlist_name: String,
    pub updated_at: u64,
    pub revision: u64,
    /// Hash of the track *set*, independent of order.
    pub track_set_hash: String,
    /// Hash of the current order.
    pub order_hash: String,
    pub tracks: Vec<TrackAnalysis>,
    pub order: Vec<String>,
    #[serde(default)]
    pub previous_order: Vec<String>,
    #[serde(default)]
    pub transitions: Vec<TransitionPlan>,
    pub stats: AnalysisStats,
    /// `true` for an in-progress snapshot: it holds the tracks analysed so far
    /// and must never be applied or uploaded, only resumed from.
    #[serde(default)]
    pub incomplete: bool,
}

/// One track's lightweight status, included in the summary so the UI can list
/// every analysed (or skipped) track without reading the full document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub key: String,
    pub title: String,
    pub status: String,
    pub bpm: f32,
    pub key_camelot: String,
}

/// Summary returned to the webview after analysis/load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSummary {
    pub playlist_id: String,
    pub file: String,
    pub revision: u64,
    pub updated_at: u64,
    pub order: Vec<String>,
    pub previous_order: Vec<String>,
    pub transitions: Vec<TransitionPreview>,
    pub tracks: Vec<TrackSummary>,
    pub stats: AnalysisStats,
    /// `true` when this is a partial snapshot rather than a finished analysis.
    pub incomplete: bool,
    /// `remote`, `local` or `none` — where the document came from.
    pub source: String,
}

impl AnalysisDocument {
    pub fn summary(&self, file: String, source: &str) -> AnalysisSummary {
        let titles: HashMap<&str, &str> = self
            .tracks
            .iter()
            .map(|track| (track.key.as_str(), track.title.as_str()))
            .collect();
        let transitions = self
            .transitions
            .iter()
            .map(|plan| TransitionPreview {
                from: plan.from.clone(),
                to: plan.to.clone(),
                from_title: titles
                    .get(plan.from.as_str())
                    .copied()
                    .unwrap_or(&plan.from)
                    .to_string(),
                to_title: titles
                    .get(plan.to.as_str())
                    .copied()
                    .unwrap_or(&plan.to)
                    .to_string(),
                out_at: plan.out_at,
                in_at: plan.in_at,
                blend_secs: plan.blend_secs,
                mode: plan.mode.clone(),
                score: plan.score,
            })
            .collect();
        AnalysisSummary {
            playlist_id: self.playlist_id.clone(),
            file,
            revision: self.revision,
            updated_at: self.updated_at,
            order: self.order.clone(),
            previous_order: self.previous_order.clone(),
            transitions,
            tracks: self
                .tracks
                .iter()
                .map(|track| TrackSummary {
                    key: track.key.clone(),
                    title: track.title.clone(),
                    status: track.status.clone(),
                    bpm: track.bpm,
                    key_camelot: track.key_camelot.clone(),
                })
                .collect(),
            stats: self.stats.clone(),
            incomplete: self.incomplete,
            source: source.to_string(),
        }
    }
}

/// Deterministic per-playlist document name.
pub fn file_for_playlist(playlist_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(playlist_id.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("{FILE_PREFIX}{}.json", &hash[..16])
}

fn round(value: f32) -> f32 {
    if value.is_finite() {
        (value * 1000.0).round() / 1000.0
    } else {
        0.0
    }
}

/// Serde default for the transition tuning fields: no correction.
fn unity() -> f32 {
    1.0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn hash_lines<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Hash of the track set, ignoring order.
pub fn track_set_hash(tracks: &[TrackInput]) -> String {
    let mut keys: Vec<&str> = tracks.iter().map(|track| track.key.as_str()).collect();
    keys.sort_unstable();
    hash_lines(keys.into_iter())
}

/// Hash of the order.
pub fn order_hash(order: &[String]) -> String {
    hash_lines(order.iter().map(String::as_str))
}

/// Split a `sourceId::path` key.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    let (source, path) = key.split_once("::")?;
    if source.is_empty() || path.is_empty() {
        return None;
    }
    Some((source, path))
}

fn build_decoder(
    reader: Box<dyn ReadSeek>,
    byte_len: Option<u64>,
) -> Result<Decoder<Box<dyn ReadSeek>>, AppError> {
    let builder = Decoder::builder()
        .with_data(reader)
        .with_seekable(byte_len.is_some());
    let builder = match byte_len {
        Some(len) => builder.with_byte_len(len),
        None => builder,
    };
    builder
        .build()
        .map_err(|error| AppError::other(error.to_string()))
}

/// Decode `max_frames` frames from the decoder's current position.
fn read_frames(
    decoder: &mut Decoder<Box<dyn ReadSeek>>,
    max_frames: usize,
    channels: u16,
) -> Vec<f32> {
    let max_samples = max_frames.saturating_mul(channels as usize);
    let mut samples = Vec::with_capacity(max_samples);
    for sample in decoder.by_ref().take(max_samples) {
        samples.push(sample);
    }
    samples
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn cancel_error() -> AppError {
    AppError::Other("分析已取消".to_string())
}

/// Removes a temporarily downloaded track when analysis leaves its scope.
struct ScratchFile(Option<PathBuf>);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!(
                    "smart-transition: failed to remove scratch {}: {error}",
                    path.display()
                );
            }
        }
    }
}

/// Unique scratch names for concurrent analyses of the same track.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Open the bytes for one track for analysis.
///
/// Local files and tracks already present in the manual/stream cache are read
/// directly. Everything else is downloaded **sequentially in one request** to a
/// scratch file first: the analysis only needs the first and last minute, but
/// seeking a remote stream makes the demuxer probe several remote ranges, which
/// is far slower on high-latency WebDAV than fetching the file in order once.
fn open_analysis_source(
    app: &AppHandle,
    cache: &Arc<StreamCache>,
    manual: &Arc<ManualCache>,
    source_id: &str,
    path: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<(Box<dyn ReadSeek>, Option<u64>, ScratchFile), AppError> {
    let source = find_source(app, source_id)?;
    if source.is_local() {
        let root = source.root_path.clone().unwrap_or_default();
        if root.trim().is_empty() {
            return Err(AppError::Store("本地来源缺少目录".to_string()));
        }
        let file = std::fs::File::open(safe_join(&PathBuf::from(root), path)?)?;
        let len = file.metadata().ok().map(|meta| meta.len());
        return Ok((Box::new(file), len, ScratchFile(None)));
    }

    let key = asset_hash(source_id, path);
    let cached = manual
        .complete_file(&key)
        .or_else(|| cache.cached_file(&key));
    if let Some(file) = cached {
        let file = std::fs::File::open(file)?;
        let len = file.metadata().ok().map(|meta| meta.len());
        return Ok((Box::new(file), len, ScratchFile(None)));
    }

    let dir = resolve_cache_dir(app).join("analysis");
    std::fs::create_dir_all(&dir)?;
    let scratch = dir.join(format!(
        "{key}-{}.audio",
        SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(error) = download_track(app, &source, path, &scratch, cancel, progress) {
        let _ = std::fs::remove_file(&scratch);
        return Err(error);
    }
    let file = match std::fs::File::open(&scratch) {
        Ok(file) => file,
        Err(error) => {
            let _ = std::fs::remove_file(&scratch);
            return Err(error.into());
        }
    };
    let len = file.metadata().ok().map(|meta| meta.len());
    Ok((Box::new(file), len, ScratchFile(Some(scratch))))
}

/// One shared blocking client so consecutive tracks reuse the same TCP/TLS
/// connections instead of paying a handshake per track.
static DOWNLOAD_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn download_client() -> &'static reqwest::blocking::Client {
    DOWNLOAD_CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

/// Fetch a whole remote track into `dest`, reporting byte progress as it goes.
///
/// One sequential GET replaces the old read-ahead cache + seek dance, so high
/// latency does not turn the head/tail windows into a pile of random Range
/// requests.
fn download_track(
    app: &AppHandle,
    source: &SourceConfig,
    path: &str,
    dest: &std::path::Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<u64, AppError> {
    let mut response = open_upstream(app, download_client(), source, path, 0)
        .map_err(|(status, message)| AppError::Http { status, message })?;
    let total = response_total(&response);
    let mut file = std::fs::File::create(dest)?;
    let mut buffer = vec![0u8; 128 * 1024];
    let mut written = 0u64;
    let mut last_emit = std::time::Instant::now();
    progress(0, total);
    loop {
        if cancelled(cancel) {
            drop(file);
            let _ = std::fs::remove_file(dest);
            return Err(cancel_error());
        }
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        written += read as u64;
        if last_emit.elapsed() >= Duration::from_millis(200)
            || total.is_some_and(|size| written >= size)
        {
            progress(written, total);
            last_emit = std::time::Instant::now();
        }
    }
    file.flush()?;
    progress(written, Some(written));
    Ok(written)
}

/// Decode the head and tail windows of one track and analyse each with `automix`.
///
/// `cancel` is polled between the expensive stages (fetch, head decode, tail
/// decode, each `automix` pass) so a running job stops within a few hundred
/// milliseconds instead of waiting for the whole track.
fn analyse_track(
    app: &AppHandle,
    cache: &Arc<StreamCache>,
    manual: &Arc<ManualCache>,
    input: &TrackInput,
    cancel: &AtomicBool,
    stage: &dyn Fn(&'static str),
    download: &dyn Fn(u64, Option<u64>),
) -> Result<TrackAnalysis, AppError> {
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let Some((source_id, path)) = split_key(&input.key) else {
        return Err(AppError::invalid_argument("key", "不是有效的曲目键"));
    };
    let started = std::time::Instant::now();
    // The reader blocks on the remote download as the decoder consumes bytes,
    // so this first stage is where most of a WebDAV track's wall time goes.
    stage("downloading");
    let (reader, byte_len, _scratch) = open_analysis_source(
        app,
        cache,
        manual,
        source_id,
        path,
        cancel,
        download,
    )?;
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let fetch_elapsed = started.elapsed();
    let mut decoder = build_decoder(reader, byte_len)?;
    let channels = decoder.channels().max(1);
    let rate = decoder.sample_rate().max(1);
    let duration = decoder
        .total_duration()
        .map(|value| value.as_secs_f32())
        .filter(|value| value.is_finite() && *value > 0.0);
    let duration_known = duration.is_some();
    let duration = duration.unwrap_or(0.0);

    // Scale the decoded edges with the track instead of always taking 60 s from
    // both ends. The head only has to cover the intro; the tail needs enough
    // room to find the outro. Tracks shorter than the two minimum windows are
    // decoded once and both edges share the whole file.
    let (head_window, tail_window, analyze_once) = if duration > 1.0 {
        if duration <= HEAD_MIN_SECS + TAIL_MIN_SECS {
            (duration, duration, true)
        } else {
            (
                head_window_secs(duration).min(duration),
                tail_window_secs(duration).min(duration),
                false,
            )
        }
    } else {
        (WINDOW_SECS, WINDOW_SECS, true)
    };
    let frames_per_sec = (channels as f32 * rate as f32).max(1.0);
    let head_frames = (head_window * rate as f32) as usize;
    let tail_frames = (tail_window * rate as f32) as usize;

    // Head: decode the first window.
    let head_samples = read_frames(&mut decoder, head_frames, channels);
    if head_samples.is_empty() {
        return Err(AppError::other("无法解码音频"));
    }
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let head_elapsed = started.elapsed();

    // Tail: seek to `duration - tail_window` and decode the rest. A very short
    // track reuses the head samples instead of seeking over itself.
    let tail_samples = if analyze_once {
        Vec::new()
    } else {
        let target = (duration - tail_window).max(0.0);
        match decoder.try_seek(std::time::Duration::from_secs_f32(target)) {
            Ok(()) => read_frames(&mut decoder, tail_frames, channels),
            Err(error) => {
                log::warn!(
                    "smart-transition: tail seek failed for {}: {error}",
                    input.key
                );
                Vec::new()
            }
        }
    };
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let decode_elapsed = started.elapsed();
    // A failed seek still leaves a usable (if less precise) tail. Record where
    // the tail window really starts so its beat times and energy curve can be
    // mapped back onto the absolute timeline.
    let tail_is_head = analyze_once || tail_samples.is_empty();
    let tail_samples = if tail_is_head {
        head_samples.clone()
    } else {
        tail_samples
    };
    let tail_offset = if tail_is_head {
        0.0
    } else {
        (duration - tail_window).max(0.0)
    };
    let head_secs = head_samples.len() as f32 / frames_per_sec;
    let tail_secs = tail_samples.len() as f32 / frames_per_sec;
    // The downbeat vote needs raw samples; compute both envelopes before the
    // buffers are moved into the analyser.
    let head_low = low_band_onset(&head_samples, rate as i32, channels as i32);
    let tail_low = if tail_is_head {
        head_low.clone()
    } else {
        low_band_onset(&tail_samples, rate as i32, channels as i32)
    };

    if cancelled(cancel) {
        return Err(cancel_error());
    }
    // The head/tail windows are fully read at this point, so the network wait
    // for this track is over; only the local DSP remains.
    stage("analyzing");
    let head_buffer = AudioBuffer {
        samples: head_samples,
        sample_rate: rate as i32,
        channels: channels as i32,
    };
    let (head_features, tail_features) = if tail_is_head {
        let features = Analyzer::new()
            .analyze(&head_buffer)
            .map_err(|error| AppError::other(error.to_string()))?;
        (features.clone(), features)
    } else {
        let tail_buffer = AudioBuffer {
            samples: tail_samples,
            sample_rate: rate as i32,
            channels: channels as i32,
        };
        // The two windows are independent, so run their DSP concurrently: a
        // full playlist now takes about half as long on a multi-core machine.
        let (head, tail) = std::thread::scope(|scope| {
            let head = scope.spawn(|| Analyzer::new().analyze(&head_buffer));
            let tail = scope.spawn(|| Analyzer::new().analyze(&tail_buffer));
            let head = head
                .join()
                .map_err(|_| AppError::other("分析线程异常退出"))?
                .map_err(|error| AppError::other(error.to_string()))?;
            let tail = tail
                .join()
                .map_err(|_| AppError::other("分析线程异常退出"))?
                .map_err(|error| AppError::other(error.to_string()))?;
            Ok::<_, AppError>((head, tail))
        })?;
        (head, tail)
    };
    if cancelled(cancel) {
        return Err(cancel_error());
    }

    log::info!(
        "smart-transition: analysed {} in {}ms (fetch {}ms, head {}ms / {:.1}s, decode {}ms / {:.1}s, dsp {}ms)",
        input.key,
        started.elapsed().as_millis(),
        fetch_elapsed.as_millis(),
        head_elapsed.as_millis() - fetch_elapsed.as_millis(),
        head_secs,
        decode_elapsed.as_millis() - head_elapsed.as_millis(),
        tail_secs,
        started.elapsed().as_millis() - decode_elapsed.as_millis(),
    );

    let head = EdgeFeatures::from_features(
        &head_features,
        downbeat_offset(&head_low, &head_features.beats),
    );
    let tail = EdgeFeatures::from_features(
        &tail_features,
        downbeat_offset(&tail_low, &tail_features.beats),
    );
    let confidence_ok = head.bpm > 1.0 && !head.key.is_empty() && duration_known;
    Ok(TrackAnalysis {
        key: input.key.clone(),
        title: input.title.clone(),
        duration: round(duration),
        duration_known,
        bpm: head.bpm,
        key_camelot: head.key.clone(),
        energy: round((head.mean_energy() + tail.mean_energy()) / 2.0),
        head,
        tail,
        tail_offset: round(tail_offset),
        head_secs: round(head_secs),
        tail_secs: round(tail_secs),
        status: if confidence_ok {
            "ok".into()
        } else {
            "low".into()
        },
        error: None,
    })
}

fn camelot_parts(key: &str) -> Option<(u8, char)> {
    let key = key.trim();
    if key.len() < 2 {
        return None;
    }
    let (number, letter) = key.split_at(key.len() - 1);
    let number: u8 = number.parse().ok()?;
    let letter = letter.chars().next()?.to_ascii_uppercase();
    if !(1..=12).contains(&number) || (letter != 'A' && letter != 'B') {
        return None;
    }
    Some((number, letter))
}

/// Weighted cost (lower is a better transition) between two analysed tracks.
///
/// Every component is normalised to roughly `0..=1`: tempo (octave-folded),
/// Camelot key distance, tail→head energy continuity, timbre similarity and an
/// analysis-confidence penalty.
pub fn transition_cost(a: &TrackAnalysis, b: &TrackAnalysis) -> f64 {
    // Tempo: fold by octaves so 90 vs 180 BPM counts as a match.
    let bpm_cost = if a.bpm > 1.0 && b.bpm > 1.0 {
        let ratio = (a.bpm / b.bpm).ln() / std::f32::consts::LN_2;
        (ratio - ratio.round()).abs() as f64 * 2.0
    } else {
        0.6
    }
    .min(1.0);

    // Key: identical Camelot is free; relative major/minor is cheap; moving
    // around the wheel costs more.
    let key_cost = match (camelot_parts(&a.key_camelot), camelot_parts(&b.key_camelot)) {
        (Some((na, la)), Some((nb, lb))) => {
            let distance = (na as i32 - nb as i32)
                .unsigned_abs()
                .min(12 - (na as i32 - nb as i32).unsigned_abs());
            let mut cost = distance as f64 * 0.06;
            if la != lb {
                cost += if distance == 0 { 0.2 } else { 0.35 };
            }
            cost.min(1.0)
        }
        _ => 0.5,
    };

    // Energy continuity: outgoing tail vs incoming head.
    let energy_cost = (a.tail.mean_energy() - b.head.mean_energy()).abs().min(1.0) as f64;

    // Timbre: cosine similarity of chroma + MFCC vectors.
    let chroma = cosine(&a.tail.chroma, &b.head.chroma);
    let mfcc = cosine(&a.tail.mfcc, &b.head.mfcc);
    let timbre_cost = (1.0 - (chroma * 0.7 + mfcc * 0.3)).clamp(0.0, 1.0) as f64;

    // Confidence: penalise tracks whose tempo/key could not be determined.
    let mut confidence: f32 = 0.0;
    if a.bpm <= 1.0 || b.bpm <= 1.0 {
        confidence += 0.5;
    }
    if camelot_parts(&a.key_camelot).is_none() || camelot_parts(&b.key_camelot).is_none() {
        confidence += 0.3;
    }
    if a.tail.empty() || b.head.empty() {
        confidence += 0.2;
    }

    0.35 * bpm_cost
        + 0.20 * key_cost
        + 0.20 * energy_cost
        + 0.15 * timbre_cost
        + 0.10 * confidence.min(1.0) as f64
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.5;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 0.5;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Build the pairwise cost matrix for the usable tracks.
fn cost_matrix(tracks: &[&TrackAnalysis]) -> Vec<Vec<f64>> {
    let n = tracks.len();
    let mut matrix = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in 0..n {
            if i != j {
                matrix[i][j] = transition_cost(tracks[i], tracks[j]);
            }
        }
    }
    matrix
}

fn path_cost(path: &[usize], matrix: &[Vec<f64>]) -> f64 {
    path.windows(2).map(|pair| matrix[pair[0]][pair[1]]).sum()
}

/// Exact solver for small playlists (Held-Karp), greedy + 2-opt above 15 tracks.
/// The exact solver is exponential: 19 tracks already spends seconds in a dev
/// build for no audible gain over the greedy path.
fn best_order(matrix: &[Vec<f64>], fixed_first: Option<usize>) -> Vec<usize> {
    let n = matrix.len();
    if n <= 1 {
        return (0..n).collect();
    }
    if n <= 15 {
        // Add a dummy node 0 with zero-cost return, turning the open path into
        // a cycle the TSP solver can handle. A fixed first track is enforced by
        // making the dummy's edge to every other track prohibitively expensive.
        let m = n + 1;
        let mut distance = vec![0.0f64; m * m];
        for i in 0..n {
            distance[i + 1] = if fixed_first.is_none() || fixed_first == Some(i) {
                0.0
            } else {
                1_000_000.0
            };
        }
        for i in 0..n {
            for j in 0..n {
                distance[(i + 1) * m + (j + 1)] = matrix[i][j];
            }
        }
        if let Ok((path, _)) = held_karp_core::solve(&distance, m) {
            let order: Vec<usize> = path
                .into_iter()
                .filter(|index| *index != 0)
                .map(|index| index - 1)
                .collect();
            if order.len() == n {
                return order;
            }
        }
    }

    // Fallback for large playlists: nearest neighbour from the fixed start,
    // then 2-opt improvement over the open path.
    let start = fixed_first.unwrap_or(0);
    let mut visited = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut current = start;
    visited[current] = true;
    order.push(current);
    while order.len() < n {
        let next = (0..n)
            .filter(|index| !visited[*index])
            .min_by(|a, b| {
                matrix[current][*a]
                    .partial_cmp(&matrix[current][*b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        visited[next] = true;
        order.push(next);
        current = next;
    }
    improve_path(&mut order, matrix, fixed_first.is_some());
    order
}

/// 2-opt on an open path.
fn improve_path(order: &mut [usize], matrix: &[Vec<f64>], fixed_first: bool) {
    let n = order.len();
    if n < 3 {
        return;
    }
    let mut best = path_cost(order, matrix);
    let lower = if fixed_first { 2 } else { 1 };
    for _ in 0..40 {
        let mut improved = false;
        for i in lower..n - 1 {
            for j in i + 1..n {
                order[i..=j].reverse();
                let cost = path_cost(order, matrix);
                if cost + 1e-9 < best {
                    best = cost;
                    improved = true;
                } else {
                    order[i..=j].reverse();
                }
            }
        }
        if !improved {
            break;
        }
    }
}

/// Beats per bar in the 4/4 grids we assume for dance/electronic material.
const BEATS_PER_BAR: f32 = 4.0;
/// Bars per musical phrase. 8 bars is the common DJ cue length.
const PHRASE_BARS: f32 = 8.0;
/// Beats in one phrase line (32 by default).
const PHRASE_BEATS: f32 = BEATS_PER_BAR * PHRASE_BARS;

/// Frame rate of the low-band onset envelope used for downbeat detection.
const DOWNBEAT_FPS: f32 = 100.0;
/// One-pole low-pass cutoff that keeps kick and bass in the downbeat vote.
const DOWNBEAT_LOW_HZ: f32 = 150.0;

/// Positive onset of the low band, sampled at [`DOWNBEAT_FPS`] frames/second.
///
/// This is the cheapest signal that tells a kick from a hi-hat: a first-order
/// low-pass followed by the positive difference of its energy. It is only ever
/// used to choose between the four bar phases, not to place mix points.
fn low_band_onset(samples: &[f32], sample_rate: i32, channels: i32) -> Vec<f32> {
    if samples.is_empty() || sample_rate <= 0 || channels <= 0 {
        return Vec::new();
    }
    let rate = sample_rate as f32;
    let channels = channels as usize;
    let hop = ((rate / DOWNBEAT_FPS) as usize).max(1);
    let frame_samples = hop * channels;
    let frames = samples.len() / frame_samples;
    if frames < 4 {
        return Vec::new();
    }
    let dt = 1.0 / rate;
    let rc = 1.0 / (std::f32::consts::TAU * DOWNBEAT_LOW_HZ);
    let alpha = dt / (rc + dt);
    let mut y = 0.0f32;
    let mut previous = 0.0f32;
    let mut onset = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * frame_samples;
        let mut sum = 0.0f32;
        for channel in 0..channels {
            sum += samples[base + channel];
        }
        let x = sum / channels as f32;
        y += alpha * (x - y);
        let energy = y * y;
        onset.push((energy - previous).max(0.0));
        previous = energy;
    }
    onset
}

/// Beat index of the first downbeat, given the low-band onset envelope.
///
/// `automix` returns onset peaks, not downbeats, so `beats[0]` may be any beat
/// of the bar. This sums the low-band onset at each beat for the four possible
/// bar phases and returns the strongest one. `0` is returned when there is not
/// enough evidence (fewer than two bars), preserving the old anchor.
fn downbeat_offset(onset: &[f32], beats: &[f32]) -> u32 {
    let frames = onset.len();
    if frames == 0 || beats.len() < 8 {
        return 0;
    }
    let mut sums = [0.0f64; 4];
    let mut counts = [0usize; 4];
    for (index, beat) in beats.iter().enumerate() {
        if !beat.is_finite() || *beat < 0.0 {
            continue;
        }
        let center = (*beat * DOWNBEAT_FPS).round() as isize;
        if center < 0 || center as usize >= frames {
            continue;
        }
        // A kick's onset lands a few frames either side of the nominal beat.
        let mut peak = 0.0f32;
        for delta in -3..=3 {
            let frame = center + delta;
            if frame >= 0 && (frame as usize) < frames {
                peak = peak.max(onset[frame as usize]);
            }
        }
        let phase = index % 4;
        sums[phase] += peak as f64;
        counts[phase] += 1;
    }
    let mut best = 0usize;
    let mut best_score = 0.0f64;
    for phase in 0..4 {
        if counts[phase] == 0 {
            continue;
        }
        let score = sums[phase] / counts[phase] as f64;
        if score > best_score + 1e-9 {
            best_score = score;
            best = phase;
        }
    }
    best as u32
}

/// The beat that starts the first bar, used as the grid anchor.
fn downbeat_time(beats: &[f32], offset: u32) -> Option<f32> {
    beats
        .get(offset as usize)
        .copied()
        .or_else(|| beats.first().copied())
}

/// A beat grid anchored on the first downbeat.
///
/// Only the head/tail windows are decoded, so the anchor is that window's first
/// downbeat (mapped to absolute time via [`TrackAnalysis::tail_offset`] for the
/// tail). Phrase lines land every `PHRASE_BEATS` beats from the anchor, so both
/// sides of a transition share the same bar phase instead of drifting by 1–3
/// beats because their first onset peak happened to fall elsewhere.
/// This mirrors ClubCraft's `analysis/beatgrid.rs` + `analysis/cues.rs` rules
/// (MIT/Apache-2.0), reimplemented here without pulling in a new dependency.
#[derive(Debug, Clone, Copy)]
struct BeatGrid {
    anchor: f32,
    period: f32,
}

impl BeatGrid {
    /// Build a grid from a tempo. When no beat was detected the grid still
    /// exists (anchored at zero) so phrase math degenerates to a regular
    /// BPM-only grid — this is our `relaxed` mode.
    fn new(bpm: f32, anchor_beat: Option<f32>) -> Option<Self> {
        if !(bpm.is_finite() && bpm > 1.0) {
            return None;
        }
        let period = 60.0 / bpm;
        if !(period.is_finite() && (0.02..=3.0).contains(&period)) {
            return None;
        }
        let anchor = anchor_beat
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
            .max(0.0);
        Some(Self { anchor, period })
    }

    fn beats_at(&self, secs: f32) -> f32 {
        (secs - self.anchor) / self.period
    }

    fn time_at(&self, beats: f32) -> f32 {
        self.anchor + beats * self.period
    }

    /// First phrase line at or after `min` (with a small tolerance so a line
    /// that sits exactly on `min` counts as "at or after").
    fn phrase_at_or_after(&self, min: f32, span: f32) -> f32 {
        let index = (self.beats_at(min) / span - 1e-3).ceil().max(0.0);
        self.time_at(index * span)
    }

    /// Last phrase line at or before `max`.
    fn phrase_at_or_before(&self, max: f32, span: f32) -> f32 {
        let index = (self.beats_at(max) / span + 1e-3).floor().max(0.0);
        self.time_at(index * span)
    }
}

/// Tempo to build the grid from, preferring the window's own estimate.
fn grid_bpm(edge_bpm: f32, track_bpm: f32) -> f32 {
    if edge_bpm > 1.0 {
        edge_bpm
    } else {
        track_bpm
    }
}

/// Absolute time of the outgoing grid's anchor.
fn tail_grid(track: &TrackAnalysis) -> Option<BeatGrid> {
    BeatGrid::new(
        grid_bpm(track.tail.bpm, track.bpm),
        // Tail beat times are relative to the tail window; lift them onto the
        // absolute timeline before using them as the anchor. The anchor is the
        // first *downbeat*, not the first onset peak.
        downbeat_time(&track.tail.beats, track.tail.downbeat_offset)
            .map(|beat| beat + track.tail_offset),
    )
}

fn head_grid(track: &TrackAnalysis) -> Option<BeatGrid> {
    BeatGrid::new(
        grid_bpm(track.head.bpm, track.bpm),
        downbeat_time(&track.head.beats, track.head.downbeat_offset),
    )
}

/// Earliest musical entry point for the incoming track.
///
/// The first 32-beat phrase line after a short, duration-scaled skip, unless
/// that would skip more than 40 % of a short track — then start at 0.
fn mix_in(grid: Option<&BeatGrid>, duration: f32) -> f32 {
    let Some(grid) = grid else {
        return 0.0;
    };
    let skip = if duration > 1.0 {
        (duration * 0.08).clamp(4.0, 15.0)
    } else {
        15.0
    };
    let line = grid.phrase_at_or_after(skip, PHRASE_BEATS);
    if duration > 1.0 && line > duration * 0.4 {
        0.0
    } else {
        line.clamp(0.0, duration.max(0.0))
    }
}

/// Latest musical exit point for the outgoing track.
///
/// The fade still finishes with the track, so the Mix Out point sits on the last
/// bar line before `musical_end - blend`. `musical_end` is the last moment with
/// meaningful audio; trailing digital silence is not a mix target.
fn mix_out(grid: Option<&BeatGrid>, musical_end: f32, tail_offset: f32) -> f32 {
    let Some(grid) = grid else {
        return (musical_end - preferred_blend_secs(None))
            .max(tail_offset.max(0.0))
            .max(0.0);
    };
    if musical_end <= 1.0 {
        return 0.0;
    }
    let blend = preferred_blend_secs(Some(grid));
    let line = grid.phrase_at_or_before((musical_end - blend).max(0.0), BEATS_PER_BAR);
    line.clamp(tail_offset.max(0.0).min(musical_end), musical_end)
}

/// Default crossfade target: four bars, shortened to fit the 2.5-second cap.
const BLEND_BARS: f32 = 4.0;
/// Hard ceiling for the envelope, even at very slow tempos.
const MAX_BLEND_SECS: f32 = 2.5;
/// Below this the transition stops sounding like a mix.
const MIN_BLEND_SECS: f32 = 2.0;
/// Tracks shorter than this get a short fade instead of a planned mix.
const MIN_TRACK_SECS: f32 = 15.0;

/// Fade length that fits the track's tempo: four bars, capped at 2.5 seconds
/// so a transition never spends too long overlapping. This is the *intended*
/// length; the actual envelope may shrink when the tail is shorter.
fn preferred_blend_secs(grid: Option<&BeatGrid>) -> f32 {
    let Some(grid) = grid else {
        return MAX_BLEND_SECS;
    };
    let bar = BEATS_PER_BAR * grid.period;
    if !(bar.is_finite() && bar > 0.0) {
        return MAX_BLEND_SECS;
    }
    let bars = (MAX_BLEND_SECS / bar).floor().clamp(1.0, BLEND_BARS);
    (bars * bar).clamp(MIN_BLEND_SECS, MAX_BLEND_SECS)
}

/// Fade length in seconds: the tempo's preferred four-bar blend, shrunk to
/// whole bars when the outgoing track has less tail than that.
///
/// Deliberately *not* "the largest phrase that fits": stretching the fade to
/// fill a long tail is what made every 120 BPM track overlap for 16 s.
fn blend_secs(grid: Option<&BeatGrid>, out_at: f32, musical_end: f32) -> f32 {
    let remaining = (musical_end - out_at).max(0.0);
    let Some(grid) = grid else {
        return remaining.clamp(1.5, MAX_BLEND_SECS);
    };
    let preferred = preferred_blend_secs(Some(grid));
    if remaining + 1e-3 >= preferred {
        return preferred;
    }
    let bar = BEATS_PER_BAR * grid.period;
    if !(bar.is_finite() && bar > 0.0) {
        return remaining;
    }
    let bars = (remaining / bar).floor();
    if bars >= 1.0 {
        (bars * bar).min(remaining)
    } else {
        remaining
    }
}

/// Grid lines spaced `span` beats apart between `min` and `max`, ascending.
fn grid_lines(grid: &BeatGrid, min: f32, max: f32, span: f32) -> Vec<f32> {
    let mut lines = Vec::new();
    if !(min.is_finite() && max.is_finite()) || max <= min {
        return lines;
    }
    let step = span * grid.period;
    if !(step.is_finite() && step > 0.0) {
        return lines;
    }
    let mut line = grid.phrase_at_or_after(min, span);
    while line <= max + 1e-3 && lines.len() < 64 {
        lines.push(line);
        line += step;
    }
    lines
}

/// Below this the tail is treated as digital silence.
const SILENCE_DB: f32 = -55.0;
/// Energy floor for backing up the loudness-based silence trim.
const SILENCE_ENERGY: f32 = 0.02;

/// Mean energy of the outgoing tail over `span` seconds starting at `at`.
fn tail_energy(track: &TrackAnalysis, at: f32, span: f32) -> Option<f32> {
    let start = (at - track.tail_offset).max(0.0);
    track.tail.energy_between(start, span.max(0.0))
}

/// Last moment of the outgoing track that still carries meaningful audio.
///
/// Digital silence at the end of a file is not a mix target; trimming it stops
/// the planner from choosing a point that is literally quiet. If the whole tail
/// is silent the file end is returned so a cue still exists.
fn musical_end(track: &TrackAnalysis) -> f32 {
    let end = track.duration.max(0.0);
    if end <= 1.0 {
        return end;
    }
    let floor = track.tail_offset.max(0.0);
    let edge = &track.tail;
    let loud_end = if edge.loudness_curve_resolution_seconds > 0.0 {
        edge.loudness_curve_db
            .iter()
            .rposition(|db| db.is_finite() && *db > SILENCE_DB)
            .map(|index| {
                track.tail_offset + (index as f32 + 1.0) * edge.loudness_curve_resolution_seconds
            })
    } else {
        None
    };
    let energy_end = edge
        .energy_curve
        .iter()
        .rposition(|value| value.is_finite() && *value > SILENCE_ENERGY)
        .map(|index| track.tail_offset + (index as f32 + 1.0) * ENERGY_RESOLUTION_SECS);
    let detected = loud_end.or(energy_end).unwrap_or(end).clamp(floor, end);
    if detected - floor < MIN_BLEND_SECS {
        // The tail is silent throughout: fall back to the file end.
        end
    } else {
        detected
    }
}

/// Find where the outgoing track steps down into its outro.
///
/// Walks forward from the start of the tail window and returns the first point
/// where the following 6–12 s are at most 60 % as loud as the preceding window
/// *and* the rest of the track stays down. A later point inside a fade-out does
/// not qualify because the walk starts at the beginning of the tail; this keeps
/// the cue at the start of the outro instead of its quietest end.
fn detect_outro_start(track: &TrackAnalysis, musical_end: f32) -> Option<f32> {
    let start = track.tail_offset.max(0.0);
    let available = musical_end - start;
    if available < 12.0 {
        return None;
    }
    let look = (track.tail_window() * 0.2)
        .clamp(6.0, 12.0)
        .min(available * 0.5);
    if look < 3.0 {
        return None;
    }
    let mut at = start + look;
    while at <= musical_end - look + 1e-3 {
        let before = tail_energy(track, at - look, look)?;
        let after = tail_energy(track, at, look)?;
        if before > SILENCE_ENERGY && after <= before * 0.6 {
            let rest = tail_energy(track, at, musical_end - at)?;
            if rest <= before * 0.7 {
                return Some(at);
            }
        }
        at += 1.0;
    }
    None
}

/// Candidate Mix Out points across the whole decoded tail window, plus the
/// fallback. The fade itself is capped at five seconds; the *start* is chosen
/// musically (outro drop, energy/timbre continuity), so it is not forced to the
/// very last bar of the file or limited to a fixed last thirty seconds.
fn mix_out_candidates(
    grid: Option<&BeatGrid>,
    duration: f32,
    musical_end: f32,
    tail_offset: f32,
    tail_window: f32,
) -> Vec<f32> {
    let fallback = mix_out(grid, musical_end, tail_offset);
    let Some(grid) = grid else {
        return vec![fallback];
    };
    if musical_end <= 1.0 {
        return vec![0.0];
    }
    let floor = tail_offset.max(0.0);
    // Never skip more than half of a short track; on longer tracks the tail
    // window already starts well past the halfway point.
    let min = (musical_end - tail_window)
        .max(floor)
        .max(duration * 0.5)
        .max(0.0);
    let max = musical_end - MIN_BLEND_SECS;
    if max <= min {
        return vec![fallback];
    }
    let mut lines = grid_lines(grid, min, max, BEATS_PER_BAR);
    if !lines.iter().any(|line| (*line - fallback).abs() < 0.05) {
        lines.push(fallback);
    }
    lines.retain(|line| line.is_finite() && *line >= 0.0 && *line <= musical_end);
    if lines.is_empty() {
        vec![fallback]
    } else {
        lines
    }
}

/// Candidate Mix In points: phrase lines in the analysed head window, so the
/// incoming track can enter a little later when that improves continuity.
fn mix_in_candidates(grid: Option<&BeatGrid>, duration: f32, head_window: f32) -> Vec<f32> {
    let fallback = mix_in(grid, duration);
    let Some(grid) = grid else {
        return vec![fallback];
    };
    if duration <= 1.0 {
        return vec![0.0];
    }
    let skip = (duration * 0.08).clamp(4.0, 15.0);
    let max = (duration * 0.4).min(head_window).max(skip);
    let mut lines = grid_lines(grid, skip, max, PHRASE_BEATS);
    lines.retain(|line| line.is_finite() && *line <= duration);
    if lines.is_empty() {
        lines.push(fallback);
    }
    lines
}

/// Fold a tempo ratio into one octave and only keep it when the correction is
/// small enough to be musical. Large differences are left to drift instead of
/// sounding like a chipmunk.
fn fold_tempo_ratio(out_bpm: f32, in_bpm: f32) -> f32 {
    if !(out_bpm.is_finite() && in_bpm.is_finite()) || out_bpm <= 1.0 || in_bpm <= 1.0 {
        return 1.0;
    }
    let mut ratio = out_bpm / in_bpm;
    if !(ratio.is_finite() && ratio > 0.0) {
        return 1.0;
    }
    while ratio < std::f32::consts::FRAC_1_SQRT_2 {
        ratio *= 2.0;
    }
    while ratio >= std::f32::consts::SQRT_2 {
        ratio /= 2.0;
    }
    if (0.90..=1.10).contains(&ratio) {
        ratio
    } else {
        1.0
    }
}

/// Linear gain that brings the incoming window to the outgoing window's level.
/// Attenuation is capped at -12 dB and boost at +12 dB; differences below
/// 0.25 dB are ignored so normal masters are not touched.
fn loudness_gain(out_db: Option<f32>, in_db: Option<f32>) -> f32 {
    match (out_db, in_db) {
        (Some(out), Some(input)) if out.is_finite() && input.is_finite() => {
            let db = (out - input).clamp(-12.0, 12.0);
            let gain = 10f32.powf(db / 20.0).clamp(0.25, 4.0);
            if (gain - 1.0).abs() < 0.03 {
                1.0
            } else {
                gain
            }
        }
        _ => 1.0,
    }
}

/// Continuity score for one candidate cue pair. Lower is better.
fn cue_score(
    out: &TrackAnalysis,
    input: &TrackAnalysis,
    out_at: f32,
    in_at: f32,
    blend: f32,
    musical_end: f32,
    outro_start: Option<f32>,
) -> (f32, Option<f32>, Option<f32>) {
    let tail_window = out.tail_window().max(1.0);
    let head_window = input.head_window().max(1.0);
    let out_energy = out
        .tail
        .energy_between(out_at - out.tail_offset, blend)
        .unwrap_or_else(|| out.tail.mean_energy());
    let in_energy = input
        .head
        .energy_between(in_at, blend)
        .unwrap_or_else(|| input.head.mean_energy());
    let energy_gap = (out_energy - in_energy).abs().min(1.0);
    let timbre_gap = (1.0 - cosine(&out.tail.chroma, &input.head.chroma)).clamp(0.0, 1.0);
    let out_db = out.tail.loudness_between(out_at - out.tail_offset, blend);
    let in_db = input.head.loudness_between(in_at, blend);
    let loud_gap = match (out_db, in_db) {
        (Some(a), Some(b)) => ((a - b).abs() / 18.0).min(1.0),
        _ => 0.0,
    };
    // Reward a step down into a quieter section: that is where an outro starts
    // and where a DJ would mix out. The lookback scales with the track so a long
    // outro is still recognised, instead of only a sharp four-second drop.
    let look = (tail_window * 0.2).clamp(6.0, 12.0);
    let before = out
        .tail
        .energy_between((out_at - look - out.tail_offset).max(0.0), look);
    let drop = match before {
        Some(before) if before > SILENCE_ENERGY => {
            ((before - out_energy).max(0.0) / before).min(1.0)
        }
        _ => 0.0,
    };
    // Pull the cue towards the detected start of the outro. Without a clear
    // step down this is zero and the mild skip penalty below applies instead.
    let proximity = outro_start
        .map(|start| {
            let scale = (tail_window * 0.15).clamp(6.0, 15.0);
            (1.0 - (out_at - start).abs() / scale).clamp(0.0, 1.0)
        })
        .unwrap_or(0.0);
    // Mildly prefer not to skip too much of the outgoing track, and avoid a
    // transition so short that it sounds like a cut.
    let skip = (musical_end - (out_at + blend)).max(0.0);
    let skip_penalty = (skip / tail_window).min(1.0) * 0.10;
    let short_penalty = ((MIN_BLEND_SECS - blend).max(0.0) / MIN_BLEND_SECS) * 0.15;
    // Prefer an earlier Mix In (less of the intro skipped) when continuity is
    // otherwise equal.
    let intro_penalty = (in_at.max(0.0) / head_window).min(1.0) * 0.05;
    // Do not choose a point that is essentially silent just because the incoming
    // track starts quietly. The reference is the whole track's energy, not the
    // quiet tail's own mean.
    let reference = out.energy.max(out.tail.mean_energy()).max(0.02);
    let silence_floor = reference * 0.15;
    let silence_penalty = if out_energy < silence_floor {
        ((silence_floor - out_energy) / silence_floor).min(1.0) * 0.20
    } else {
        0.0
    };
    let score = energy_gap * 0.30
        + timbre_gap * 0.30
        + loud_gap * 0.10
        + skip_penalty
        + short_penalty
        + intro_penalty
        - drop * 0.20
        - proximity * 0.15
        + silence_penalty;
    (score, out_db, in_db)
}

fn plan_transition(out: &TrackAnalysis, input: &TrackAnalysis) -> TransitionPlan {
    let out_grid = tail_grid(out);
    let in_grid = head_grid(input);
    let musical_end = musical_end(out);
    let outro_start = detect_outro_start(out, musical_end);

    // `strict` means both windows carried real beat detections; `relaxed` means
    // at least one grid had to fall back to the BPM alone; `fallback` is a
    // tempo-less pair with no musical grid at all.
    let out_has_beats = !out.tail.beats.is_empty();
    let in_has_beats = !input.head.beats.is_empty();
    let mode = if out_has_beats && in_has_beats {
        "strict"
    } else if out_grid.is_some() || in_grid.is_some() {
        "relaxed"
    } else {
        "fallback"
    };

    // Very short tracks have no room for a planned mix; fade quickly instead of
    // skipping most of either track.
    let short_track = out.duration < MIN_TRACK_SECS || input.duration < MIN_TRACK_SECS;
    let (mut out_at, mut in_at, mut blend) = if short_track {
        let blend = (out.duration.min(input.duration) * 0.1).clamp(0.5, 1.5);
        ((musical_end - blend).max(0.0), 0.0, blend)
    } else {
        let out_at = mix_out(out_grid.as_ref(), musical_end, out.tail_offset);
        let in_at = mix_in(in_grid.as_ref(), input.duration);
        let blend = blend_secs(out_grid.as_ref(), out_at, musical_end);
        (out_at, in_at, blend)
    };
    let mut out_db = out.tail.loudness_between(out_at - out.tail_offset, blend);
    let mut in_db = input.head.loudness_between(in_at, blend);

    // Whenever both sides have a tempo grid, try every sensible bar/phrase
    // pair across the decoded tail and keep the most musical one. Only a truly
    // tempo-less pair (or a very short track) uses the fixed fallbacks.
    if !short_track {
        if let (Some(out_grid), Some(in_grid)) = (out_grid.as_ref(), in_grid.as_ref()) {
            let outs = mix_out_candidates(
                Some(out_grid),
                out.duration,
                musical_end,
                out.tail_offset,
                out.tail_window(),
            );
            let ins = mix_in_candidates(Some(in_grid), input.duration, input.head_window());
            let mut best: Option<(f32, f32, f32, f32, Option<f32>, Option<f32>)> = None;
            for candidate_out in outs {
                let candidate_blend = blend_secs(Some(out_grid), candidate_out, musical_end);
                if candidate_blend < MIN_BLEND_SECS {
                    continue;
                }
                for candidate_in in &ins {
                    if *candidate_in > input.duration.max(0.0) {
                        continue;
                    }
                    let (score, db_out, db_in) = cue_score(
                        out,
                        input,
                        candidate_out,
                        *candidate_in,
                        candidate_blend,
                        musical_end,
                        outro_start,
                    );
                    if best
                        .as_ref()
                        .map(|current| score < current.0)
                        .unwrap_or(true)
                    {
                        best = Some((
                            score,
                            candidate_out,
                            *candidate_in,
                            candidate_blend,
                            db_out,
                            db_in,
                        ));
                    }
                }
            }
            if let Some((_, best_out, best_in, best_blend, best_out_db, best_in_db)) = best {
                out_at = best_out;
                in_at = best_in;
                blend = best_blend;
                out_db = best_out_db;
                in_db = best_in_db;
            }
        }
    }

    let tempo_ratio = fold_tempo_ratio(
        grid_bpm(out.tail.bpm, out.bpm),
        grid_bpm(input.head.bpm, input.bpm),
    );
    let in_gain = loudness_gain(out_db, in_db);

    let (cue, _, _) = cue_score(out, input, out_at, in_at, blend, musical_end, outro_start);
    let mode_penalty = match mode {
        "strict" => 0.0,
        "relaxed" => 0.12,
        _ => 0.25,
    };
    let score = (cue + mode_penalty).min(1.0);

    let curve = if blend >= 4.0 {
        "linearMedium"
    } else {
        "linearFast"
    };
    TransitionPlan {
        from: out.key.clone(),
        to: input.key.clone(),
        out_at: round(out_at),
        in_at: round(in_at),
        blend_secs: round(blend),
        fade_out_curve: curve.to_string(),
        fade_in_curve: curve.to_string(),
        mode: mode.to_string(),
        tempo_ratio: round(tempo_ratio),
        in_gain: round(in_gain),
        score: round(score),
    }
}

/// Generate a transition for each adjacent usable pair in `order`.
fn build_transitions(tracks: &[TrackAnalysis], order: &[String]) -> Vec<TransitionPlan> {
    let by_key: HashMap<&str, &TrackAnalysis> = tracks
        .iter()
        .map(|track| (track.key.as_str(), track))
        .collect();
    let mut transitions = Vec::new();
    for pair in order.windows(2) {
        if let (Some(out), Some(input)) =
            (by_key.get(pair[0].as_str()), by_key.get(pair[1].as_str()))
        {
            if out.plan_ready() && input.plan_ready() {
                transitions.push(plan_transition(out, input));
            }
        }
    }
    transitions
}

/// Recompute only the transition plans from the cached per-track features.
///
/// A change to the planning model must not force every track to be decoded
/// again: decoding and the DSP passes dominate the run time, while planning is
/// a cheap pass over the stored features. Call this after loading a document
/// whose [`ALGORITHM_VERSION`] is older than the current one. Changes to the
/// extracted features themselves are invalidated through [`SCHEMA_VERSION`].
pub fn refresh_plans(document: &mut AnalysisDocument) {
    if document.algorithm_version == ALGORITHM_VERSION {
        return;
    }
    document.transitions = build_transitions(&document.tracks, &document.order);
    document.algorithm_version = ALGORITHM_VERSION;
}

/// Build the analysis document for a set of tracks.
///
/// `smart_order` selects the Held-Karp ordering (finished analysis) versus the
/// input order used by in-progress snapshots, which keeps partial updates cheap
/// even for large playlists.
#[allow(clippy::too_many_arguments)]
fn assemble_document(
    playlist_id: &str,
    playlist_name: &str,
    revision: u64,
    inputs: &[TrackInput],
    tracks: Vec<TrackAnalysis>,
    previous_order: Vec<String>,
    stats: AnalysisStats,
    fixed_first: Option<&str>,
    smart_order: bool,
) -> AnalysisDocument {
    let order: Vec<String> = if smart_order {
        // Usable tracks take part in ordering; skipped ones are appended in
        // input order so the playlist never loses an entry.
        let usable_indices: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.usable())
            .map(|(index, _)| index)
            .collect();
        let usable_refs: Vec<&TrackAnalysis> =
            usable_indices.iter().map(|index| &tracks[*index]).collect();
        let matrix = cost_matrix(&usable_refs);
        let fixed_index = fixed_first.and_then(|key| {
            usable_indices
                .iter()
                .position(|index| tracks[*index].key == key)
        });
        let order_indices = best_order(&matrix, fixed_index);
        let mut order: Vec<String> = order_indices
            .iter()
            .map(|index| tracks[usable_indices[*index]].key.clone())
            .collect();
        for track in tracks.iter() {
            if !track.usable() {
                order.push(track.key.clone());
            }
        }
        order
    } else {
        // In-progress snapshot: keep the input order, no TSP solve.
        tracks.iter().map(|track| track.key.clone()).collect()
    };

    let transitions = build_transitions(&tracks, &order);

    let mut document = AnalysisDocument {
        schema: SCHEMA_VERSION,
        algorithm_version: ALGORITHM_VERSION,
        playlist_id: playlist_id.to_string(),
        playlist_name: playlist_name.to_string(),
        updated_at: now_ms(),
        revision,
        track_set_hash: track_set_hash(inputs),
        order_hash: order_hash(&order),
        tracks,
        order,
        previous_order,
        transitions,
        stats,
        incomplete: !smart_order,
    };
    // Keep `previousOrder` meaningfully pointing at the pre-analysis order.
    if document.previous_order.is_empty() {
        document.previous_order = inputs.iter().map(|input| input.key.clone()).collect();
    }
    document
}

/// A placeholder entry for a track that could not be analysed, so the playlist
/// keeps its original order and the UI can mark the row as skipped.
fn skipped_track(input: &TrackInput, error: &str) -> TrackAnalysis {
    TrackAnalysis {
        key: input.key.clone(),
        title: input.title.clone(),
        duration: 0.0,
        duration_known: false,
        bpm: 0.0,
        key_camelot: String::new(),
        energy: 0.0,
        head: EdgeFeatures::default(),
        tail: EdgeFeatures::default(),
        tail_offset: 0.0,
        head_secs: 0.0,
        tail_secs: 0.0,
        status: "skipped".into(),
        error: Some(error.to_string()),
    }
}

/// Analyse a playlist, reusing cached per-track features where possible.
///
/// `cached` is a previously stored document for the same playlist; tracks whose
/// key already appears there are not decoded again. The order is recomputed
/// from the feature set, so an order-only change is a pure in-memory operation.
///
/// `partial` is called after every track with the tracks analysed so far, so the
/// UI can show live results and a cancelled run still leaves a resumable file.
#[allow(clippy::too_many_arguments)]
pub fn analyze_playlist(
    app: &AppHandle,
    cache: &Arc<StreamCache>,
    manual: &Arc<ManualCache>,
    playlist_id: &str,
    playlist_name: &str,
    inputs: &[TrackInput],
    previous_order: Vec<String>,
    cached: Option<&AnalysisDocument>,
    fixed_first: Option<&str>,
    revision: u64,
    cancel: &AtomicBool,
    stage: &(dyn Fn(usize, &str, &str, &'static str) + Sync),
    download: &(dyn Fn(usize, &str, &str, u64, Option<u64>) + Sync),
    progress: &dyn Fn(usize, usize),
    partial: &dyn Fn(&AnalysisDocument),
) -> Result<AnalysisDocument, AppError> {
    let mut reusable: HashMap<String, TrackAnalysis> = cached
        .map(|document| {
            document
                .tracks
                .iter()
                .map(|track| (track.key.clone(), track.clone()))
                .collect()
        })
        .unwrap_or_default();

    let total = inputs.len();
    let mut slots: Vec<Option<TrackAnalysis>> = vec![None; total];
    let mut stats = AnalysisStats::default();
    let mut completed = 0usize;
    progress(0, total);

    // Reuse cached features up front: this is pure in-memory work, so it does
    // not need a worker.
    for (index, input) in inputs.iter().enumerate() {
        if cancelled(cancel) {
            return Err(cancel_error());
        }
        if let Some(track) = reusable.remove(&input.key) {
            if track.usable() {
                // Refresh the title in case it was edited.
                let mut track = track;
                track.title = input.title.clone();
                stats.reused += 1;
                stats.analyzed += 1;
                slots[index] = Some(track);
                completed += 1;
                stage(index, &input.key, &input.title, "cached");
                progress(completed, total);
                let snapshot: Vec<TrackAnalysis> = slots.iter().flatten().cloned().collect();
                partial(&assemble_document(
                    playlist_id,
                    playlist_name,
                    revision,
                    inputs,
                    snapshot,
                    previous_order.clone(),
                    stats.clone(),
                    None,
                    false,
                ));
            }
        }
    }

    // The remaining tracks each need a sequential download plus DSP. Run a few
    // at a time so the network waits on several tracks in parallel instead of
    // paying the round-trip latency one track after another. Results are still
    // folded back in input order by this thread, so partial snapshots and the
    // final playlist order stay deterministic.
    let pending: Vec<usize> = (0..total).filter(|index| slots[*index].is_none()).collect();
    if !pending.is_empty() && !cancelled(cancel) {
        let worker_count = pending
            .len()
            .min(
                std::thread::available_parallelism()
                    .map(|value| value.get())
                    .unwrap_or(1)
                    .clamp(2, 4),
            )
            .max(1);
        let (sender, receiver) = std::sync::mpsc::channel::<(usize, Result<TrackAnalysis, AppError>)>();
        let queue = AtomicUsize::new(0);
        let queue = &queue;
        let pending = &pending;
        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let sender = sender.clone();
                scope.spawn(move || loop {
                    let cursor = queue.fetch_add(1, Ordering::Relaxed);
                    if cursor >= pending.len() {
                        break;
                    }
                    let index = pending[cursor];
                    let input = &inputs[index];
                    let result = analyse_track(app, cache, manual, input, cancel, &|value| {
                        stage(index, &input.key, &input.title, value)
                    }, &|done, total| {
                        download(index, &input.key, &input.title, done, total)
                    });
                    if cancelled(cancel) {
                        break;
                    }
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                });
            }
            // Drop this thread's sender so the loop below ends once every
            // worker has finished.
            drop(sender);
            for (index, result) in receiver {
                let input = &inputs[index];
                match result {
                    Ok(track) => {
                        if track.status != "ok" {
                            stats.low_confidence += 1;
                        }
                        stats.analyzed += 1;
                        slots[index] = Some(track);
                        stage(index, &input.key, &input.title, "done");
                    }
                    Err(error) => {
                        if cancelled(cancel) {
                            break;
                        }
                        log::warn!("smart-transition: {} failed: {error}", input.key);
                        stats.skipped += 1;
                        slots[index] = Some(skipped_track(input, &error.to_string()));
                        stage(index, &input.key, &input.title, "skipped");
                    }
                }
                completed += 1;
                progress(completed, total);
                // Snapshot after every track so the UI shows live results and
                // a cancellation keeps the work done so far.
                let snapshot: Vec<TrackAnalysis> = slots.iter().flatten().cloned().collect();
                partial(&assemble_document(
                    playlist_id,
                    playlist_name,
                    revision,
                    inputs,
                    snapshot,
                    previous_order.clone(),
                    stats.clone(),
                    None,
                    false,
                ));
            }
        });
    }
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    stage(total, "", "", "planning");
    progress(total, total);
    let tracks: Vec<TrackAnalysis> = slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            slot.unwrap_or_else(|| skipped_track(&inputs[index], "分析未完成"))
        })
        .collect();
    Ok(assemble_document(
        playlist_id,
        playlist_name,
        revision,
        inputs,
        tracks,
        previous_order,
        stats,
        fixed_first,
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(bpm: f32, key: &str) -> TrackAnalysis {
        TrackAnalysis {
            key: format!("s::{bpm}"),
            title: bpm.to_string(),
            duration: 200.0,
            duration_known: true,
            bpm,
            key_camelot: key.to_string(),
            energy: 0.5,
            head: EdgeFeatures {
                bpm,
                key: key.to_string(),
                energy_curve: vec![0.5],
                ..Default::default()
            },
            tail: EdgeFeatures {
                bpm,
                key: key.to_string(),
                energy_curve: vec![0.5],
                ..Default::default()
            },
            tail_offset: 0.0,
            head_secs: 40.0,
            tail_secs: 60.0,
            status: "ok".into(),
            error: None,
        }
    }

    #[test]
    fn identical_tracks_are_cheap() {
        assert!(transition_cost(&track(120.0, "8A"), &track(120.0, "8A")) < 0.4);
    }

    #[test]
    fn octave_tempos_are_compatible() {
        let near = transition_cost(&track(90.0, "8A"), &track(180.0, "8A"));
        let far = transition_cost(&track(90.0, "8A"), &track(128.0, "8A"));
        assert!(near < far);
    }

    #[test]
    fn held_karp_prefers_ordered_chain() {
        let matrix = vec![
            vec![0.0, 0.1, 0.9],
            vec![0.9, 0.0, 0.1],
            vec![0.1, 0.9, 0.0],
        ];
        assert_eq!(best_order(&matrix, None), vec![0, 1, 2]);
    }

    #[test]
    fn refresh_plans_reuses_features_across_algorithm_versions() {
        let mut out = track(120.0, "8A");
        out.key = "a".into();
        out.head.loudness_curve_db = vec![-12.0];
        out.tail.loudness_curve_db = vec![-12.0];
        let mut input = track(120.0, "8A");
        input.key = "b".into();
        input.head.loudness_curve_db = vec![-12.0];
        input.tail.loudness_curve_db = vec![-12.0];
        let mut document = AnalysisDocument {
            schema: SCHEMA_VERSION,
            algorithm_version: 0,
            playlist_id: "p".into(),
            playlist_name: "P".into(),
            updated_at: 0,
            revision: 7,
            track_set_hash: String::new(),
            order_hash: String::new(),
            order: vec![out.key.clone(), input.key.clone()],
            tracks: vec![out, input],
            previous_order: Vec::new(),
            transitions: Vec::new(),
            stats: AnalysisStats::default(),
            incomplete: false,
        };
        refresh_plans(&mut document);
        assert_eq!(document.algorithm_version, ALGORITHM_VERSION);
        assert_eq!(document.transitions.len(), 1);
        // Refreshing the plans must not touch the stored revision.
        assert_eq!(document.revision, 7);
    }

    #[test]
    fn fixed_first_is_honoured() {
        let matrix = vec![
            vec![0.0, 0.1, 0.9],
            vec![0.9, 0.0, 0.1],
            vec![0.1, 0.9, 0.0],
        ];
        let order = best_order(&matrix, Some(1));
        assert_eq!(order.first(), Some(&1));
        assert_eq!(order.len(), 3);
    }

    fn grid(bpm: f32, anchor: f32) -> BeatGrid {
        BeatGrid::new(bpm, Some(anchor)).unwrap()
    }

    #[test]
    fn mix_in_snaps_to_the_first_phrase_after_15s() {
        // 120 BPM => 0.5 s per beat, a 32-beat phrase is 16 s.
        let g = grid(120.0, 0.0);
        assert!((mix_in(Some(&g), 200.0) - 16.0).abs() < 1e-3);
    }

    #[test]
    fn mix_in_starts_at_zero_when_it_would_skip_most_of_a_short_track() {
        let g = grid(120.0, 0.0);
        // 16 s is 80 % of a 20 s track: outside the 40 % guard.
        assert_eq!(mix_in(Some(&g), 20.0), 0.0);
    }

    #[test]
    fn mix_out_starts_the_fade_on_the_tracks_final_bar() {
        let g = grid(120.0, 0.0);
        // Two seconds at 120 BPM = one bar; the last bar line before 198 is 198.
        assert!((mix_out(Some(&g), 200.0, 120.0) - 198.0).abs() < 1e-3);
    }

    #[test]
    fn blend_is_capped_at_two_and_a_half_seconds() {
        // 60 BPM would prefer four bars, but the hard ceiling wins.
        let slow = grid(60.0, 0.0);
        assert!((blend_secs(Some(&slow), 184.0, 200.0) - 2.5).abs() < 1e-3);
        let medium = grid(120.0, 0.0);
        // One bar at 120 BPM = 2 s, under the 2.5 s ceiling.
        assert!((blend_secs(Some(&medium), 184.0, 200.0) - 2.0).abs() < 1e-3);
        // Less tail: shrink to whole bars instead of stretching the envelope.
        assert!((blend_secs(Some(&medium), 197.0, 200.0) - 2.0).abs() < 1e-3);
        // A grid-less track is capped too.
        assert!(blend_secs(None, 0.0, 100.0) <= MAX_BLEND_SECS);
    }

    #[test]
    fn tempo_ratio_folds_octaves_and_ignores_far_tempos() {
        assert!((fold_tempo_ratio(90.0, 180.0) - 1.0).abs() < 1e-3);
        assert!((fold_tempo_ratio(124.0, 128.0) - 0.96875).abs() < 1e-3);
        assert_eq!(fold_tempo_ratio(100.0, 128.0), 1.0);
        assert_eq!(fold_tempo_ratio(0.0, 128.0), 1.0);
    }

    #[test]
    fn loudness_gain_matches_a_quieter_incoming_track() {
        let gain = loudness_gain(Some(-10.0), Some(-16.0));
        assert!(gain > 1.5 && gain < 2.1);
        // A negligible difference leaves the master untouched.
        assert_eq!(loudness_gain(Some(-10.0), Some(-10.05)), 1.0);
    }

    #[test]
    fn cue_search_prefers_the_end_when_there_is_no_outro() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 140.0;
        out.tail.beats = vec![0.0, 0.5];
        out.tail.energy_curve = vec![0.2; 120];
        out.tail.loudness_curve_db = vec![-12.0; 600];
        out.tail.loudness_curve_resolution_seconds = 0.1;
        let mut input = track(120.0, "8A");
        input.head.beats = vec![0.0, 0.5];
        input.head.energy_curve = vec![0.2; 120];
        input.head.loudness_curve_db = vec![-12.0; 600];
        input.head.loudness_curve_resolution_seconds = 0.1;

        let plan = plan_transition(&out, &input);
        let fallback = mix_out(Some(&grid(120.0, 140.0)), 200.0, 140.0);
        assert_eq!(plan.mode, "strict");
        // The envelope stays anchored to the track's end and is capped at five
        // seconds instead of being stretched to fill the tail.
        assert!(plan.out_at >= fallback - 1e-3);
        assert!(plan.out_at + plan.blend_secs <= 200.0 + 1e-3);
        assert!(plan.out_at + plan.blend_secs >= 198.0);
        assert!(plan.blend_secs <= MAX_BLEND_SECS);
        assert!((plan.in_gain - 1.0).abs() < 1e-3);
        assert!((plan.tempo_ratio - 1.0).abs() < 1e-3);
    }

    #[test]
    fn cue_search_moves_to_the_outro_when_energy_drops() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 140.0;
        out.tail.beats = vec![0.0, 0.5];
        // 60 s tail at 0.5 s resolution: full energy until 180 s, then a quiet
        // 20 s outro. The mix-out should land on the step down, not the file end.
        let mut energy = vec![0.8; 120];
        energy[80..].fill(0.2);
        out.tail.energy_curve = energy;
        out.tail.loudness_curve_db = vec![-12.0; 600];
        out.tail.loudness_curve_resolution_seconds = 0.1;
        let mut input = track(120.0, "8A");
        input.head.beats = vec![0.0, 0.5];
        input.head.energy_curve = vec![0.2; 120];
        input.head.loudness_curve_db = vec![-12.0; 600];
        input.head.loudness_curve_resolution_seconds = 0.1;

        let plan = plan_transition(&out, &input);
        assert_eq!(plan.mode, "strict");
        assert!(
            (178.0..=184.0).contains(&plan.out_at),
            "expected the outro start, got {}",
            plan.out_at
        );
        assert!(plan.out_at + plan.blend_secs <= 200.0 + 1e-3);
        assert!(plan.blend_secs <= MAX_BLEND_SECS);
    }

    #[test]
    fn cue_search_reaches_an_outro_beyond_the_last_thirty_seconds() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 140.0;
        out.tail_secs = 60.0;
        out.tail.beats = vec![0.0, 0.5];
        // The outro starts at 150 s, i.e. 50 s before the end. The old search
        // window began at duration - 30 = 170 s and could never reach it.
        let mut energy = vec![0.8; 120];
        energy[20..].fill(0.2);
        out.tail.energy_curve = energy;
        out.tail.loudness_curve_db = vec![-12.0; 600];
        out.tail.loudness_curve_resolution_seconds = 0.1;
        let mut input = track(120.0, "8A");
        input.head.beats = vec![0.0, 0.5];
        input.head.energy_curve = vec![0.2; 120];
        input.head.loudness_curve_db = vec![-12.0; 600];
        input.head.loudness_curve_resolution_seconds = 0.1;

        let plan = plan_transition(&out, &input);
        assert!(
            (148.0..=156.0).contains(&plan.out_at),
            "expected the early outro start, got {}",
            plan.out_at
        );
        assert!(plan.out_at + plan.blend_secs <= 200.0 + 1e-3);
    }

    #[test]
    fn trailing_silence_is_not_a_mix_target() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 140.0;
        out.tail_secs = 60.0;
        let mut energy = vec![0.8; 120];
        energy[100..].fill(0.0);
        out.tail.energy_curve = energy;
        // Last 10 s are silent, so the musical end is at 190 s.
        assert!((musical_end(&out) - 190.0).abs() < 1.0);
    }

    #[test]
    fn cue_stays_before_a_fading_silent_tail() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 140.0;
        out.tail_secs = 60.0;
        out.tail.beats = vec![0.0, 0.5];
        // Full energy until 170 s, a 25 s fade-out, then 5 s of silence.
        let mut energy = vec![0.8; 120];
        for (i, value) in energy.iter_mut().enumerate().take(110).skip(60) {
            let t = (i as f32 - 60.0) / 50.0;
            *value = 0.8 * (1.0 - t);
        }
        energy[110..].fill(0.0);
        out.tail.energy_curve = energy;
        // The loudness curve agrees that the last 5 s are silent.
        let mut loud = vec![-12.0; 600];
        loud[550..].fill(-120.0);
        out.tail.loudness_curve_db = loud;
        out.tail.loudness_curve_resolution_seconds = 0.1;
        let mut input = track(120.0, "8A");
        input.head.beats = vec![0.0, 0.5];
        input.head.energy_curve = vec![0.2; 120];
        input.head.loudness_curve_db = vec![-12.0; 600];
        input.head.loudness_curve_resolution_seconds = 0.1;

        let plan = plan_transition(&out, &input);
        assert!(
            plan.out_at < 192.0,
            "expected the cue before the silent tail, got {}",
            plan.out_at
        );
        assert!(plan.out_at > 160.0);
    }

    #[test]
    fn edge_windows_scale_with_track_length() {
        assert!((head_window_secs(240.0) - 45.0).abs() < 1e-3);
        assert!((tail_window_secs(240.0) - 72.0).abs() < 1e-3);
        assert!((head_window_secs(60.0) - 20.0).abs() < 1e-3);
        assert!((tail_window_secs(60.0) - 20.0).abs() < 1e-3);
        assert!((tail_window_secs(600.0) - 90.0).abs() < 1e-3);
    }

    #[test]
    fn very_short_tracks_get_a_short_fade() {
        let mut out = track(120.0, "8A");
        out.duration = 12.0;
        out.head_secs = 12.0;
        out.tail_secs = 12.0;
        out.head.loudness_curve_db = vec![-12.0; 120];
        out.head.loudness_curve_resolution_seconds = 0.1;
        out.tail.loudness_curve_db = vec![-12.0; 120];
        out.tail.loudness_curve_resolution_seconds = 0.1;
        let mut input = track(120.0, "8A");
        input.duration = 12.0;
        input.head_secs = 12.0;
        input.tail_secs = 12.0;
        input.head.loudness_curve_db = vec![-12.0; 120];
        input.head.loudness_curve_resolution_seconds = 0.1;
        input.tail.loudness_curve_db = vec![-12.0; 120];
        input.tail.loudness_curve_resolution_seconds = 0.1;

        let plan = plan_transition(&out, &input);
        assert!(plan.blend_secs <= 1.5 + 1e-3);
        assert!(plan.out_at + plan.blend_secs <= 12.0 + 1e-3);
        assert_eq!(plan.in_at, 0.0);
    }

    #[test]
    fn plan_is_relaxed_without_beats_but_with_tempo() {
        let plan = plan_transition(&track(120.0, "8A"), &track(120.0, "8A"));
        assert_eq!(plan.mode, "relaxed");
        assert!(plan.out_at > 0.0);
        assert!(plan.blend_secs > 0.0);
        // 120 BPM used to pick a 32-beat / 16 s phrase just because it fit.
        assert!(plan.blend_secs <= MAX_BLEND_SECS);
    }

    #[test]
    fn plan_is_strict_when_both_windows_have_beats() {
        let mut out = track(120.0, "8A");
        out.tail.beats = vec![0.5, 1.0, 1.5];
        out.tail_offset = 120.0;
        let mut input = track(120.0, "8A");
        input.head.beats = vec![0.1, 0.6, 1.1];
        let plan = plan_transition(&out, &input);
        assert_eq!(plan.mode, "strict");
        assert!(plan.in_at >= 0.0);
        assert!(plan.out_at > 0.0);
    }

    #[test]
    fn unknown_duration_is_usable_but_never_planned() {
        let mut unknown = track(120.0, "8A");
        unknown.duration = 0.0;
        unknown.duration_known = false;
        // `usable` requires non-empty loudness curves.
        unknown.head.loudness_curve_db = vec![-20.0];
        unknown.tail.loudness_curve_db = vec![-20.0];
        // It may still take part in ordering…
        assert!(unknown.usable());
        // …but placing an absolute cue would be a guess, so no plan.
        assert!(!unknown.plan_ready());
    }

    #[test]
    fn downbeat_offset_finds_the_strongest_bar_phase() {
        // 120 BPM => one beat every 0.5 s (50 frames at 100 fps).
        let beats: Vec<f32> = (0..16).map(|beat| beat as f32 * 0.5).collect();
        let mut onset = vec![0.1f32; 1000];
        for beat in (2..16).step_by(4) {
            let frame = (beat as f32 * 0.5 * DOWNBEAT_FPS) as usize;
            onset[frame] = 1.0;
        }
        assert_eq!(downbeat_offset(&onset, &beats), 2);
        // Fewer than two bars is not enough evidence: keep the old anchor.
        assert_eq!(downbeat_offset(&onset, &beats[..4]), 0);
    }

    #[test]
    fn low_band_onset_prefers_kicks_over_quiet_beats() {
        let rate = 44_100i32;
        let mut samples = vec![0.0f32; rate as usize * 8];
        let beats: Vec<f32> = (0..16).map(|beat| beat as f32 * 0.5).collect();
        // A short 60 Hz burst on the third beat of every bar (phase 2), which
        // is exactly what a kick on the downbeat looks like to a low-pass.
        for beat in (2..16).step_by(4) {
            let start = (beats[beat] * rate as f32) as usize;
            let len = ((rate as f32 * 0.08) as usize).min(samples.len() - start);
            for offset in 0..len {
                let t = offset as f32 / rate as f32;
                samples[start + offset] = 0.8 * (std::f32::consts::TAU * 60.0 * t).sin();
            }
        }
        let onset = low_band_onset(&samples, rate, 1);
        assert_eq!(downbeat_offset(&onset, &beats), 2);
    }

    #[test]
    fn beat_grid_anchors_on_the_downbeat_not_the_first_beat() {
        let mut out = track(120.0, "8A");
        out.tail_offset = 100.0;
        out.tail.beats = vec![10.0, 10.5, 11.0, 11.5, 12.0];
        out.tail.downbeat_offset = 2;
        let out_grid = tail_grid(&out).unwrap();
        assert!((out_grid.anchor - 111.0).abs() < 1e-3);

        let mut input = track(120.0, "8A");
        input.head.beats = vec![2.0, 2.5, 3.0, 3.5];
        input.head.downbeat_offset = 3;
        let in_grid = head_grid(&input).unwrap();
        assert!((in_grid.anchor - 3.5).abs() < 1e-3);
    }
}
