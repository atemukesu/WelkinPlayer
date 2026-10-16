// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! The playback engine: a decode worker thread feeding a ring buffer that
//! `rodio`'s output callback drains.
//!
//! * A `Shared` snapshot (atomics + a couple of mutexes) is the only state read
//!   by the transport commands and the event emitter; it never touches the
//!   audio thread.
//! * The worker owns the `OutputStream` (platform streams are not `Send`, so it
//!   must stay on the thread that built it), the current `Decoder` and the ring
//!   producer. Commands arrive over an `mpsc` channel.
//! * `RingSource` is a non-blocking `Source` handed to the mixer. It yields
//!   silence on underrun and applies volume/mute/pause, so a slow network read
//!   can never stall the realtime callback.

use std::collections::VecDeque;
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Source};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::core::error::AppError;
use crate::playback::source::{open_track, ReadSeek};
use crate::streaming::smart_cache::SmartCache;
use crate::streaming::stream_cache::StreamCache;

/// How often the playhead snapshot is pushed to the frontend (milliseconds).
const STATE_INTERVAL_MS: u64 = 250;
/// Decoded samples produced per decode chunk before offering them to the ring.
const DECODE_CHUNK: usize = 8192;
/// Ring buffer length in milliseconds of output (already time-stretched).
/// Keeping it short makes a tempo change audible promptly while still
/// covering normal scheduling jitter.
const BUFFER_MILLIS: usize = 400;
/// Pre-stretch this much output before starting a new source after load/seek.
const PREFILL_MILLIS: usize = 150;
/// Forward-decoding fallback is intentionally bounded. A remote stream that
/// cannot seek must not freeze the transport thread decoding minutes of audio.
const MAX_FORWARD_SEEK_SECONDS: f64 = 8.0;
const SOURCE_STATE_BLOCK: usize = 256;

fn output_samples(rate: u32, channels: u16, millis: usize) -> usize {
    (rate as usize * millis / 1000).max(1) * channels.max(1) as usize
}
/// Interleaved output samples produced per stretcher refill on the worker.
const STRETCH_REFILL: usize = 8192;
/// Number of output frames represented by one realtime waveform sample.
const WAVEFORM_BUCKET_FRAMES: usize = 1024;
const WAVEFORM_SLOTS: usize = 96;
const SPECTRUM_SIZE: usize = 2048;

/// A transport error surfaced to the frontend on `audio:error`.
#[derive(Clone, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumPayload {
    pub sample_rate: u32,
    pub magnitudes: Vec<f32>,
}

/// State snapshot pushed on `audio:state` (and returned by `audio_state`).
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: f64,
    pub ended: bool,
}

/// Lock-light state shared between the worker and the command/event threads.
pub struct Shared {
    /// Bumped on every load/seek/stop; a stale `RingSource` ends itself.
    generation: AtomicU64,
    /// Generation-local counters actually updated by the output callback.
    progress: Mutex<Arc<Progress>>,
    /// Times the output callback ran dry while a track was loading/playing.
    underruns: AtomicU64,
    channels: AtomicU32,
    sample_rate: AtomicU32,
    duration_ms: AtomicU64,
    transport: AtomicU8,
    worker_started: AtomicBool,
    muted: AtomicBool,
    /// Volume as `f32` bits, `0.0..=1.0`.
    volume: AtomicU32,
    /// Playback-rate multiplier as `f32` bits (`1.0` = unchanged).
    tempo: AtomicU32,
    failure: Mutex<Option<ErrorInfo>>,
    /// `(source_id, path)` of the loaded track, for cache progress reporting.
    location: Mutex<Option<(String, String)>>,
    /// Playback-rate segments, mapping played output samples to source seconds.
    /// A new entry is recorded whenever the tempo changes so the playhead stays
    /// continuous even though the stretcher only ever sees constant-rate runs.
    segments: Mutex<VecDeque<Segment>>,
    /// Peak amplitudes written by the realtime output callback. Each slot is an
    /// atomic bit pattern so the callback never takes a lock or allocates.
    waveform: [AtomicU32; WAVEFORM_SLOTS],
    waveform_sequence: AtomicU64,
    spectrum: [AtomicU32; SPECTRUM_SIZE],
    spectrum_sequence: AtomicU64,
}

/// One constant-tempo run of playback. `out` is an interleaved output-sample
/// index; `pos_secs` is the source position at that index.
#[derive(Clone, Copy)]
struct Segment {
    out: u64,
    pos_secs: f64,
    tempo: f32,
}

/// Each source owns its counters. An in-flight old callback cannot update the
/// playhead or ended flag belonging to a newly installed source.
#[derive(Default)]
struct Progress {
    frames: AtomicU64,
    ended: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Transport {
    Idle,
    Loading,
    Paused,
    Playing,
    Failed,
}

impl Shared {
    fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            progress: Mutex::new(Arc::new(Progress::default())),
            underruns: AtomicU64::new(0),
            channels: AtomicU32::new(2),
            sample_rate: AtomicU32::new(44_100),
            duration_ms: AtomicU64::new(0),
            transport: AtomicU8::new(Transport::Idle as u8),
            worker_started: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            tempo: AtomicU32::new(1.0f32.to_bits()),
            failure: Mutex::new(None),
            location: Mutex::new(None),
            segments: Mutex::new(VecDeque::new()),
            waveform: std::array::from_fn(|_| AtomicU32::new(0)),
            waveform_sequence: AtomicU64::new(0),
            spectrum: std::array::from_fn(|_| AtomicU32::new(0)),
            spectrum_sequence: AtomicU64::new(0),
        }
    }

    /// Requested playback state: a track is loaded and not paused.
    pub fn is_paused(&self) -> bool {
        self.transport.load(Ordering::Relaxed) != Transport::Playing as u8
    }

    pub fn set_paused(&self, paused: bool) {
        // Play/Pause cannot turn an empty or failed engine into a loaded one.
        let _ = self
            .transport
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |state| {
                if state == Transport::Playing as u8 || state == Transport::Paused as u8 {
                    Some(if paused {
                        Transport::Paused as u8
                    } else {
                        Transport::Playing as u8
                    })
                } else {
                    None
                }
            });
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Set the output volume from a percentage (0-100).
    pub fn set_volume_percent(&self, percent: f64) {
        let value = if percent.is_finite() {
            (percent / 100.0).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        self.volume.store(value.to_bits(), Ordering::Relaxed);
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// Set the playback-rate multiplier, clamped to the range the WSOLA
    /// stretcher supports. A non-finite or non-positive value resets to 1.0.
    pub fn set_tempo(&self, rate: f32) {
        let value = if rate.is_finite() && rate > 0.0 {
            rate.clamp(wsola::MIN_TEMPO, wsola::MAX_TEMPO)
        } else {
            1.0
        };
        self.tempo.store(value.to_bits(), Ordering::Relaxed);
    }

    fn tempo(&self) -> f32 {
        f32::from_bits(self.tempo.load(Ordering::Relaxed))
    }

    fn progress(&self) -> Arc<Progress> {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn played(&self) -> u64 {
        self.progress().frames.load(Ordering::Relaxed)
    }

    fn has_ended(&self) -> bool {
        self.progress().ended.load(Ordering::Relaxed)
    }

    fn set_ended(&self, ended: bool) {
        self.progress().ended.store(ended, Ordering::Relaxed);
    }

    /// Position snapshot in milliseconds, derived from the played output.
    fn position_ms(&self) -> f64 {
        self.position_secs() * 1000.0
    }

    /// Source position (seconds) at the played output sample, mapped through the
    /// tempo segments so a rate change never skews the reported position.
    fn position_secs(&self) -> f64 {
        let frames_per_sec = self.frames_per_sec();
        let mut segments = self
            .segments
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let played = self.played();
        // Each expired segment is removed once; no full scan on every tick.
        while segments.get(1).is_some_and(|segment| segment.out <= played) {
            segments.pop_front();
        }
        let Some(segment) = segments.front().copied() else {
            return 0.0;
        };
        segment.pos_secs
            + played.saturating_sub(segment.out) as f64 / frames_per_sec * segment.tempo as f64
    }

    /// Source position (seconds) at an arbitrary produced output index, used by
    /// the worker to anchor a new tempo segment.
    fn position_at(&self, out: u64) -> f64 {
        let frames_per_sec = self.frames_per_sec();
        let segments = self
            .segments
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let index = segments
            .partition_point(|segment| segment.out <= out)
            .saturating_sub(1);
        let segment = segments.get(index);
        match segment {
            Some(segment) => {
                segment.pos_secs
                    + out.saturating_sub(segment.out) as f64 / frames_per_sec * segment.tempo as f64
            }
            None => 0.0,
        }
    }

    /// Reset the playhead mapping to a fresh constant-tempo segment.
    fn reset_position(&self, base_secs: f64, tempo: f32) {
        let mut segments = self
            .segments
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(Progress::default());
        segments.clear();
        segments.push_back(Segment {
            out: 0,
            pos_secs: base_secs,
            tempo,
        });
        self.waveform_sequence.store(0, Ordering::Release);
        for sample in &self.waveform {
            sample.store(0, Ordering::Relaxed);
        }
        self.spectrum_sequence.store(0, Ordering::Release);
        for sample in &self.spectrum {
            sample.store(0, Ordering::Relaxed);
        }
    }

    fn waveform_samples(&self) -> (u64, Vec<f32>) {
        let sequence = self.waveform_sequence.load(Ordering::Acquire);
        let count = sequence.min(WAVEFORM_SLOTS as u64) as usize;
        let start = sequence.saturating_sub(count as u64);
        let samples = (0..count)
            .map(|offset| {
                f32::from_bits(
                    self.waveform[(start as usize + offset) % WAVEFORM_SLOTS]
                        .load(Ordering::Relaxed),
                )
            })
            .collect();
        (sequence, samples)
    }

    /// Publish one completed waveform bucket from the output callback.
    fn push_waveform_sample(&self, peak: f32) {
        let sequence = self.waveform_sequence.load(Ordering::Relaxed);
        self.waveform[(sequence as usize) % WAVEFORM_SLOTS]
            .store(peak.to_bits(), Ordering::Relaxed);
        self.waveform_sequence
            .store(sequence + 1, Ordering::Release);
    }

    fn push_spectrum_sample(&self, sample: f32) {
        let sequence = self.spectrum_sequence.load(Ordering::Relaxed);
        self.spectrum[(sequence as usize) % SPECTRUM_SIZE]
            .store(sample.to_bits(), Ordering::Relaxed);
        self.spectrum_sequence
            .store(sequence + 1, Ordering::Release);
    }

    fn spectrum_samples(&self) -> Vec<f32> {
        let sequence = self.spectrum_sequence.load(Ordering::Acquire);
        let count = sequence.min(SPECTRUM_SIZE as u64) as usize;
        let start = sequence.saturating_sub(count as u64);
        (0..count)
            .map(|offset| {
                f32::from_bits(
                    self.spectrum[(start as usize + offset) % SPECTRUM_SIZE]
                        .load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// Anchor a new constant-tempo segment at the given produced output index.
    fn push_segment(&self, out: u64, pos_secs: f64, tempo: f32) {
        let mut segments = self
            .segments
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if segments.back().is_some_and(|segment| segment.out == out) {
            segments.pop_back();
        }
        segments.push_back(Segment {
            out,
            pos_secs,
            tempo,
        });
    }

    /// Interleaved samples per second (channels × sample rate).
    fn frames_per_sec(&self) -> f64 {
        let channels = self.channels.load(Ordering::Relaxed).max(1) as f64;
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1) as f64;
        rate * channels
    }

    /// Full snapshot for the initial `audio_state` pull.
    pub fn snapshot(&self) -> AudioState {
        let playing = !self.is_paused();
        let ended = self.has_ended();
        AudioState {
            playing: playing && !ended,
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms.load(Ordering::Relaxed) as f64,
            ended,
        }
    }
}

/// Fixed-capacity SPSC ring shared between the decode worker and the mixer.
///
/// Only the worker owns this producer; only RingSource owns its consumer.
/// rtrb owns the shared allocation and handles publication, wraparound and
/// destruction after both endpoints are dropped. No application unsafe code is
/// needed. Load/seek create a new pair; changing tempo preserves the pair.
struct Ring {
    producer: Producer<f32>,
    capacity: usize,
    done: Arc<AtomicBool>,
}

impl Ring {
    fn new(capacity: usize) -> (Self, Consumer<f32>) {
        let capacity = capacity.max(1);
        let (producer, consumer) = RingBuffer::new(capacity);
        (
            Self {
                producer,
                capacity,
                done: Arc::new(AtomicBool::new(true)),
            },
            consumer,
        )
    }

    fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    fn len(&self) -> usize {
        self.capacity.saturating_sub(self.producer.slots())
    }

    /// Room left for new samples.
    fn space(&self) -> usize {
        self.producer.slots()
    }

    /// Push as many samples as fit; returns how many were accepted.
    fn push_slice(&mut self, values: &[f32]) -> usize {
        let mut count = 0;
        for &value in values {
            if self.producer.push(value).is_err() {
                break;
            }
            count += 1;
        }
        count
    }
}

/// Non-blocking `Source` for the mixer. Declares the file's native rate and
/// channel count; `rodio` resamples/remaps it to the device configuration.
///
/// The ring holds already time-stretched output produced on the worker, so the
/// output callback only pops samples (and applies volume/mute). Playback-rate
/// changes are applied upstream, keeping the realtime path allocation- and
/// DSP-free.
struct RingSource {
    consumer: Consumer<f32>,
    done: Arc<AtomicBool>,
    shared: Arc<Shared>,
    progress: Arc<Progress>,
    generation: u64,
    channels: u16,
    rate: u32,
    state_left: usize,
    paused: bool,
    gain: f32,
    target_gain: f32,
    gain_step: f32,
    ramp_frames: u32,
    channel_cursor: u16,
    waveform_peak: f32,
    waveform_frames: usize,
    spectrum_sum: f32,
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.shared.generation.load(Ordering::Relaxed) != self.generation {
            return None;
        }
        if self.state_left == 0 {
            self.paused = self.shared.is_paused();
            let target = if self.shared.muted.load(Ordering::Relaxed) {
                0.0
            } else {
                self.shared.volume()
            };
            if target != self.target_gain {
                self.target_gain = target;
                self.ramp_frames = (self.rate / 100).max(1);
                self.gain_step = (target - self.gain) / self.ramp_frames as f32;
            }
            self.state_left =
                SOURCE_STATE_BLOCK.div_ceil(self.channels as usize) * self.channels as usize;
        }
        self.state_left -= 1;
        // Paused: emit silence without consuming, so the playhead freezes.
        if self.paused {
            return Some(0.0);
        }
        // After observing EOF, recheck the queue: an earlier empty observation
        // may have preceded the producer's final push. Release/acquire on done
        // publishes all final samples before termination is considered.
        let mut popped = self.consumer.pop().ok();
        let done = popped.is_none() && self.done.load(Ordering::Acquire);
        if done {
            popped = self.consumer.pop().ok();
        }
        match popped {
            Some(value) => {
                self.progress.frames.fetch_add(1, Ordering::Relaxed);
                self.waveform_peak = self.waveform_peak.max(value.abs());
                self.spectrum_sum += value;
                if self.channel_cursor + 1 >= self.channels {
                    self.shared
                        .push_spectrum_sample(self.spectrum_sum / self.channels as f32);
                    self.spectrum_sum = 0.0;
                    self.waveform_frames += 1;
                    if self.waveform_frames >= WAVEFORM_BUCKET_FRAMES {
                        self.shared
                            .push_waveform_sample(self.waveform_peak.clamp(0.0, 1.0));
                        self.waveform_peak = 0.0;
                        self.waveform_frames = 0;
                    }
                }
                if self.channel_cursor == 0 && self.ramp_frames > 0 {
                    self.gain += self.gain_step;
                    self.ramp_frames -= 1;
                    if self.ramp_frames == 0 {
                        self.gain = self.target_gain;
                    }
                }
                self.channel_cursor = (self.channel_cursor + 1) % self.channels;
                Some(value * self.gain)
            }
            None => {
                if done {
                    self.progress.ended.store(true, Ordering::Relaxed);
                    None
                } else {
                    // Underrun: the decode worker has not caught up yet.
                    self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                    Some(0.0)
                }
            }
        }
    }
}

impl Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Transport commands sent to the worker. Load/Play/Pause/Seek/Stop all go
/// through this channel so their order is preserved (a play issued right after
/// a load must not be dropped by the load's own state reset).
pub enum Command {
    Load {
        source_id: String,
        path: String,
        autoplay: bool,
    },
    SetTempo {
        rate: f32,
    },
    Play,
    Pause,
    Seek {
        position_secs: f64,
    },
    Stop,
}

/// Handle held in Tauri managed state.
pub struct Engine {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    shutdown: Arc<AtomicBool>,
}

impl Engine {
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    /// Shared snapshot, for the event thread and the `audio_state` command.
    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    /// Queue a transport command for the worker.
    pub fn send(&self, command: Command) -> Result<(), AppError> {
        log::debug!("playback: transport command queued");
        self.tx
            .send(command)
            .map_err(|_| AppError::other("播放引擎已停止"))
    }
}

/// Start the engine (worker + event threads) and return its handle.
pub fn start(app: AppHandle, cache: Arc<StreamCache>, smart: Arc<SmartCache>) -> Engine {
    log::info!("playback: starting worker and event threads");
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared::new());
    let shutdown = Arc::new(AtomicBool::new(false));

    let worker_shared = shared.clone();
    let worker_app = app.clone();
    let worker_shutdown = shutdown.clone();
    std::thread::Builder::new()
        .name("welkin-playback-worker".to_string())
        .spawn(move || worker::run(worker_app, cache, smart, worker_shared, worker_shutdown, rx))
        .unwrap_or_else(|error| panic!("failed to start playback worker: {error}"));

    let events_shared = shared.clone();
    let events_shutdown = shutdown.clone();
    std::thread::Builder::new()
        .name("welkin-playback-events".to_string())
        .spawn(move || events::run(app, events_shared, events_shutdown))
        .unwrap_or_else(|error| panic!("failed to start playback events: {error}"));

    Engine {
        tx,
        shared,
        shutdown,
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

mod events {
    use super::*;

    /// Push periodic state snapshots and one-shot ended/error events.
    pub fn run(app: AppHandle, shared: Arc<Shared>, shutdown: Arc<AtomicBool>) {
        let mut last_ended_generation = u64::MAX;
        let mut last_underruns = 0u64;
        let mut state_ticks = 0u64;
        let mut last_waveform_sequence = 0u64;
        let mut planner = rustfft::FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(SPECTRUM_SIZE);
        while !shutdown.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(50));
            if shutdown.load(Ordering::Acquire) {
                break;
            }

            let state = shared.snapshot();
            // State remains intentionally low frequency; waveform samples use
            // the faster loop below without making transport events noisy.
            state_ticks += 1;
            let emit_state = state_ticks >= (STATE_INTERVAL_MS / 50);
            if emit_state {
                state_ticks = 0;
                let _ = app.emit("audio:state", state);
                log::debug!(
                    "playback: state playing={} pos={:.3}s dur={:.3}s ended={} played={}",
                    state.playing,
                    state.position_ms / 1000.0,
                    state.duration_ms / 1000.0,
                    state.ended,
                    shared.played(),
                );
            }
            let (waveform_sequence, waveform) = shared.waveform_samples();
            let should_emit_waveform = waveform_sequence != last_waveform_sequence;
            if should_emit_waveform {
                last_waveform_sequence = waveform_sequence;
                let _ = app.emit("audio:waveform", waveform);
            }
            let samples = shared.spectrum_samples();
            if samples.len() == SPECTRUM_SIZE {
                let mut buffer: Vec<rustfft::num_complex::Complex<f32>> = samples
                    .into_iter()
                    .map(|sample| rustfft::num_complex::Complex::new(sample, 0.0))
                    .collect();
                let mean =
                    buffer.iter().map(|sample| sample.re).sum::<f32>() / SPECTRUM_SIZE as f32;
                for sample in &mut buffer {
                    sample.re -= mean;
                }
                fft.process(&mut buffer);
                let magnitudes = buffer[..SPECTRUM_SIZE / 2]
                    .iter()
                    .map(|sample| (sample.norm() / SPECTRUM_SIZE as f32 * 2.0).min(1.0))
                    .collect();
                let _ = app.emit(
                    "audio:spectrum",
                    SpectrumPayload {
                        sample_rate: shared.sample_rate.load(Ordering::Relaxed),
                        magnitudes,
                    },
                );
            }
            // Surface buffer underruns: silence injected before the decoder
            // caught up (debug level; hidden at the default `Info` cap).
            let underruns = shared.underruns.load(Ordering::Relaxed);
            if underruns > last_underruns {
                log::debug!(
                    "playback: {} underrun sample(s) since last tick (playing={}, ended={})",
                    underruns - last_underruns,
                    state.playing,
                    shared.has_ended(),
                );
                last_underruns = underruns;
            }

            if let Some(failure) = shared
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = app.emit("audio:error", failure);
            }

            let generation = shared.generation.load(Ordering::Relaxed);
            if state.ended && generation != last_ended_generation {
                last_ended_generation = generation;
                let _ = app.emit("audio:ended", ());
            }
        }
    }
}

mod worker {
    use super::*;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn slow_tempo_does_not_decode_ahead_to_eof_or_build_a_huge_flush_tail() {
            let rate = 1000u32;
            let channels = 2u16;
            let samples = rate * channels as u32 * 40;
            let data_len = samples * 2;
            let mut wav = Vec::new();
            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&(36 + data_len).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&channels.to_le_bytes());
            wav.extend_from_slice(&rate.to_le_bytes());
            wav.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
            wav.extend_from_slice(&(channels * 2).to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&data_len.to_le_bytes());
            wav.resize(44 + data_len as usize, 0);
            let byte_len = wav.len() as u64;
            let decoder =
                build_decoder(Box::new(std::io::Cursor::new(wav)), Some(byte_len)).unwrap();
            let shared = Shared::new();
            shared.channels.store(channels as u32, Ordering::Relaxed);
            shared.sample_rate.store(rate, Ordering::Relaxed);
            shared.set_tempo(0.25);
            shared.reset_position(0.0, 0.25);
            let mut current = Current {
                decoder,
                pending: Vec::new(),
                source_done: false,
                stretch: build_stretch(rate, channels, 0.25).unwrap(),
                applied_tempo: 0.25,
                out_made: 0,
                scratch: Vec::new(),
                tail: Vec::new(),
                tail_pos: 0,
                flushed: false,
                source_id: String::new(),
                path: String::new(),
                last_trace: std::time::Instant::now(),
            };
            let (mut ring, mut consumer) = Ring::new(800);
            ring.done.store(false, Ordering::Release);
            for _ in 0..30 {
                Worker::pump(&shared, &mut ring, &mut current);
                while consumer.pop().is_ok() {}
            }
            assert!(
                !current.source_done,
                "decoded 40 seconds before playing even 12"
            );
            assert!(!current.flushed);
            assert!(current.stretch.buffered() < DECODE_CHUNK);
        }
    }

    /// Build a rodio decoder that reports the stream's real byte length. The
    /// length is what lets Symphonia's demuxers binary-search for a seek target;
    /// without it seeking degrades to forward-only (see `Cargo.toml`).
    fn build_decoder(
        reader: Box<dyn ReadSeek>,
        byte_len: Option<u64>,
    ) -> Result<Decoder<Box<dyn ReadSeek>>, AppError> {
        // A growing remote cache has a Seek implementation, but its end
        // position is not known yet. Marking it seekable makes Symphonia probe
        // with SeekFrom::End and wait for the entire download before emitting
        // the first sample. Only a source with a real byte length can safely
        // use the random-access decoder path.
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

    struct Current {
        decoder: Decoder<Box<dyn ReadSeek>>,
        /// Decoded source samples not yet handed to the stretcher.
        pending: Vec<f32>,
        source_done: bool,
        /// Pitch-preserving time-stretcher applied inline before the ring.
        stretch: wsola::TimeStretch,
        /// Tempo most recently applied to `stretch`.
        applied_tempo: f32,
        /// Interleaved output samples produced so far (anchors tempo segments).
        out_made: u64,
        /// Reused buffer for pulling stretched samples.
        scratch: Vec<f32>,
        /// End-of-stream overlap tail not yet pushed to the ring.
        tail: Vec<f32>,
        tail_pos: usize,
        flushed: bool,
        source_id: String,
        path: String,
        /// Throttles the per-second worker diagnostic line.
        last_trace: std::time::Instant,
    }

    /// Build a stretcher for the given stream format at the current tempo.
    fn build_stretch(rate: u32, channels: u16, tempo: f32) -> Result<wsola::TimeStretch, AppError> {
        let mut stretch = wsola::TimeStretch::new(
            NonZeroU32::new(rate.max(1)).expect("non-zero sample rate"),
            NonZeroU16::new(channels.max(1)).expect("non-zero channel count"),
        )
        .map_err(|error| AppError::other(error.to_string()))?;
        stretch.set_tempo(tempo);
        Ok(stretch)
    }

    struct Worker {
        app: AppHandle,
        cache: Arc<StreamCache>,
        smart: Arc<SmartCache>,
        shared: Arc<Shared>,
        ring: Ring,
        shutdown: Arc<AtomicBool>,
        /// Kept alive for the worker's lifetime; sources are added to its mixer.
        stream: OutputStream,
        current: Option<Current>,
        pending_tempo: Option<f32>,
        /// Throttles the worker-loop heartbeat.
        last_trace: std::time::Instant,
    }

    pub fn run(
        app: AppHandle,
        cache: Arc<StreamCache>,
        smart: Arc<SmartCache>,
        shared: Arc<Shared>,
        shutdown: Arc<AtomicBool>,
        rx: Receiver<Command>,
    ) {
        log::info!("playback: worker thread starting, opening output stream");
        shared.worker_started.store(true, Ordering::Release);
        let Ok(stream) = OutputStreamBuilder::open_default_stream() else {
            log::error!("playback engine: no audio output device available");
            *shared
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(ErrorInfo {
                code: "AUDIO_DEVICE".to_string(),
                message: "无法打开音频输出设备".to_string(),
            });
            return;
        };
        log::info!("playback: output stream opened, worker started");

        let mut worker = Worker {
            app,
            cache,
            smart,
            shared,
            shutdown,
            ring: Ring::new(1).0,
            stream,
            current: None,
            pending_tempo: None,
            last_trace: std::time::Instant::now(),
        };
        worker.loop_forever(rx);
    }

    impl Worker {
        fn loop_forever(&mut self, rx: Receiver<Command>) {
            while !self.shutdown.load(Ordering::Acquire) {
                // Drain every queued command first so seeks/pause stay snappy.
                loop {
                    match rx.try_recv() {
                        Ok(command) => self.apply(command),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                if let Some(rate) = self.pending_tempo.take() {
                    self.set_tempo(rate);
                }

                let idle = match self.current.as_mut() {
                    Some(current) if !self.shared.is_paused() && !self.ring.is_done() => {
                        Worker::pump(&self.shared, &mut self.ring, current);
                        self.ring.is_full()
                    }
                    _ => true,
                };

                if self.last_trace.elapsed() >= Duration::from_secs(1) {
                    self.last_trace = std::time::Instant::now();
                    log::debug!(
                        "playback: loop idle={} paused={} done={} ring={}",
                        idle,
                        self.shared.is_paused(),
                        self.ring.is_done(),
                        self.ring.len(),
                    );
                }

                if idle {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(command) => self.apply(command),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        }

        fn apply(&mut self, command: Command) {
            match command {
                Command::Load {
                    source_id,
                    path,
                    autoplay,
                } => {
                    log::info!("playback: cmd Load {source_id}::{path} autoplay={autoplay}");
                    self.load(&source_id, &path, autoplay)
                }
                Command::SetTempo { rate } => {
                    log::debug!("playback: cmd SetTempo {rate}");
                    // Coalesce slider events drained in the same worker turn;
                    // only the newest value needs a WSOLA/ring transition.
                    self.pending_tempo = Some(rate)
                }

                Command::Play => {
                    log::debug!("playback: cmd Play");
                    self.shared.set_paused(false)
                }
                Command::Pause => {
                    log::debug!("playback: cmd Pause");
                    self.shared.set_paused(true)
                }
                Command::Seek { position_secs } => {
                    log::debug!("playback: cmd Seek {position_secs:.3}s");
                    self.seek(position_secs)
                }
                Command::Stop => {
                    log::debug!("playback: cmd Stop");
                    self.stop()
                }
            }
        }

        /// Keep queued audio intact. The bounded output ring limits latency;
        /// rebuilding it here would skip audio and introduce an underrun.
        fn set_tempo(&mut self, rate: f32) {
            self.shared.set_tempo(rate);
        }

        fn load(&mut self, source_id: &str, path: &str, autoplay: bool) {
            // Do not let the old source continue while the new source is being
            // opened. Apply autoplay only after the initial prefill.
            // Invalidate it before doing the potentially blocking open.  If
            // opening or decoding fails, Play must not resurrect the previous
            // track through `current`.
            self.bump_generation();
            self.shared.set_paused(true);
            self.shared
                .transport
                .store(Transport::Loading as u8, Ordering::Relaxed);
            self.ring = Ring::new(1).0;
            self.current = None;
            *self
                .shared
                .location
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            // Reset the reported playhead right away: opening the new source can
            // block (network), and until then the event thread would keep
            // publishing the previous track's position, snapping the progress
            // bar back after the frontend already cleared it.
            self.shared.reset_position(0.0, self.shared.tempo());
            self.shared.duration_ms.store(0, Ordering::Relaxed);
            self.shared.set_ended(false);
            let open_started = std::time::Instant::now();
            let (reader, byte_len) =
                match open_track(&self.app, &self.cache, &self.smart, source_id, path) {
                    Ok(opened) => opened,
                    Err(error) => {
                        log::warn!(
                            "playback: open_track failed in {:?}: {error}",
                            open_started.elapsed()
                        );
                        self.fail(error.code(), &error.to_string());
                        return;
                    }
                };
            log::debug!(
                "playback: open_track ok in {:?} (byte_len={byte_len:?})",
                open_started.elapsed()
            );
            let decoder = match build_decoder(reader, byte_len) {
                Ok(decoder) => decoder,
                Err(error) => {
                    self.fail(error.code(), &error.to_string());
                    return;
                }
            };

            let channels = decoder.channels().max(1);
            let rate = decoder.sample_rate().max(1);
            let duration_ms = decoder
                .total_duration()
                .map(|value| value.as_millis() as u64)
                .unwrap_or(0);
            let capacity = output_samples(rate, channels, BUFFER_MILLIS);

            let generation = self.bump_generation();
            let (ring, consumer) = Ring::new(capacity);
            self.ring = ring;
            self.ring.done.store(false, Ordering::Relaxed);
            self.shared
                .channels
                .store(channels as u32, Ordering::Relaxed);
            self.shared.sample_rate.store(rate, Ordering::Relaxed);
            self.shared
                .duration_ms
                .store(duration_ms, Ordering::Relaxed);
            self.shared.set_ended(false);
            let tempo = self.shared.tempo();
            self.shared.reset_position(0.0, tempo);
            *self
                .shared
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            *self
                .shared
                .location
                .lock()
                .unwrap_or_else(|error| error.into_inner()) =
                Some((source_id.to_string(), path.to_string()));
            self.shared
                .transport
                .store(Transport::Paused as u8, Ordering::Relaxed);

            let stretch = match build_stretch(rate, channels, tempo) {
                Ok(stretch) => stretch,
                Err(error) => {
                    self.fail(error.code(), &error.to_string());
                    return;
                }
            };
            self.append_source(generation, channels, rate, consumer);
            self.current = Some(Current {
                decoder,
                pending: Vec::with_capacity(DECODE_CHUNK),
                source_done: false,
                stretch,
                applied_tempo: tempo,
                out_made: 0,
                scratch: Vec::with_capacity(STRETCH_REFILL),
                tail: Vec::new(),
                tail_pos: 0,
                flushed: false,
                source_id: source_id.to_string(),
                path: path.to_string(),
                last_trace: std::time::Instant::now(),
            });
            self.prefill();
            self.shared.set_paused(!autoplay);
            log::debug!(
                "playback: load ready {source_id}::{path} channels={channels} rate={rate} duration_ms={duration_ms} ring_len={} ring_done={}",
                self.ring.len(),
                self.ring.is_done(),
            );
        }

        fn seek(&mut self, position_secs: f64) {
            let tempo = self.shared.tempo();
            let Some(current) = self.current.as_mut() else {
                log::warn!("playback: seek to {position_secs:.3}s ignored, no track loaded");
                return;
            };
            // `Duration::from_secs_f64` panics for NaN, infinity, and values
            // outside Duration's range. Commands come from the frontend, so
            // sanitize them at the transport boundary.
            let target = if position_secs.is_finite() {
                position_secs.max(0.0).min(1.0e12)
            } else {
                0.0
            };
            let was_paused = self.shared.is_paused();
            log::debug!("playback: seek start target={target:.3}s was_paused={was_paused}");
            self.shared.set_paused(true);
            let seek_started = std::time::Instant::now();
            let seek_result = current.decoder.try_seek(Duration::from_secs_f64(target));
            log::debug!(
                "playback: try_seek {target:.3}s -> {:?} in {:?}",
                seek_result
                    .as_ref()
                    .map(|_| "ok")
                    .map_err(|error| error.to_string()),
                seek_started.elapsed(),
            );
            if let Err(error) = seek_result {
                // Some Symphonia format readers (notably a partially cached
                // remote MP3/M4A stream) report themselves as unseekable even
                // though the byte source implements Seek. Recreate the decoder
                // and advance it from the beginning instead of keeping the old
                // decoder alive. Keeping the old decoder here was the source of
                // both wrong progress and clicks after dragging.
                let source_id = current.source_id.clone();
                let path = current.path.clone();
                log::debug!("decoder seek unavailable, rebuilding at {target}s: {error}");
                match open_track(&self.app, &self.cache, &self.smart, &source_id, &path)
                    .and_then(|(reader, byte_len)| build_decoder(reader, byte_len))
                {
                    Ok(mut decoder) => {
                        // The freshly opened source may now report a byte length
                        // (the file/cache finished), so a real seek can succeed
                        // where the previous decoder could not.
                        if decoder.try_seek(Duration::from_secs_f64(target)).is_ok() {
                            log::debug!(
                                "playback: rebuilt decoder seeked directly to {target:.3}s"
                            );
                        } else {
                            if target > MAX_FORWARD_SEEK_SECONDS {
                                log::warn!("playback: refusing slow forward seek to {target:.3}s");
                                *self
                                    .shared
                                    .failure
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner()) = Some(ErrorInfo {
                                    code: "SEEK_UNSUPPORTED".to_string(),
                                    message: "当前音频流不支持此位置的快速定位".to_string(),
                                });
                                self.shared.set_paused(was_paused);
                                return;
                            }
                            let channels = decoder.channels().max(1) as u64;
                            let rate = decoder.sample_rate().max(1) as u64;
                            let samples = (target * rate as f64 * channels as f64) as u64;
                            log::debug!(
                                "playback: rebuilding decoder, skipping {samples} samples ({target:.3}s @ {rate}Hz/{channels}ch)",
                            );
                            let rebuild_started = std::time::Instant::now();
                            for _ in 0..samples {
                                if decoder.next().is_none() {
                                    break;
                                }
                            }
                            log::debug!(
                                "playback: decoder rebuild catch-up took {:?}",
                                rebuild_started.elapsed(),
                            );
                        }
                        current.decoder = decoder;
                    }
                    Err(rebuild_error) => {
                        log::warn!("playback seek rebuild failed: {rebuild_error}");
                        self.shared.set_paused(was_paused);
                        return;
                    }
                }
            }
            current.pending.clear();
            current.source_done = false;
            current.out_made = 0;
            current.flushed = false;
            current.tail.clear();
            current.tail_pos = 0;
            // A seek invalidates everything the stretcher had buffered.
            current.stretch.reset();
            current.stretch.set_tempo(tempo);
            current.applied_tempo = tempo;
            current.last_trace = std::time::Instant::now();

            let channels = self.shared.channels.load(Ordering::Relaxed).max(1) as u64;
            let rate = self.shared.sample_rate.load(Ordering::Relaxed).max(1) as u64;
            let generation = self.bump_generation();
            let (ring, consumer) =
                Ring::new(output_samples(rate as u32, channels as u16, BUFFER_MILLIS));
            self.ring = ring;
            self.ring.done.store(false, Ordering::Relaxed);
            self.shared.reset_position(target, tempo);
            self.shared.set_ended(false);
            self.append_source(generation, channels as u16, rate as u32, consumer);
            log::debug!(
                "playback: seek appended source gen={generation} channels={channels} rate={rate}",
            );
            self.prefill();
            log::debug!(
                "playback: seek prefill done ring_len={} ring_done={}",
                self.ring.len(),
                self.ring.is_done(),
            );
            self.shared.set_paused(was_paused);
            log::debug!("playback: seek complete target={target:.3}s paused={was_paused}");
        }

        /// Fill the ring with a little time-stretched audio before the output
        /// callback may consume it, so playback does not start on an underrun.
        fn prefill(&mut self) {
            let target = output_samples(
                self.shared.sample_rate.load(Ordering::Relaxed),
                self.shared.channels.load(Ordering::Relaxed) as u16,
                PREFILL_MILLIS,
            );
            while self.ring.len() < target && !self.ring.is_done() {
                let Some(current) = self.current.as_mut() else {
                    break;
                };
                let before = self.ring.len();
                Self::pump(&self.shared, &mut self.ring, current);
                if self.ring.len() == before {
                    // Input is starved or the stream already ended; the main
                    // loop will keep filling as more becomes available.
                    break;
                }
            }
            if self.ring.is_done() {
                log::debug!(
                    "playback: prefill reached end of stream (ring_len={} target={target})",
                    self.ring.len(),
                );
            }
        }

        fn stop(&mut self) {
            self.bump_generation();
            self.ring = Ring::new(1).0;
            self.ring.done.store(true, Ordering::Release);
            self.shared
                .transport
                .store(Transport::Idle as u8, Ordering::Relaxed);
            self.shared.set_paused(true);
            self.shared.duration_ms.store(0, Ordering::Relaxed);
            self.shared.set_ended(false);
            self.shared.reset_position(0.0, self.shared.tempo());
            *self
                .shared
                .location
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            self.current = None;
        }

        /// Decode, time-stretch and push to the ring. Works incrementally so the
        /// command loop stays responsive while the ring is full or input slow.
        fn pump(shared: &Shared, ring: &mut Ring, current: &mut Current) {
            let channels = shared.channels.load(Ordering::Relaxed).max(1) as usize;

            if current.last_trace.elapsed() >= Duration::from_secs(1) {
                current.last_trace = std::time::Instant::now();
                log::debug!(
                    "playback: worker ring={} space={} pending={} source_done={} buffered={} out_made={} tempo={} flushed={}",
                    ring.len(),
                    ring.space(),
                    current.pending.len(),
                    current.source_done,
                    current.stretch.buffered(),
                    current.out_made,
                    current.applied_tempo,
                    current.flushed,
                );
            }

            // Push any buffered end-of-stream tail first so ordering is kept.
            Self::push_tail(ring, current);
            if current.flushed && current.tail_pos >= current.tail.len() {
                ring.done.store(true, Ordering::Release);
                return;
            }

            // Do not decode or stretch ahead while the ring is full.
            if ring.space() == 0 {
                return;
            }

            // Apply a playback-rate change: anchor a new tempo segment at the
            // source position of the current output index, then retune.
            let tempo = shared.tempo();
            if (tempo - current.applied_tempo).abs() > 1e-4 {
                // Output already inside WSOLA still belongs to the old tempo.
                let boundary = current.out_made + current.stretch.available() as u64;
                let position = shared.position_at(boundary);
                shared.push_segment(boundary, position, tempo);
                current.stretch.set_tempo(tempo);
                current.applied_tempo = tempo;
            }

            // Drain buffered WSOLA input before reading upstream again. At slow
            // tempos one decode chunk produces several output chunks; feeding
            // another on every call grows input without bound and eventually
            // turns flush() into a long, synchronous DSP operation.
            if Self::pull_output(ring, current, channels) > 0 {
                return;
            }

            // Only decode when WSOLA actually needs more input.
            if !current.source_done && current.pending.len() < channels {
                let decode_started = std::time::Instant::now();
                while current.pending.len() < DECODE_CHUNK {
                    match current.decoder.next() {
                        Some(sample) => current.pending.push(sample),
                        None => {
                            // Do not discard the final partial chunk. Dropping
                            // it causes an abrupt cutoff and an audible click.
                            current.source_done = true;
                            log::debug!("playback: decoder reached end of stream");
                            break;
                        }
                    }
                }
                // A blocking network read on the worker shows up here: the
                // refill is normally sub-millisecond, so anything slow means the
                // decoder is waiting on bytes (the classic post-seek stall).
                let elapsed = decode_started.elapsed();
                if !current.source_done && elapsed > Duration::from_millis(200) {
                    log::warn!(
                        "playback: decode refill blocked for {elapsed:?} ({} samples)",
                        current.pending.len(),
                    );
                }
            }

            // Hand whole frames to the stretcher (it requires frame alignment).
            let aligned = current.pending.len() / channels * channels;
            if aligned > 0 {
                current.stretch.push(&current.pending[..aligned]);
                current.pending.drain(0..aligned);
            }

            Self::pull_output(ring, current, channels);

            // Once the source is exhausted, emit the final overlap tail exactly
            // once (when only a sub-frame remainder is left in `pending`).
            if current.source_done && current.pending.len() < channels && !current.flushed {
                current.tail.clear();
                current.tail.extend(current.stretch.flush());
                current.tail_pos = 0;
                current.flushed = true;
                Self::push_tail(ring, current);
            }

            if current.flushed && current.tail_pos >= current.tail.len() {
                ring.done.store(true, Ordering::Release);
            }
        }

        fn pull_output(ring: &mut Ring, current: &mut Current, channels: usize) -> usize {
            let want = ring.space().min(STRETCH_REFILL) / channels * channels;
            if want > 0 {
                {
                    let Current {
                        stretch, scratch, ..
                    } = current;
                    scratch.clear();
                    scratch.extend(stretch.pull(want));
                }
                let count = current.scratch.len();
                let mut offset = 0;
                while offset < count {
                    let pushed = ring.push_slice(&current.scratch[offset..]);
                    if pushed == 0 {
                        break;
                    }
                    offset += pushed;
                }
                current.out_made += offset as u64;
                return offset;
            }
            0
        }

        /// Push as much of the flushed tail as the ring accepts, keeping the
        /// remainder for the next call.
        fn push_tail(ring: &mut Ring, current: &mut Current) {
            while current.tail_pos < current.tail.len() {
                let pushed = ring.push_slice(&current.tail[current.tail_pos..]);
                if pushed == 0 {
                    break;
                }
                current.tail_pos += pushed;
                current.out_made += pushed as u64;
            }
        }

        /// Bump the generation counter, invalidating any `RingSource` still held
        /// by the mixer, and return the new value.
        fn bump_generation(&self) -> u64 {
            self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1
        }

        /// Add a fresh `RingSource` to the mixer for the given generation.
        fn append_source(
            &self,
            generation: u64,
            channels: u16,
            rate: u32,
            consumer: Consumer<f32>,
        ) {
            self.stream.mixer().add(RingSource {
                done: self.ring.done.clone(),
                consumer,
                shared: self.shared.clone(),
                progress: self.shared.progress(),
                generation,
                channels,
                rate,
                state_left: 0,
                paused: true,
                gain: 0.0,
                target_gain: -1.0,
                gain_step: 0.0,
                ramp_frames: 0,
                channel_cursor: 0,
                waveform_peak: 0.0,
                waveform_frames: 0,
                spectrum_sum: 0.0,
            });
        }

        /// Record a fatal transport/decode error for the event thread.
        fn fail(&mut self, code: &str, message: &str) {
            self.bump_generation();
            *self
                .shared
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            });
            self.ring.done.store(true, Ordering::Release);
            self.shared
                .transport
                .store(Transport::Failed as u8, Ordering::Relaxed);
            // An error is not a natural end and must not auto-advance a queue.
            self.shared.set_ended(false);
            self.shared.duration_ms.store(0, Ordering::Relaxed);
            self.shared.set_paused(true);
            *self
                .shared
                .location
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            self.current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(shared: Arc<Shared>, ring: &Ring, consumer: Consumer<f32>) -> RingSource {
        RingSource {
            consumer,
            done: ring.done.clone(),
            progress: shared.progress(),
            generation: shared.generation.load(Ordering::Relaxed),
            shared,
            channels: 2,
            rate: 1000,
            state_left: 0,
            paused: false,
            gain: 0.0,
            target_gain: -1.0,
            gain_step: 0.0,
            ramp_frames: 0,
            channel_cursor: 0,
            waveform_peak: 0.0,
            waveform_frames: 0,
            spectrum_sum: 0.0,
        }
    }

    #[test]
    fn ring_empty_full_wrap_and_endpoint_drop() {
        let (mut ring, mut consumer) = Ring::new(3);
        assert!(consumer.pop().is_err());
        assert_eq!(ring.push_slice(&[1.0, 2.0, 3.0, 4.0]), 3);
        assert!(ring.is_full());
        assert_eq!(consumer.pop(), Ok(1.0));
        assert_eq!(ring.push_slice(&[4.0]), 1);
        drop(ring);
        assert_eq!(consumer.pop(), Ok(2.0));
        assert_eq!(consumer.pop(), Ok(3.0));
        assert_eq!(consumer.pop(), Ok(4.0));
        assert!(consumer.pop().is_err());

        let (mut ring, consumer) = Ring::new(1);
        drop(consumer);
        assert_eq!(ring.push_slice(&[5.0]), 1);
        drop(ring);
    }

    #[test]
    fn concurrent_ring_preserves_order_across_many_wraps() {
        let (mut ring, mut consumer) = Ring::new(7);
        let writer = std::thread::spawn(move || {
            for sample in 0..100_000 {
                while ring.push_slice(&[sample as f32]) == 0 {
                    std::thread::yield_now();
                }
            }
        });
        for sample in 0..100_000 {
            loop {
                if let Ok(value) = consumer.pop() {
                    assert_eq!(value, sample as f32);
                    break;
                }
                std::thread::yield_now();
            }
        }
        writer.join().unwrap();
    }

    #[test]
    fn eof_drains_all_samples() {
        let shared = Arc::new(Shared::new());
        shared
            .transport
            .store(Transport::Paused as u8, Ordering::Relaxed);
        shared.set_paused(false);
        let (mut ring, consumer) = Ring::new(4);
        ring.push_slice(&[1.0; 4]);
        ring.done.store(true, Ordering::Release);
        let mut source = source(shared.clone(), &ring, consumer);
        for _ in 0..4 {
            assert!(source.next().is_some());
        }
        assert_eq!(source.next(), None);
        assert!(shared.has_ended());
    }

    #[test]
    fn generation_invalidates_mid_block_without_consuming_more_audio() {
        let shared = Arc::new(Shared::new());
        shared
            .transport
            .store(Transport::Paused as u8, Ordering::Relaxed);
        shared.set_paused(false);
        let (mut ring, consumer) = Ring::new(4);
        ring.push_slice(&[1.0; 4]);
        let mut source = source(shared.clone(), &ring, consumer);
        assert!(source.next().is_some());
        assert!(source.state_left > 0);
        shared.generation.fetch_add(1, Ordering::SeqCst);
        assert_eq!(source.next(), None);
        assert_eq!(shared.played(), 1);
        assert_eq!(ring.len(), 3);
    }

    #[test]
    fn stale_source_counters_cannot_touch_new_position_or_ended() {
        let shared = Shared::new();
        let old = shared.progress();
        shared.reset_position(12.0, 1.0);
        old.frames.fetch_add(100, Ordering::Relaxed);
        old.ended.store(true, Ordering::Relaxed);
        assert_eq!(shared.played(), 0);
        assert!(!shared.has_ended());
        assert_eq!(shared.position_secs(), 12.0);
    }

    #[test]
    fn volume_ramp_keeps_stereo_channels_equal() {
        let shared = Arc::new(Shared::new());
        shared
            .transport
            .store(Transport::Paused as u8, Ordering::Relaxed);
        shared.set_paused(false);
        let (mut ring, consumer) = Ring::new(20);
        ring.push_slice(&[1.0; 20]);
        let mut source = source(shared.clone(), &ring, consumer);
        let mut previous = 0.0;
        for _ in 0..10 {
            let left = source.next().unwrap();
            let right = source.next().unwrap();
            assert_eq!(left, right);
            assert!(left >= previous && left - previous <= 0.101);
            previous = left;
        }
        shared.set_volume_percent(f64::NAN);
        assert_eq!(shared.volume(), 0.0);
    }

    #[test]
    fn segment_boundaries_remain_continuous_and_same_index_is_coalesced() {
        let shared = Shared::new();
        shared.sample_rate.store(1000, Ordering::Relaxed);
        shared.channels.store(2, Ordering::Relaxed);
        shared.reset_position(10.0, 1.0);
        shared.push_segment(2000, 11.0, 2.0);
        shared.push_segment(2000, 11.0, 0.5);
        assert_eq!(shared.position_at(1000), 10.5);
        assert_eq!(shared.position_at(4000), 11.5);
        shared.progress().frames.store(4000, Ordering::Relaxed);
        assert_eq!(shared.position_secs(), 11.5);
        assert_eq!(shared.segments.lock().unwrap().len(), 1);
    }

    #[test]
    fn output_capacity_is_always_frame_aligned() {
        for channels in [1, 2, 6, 8] {
            for rate in [1, 11024, 44100, 48000, 192000] {
                let count = output_samples(rate, channels, BUFFER_MILLIS);
                assert!(count >= channels as usize);
                assert_eq!(count % channels as usize, 0);
            }
        }
    }

    #[test]
    fn play_does_not_resurrect_idle_or_failed_transport() {
        let shared = Shared::new();
        shared.set_paused(false);
        assert!(!shared.snapshot().playing);
        shared
            .transport
            .store(Transport::Failed as u8, Ordering::Relaxed);
        shared.set_paused(false);
        assert!(!shared.snapshot().playing);
        assert!(!shared.snapshot().ended);
        shared
            .transport
            .store(Transport::Paused as u8, Ordering::Relaxed);
        shared.set_paused(false);
        assert!(shared.snapshot().playing);
        shared.set_paused(true);
        assert!(!shared.snapshot().playing);
    }
}
