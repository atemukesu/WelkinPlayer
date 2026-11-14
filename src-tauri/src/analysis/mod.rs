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
//! the middle of the file is never fetched or analysed. That is enough for both
//! the outgoing (tail) and incoming (head) halves of every transition.
//!
//! The resulting document is small, plain JSON and is stored per playlist, both
//! in the local cache and (when a sync source exists) on WebDAV.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use automix::{Analyzer, AudioBuffer, TrackFeatures};
use rodio::{Decoder, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::core::error::AppError;
use crate::playback::source::{open_track_for_analysis, ReadSeek};
use crate::streaming::manual_cache::ManualCache;
use crate::streaming::stream_cache::StreamCache;

/// Seconds decoded from the start *and* the end of each track.
pub const WINDOW_SECS: f32 = 60.0;
/// Bump when the analysis pipeline or cost model changes.
pub const ALGORITHM_VERSION: u32 = 3;
/// On-disk document schema.
pub const SCHEMA_VERSION: u32 = 1;
/// Prefix that scopes transition documents away from audio and other objects.
pub const FILE_PREFIX: &str = "welkin-transition-";

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
}

impl EdgeFeatures {
    fn from_features(features: &TrackFeatures) -> Self {
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
        }
    }

    fn empty(&self) -> bool {
        self.loudness_curve_db.is_empty()
    }

    /// Mean RMS energy across the window (used for continuity costs).
    fn mean_energy(&self) -> f32 {
        if self.energy_curve.is_empty() {
            return 0.0;
        }
        self.energy_curve.iter().copied().sum::<f32>() / self.energy_curve.len() as f32
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
    /// `ok` or `skipped`; skipped tracks never take part in ordering.
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl TrackAnalysis {
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
) -> Result<TrackAnalysis, AppError> {
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let Some((source_id, path)) = split_key(&input.key) else {
        return Err(AppError::invalid_argument("key", "不是有效的曲目键"));
    };
    let started = std::time::Instant::now();
    let (reader, byte_len) = open_track_for_analysis(
        app,
        cache,
        manual,
        source_id,
        path,
        // Head window plus a little slack for the demuxer's probe reads. The
        // tail seek rebases the downloader onto the final minute after this.
        WINDOW_SECS as f64 + 10.0,
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
    let window_frames = (WINDOW_SECS * rate as f32) as usize;

    // Head: decode the first window.
    let head_samples = read_frames(&mut decoder, window_frames, channels);
    if head_samples.is_empty() {
        return Err(AppError::other("无法解码音频"));
    }
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let head_elapsed = started.elapsed();

    // Tail: seek to duration - window and decode the rest. When the track is
    // shorter than the window the head already covers it.
    let tail_samples = if duration > WINDOW_SECS + 0.5 {
        let target = duration - WINDOW_SECS;
        match decoder.try_seek(std::time::Duration::from_secs_f32(target)) {
            Ok(()) => read_frames(&mut decoder, window_frames, channels),
            Err(error) => {
                log::warn!(
                    "smart-transition: tail seek failed for {}: {error}",
                    input.key
                );
                Vec::new()
            }
        }
    } else {
        head_samples.clone()
    };
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let decode_elapsed = started.elapsed();
    // A failed seek still leaves a usable (if less precise) tail. Record where
    // the tail window really starts so its beat times and energy curve can be
    // mapped back onto the absolute timeline.
    let tail_offset = if tail_samples.is_empty() {
        0.0
    } else if duration > WINDOW_SECS + 0.5 {
        (duration - WINDOW_SECS).max(0.0)
    } else {
        0.0
    };
    let tail_samples = if tail_samples.is_empty() {
        head_samples.clone()
    } else {
        tail_samples
    };

    let mut analyzer = Analyzer::new();
    let head_buffer = AudioBuffer {
        samples: head_samples,
        sample_rate: rate as i32,
        channels: channels as i32,
    };
    let tail_buffer = AudioBuffer {
        samples: tail_samples,
        sample_rate: rate as i32,
        channels: channels as i32,
    };
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let head_features = analyzer
        .analyze(&head_buffer)
        .map_err(|error| AppError::other(error.to_string()))?;
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    let tail_features = analyzer
        .analyze(&tail_buffer)
        .map_err(|error| AppError::other(error.to_string()))?;

    log::info!(
        "smart-transition: analysed {} in {}ms (fetch {}ms, head {}ms, decode {}ms, dsp {}ms)",
        input.key,
        started.elapsed().as_millis(),
        fetch_elapsed.as_millis(),
        head_elapsed.as_millis() - fetch_elapsed.as_millis(),
        decode_elapsed.as_millis() - head_elapsed.as_millis(),
        started.elapsed().as_millis() - decode_elapsed.as_millis(),
    );

    let head = EdgeFeatures::from_features(&head_features);
    let tail = EdgeFeatures::from_features(&tail_features);
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

/// Exact solver for small playlists (Held-Karp), greedy + 2-opt above 19 tracks.
fn best_order(matrix: &[Vec<f64>], fixed_first: Option<usize>) -> Vec<usize> {
    let n = matrix.len();
    if n <= 1 {
        return (0..n).collect();
    }
    if n <= 19 {
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

/// A beat grid anchored on the first detected beat.
///
/// Only the head/tail windows are decoded, so the anchor is the first beat of
/// that window (mapped to absolute time via [`TrackAnalysis::tail_offset`] for
/// the tail). Phrase lines land every `PHRASE_BEATS` beats from the anchor.
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
    fn new(bpm: f32, first_beat: Option<f32>) -> Option<Self> {
        if !(bpm.is_finite() && bpm > 1.0) {
            return None;
        }
        let period = 60.0 / bpm;
        if !(period.is_finite() && (0.02..=3.0).contains(&period)) {
            return None;
        }
        let anchor = first_beat
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
        // absolute timeline before using them as the anchor.
        track
            .tail
            .beats
            .first()
            .map(|beat| beat + track.tail_offset),
    )
}

fn head_grid(track: &TrackAnalysis) -> Option<BeatGrid> {
    BeatGrid::new(
        grid_bpm(track.head.bpm, track.bpm),
        track.head.beats.first().copied(),
    )
}

/// Earliest musical entry point for the incoming track.
///
/// ClubCraft's fallback: the first 32-beat phrase line at least 15 s in,
/// unless that would skip more than 40 % of a short track — then start at 0.
fn mix_in(grid: Option<&BeatGrid>, duration: f32) -> f32 {
    let Some(grid) = grid else {
        return 0.0;
    };
    let line = grid.phrase_at_or_after(15.0, PHRASE_BEATS);
    if duration > 1.0 && line > duration * 0.4 {
        0.0
    } else {
        line.clamp(0.0, duration.max(0.0))
    }
}

/// Latest musical exit point for the outgoing track.
///
/// ClubCraft's fallback: the last 32-beat phrase line at least 30 s before the
/// end, staying inside the analysed tail window. When no full phrase fits,
/// drop to 16 beats before the end.
fn mix_out(grid: Option<&BeatGrid>, duration: f32, tail_offset: f32) -> f32 {
    let Some(grid) = grid else {
        // No usable tempo: leave a safe two-bar-ish tail.
        return (duration - 8.0).max(0.0);
    };
    if duration <= 1.0 {
        return 0.0;
    }
    let line = grid.phrase_at_or_before(duration - 30.0, PHRASE_BEATS);
    if line >= tail_offset && line > 1.0 {
        return line.min(duration);
    }
    // No full phrase inside the analysed tail: 16 beats before the end,
    // clamped into the window we actually decoded.
    let fallback = grid.time_at((grid.beats_at(duration).floor() - 16.0).max(0.0));
    fallback.clamp(tail_offset.max(0.0).min(duration), duration)
}

/// Pick the largest whole phrase (32, 16, 8 or 4 beats) that still fits before
/// the outgoing track ends. Returns the fade length in seconds.
fn blend_secs(grid: Option<&BeatGrid>, out_at: f32, duration: f32) -> f32 {
    let remaining = (duration - out_at).max(0.0);
    let Some(grid) = grid else {
        return remaining.clamp(1.5, 8.0);
    };
    for beats in [PHRASE_BEATS, PHRASE_BEATS / 2.0, PHRASE_BEATS / 4.0, PHRASE_BEATS / 8.0] {
        let secs = beats * grid.period;
        if secs <= remaining + 1e-3 {
            return secs;
        }
    }
    remaining
}

fn plan_transition(out: &TrackAnalysis, input: &TrackAnalysis) -> TransitionPlan {
    let out_grid = tail_grid(out);
    let in_grid = head_grid(input);

    let out_at = mix_out(out_grid.as_ref(), out.duration, out.tail_offset);
    let in_at = mix_in(in_grid.as_ref(), input.duration);
    let blend = blend_secs(out_grid.as_ref(), out_at, out.duration);

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

    // Energy continuity + timbre similarity, plus a small penalty when the
    // grid had to be approximated. Lower is a better mix point.
    let energy_gap = (out.tail.mean_energy() - input.head.mean_energy())
        .abs()
        .min(1.0);
    let timbre_gap = (1.0 - cosine(&out.tail.chroma, &input.head.chroma)).clamp(0.0, 1.0);
    let mode_penalty = match mode {
        "strict" => 0.0,
        "relaxed" => 0.12,
        _ => 0.25,
    };
    let score = (energy_gap * 0.55 + timbre_gap * 0.30 + mode_penalty).min(1.0);

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
        score: round(score),
    }
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

    // Generate a transition for each adjacent usable pair.
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
    let mut tracks = Vec::with_capacity(total);
    let mut stats = AnalysisStats::default();
    progress(0, total);
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
                tracks.push(track);
                progress(index + 1, total);
                partial(&assemble_document(
                    playlist_id,
                    playlist_name,
                    revision,
                    inputs,
                    tracks.clone(),
                    previous_order.clone(),
                    stats.clone(),
                    None,
                    false,
                ));
                continue;
            }
        }
        match analyse_track(app, cache, manual, input, cancel) {
            Ok(track) => {
                if track.status != "ok" {
                    stats.low_confidence += 1;
                }
                stats.analyzed += 1;
                tracks.push(track);
            }
            Err(error) => {
                if cancelled(cancel) {
                    return Err(cancel_error());
                }
                log::warn!("smart-transition: {} failed: {error}", input.key);
                stats.skipped += 1;
                tracks.push(TrackAnalysis {
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
                    status: "skipped".into(),
                    error: Some(error.to_string()),
                });
            }
        }
        // Snapshot after every track so the UI shows live results and a
        // cancellation keeps the work done so far.
        progress(index + 1, total);
        partial(&assemble_document(
            playlist_id,
            playlist_name,
            revision,
            inputs,
            tracks.clone(),
            previous_order.clone(),
            stats.clone(),
            None,
            false,
        ));
    }
    if cancelled(cancel) {
        return Err(cancel_error());
    }
    progress(total, total);
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
    fn mix_out_uses_the_last_phrase_at_least_30s_before_the_end() {
        let g = grid(120.0, 0.0);
        // Phrase lines at 0, 16, ... 160; 160 is the last one <= 170.
        assert!((mix_out(Some(&g), 200.0, 120.0) - 160.0).abs() < 1e-3);
    }

    #[test]
    fn blend_uses_whole_phrases_that_fit() {
        let g = grid(120.0, 0.0);
        assert!((blend_secs(Some(&g), 184.0, 200.0) - 16.0).abs() < 1e-3);
        // Only 10 s remain: a 16-beat (8 s) phrase fits, 32 beats do not.
        assert!((blend_secs(Some(&g), 190.0, 200.0) - 8.0).abs() < 1e-3);
    }

    #[test]
    fn plan_is_relaxed_without_beats_but_with_tempo() {
        let plan = plan_transition(&track(120.0, "8A"), &track(120.0, "8A"));
        assert_eq!(plan.mode, "relaxed");
        assert!(plan.out_at > 0.0);
        assert!(plan.blend_secs > 0.0);
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
}
