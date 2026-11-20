// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! LFO-modulated delay shared by the chorus and the flanger.
//!
//! The two effects differ only in tuning: the chorus uses a longer base delay
//! (about 15 ms) with no feedback, while the flanger sits much closer to the
//! dry signal (about 2 ms) and feeds a little of the output back. Keeping one
//! implementation means the realtime path has a single allocation pattern and a
//! single fractional-read code path to reason about.

use std::f32::consts::TAU;

use crate::playback::delay::DelayLine;

/// Feedback is capped so the flanger stays a comb filter, not a runaway loop.
pub const MAX_FEEDBACK: f32 = 0.85;

pub struct ModDelay {
    lines: Vec<DelayLine>,
    /// One-pole low-pass state for the feedback path, per channel.
    damp: Vec<f32>,
    sample_rate: f32,
    channels: usize,
    base_samples: f32,
    max_depth_samples: f32,
    rate_hz: f32,
    phase: f32,
    enabled: bool,
    target_depth: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
}

impl ModDelay {
    /// `base_ms` is the unmodulated tap, `max_depth_ms` bounds the LFO swing and
    /// must be smaller than `base_ms` so the tap never crosses zero.
    pub fn new(
        sample_rate: u32,
        channels: u16,
        base_ms: f32,
        max_depth_ms: f32,
        rate_hz: f32,
    ) -> Self {
        let rate = sample_rate.max(1) as f32;
        let channels = channels.max(1) as usize;
        let max_samples = ((base_ms + max_depth_ms) * 0.001 * rate) as usize + 4;
        Self {
            lines: (0..channels).map(|_| DelayLine::new(max_samples)).collect(),
            damp: vec![0.0; channels],
            sample_rate: rate,
            channels,
            base_samples: base_ms * 0.001 * rate,
            max_depth_samples: max_depth_ms * 0.001 * rate,
            rate_hz: rate_hz.max(0.01),
            phase: 0.0,
            enabled: false,
            target_depth: 0.0,
            depth: 0.0,
            feedback: 0.0,
            mix: 0.0,
        }
    }

    /// `depth` is normalised: 0 is no modulation, 1 the full LFO swing.
    pub fn configure(&mut self, enabled: bool, depth: f32, feedback: f32, mix: f32) {
        if self.enabled && !enabled {
            for line in &mut self.lines {
                line.clear();
            }
            self.damp.fill(0.0);
            self.phase = 0.0;
            self.depth = 0.0;
        }
        let was_enabled = self.enabled;
        self.enabled = enabled;
        self.target_depth = if depth.is_finite() {
            depth.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !was_enabled && enabled {
            // Start at the requested depth instead of ramping up from silence.
            self.depth = self.target_depth;
        }
        self.feedback = if feedback.is_finite() {
            feedback.clamp(0.0, MAX_FEEDBACK)
        } else {
            0.0
        };
        self.mix = if mix.is_finite() {
            mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn process(&mut self, input: f32, channel: usize) -> f32 {
        if !self.enabled {
            return input;
        }
        let index = channel % self.channels;
        // Halve the phase offset per channel so stereo widens instead of
        // modulating both sides in lockstep.
        let lfo = (TAU * (self.phase + 0.5 * index as f32)).sin();
        let delay = self.base_samples + self.depth * self.max_depth_samples * lfo;
        let delayed = self.lines[index].read(delay.max(1.0));
        self.damp[index] += (delayed - self.damp[index]) * 0.35;
        self.lines[index].write(input + self.damp[index] * self.feedback);
        // The LFO only advances once per frame (after the last channel).
        if index + 1 == self.channels {
            self.phase = (self.phase + self.rate_hz / self.sample_rate).fract();
            self.depth += (self.target_depth - self.depth) * 0.001;
        }
        input * (1.0 - self.mix) + delayed * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_mod_delay_is_a_passthrough() {
        let mut chorus = ModDelay::new(48_000, 2, 15.0, 8.0, 0.6);
        for sample in [-0.25, 0.0, 0.75] {
            assert_eq!(chorus.process(sample, 1), sample);
        }
    }

    #[test]
    fn modulation_stays_bounded_with_feedback() {
        let mut flanger = ModDelay::new(48_000, 2, 2.5, 2.0, 0.25);
        flanger.configure(true, 1.0, MAX_FEEDBACK, 1.0);
        let mut peak = 0.0f32;
        for index in 0..200_000 {
            let channel = index % 2;
            let input = ((index as f32) * 0.02).sin() * 0.5;
            let output = flanger.process(input, channel);
            assert!(output.is_finite());
            peak = peak.max(output.abs());
        }
        assert!(peak <= 12.0, "flanger ran away: {peak}");
    }

    #[test]
    fn mix_zero_keeps_the_dry_signal() {
        let mut chorus = ModDelay::new(48_000, 1, 15.0, 8.0, 0.6);
        chorus.configure(true, 0.8, 0.0, 0.0);
        for index in 0..2_000 {
            let input = ((index as f32) * 0.01).sin() * 0.3;
            assert!((chorus.process(input, 0) - input).abs() < 1e-6);
        }
    }
}
