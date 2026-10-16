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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Source};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::core::error::AppError;
use crate::playback::source::{open_track, ReadSeek};
use crate::playback::spectrum::Spectrum;
use crate::streaming::smart_cache::SmartCache;
use crate::streaming::stream_cache::StreamCache;

/// How often the playhead snapshot is pushed to the frontend (milliseconds).
const STATE_INTERVAL_MS: u64 = 250;
/// Decoded samples produced per decode chunk before offering them to the ring.
const DECODE_CHUNK: usize = 8192;
/// Ring buffer length, in seconds of the file's own rate * channels.
const BUFFER_SECONDS: usize = 10;
/// Decode this much before starting a new source after load/seek.
const PREFILL_SECONDS: usize = 2;

/// A transport error surfaced to the frontend on `audio:error`.
#[derive(Clone, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
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
    /// Interleaved samples actually consumed by the output callback.
    frames: AtomicU64,
    /// Times the output callback ran dry while a track was loading/playing.
    underruns: AtomicU64,
    channels: AtomicU32,
    sample_rate: AtomicU32,
    duration_ms: AtomicU64,
    active: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    /// Volume as `f32` bits, `0.0..=1.0`.
    volume: AtomicU32,
    ended: AtomicBool,
    failure: Mutex<Option<ErrorInfo>>,
    /// `(source_id, path)` of the loaded track, for cache progress reporting.
    location: Mutex<Option<(String, String)>>,
    /// Sample tap for the visualiser. Fed by the decode worker, read by its own
    /// emitter thread; the audio callback never touches it.
    pub spectrum: Arc<Spectrum>,
}

impl Shared {
    fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            channels: AtomicU32::new(2),
            sample_rate: AtomicU32::new(44_100),
            duration_ms: AtomicU64::new(0),
            active: AtomicBool::new(false),
            paused: AtomicBool::new(true),
            muted: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            ended: AtomicBool::new(false),
            failure: Mutex::new(None),
            location: Mutex::new(None),
            spectrum: Arc::new(Spectrum::new()),
        }
    }

    /// Requested playback state: a track is loaded and not paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Set the output volume from a percentage (0-100).
    pub fn set_volume_percent(&self, percent: f64) {
        let value = (percent / 100.0).clamp(0.0, 1.0) as f32;
        self.volume.store(value.to_bits(), Ordering::Relaxed);
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// Position snapshot in milliseconds, derived from consumed frames.
    fn position_ms(&self) -> f64 {
        let channels = self.channels.load(Ordering::Relaxed).max(1) as f64;
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1) as f64;
        let frames = self.frames.load(Ordering::Relaxed) as f64;
        frames / (rate * channels) * 1000.0
    }

    /// Full snapshot for the initial `audio_state` pull.
    pub fn snapshot(&self) -> AudioState {
        let active = self.active.load(Ordering::Relaxed);
        let paused = self.paused.load(Ordering::Relaxed);
        let ended = self.ended.load(Ordering::Relaxed);
        AudioState {
            playing: active && !paused && !ended,
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms.load(Ordering::Relaxed) as f64,
            ended,
        }
    }
}

/// Ring buffer shared between the decode worker (producer) and the mixer
/// (consumer). `try_lock` on the consumer side keeps the audio callback
/// non-blocking.
struct Ring {
    inner: Mutex<RingInner>,
    done: AtomicBool,
}

struct RingInner {
    data: VecDeque<f32>,
    capacity: usize,
}

impl Ring {
    fn new() -> Self {
        Self {
            inner: Mutex::new(RingInner {
                data: VecDeque::new(),
                capacity: 0,
            }),
            done: AtomicBool::new(true),
        }
    }

    fn reset(&self, capacity: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.data.clear();
        inner.capacity = capacity.max(1);
    }

    fn clear(&self) {
        self.inner.lock().unwrap().data.clear();
    }

    fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    fn is_full(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.capacity == 0 || inner.data.len() >= inner.capacity
    }

    fn len(&self) -> usize {
        self.inner.lock().unwrap().data.len()
    }

    /// Push as many samples as fit; returns how many were accepted.
    fn push_slice(&self, values: &[f32]) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let space = inner.capacity.saturating_sub(inner.data.len());
        // Keep the producer critical section short so the realtime consumer
        // does not need to inject a zero while a large decode chunk is copied.
        let count = space.min(values.len()).min(256);
        inner.data.extend(values[..count].iter().copied());
        count
    }
}

/// Non-blocking `Source` for the mixer. Declares the file's native rate and
/// channel count; `rodio` resamples/remaps it to the device configuration.
struct RingSource {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    generation: u64,
    channels: u16,
    rate: u32,
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.shared.generation.load(Ordering::Relaxed) != self.generation {
            return None;
        }
        // Paused: emit silence without consuming, so the playhead freezes.
        if self.shared.paused.load(Ordering::Relaxed) {
            return Some(0.0);
        }

        // A short lock wait is preferable to injecting a zero between two
        // non-zero samples. Such an isolated zero is heard as a click/pop.
        let popped = Some(self.ring.inner.lock().unwrap().data.pop_front());

        match popped {
            None => Some(0.0),
            Some(Some(value)) => {
                self.shared.frames.fetch_add(1, Ordering::Relaxed);
                if self.shared.muted.load(Ordering::Relaxed) {
                    Some(0.0)
                } else {
                    Some(value * self.shared.volume())
                }
            }
            Some(None) => {
                if self.ring.done.load(Ordering::Relaxed) {
                    self.shared.ended.store(true, Ordering::Relaxed);
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
    Load { source_id: String, path: String },
    Play,
    Pause,
    Seek { position_secs: f64 },
    Stop,
}

/// Handle held in Tauri managed state.
pub struct Engine {
    tx: Sender<Command>,
    shared: Arc<Shared>,
}

impl Engine {
    /// Shared snapshot, for the event thread and the `audio_state` command.
    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    /// Queue a transport command for the worker.
    pub fn send(&self, command: Command) -> Result<(), AppError> {
        self.tx
            .send(command)
            .map_err(|_| AppError::other("播放引擎已停止"))
    }
}

/// Start the engine (worker + event threads) and return its handle.
pub fn start(app: AppHandle, cache: Arc<StreamCache>, smart: Arc<SmartCache>) -> Engine {
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared::new());
    let app_handle = app.clone();

    let worker_shared = shared.clone();
    let worker_app = app.clone();
    std::thread::spawn(move || worker::run(worker_app, cache, smart, worker_shared, rx));

    let events_shared = shared.clone();
    std::thread::spawn(move || events::run(app, events_shared));

    // The visualiser needs frames far faster than the 250 ms state tick, and only
    // while one is subscribed, so it gets a thread of its own.
    let spectrum_shared = shared.clone();
    std::thread::spawn(move || events::run_spectrum(app_handle, spectrum_shared));

    Engine { tx, shared }
}

mod events {
    use super::*;

    /// Push periodic state snapshots and one-shot ended/error events.
    pub fn run(app: AppHandle, shared: Arc<Shared>) {
        let mut last_ended_generation = u64::MAX;
        let mut last_underruns = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(STATE_INTERVAL_MS));

            let state = shared.snapshot();
            let _ = app.emit("audio:state", state);

            // Surface buffer underruns: silence injected before the decoder
            // caught up (debug level; hidden at the default `Info` cap).
            let underruns = shared.underruns.load(Ordering::Relaxed);
            if underruns > last_underruns {
                log::debug!(
                    "playback: {} underrun sample(s) since last tick (playing={}, ended={})",
                    underruns - last_underruns,
                    state.playing,
                    shared.ended.load(Ordering::Relaxed),
                );
                last_underruns = underruns;
            }

            if let Some(failure) = shared.failure.lock().unwrap().take() {
                let _ = app.emit("audio:error", failure);
            }

            let generation = shared.generation.load(Ordering::Relaxed);
            if state.ended && generation != last_ended_generation {
                last_ended_generation = generation;
                let _ = app.emit("audio:ended", ());
            }
        }
    }

    /// Offer analyser frames while a visualiser is subscribed.
    ///
    /// Idle — no analysis, no allocation, no IPC — whenever nothing is on screen,
    /// so keeping the full-screen player closed costs nothing.
    pub fn run_spectrum(app: AppHandle, shared: Arc<Shared>) {
        let mut last_seq = 0u64;
        loop {
            let spectrum = &shared.spectrum;
            let fps = spectrum.fps().max(1) as u64;
            std::thread::sleep(Duration::from_millis(1000 / fps));
            if !spectrum.is_enabled() {
                continue;
            }
            if let Some(frame) = spectrum.frame_if_new(last_seq) {
                last_seq = frame.seq;
                let _ = app.emit("audio:spectrum", frame);
            }
        }
    }
}

mod worker {
    use super::*;

    /// Build a rodio decoder that reports the stream's real byte length. The
    /// length is what lets Symphonia's demuxers binary-search for a seek target;
    /// without it seeking degrades to forward-only (see `Cargo.toml`).
    fn build_decoder(
        reader: Box<dyn ReadSeek>,
        byte_len: Option<u64>,
    ) -> Result<Decoder<Box<dyn ReadSeek>>, AppError> {
        let builder = Decoder::builder().with_data(reader).with_seekable(true);
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
        pending: Vec<f32>,
        pending_offset: usize,
        source_done: bool,
        source_id: String,
        path: String,
    }

    struct Worker {
        app: AppHandle,
        cache: Arc<StreamCache>,
        smart: Arc<SmartCache>,
        shared: Arc<Shared>,
        ring: Arc<Ring>,
        /// Kept alive for the worker's lifetime; sources are added to its mixer.
        stream: OutputStream,
        current: Option<Current>,
    }

    pub fn run(
        app: AppHandle,
        cache: Arc<StreamCache>,
        smart: Arc<SmartCache>,
        shared: Arc<Shared>,
        rx: Receiver<Command>,
    ) {
        let Ok(stream) = OutputStreamBuilder::open_default_stream() else {
            log::error!("playback engine: no audio output device available");
            *shared.failure.lock().unwrap() = Some(ErrorInfo {
                code: "AUDIO_DEVICE".to_string(),
                message: "无法打开音频输出设备".to_string(),
            });
            return;
        };

        let mut worker = Worker {
            app,
            cache,
            smart,
            shared,
            ring: Arc::new(Ring::new()),
            stream,
            current: None,
        };
        worker.loop_forever(rx);
    }

    impl Worker {
        fn loop_forever(&mut self, rx: Receiver<Command>) {
            loop {
                // Drain every queued command first so seeks/pause stay snappy.
                loop {
                    match rx.try_recv() {
                        Ok(command) => self.apply(command),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                let idle = match self.current.as_mut() {
                    Some(current)
                        if !self.shared.is_paused() && !self.ring.is_done() =>
                    {
                        Worker::pump(&self.shared, &self.ring, current);
                        self.ring.is_full()
                    }
                    _ => true,
                };

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
                Command::Load { source_id, path } => {
                    log::debug!("playback: cmd Load {source_id}::{path}");
                    self.load(&source_id, &path)
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

        fn load(&mut self, source_id: &str, path: &str) {
            // Do not let the old source continue while the new source is being
            // opened. audio_play is sent after audio_load by the frontend.
            self.shared.set_paused(true);
            // Reset the reported playhead right away: opening the new source can
            // block (network), and until then the event thread would keep
            // publishing the previous track's position, snapping the progress
            // bar back after the frontend already cleared it.
            self.shared.frames.store(0, Ordering::Relaxed);
            self.shared.duration_ms.store(0, Ordering::Relaxed);
            self.shared.ended.store(false, Ordering::Relaxed);
            let open_started = std::time::Instant::now();
            let (reader, byte_len) = match open_track(&self.app, &self.cache, &self.smart, source_id, path) {
                Ok(opened) => opened,
                Err(error) => {
                    log::warn!("playback: open_track failed in {:?}: {error}", open_started.elapsed());
                    self.fail(error.code(), &error.to_string());
                    return;
                }
            };
            log::debug!("playback: open_track ok in {:?} (byte_len={byte_len:?})", open_started.elapsed());
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
            let capacity = rate as usize * channels as usize * BUFFER_SECONDS;

            let generation = self.bump_generation();
            self.ring.reset(capacity);
            self.ring.done.store(false, Ordering::Relaxed);
            self.shared.frames.store(0, Ordering::Relaxed);
            self.shared.channels.store(channels as u32, Ordering::Relaxed);
            self.shared.sample_rate.store(rate, Ordering::Relaxed);
            self.shared.duration_ms.store(duration_ms, Ordering::Relaxed);
            self.shared.ended.store(false, Ordering::Relaxed);
            *self.shared.failure.lock().unwrap() = None;
            *self.shared.location.lock().unwrap() = Some((source_id.to_string(), path.to_string()));
            self.shared.active.store(true, Ordering::Relaxed);

            self.append_source(generation, channels, rate);
            self.current = Some(Current {
                decoder,
                pending: Vec::with_capacity(DECODE_CHUNK),
                pending_offset: 0,
                source_done: false,
                source_id: source_id.to_string(),
                path: path.to_string(),
            });
            self.prefill();
            log::debug!(
                "playback: load ready {source_id}::{path} channels={channels} rate={rate} duration_ms={duration_ms} ring_len={} ring_done={}",
                self.ring.len(),
                self.ring.is_done(),
            );
        }

        fn seek(&mut self, position_secs: f64) {
            let Some(current) = self.current.as_mut() else {
                log::warn!("playback: seek to {position_secs:.3}s ignored, no track loaded");
                return;
            };
            let target = position_secs.max(0.0);
            let was_paused = self.shared.is_paused();
            log::debug!("playback: seek start target={target:.3}s was_paused={was_paused}");
            self.shared.set_paused(true);
            let seek_started = std::time::Instant::now();
            let seek_result = current.decoder.try_seek(Duration::from_secs_f64(target));
            log::debug!(
                "playback: try_seek {target:.3}s -> {:?} in {:?}",
                seek_result.as_ref().map(|_| "ok").map_err(|error| error.to_string()),
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
                            log::debug!("playback: rebuilt decoder seeked directly to {target:.3}s");
                        } else {
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
            current.pending_offset = 0;
            current.source_done = false;

            let channels = self.shared.channels.load(Ordering::Relaxed).max(1) as u64;
            let rate = self.shared.sample_rate.load(Ordering::Relaxed).max(1) as u64;
            let generation = self.bump_generation();
            self.ring.clear();
            self.ring.done.store(false, Ordering::Relaxed);
            self.shared
                .frames
                .store((target * rate as f64) as u64 * channels, Ordering::Relaxed);
            self.shared.ended.store(false, Ordering::Relaxed);
            self.append_source(generation, channels as u16, rate as u32);
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

        /// Ensure a newly loaded/seeking source has audio ready before the
        /// output callback is allowed to consume it. This prevents a zero
        /// sample gap (heard as a click) at every seek on a remote track.
        fn prefill(&mut self) {
            let target = self.shared.sample_rate.load(Ordering::Relaxed) as usize
                * self.shared.channels.load(Ordering::Relaxed).max(1) as usize
                * PREFILL_SECONDS;
            while self.ring.len() < target {
                let Some(current) = self.current.as_mut() else { break };
                if current.source_done && current.pending_offset >= current.pending.len() {
                    self.ring.done.store(true, Ordering::Relaxed);
                    break;
                }
                Self::pump(&self.shared, &self.ring, current);
                if self.ring.is_done() && self.ring.len() == 0 {
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
            self.ring.reset(1);
            self.ring.done.store(true, Ordering::Relaxed);
            self.shared.active.store(false, Ordering::Relaxed);
            self.shared.set_paused(true);
            *self.shared.location.lock().unwrap() = None;
            self.current = None;
        }

        /// Decode up to one chunk and push what fits into the ring. Leftovers
        /// are kept for the next call so the command loop stays responsive even
        /// while the ring is full.
        fn pump(shared: &Shared, ring: &Ring, current: &mut Current) {
            let generation = shared.generation.load(Ordering::Relaxed);

            if current.pending_offset >= current.pending.len() {
                current.pending.clear();
                current.pending_offset = 0;
                let decode_started = std::time::Instant::now();
                while current.pending.len() < DECODE_CHUNK && !current.source_done {
                    match current.decoder.next() {
                        Some(sample) => current.pending.push(sample),
                        None => {
                            // Do not discard the final partial chunk. Dropping
                            // it causes an abrupt cutoff and an audible click.
                            current.source_done = true;
                        }
                    }
                }
                // A blocking network read on the worker shows up here: the
                // refill is normally sub-millisecond, so anything slow means the
                // decoder is waiting on bytes (the classic post-seek stall).
                let elapsed = decode_started.elapsed();
                if current.source_done {
                    log::debug!("playback: decoder reached end of stream");
                } else if elapsed > Duration::from_millis(200) {
                    log::warn!(
                        "playback: decode refill blocked for {elapsed:?} ({} samples)",
                        current.pending.len(),
                    );
                }
            }

            if shared.generation.load(Ordering::Relaxed) != generation {
                return;
            }
            let pushed = ring.push_slice(&current.pending[current.pending_offset..]);
            // Visualiser tap: the same samples, on this worker thread, after they
            // were offered to the ring, so analysis can never delay audio.
            if pushed > 0 {
                shared.spectrum.push(
                    &current.pending[current.pending_offset..current.pending_offset + pushed],
                    shared.channels.load(Ordering::Relaxed),
                    shared.sample_rate.load(Ordering::Relaxed),
                );
            }
            current.pending_offset += pushed;
            if current.source_done && current.pending_offset >= current.pending.len() {
                ring.done.store(true, Ordering::Relaxed);
            }
        }

        /// Bump the generation counter, invalidating any `RingSource` still held
        /// by the mixer, and return the new value.
        fn bump_generation(&self) -> u64 {
            self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1
        }

        /// Add a fresh `RingSource` to the mixer for the given generation.
        fn append_source(&self, generation: u64, channels: u16, rate: u32) {
            self.stream.mixer().add(RingSource {
                ring: self.ring.clone(),
                shared: self.shared.clone(),
                generation,
                channels,
                rate,
            });
        }

        /// Record a fatal transport/decode error for the event thread.
        fn fail(&mut self, code: &str, message: &str) {
            *self.shared.failure.lock().unwrap() = Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            });
            self.ring.done.store(true, Ordering::Relaxed);
            self.shared.active.store(false, Ordering::Relaxed);
            self.shared.ended.store(true, Ordering::Relaxed);
            self.shared.set_paused(true);
            self.current = None;
        }
    }
}
