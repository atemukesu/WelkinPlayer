// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Per-deck bass EQ used by the crossfade.
//!
//! The crossfade used to apply one equal-power gain to each deck, so both
//! tracks kept their full bass for the whole overlap. Two kick drums and two
//! basslines then sum into a muddy low end that no fade length can fix.
//!
//! Real DJ mixers solve this with a low-band EQ: the outgoing deck's bass is
//! cut away before the incoming deck's bass is brought in. This module is that
//! EQ, implemented as one RBJ low-shelf biquad per deck and channel. At 0 dB
//! its coefficients are exactly unity, so a normal (non-fading) deck is not
//! touched; during a crossfade the engine drives it down to
//! [`BASS_CUT_DB`] on the outgoing deck and back up on the incoming one.
//!
//! A shelf, unlike `sample - lowpass`, is a clean bass control: attenuating it
//! really removes the low end instead of leaving a phase-shifted residual.
//! The project already vendors `automix`, whose private `EQ3Band` uses the same
//! biquad form, but it is not exposed and is fixed at deck load time, so a
//! small local implementation is simpler than adapting it.

use std::f32::consts::PI;

/// Corner of the bass shelf. Below this the outgoing deck is cut away.
pub const BASS_SHELF_HZ: f32 = 150.0;
/// How far the outgoing bass is cut. Deep enough to remove a kick/bassline,
/// shallow enough that the shelf keeps a natural slope instead of sounding
/// like a filter sweep.
pub const BASS_CUT_DB: f32 = -24.0;

#[derive(Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    /// RBJ audio EQ cookbook low-shelf, normalised by `a0`.
    ///
    /// `slope` is the shelf slope (`S = 1`).
    fn low_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let rate = sample_rate.max(1.0);
        let freq = freq.clamp(1.0, rate * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 * 0.5 * std::f32::consts::SQRT_2;
        let beta = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + beta;
        Self {
            b0: a * ((a + 1.0) - (a - 1.0) * cos_w0 + beta) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * cos_w0 - beta) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cos_w0 - beta) / a0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    /// Transposed direct form II: one multiply-add chain per sample.
    #[inline]
    fn process(&mut self, x: f32, c: &BiquadCoeffs) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// One deck's bass shelf. State is kept per output channel so a stereo signal
/// is filtered independently and the two channels stay identical.
pub struct FadeEq {
    sample_rate: f32,
    coeffs: BiquadCoeffs,
    state: [BiquadState; 2],
    gain_db: f32,
}

impl FadeEq {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            sample_rate: rate,
            coeffs: BiquadCoeffs::low_shelf(rate, BASS_SHELF_HZ, 0.0),
            state: [BiquadState::default(); 2],
            gain_db: 0.0,
        }
    }

    /// Set the shelf gain in dB. Recomputes coefficients only when it changed
    /// enough to be audible, so the audio thread does no needless trig.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        if (gain_db - self.gain_db).abs() < 0.01 {
            return;
        }
        self.gain_db = gain_db;
        self.coeffs = BiquadCoeffs::low_shelf(self.sample_rate, BASS_SHELF_HZ, gain_db);
    }

    /// Apply the shelf to one sample of `channel`.
    ///
    /// Output channels beyond stereo share the right-channel state, which is
    /// fine because the engine only ever mixes mono or stereo.
    #[inline]
    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        if self.gain_db.abs() < 0.001 {
            return sample;
        }
        self.state[channel.min(1)].process(sample, &self.coeffs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(sample_rate: f32, freq: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn db(a: f32, b: f32) -> f32 {
        20.0 * (a / b).log10()
    }

    #[test]
    fn unity_gain_is_transparent() {
        let mut eq = FadeEq::new(48_000);
        for (index, sample) in tone(48_000.0, 440.0, 480).iter().enumerate() {
            assert_eq!(eq.process(*sample, index % 2), *sample);
        }
    }

    #[test]
    fn cut_removes_bass_but_keeps_treble() {
        let mut eq = FadeEq::new(48_000);
        eq.set_gain_db(BASS_CUT_DB);
        let bass = tone(48_000.0, 60.0, 9_600);
        let treble = tone(48_000.0, 4_000.0, 9_600);
        let mut bass_out = Vec::new();
        let mut treble_out = Vec::new();
        for i in 0..9_600 {
            let low = eq.process(bass[i], 0);
            let high = eq.process(treble[i], 1);
            // Skip the filter's start-up transient before measuring.
            if i >= 2_000 {
                bass_out.push(low);
                treble_out.push(high);
            }
        }
        assert!(db(rms(&bass_out), rms(&bass[2_000..])) < -12.0);
        assert!(db(rms(&treble_out), rms(&treble[2_000..])).abs() < 0.5);
    }
}
