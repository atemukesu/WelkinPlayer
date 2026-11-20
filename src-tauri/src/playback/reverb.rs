// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Stereo algorithmic reverb based on Freeverb's comb/allpass topology.
//!
//! Each channel keeps its own set of damped comb filters and series allpasses,
//! with the classic 23-sample stereo spread. Processing per channel (instead of
//! summing to mono first) lets the existing sample-by-sample hook stay
//! unchanged; the result is the same wide, dense tail.
//!
//! All buffers are allocated in [`Reverb::new`], so [`Reverb::configure`] only
//! writes coefficients and never allocates on the audio thread.

/// Freeverb comb lengths at 44.1 kHz, low to high.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
/// Freeverb allpass lengths at 44.1 kHz.
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
/// Offset applied to the right channel's delay lengths.
const STEREO_SPREAD: usize = 23;
/// Input scaling from the original Freeverb.
const INPUT_GAIN: f32 = 0.015;
/// Wet output scaling, chosen so a 25 % mix sits behind the dry signal.
const WET_SCALE: f32 = 3.0;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl Comb {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(8)],
            index: 0,
            store: 0.0,
            feedback: 0.0,
            damp1: 0.0,
            damp2: 1.0,
        }
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.store = 0.0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.store = output * self.damp2 + self.store * self.damp1;
        // Flush denormals so a long quiet tail does not burn CPU.
        if self.store.abs() < 1e-20 {
            self.store = 0.0;
        }
        self.buffer[self.index] = input + self.store * self.feedback;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        output
    }
}

struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}

impl Allpass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(8)],
            index: 0,
        }
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = -input + buffered;
        let value = input + buffered * 0.5;
        self.buffer[self.index] = if value.abs() < 1e-20 { 0.0 } else { value };
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        output
    }
}

pub struct Reverb {
    /// `channels * 8` combs, grouped per channel.
    combs: Vec<Comb>,
    /// `channels * 4` allpasses, grouped per channel.
    allpasses: Vec<Allpass>,
    channels: usize,
    enabled: bool,
    size: f32,
    damping: f32,
    mix: f32,
}

impl Reverb {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let channels = channels.max(1) as usize;
        let scale = sample_rate.max(1) as f32 / 44_100.0;
        let spread = |channel: usize| (channel % 2) * STEREO_SPREAD;
        let tuned = |length: usize, channel: usize| {
            ((length + spread(channel)) as f32 * scale).max(8.0) as usize
        };
        let combs = (0..channels)
            .flat_map(|channel| {
                COMB_TUNING
                    .iter()
                    .map(move |&length| Comb::new(tuned(length, channel)))
            })
            .collect();
        let allpasses = (0..channels)
            .flat_map(|channel| {
                ALLPASS_TUNING
                    .iter()
                    .map(move |&length| Allpass::new(tuned(length, channel)))
            })
            .collect();
        Self {
            combs,
            allpasses,
            channels,
            enabled: false,
            size: 0.5,
            damping: 0.5,
            mix: 0.0,
        }
    }

    /// `size`, `damping` and `mix` are normalised to 0..1.
    pub fn configure(&mut self, enabled: bool, size: f32, damping: f32, mix: f32) {
        if self.enabled && !enabled {
            for comb in &mut self.combs {
                comb.clear();
            }
            for allpass in &mut self.allpasses {
                allpass.clear();
            }
        }
        self.enabled = enabled;
        self.size = if size.is_finite() {
            size.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.damping = if damping.is_finite() {
            damping.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.mix = if mix.is_finite() {
            mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let feedback = 0.7 + self.size * 0.28;
        let damp1 = self.damping * 0.4;
        for comb in &mut self.combs {
            comb.feedback = feedback;
            comb.damp1 = damp1;
            comb.damp2 = 1.0 - damp1;
        }
    }

    pub fn process(&mut self, input: f32, channel: usize) -> f32 {
        if !self.enabled {
            return input;
        }
        let index = channel % self.channels;
        let scaled = input * INPUT_GAIN;
        let mut wet = 0.0;
        let comb_base = index * COMB_TUNING.len();
        for comb in &mut self.combs[comb_base..comb_base + COMB_TUNING.len()] {
            wet += comb.process(scaled);
        }
        let allpass_base = index * ALLPASS_TUNING.len();
        for allpass in &mut self.allpasses[allpass_base..allpass_base + ALLPASS_TUNING.len()] {
            wet = allpass.process(wet);
        }
        input * (1.0 - self.mix) + wet * WET_SCALE * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_reverb_is_a_passthrough() {
        let mut reverb = Reverb::new(48_000, 2);
        for sample in [-0.5, 0.0, 0.5] {
            assert_eq!(reverb.process(sample, 0), sample);
        }
    }

    #[test]
    fn impulse_produces_a_bounded_tail() {
        let mut reverb = Reverb::new(48_000, 2);
        reverb.configure(true, 0.8, 0.5, 1.0);
        let mut energy = 0.0f32;
        for index in 0..96_000 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let left = reverb.process(input, 0);
            let right = reverb.process(input, 1);
            assert!(left.is_finite() && right.is_finite());
            energy += left * left + right * right;
            assert!(left.abs() < 4.0 && right.abs() < 4.0);
        }
        // The tail has real energy long after the impulse.
        assert!(energy > 0.01, "tail was silent: {energy}");
    }

    #[test]
    fn clearing_on_disable_drops_the_tail() {
        let mut reverb = Reverb::new(48_000, 1);
        reverb.configure(true, 0.9, 0.2, 1.0);
        for _ in 0..2_000 {
            reverb.process(1.0, 0);
        }
        reverb.configure(false, 0.9, 0.2, 1.0);
        reverb.configure(true, 0.9, 0.2, 1.0);
        assert!(reverb.process(0.0, 0).abs() < 1e-6);
    }
}
