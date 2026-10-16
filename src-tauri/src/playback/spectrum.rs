// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Spectrum analysis for the full-screen visualiser.
//!
//! `Worker::pump` hands every decoded sample to [`Spectrum::push`] *after* the
//! samples have been offered to the ring buffer, on the decode worker thread, so
//! the analysis can never delay audio: the realtime callback only ever touches
//! the ring.
//!
//! A 2048-sample window is filled at the file's native rate, mono-downmixed,
//! Hann-windowed and transformed with a small iterative radix-2 FFT. The bins are
//! folded into logarithmically spaced bands (so bass does not dominate the
//! picture), converted to decibels and smoothed with a fast attack / slow release
//! — that asymmetry is what makes a bar graph look like it follows the music
//! instead of flickering with it.
//!
//! Nothing here polls the frontend: `events::run_spectrum` emits `audio:spectrum`
//! whenever a fresh frame exists, and only while a visualiser is subscribed.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;

/// Samples analysed per frame (power of two).
const WINDOW: usize = 2048;
/// Lowest and highest band edge, in hertz.
const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 16_000.0;
/// Magnitude range mapped onto 0..1, in decibels.
const FLOOR_DB: f32 = -78.0;
const CEIL_DB: f32 = -8.0;
/// Per-frame smoothing: quick rise, slow fall.
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.14;
/// Band-count limits accepted from the frontend.
const MIN_BANDS: usize = 8;
const MAX_BANDS: usize = 256;
/// Emitter cadence limits, in frames per second.
pub const MIN_FPS: u32 = 5;
pub const MAX_FPS: u32 = 60;

/// One analysis frame, pushed to the frontend on `audio:spectrum`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumFrame {
    /// Monotonic counter; lets the emitter skip frames nothing consumed.
    pub seq: u64,
    /// Band magnitudes, `0..=255`, low frequencies first.
    pub bands: Vec<u8>,
}

struct Inner {
    /// Rolling time-domain window, filled sample by sample.
    window: Vec<f32>,
    position: usize,
    /// Scratch buffers, kept across frames so analysis never allocates.
    re: Vec<f32>,
    im: Vec<f32>,
    /// Smoothed band levels, `0.0..=1.0`.
    levels: Vec<f32>,
    frame: SpectrumFrame,
    sample_rate: u32,
    channels: u32,
    /// Set when the format changes, so a stale window is not analysed.
    stale: bool,
}

/// Lock-light holder for the analyser state.
pub struct Spectrum {
    inner: Mutex<Inner>,
    enabled: AtomicBool,
    bands: AtomicU32,
    fps: AtomicU32,
    seq: AtomicU64,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

impl Spectrum {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                window: vec![0.0; WINDOW],
                position: 0,
                re: vec![0.0; WINDOW],
                im: vec![0.0; WINDOW],
                levels: vec![0.0; 64],
                frame: SpectrumFrame { seq: 0, bands: Vec::new() },
                sample_rate: 44_100,
                channels: 2,
                stale: true,
            }),
            enabled: AtomicBool::new(false),
            bands: AtomicU32::new(64),
            fps: AtomicU32::new(30),
            seq: AtomicU64::new(0),
        }
    }

    /// The frontend subscribes while a visualiser is on screen.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            let mut inner = self.inner.lock().unwrap();
            inner.position = 0;
            inner.stale = true;
            for level in inner.levels.iter_mut() {
                *level = 0.0;
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Band count to fold the spectrum into, clamped to a sane range.
    pub fn set_bands(&self, bands: usize) {
        let bands = bands.clamp(MIN_BANDS, MAX_BANDS);
        if self.bands.swap(bands as u32, Ordering::Relaxed) as usize != bands {
            let mut inner = self.inner.lock().unwrap();
            inner.levels = vec![0.0; bands];
        }
    }

    pub fn bands(&self) -> usize {
        self.bands.load(Ordering::Relaxed) as usize
    }

    pub fn set_fps(&self, fps: u32) {
        self.fps.store(fps.clamp(MIN_FPS, MAX_FPS), Ordering::Relaxed);
    }

    pub fn fps(&self) -> u32 {
        self.fps.load(Ordering::Relaxed).clamp(MIN_FPS, MAX_FPS)
    }

    /// Analyse interleaved samples decoded at `sample_rate`.
    ///
    /// Called from the decode worker; a contended lock simply drops the samples,
    /// which is invisible to the listener and keeps the worker moving.
    pub fn push(&self, samples: &[f32], channels: u32, sample_rate: u32) {
        if !self.is_enabled() || samples.is_empty() {
            return;
        }
        let channels = channels.max(1) as usize;
        let Ok(mut inner) = self.inner.try_lock() else {
            return;
        };

        if inner.sample_rate != sample_rate || inner.channels != channels as u32 {
            inner.sample_rate = sample_rate;
            inner.channels = channels as u32;
            inner.position = 0;
            inner.stale = true;
        }

        let mut index = 0;
        while index + channels <= samples.len() {
            let mut sum = 0.0;
            for channel in 0..channels {
                sum += samples[index + channel];
            }
            let position = inner.position;
            inner.window[position] = sum / channels as f32;
            inner.position = position + 1;
            index += channels;

            if inner.position == WINDOW {
                inner.position = 0;
                let bands = self.bands.load(Ordering::Relaxed) as usize;
                Self::analyse(&mut inner, bands);
                inner.stale = false;
                self.seq.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// The latest frame, if a new one has been produced since `seq`.
    pub fn frame_if_new(&self, seq: u64) -> Option<SpectrumFrame> {
        let inner = self.inner.lock().unwrap();
        if inner.stale || inner.frame.seq == seq || inner.frame.bands.is_empty() {
            return None;
        }
        Some(inner.frame.clone())
    }

    /// Window, transform, fold into bands, smooth — all in place.
    fn analyse(inner: &mut Inner, bands: usize) {
        let n = WINDOW;
        let rate = inner.sample_rate.max(1) as f32;

        // Hann window, written straight into the real scratch buffer.
        for i in 0..n {
            let taper = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos();
            inner.re[i] = inner.window[i] * taper;
            inner.im[i] = 0.0;
        }
        fft(&mut inner.re, &mut inner.im);

        if inner.levels.len() != bands {
            inner.levels = vec![0.0; bands];
        }
        inner.frame.bands.resize(bands, 0);

        // Logarithmic band edges over the audible range.
        let ratio = HIGH_HZ / LOW_HZ;
        for band in 0..bands {
            let low = LOW_HZ * ratio.powf(band as f32 / bands as f32);
            let high = LOW_HZ * ratio.powf((band + 1) as f32 / bands as f32);
            let first = ((low / rate) * n as f32).floor().max(1.0) as usize;
            let last = (((high / rate) * n as f32).ceil() as usize).min(n / 2).max(first + 1);

            let mut peak = 0.0f32;
            for bin in first..last.min(n / 2) {
                let magnitude = (inner.re[bin] * inner.re[bin] + inner.im[bin] * inner.im[bin]).sqrt();
                if magnitude > peak {
                    peak = magnitude;
                }
            }

            // Magnitude -> decibels -> 0..1, with a reference that keeps normal
            // programme material well inside the range.
            let db = 20.0 * (peak / (n as f32 / 2.0) + 1e-9).log10();
            let value = ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0);

            let level = &mut inner.levels[band];
            let weight = if value > *level { ATTACK } else { RELEASE };
            *level += (value - *level) * weight;
            inner.frame.bands[band] = (*level * 255.0).clamp(0.0, 255.0) as u8;
        }

        inner.frame.seq += 1;
    }
}

/// In-place iterative radix-2 FFT. `im` must be zeroed on entry.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    if n < 2 || !n.is_power_of_two() {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies, doubling the transform length each pass.
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        let (step_re, step_im) = (angle.cos(), angle.sin());
        let half = len / 2;
        let mut start = 0;
        while start < n {
            let (mut tw_re, mut tw_im) = (1.0f32, 0.0f32);
            for k in 0..half {
                let (ur, ui) = (re[start + k], im[start + k]);
                let (xr, xi) = (re[start + k + half], im[start + k + half]);
                let vr = xr * tw_re - xi * tw_im;
                let vi = xr * tw_im + xi * tw_re;
                re[start + k] = ur + vr;
                im[start + k] = ui + vi;
                re[start + k + half] = ur - vr;
                im[start + k + half] = ui - vi;
                let next_re = tw_re * step_re - tw_im * step_im;
                tw_im = tw_re * step_im + tw_im * step_re;
                tw_re = next_re;
            }
            start += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32) -> Vec<f32> {
        (0..WINDOW)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    /// A 1 kHz tone must land near the middle of a 40 Hz - 16 kHz log axis.
    #[test]
    fn a_sine_lands_in_the_expected_band() {
        let spectrum = Spectrum::new();
        spectrum.set_bands(32);
        spectrum.set_enabled(true);
        spectrum.push(&sine(1_000.0, 48_000), 1, 48_000);

        let frame = spectrum.frame_if_new(0).expect("a frame was produced");
        assert_eq!(frame.bands.len(), 32);

        let peak = frame
            .bands
            .iter()
            .enumerate()
            .max_by_key(|(_, value)| **value)
            .map(|(index, _)| index)
            .unwrap();
        // log10(1000/40) / log10(16000/40) * 32 ~= 17
        assert!((15..=20).contains(&peak), "1 kHz peaked in band {peak}");
    }

    /// Band edges above Nyquist must not panic on odd rates.
    #[test]
    fn low_sample_rates_are_handled() {
        let spectrum = Spectrum::new();
        spectrum.set_bands(64);
        spectrum.set_enabled(true);
        // The window counts mono samples, so stereo needs twice the input.
        let mono = sine(440.0, 8_000);
        let mut interleaved = Vec::with_capacity(mono.len() * 2);
        for value in mono {
            interleaved.push(value);
            interleaved.push(value);
        }
        spectrum.push(&interleaved, 2, 8_000);
        assert!(spectrum.frame_if_new(0).is_some());
    }

    /// Silence stays at the floor and the analyser stays allocation-stable.
    #[test]
    fn silence_stays_at_the_floor() {
        let spectrum = Spectrum::new();
        spectrum.set_bands(16);
        spectrum.set_enabled(true);
        let silent = vec![0.0; WINDOW * 2];
        spectrum.push(&silent, 2, 44_100);

        let frame = spectrum.frame_if_new(0).expect("a frame was produced");
        assert!(frame.bands.iter().all(|value| *value == 0), "silence was not flat");
        // Feeding more silence keeps producing frames without panicking.
        spectrum.push(&silent, 2, 44_100);
        assert!(spectrum.frame_if_new(frame.seq).is_some());
    }

    /// Disabled means disabled: no frames, no work.
    #[test]
    fn disabled_produces_nothing() {
        let spectrum = Spectrum::new();
        spectrum.push(&sine(440.0, 44_100), 2, 44_100);
        assert!(spectrum.frame_if_new(0).is_none());
    }
}
