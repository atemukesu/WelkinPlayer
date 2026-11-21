// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Look-ahead peak limiter at the end of the playback chain.
//!
//! The limiter sits after the graphic EQ, the effects box and the volume gain,
//! so a bass boost can no longer push the output past 0 dBFS. It is the last
//! stage before the crossfade bus headroom and the device:
//!
//! ```text
//! sample → fade EQ → graphic EQ → effects → volume → limiter → bus → device
//! ```
//!
//! A delay line holds `lookahead_ms` of audio while a monotonic queue tracks the
//! *minimum* gain required by every frame in that window. The gain therefore
//! starts falling before a peak reaches the output, which keeps the attack
//! transient instead of clipping it. Stereo is linked by taking the peak across
//! all channels of a frame, so the stereo image never wanders.
//!
//! The ceiling is always the threshold: the product spec asks for a single
//! control, and `threshold / peak` gain plus a final clamp guarantees the output
//! stays below it. True-peak detection (2× oversampling) is not implemented yet;
//! the default `-1 dBFS` threshold reserves room for the inter-sample peaks of a
//! normal master.
//!
//! Everything is preallocated: after construction the realtime callback only
//! pushes/pops the gain queue and walks the delay line, never allocating.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Accepted threshold range. `0 dBFS` is deliberately not reachable: at least a
/// little headroom keeps inter-sample peaks below full scale.
pub const MIN_THRESHOLD_DB: f32 = -12.0;
pub const MAX_THRESHOLD_DB: f32 = -0.1;
pub const MIN_RELEASE_MS: f32 = 20.0;
pub const MAX_RELEASE_MS: f32 = 250.0;
pub const MIN_LOOKAHEAD_MS: f32 = 1.0;
pub const MAX_LOOKAHEAD_MS: f32 = 10.0;
/// Attack is derived from the look-ahead: the gain is already moving before the
/// peak arrives, so the smoothing only needs to remove the corner.
const ATTACK_RATIO: f32 = 0.8;

/// Limiter settings as seen by the frontend and stored in the shared atomics.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LimiterSettings {
    pub enabled: bool,
    pub threshold_db: f32,
    pub release_ms: f32,
    pub lookahead_ms: f32,
}

impl Default for LimiterSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_db: -1.0,
            release_ms: 80.0,
            lookahead_ms: 5.0,
        }
    }
}

impl LimiterSettings {
    /// Clamp every field into the range the DSP accepts. Both the command
    /// handler and the engine run this, so a hand-written invoke cannot push a
    /// threshold of 0 dB or a 1 ms look-ahead into the audio thread.
    pub fn sanitized(mut self) -> Self {
        self.threshold_db = if self.threshold_db.is_finite() {
            self.threshold_db.clamp(MIN_THRESHOLD_DB, MAX_THRESHOLD_DB)
        } else {
            -1.0
        };
        self.release_ms = if self.release_ms.is_finite() {
            self.release_ms.clamp(MIN_RELEASE_MS, MAX_RELEASE_MS)
        } else {
            80.0
        };
        self.lookahead_ms = if self.lookahead_ms.is_finite() {
            self.lookahead_ms.clamp(MIN_LOOKAHEAD_MS, MAX_LOOKAHEAD_MS)
        } else {
            5.0
        };
        self
    }
}

/// Monotonic minimum queue over recent frames.
///
/// Stores `(frame index, required gain)` in non-decreasing gain order, so the
/// front is always the smallest gain in the look-ahead window. Every entry is
/// pushed and popped at most once, which keeps the per-frame cost O(1).
struct GainWindow {
    entries: VecDeque<(u64, f32)>,
}

impl GainWindow {
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    /// Add the required gain of `index`, dropping dominated (larger) entries.
    fn push(&mut self, index: u64, gain: f32) {
        while let Some(&(_, back)) = self.entries.back() {
            if back >= gain {
                self.entries.pop_back();
            } else {
                break;
            }
        }
        self.entries.push_back((index, gain));
    }

    /// Drop entries older than `oldest`.
    fn evict_before(&mut self, oldest: u64) {
        while let Some(&(index, _)) = self.entries.front() {
            if index < oldest {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }

    fn min(&self) -> f32 {
        self.entries.front().map(|&(_, gain)| gain).unwrap_or(1.0)
    }
}

/// One look-ahead limiter instance (one per deck).
pub struct Limiter {
    channels: usize,
    rate: f32,
    enabled: bool,
    /// Linear headroom; also used as the hard ceiling.
    ceiling: f32,
    lookahead_frames: usize,
    /// Interleaved delay line. It is sized for the longest look-ahead once, at
    /// construction, so changing the look-ahead while playing never allocates
    /// on the audio thread.
    delay: Vec<f32>,
    write: usize,
    window: GainWindow,
    /// Frame counter for the gain window.
    frame_index: u64,
    /// Peak of the frame currently being collected.
    inbound_peak: f32,
    /// Samples of the frame currently being collected, one per channel.
    collected: Vec<f32>,
    /// Gain applied to the previous frame and the value it is heading to.
    gain: f32,
    target_gain: f32,
    attack_coef: f32,
    release_coef: f32,
    /// One frame of pending output, returned while the next frame is collected.
    out_frame: Vec<f32>,
    meter_in: f32,
    meter_out: f32,
}

impl Limiter {
    pub fn new(rate: u32, channels: u16) -> Self {
        let channels = (channels as usize).max(1);
        let rate = rate.max(8_000) as f32;
        let active_frames = lookahead_frames(5.0, rate);
        // Reserve the longest look-ahead up front. `configure` only ever moves
        // the read offset inside this buffer, so a knob turn is allocation-free.
        let max_frames = lookahead_frames(MAX_LOOKAHEAD_MS, rate);
        let mut limiter = Self {
            channels,
            rate,
            enabled: true,
            ceiling: 10f32.powf(-1.0 / 20.0),
            lookahead_frames: active_frames,
            delay: vec![0.0; (max_frames + 1) * channels],
            write: 0,
            window: GainWindow::new(max_frames + 1),
            frame_index: 0,
            inbound_peak: 0.0,
            collected: vec![0.0; channels],
            gain: 1.0,
            target_gain: 1.0,
            attack_coef: 1.0,
            release_coef: 0.1,
            out_frame: vec![0.0; channels],
            meter_in: 0.0,
            meter_out: 0.0,
        };
        limiter.set_times(5.0, 80.0);
        limiter
    }

    fn set_times(&mut self, lookahead_ms: f32, release_ms: f32) {
        let attack_ms = (lookahead_ms * ATTACK_RATIO).max(0.2);
        self.attack_coef = smoothing_coef(attack_ms, self.rate);
        self.release_coef = smoothing_coef(release_ms, self.rate);
    }

    /// Apply the current settings. Called once per state block; only a change
    /// in look-ahead or the enable flag resets the DSP state.
    pub fn configure(&mut self, settings: LimiterSettings) {
        let settings = settings.sanitized();
        if settings.enabled != self.enabled {
            // Toggling the box must not replay stale samples or a stale gain.
            self.reset();
            self.enabled = settings.enabled;
        }
        let frames = lookahead_frames(settings.lookahead_ms, self.rate);
        if frames != self.lookahead_frames {
            // The buffer already covers the longest look-ahead; only the read
            // offset changes, so clear the stale tail and keep going.
            self.lookahead_frames = frames;
            self.reset();
        }
        self.ceiling = 10f32.powf(settings.threshold_db / 20.0);
        self.set_times(settings.lookahead_ms, settings.release_ms);
    }

    /// Clear the delay line and gain envelope. Used on seek, track change and
    /// output-rate changes so the previous position never bleeds into a new one.
    pub fn reset(&mut self) {
        self.delay.fill(0.0);
        self.write = 0;
        self.window.clear();
        self.frame_index = 0;
        self.inbound_peak = 0.0;
        self.collected.fill(0.0);
        self.gain = 1.0;
        self.target_gain = 1.0;
        self.out_frame.fill(0.0);
        self.meter_in = 0.0;
        self.meter_out = 0.0;
    }

    /// Peak of the input and output since the last call, for the UI graph.
    pub fn take_meter(&mut self) -> (f32, f32) {
        let input = self.meter_in;
        let output = self.meter_out;
        self.meter_in = 0.0;
        self.meter_out = 0.0;
        (input, output)
    }

    /// Current gain reduction in dB (0 when the limiter is not engaging).
    pub fn gain_reduction_db(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        (20.0 * self.gain.max(1.0e-4).log10()).clamp(-60.0, 0.0)
    }

    /// Process one interleaved sample.
    ///
    /// The caller must pass channels in order (`0 .. channels`) for every frame,
    /// which is exactly how [`crate::playback::engine::RingSource`] iterates.
    #[inline]
    pub fn process(&mut self, channel: usize, sample: f32) -> f32 {
        if !self.enabled {
            let peak = sample.abs();
            self.meter_in = self.meter_in.max(peak);
            self.meter_out = self.meter_out.max(peak);
            return sample;
        }
        let channel = channel.min(self.channels - 1);
        // The value produced for this frame was computed when the previous
        // frame completed, so the output is always one frame behind the input.
        let output = self.out_frame[channel];
        let peak = sample.abs();
        self.collected[channel] = sample;
        self.inbound_peak = self.inbound_peak.max(peak);
        if channel + 1 == self.channels {
            self.finish_frame();
        }
        output
    }

    /// Close the frame that was just collected and prepare the next output
    /// frame. Runs once per frame on the audio thread.
    fn finish_frame(&mut self) {
        let frame_index = self.frame_index;
        let required = required_gain(self.inbound_peak, self.ceiling);
        // The window spans the look-ahead plus the frame being delayed, so the
        // gain starts moving before the peak reaches the output.
        self.window.push(frame_index, required);
        self.window
            .evict_before(frame_index.saturating_sub(self.lookahead_frames as u64));
        self.target_gain = self.window.min();

        let coefficient = if self.target_gain < self.gain {
            self.attack_coef
        } else {
            self.release_coef
        };
        let start_gain = self.gain;
        let target = start_gain + (self.target_gain - start_gain) * coefficient;
        let channels = self.channels;
        let capacity = self.delay.len();
        let delay_samples = self.lookahead_frames * channels;
        let mut frame_out_peak = 0.0f32;
        for index in 0..channels {
            let read = (self.write + capacity - delay_samples) % capacity;
            let delayed = self.delay[read];
            // Linear per-sample interpolation across the frame removes any
            // zipper noise from the per-frame gain update.
            let mix = (index + 1) as f32 / channels as f32;
            let gain = start_gain + (target - start_gain) * mix;
            let out = (delayed * gain).clamp(-self.ceiling, self.ceiling);
            self.out_frame[index] = out;
            frame_out_peak = frame_out_peak.max(out.abs());
            self.delay[self.write] = self.collected[index];
            self.write = (self.write + 1) % capacity;
        }
        self.gain = target;
        self.meter_in = self.meter_in.max(self.inbound_peak);
        self.meter_out = self.meter_out.max(frame_out_peak);
        self.inbound_peak = 0.0;
        self.frame_index = frame_index + 1;
    }
}

/// Convert a time constant to a one-pole coefficient at `rate`.
fn smoothing_coef(ms: f32, rate: f32) -> f32 {
    let samples = (ms * 0.001 * rate).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

/// Look-ahead length in frames, at least one and at most `MAX_LOOKAHEAD_MS`.
fn lookahead_frames(ms: f32, rate: f32) -> usize {
    let frames = (ms.clamp(MIN_LOOKAHEAD_MS, MAX_LOOKAHEAD_MS) * 0.001 * rate).round();
    (frames as usize).max(1)
}

/// Gain that keeps `peak` under `ceiling`.
#[inline]
fn required_gain(peak: f32, ceiling: f32) -> f32 {
    if peak > ceiling && peak > 0.0 {
        (ceiling / peak).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(threshold_db: f32) -> LimiterSettings {
        LimiterSettings {
            enabled: true,
            threshold_db,
            release_ms: 80.0,
            lookahead_ms: 5.0,
        }
    }

    /// Feed `frames` interleaved stereo samples through the limiter.
    fn run(limiter: &mut Limiter, samples: &[(f32, f32)]) -> Vec<(f32, f32)> {
        let mut out = Vec::with_capacity(samples.len());
        for &(left, right) in samples {
            let l = limiter.process(0, left);
            let r = limiter.process(1, right);
            out.push((l, r));
        }
        out
    }

    #[test]
    fn quiet_material_is_not_limited() {
        let mut limiter = Limiter::new(44_100, 2);
        limiter.configure(settings(-1.0));
        let input: Vec<(f32, f32)> = (0..4410)
            .map(|index| {
                let phase = index as f32 / 44_100.0 * 440.0 * std::f32::consts::TAU;
                (phase.sin() * 0.1, phase.sin() * 0.1)
            })
            .collect();
        let out = run(&mut limiter, &input);
        // Ignore the look-ahead warm-up, then compare peaks.
        let peak = out[300..]
            .iter()
            .fold(0.0f32, |peak, (left, _)| peak.max(left.abs()));
        assert!(peak <= 0.105, "quiet signal gained: {peak}");
        assert!((limiter.gain_reduction_db()).abs() < 0.01);
    }

    #[test]
    fn hot_material_is_kept_below_the_ceiling() {
        let mut limiter = Limiter::new(44_100, 2);
        limiter.configure(settings(-1.0));
        let ceiling = 10f32.powf(-1.0 / 20.0);
        // A loud, steady 220 Hz tone at +6 dBFS.
        let input: Vec<(f32, f32)> = (0..44_100)
            .map(|index| {
                let phase = index as f32 / 44_100.0 * 220.0 * std::f32::consts::TAU;
                let value = phase.sin() * 2.0;
                (value, value)
            })
            .collect();
        let out = run(&mut limiter, &input);
        let peak = out[2_000..]
            .iter()
            .fold(0.0f32, |peak, (left, _)| peak.max(left.abs()));
        assert!(peak <= ceiling + 1.0e-5, "output {peak} exceeded {ceiling}");
        assert!(limiter.gain_reduction_db() < -3.0);
    }

    #[test]
    fn stereo_link_applies_one_gain_to_both_channels() {
        let mut limiter = Limiter::new(48_000, 2);
        limiter.configure(settings(-1.0));
        // Left channel hot, right channel quiet. The linked gain must pull the
        // right channel down by the same amount, preserving the image.
        let mut out = Vec::new();
        for index in 0..48_000 {
            let phase = index as f32 / 48_000.0 * 100.0 * std::f32::consts::TAU;
            let left = phase.sin() * 1.5;
            let right = phase.sin() * 0.1;
            out.push((limiter.process(0, left), limiter.process(1, right)));
        }
        let tail = &out[5_000..];
        let left_peak = tail.iter().fold(0.0f32, |p, (l, _)| p.max(l.abs()));
        let right_peak = tail.iter().fold(0.0f32, |p, (_, r)| p.max(r.abs()));
        let ratio = right_peak / left_peak;
        assert!((ratio - 0.1 / 1.5).abs() < 0.02, "image moved: {ratio}");
    }

    #[test]
    fn reset_clears_the_delay_line() {
        let mut limiter = Limiter::new(44_100, 2);
        limiter.configure(settings(-1.0));
        let _ = run(&mut limiter, &[(1.5, 1.5); 2_000]);
        limiter.reset();
        let out = run(&mut limiter, &[(0.0, 0.0); 2_000]);
        let peak = out.iter().fold(0.0f32, |p, (l, _)| p.max(l.abs()));
        assert!(peak < 1.0e-6, "stale audio after reset: {peak}");
    }
}
