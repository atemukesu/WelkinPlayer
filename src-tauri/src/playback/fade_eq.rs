// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Per-deck three-band EQ used by the crossfade.
//!
//! The crossfade used to apply one equal-power gain to each deck, so both
//! tracks kept their full bass for the whole overlap. Two kick drums and two
//! basslines then sum into a muddy low end that no fade length can fix.
//!
//! A bass swap alone is enough for a two-second cut, but a longer blend leaves
//! two complete mixes playing at once: vocals, snares and hi-hats fight across
//! the whole midrange. This module therefore exposes three bands — a low shelf,
//! a presence peak and a high shelf — and the engine hands the dominant band
//! from the outgoing deck to the incoming one as the fade progresses.
//!
//! At 0 dB every biquad is exactly unity, so a normal (non-fading) deck is not
//! touched; during a crossfade the engine drives each band independently.
//! A shelf/peak, unlike `sample - lowpass`, is a clean control: attenuating it
//! really removes that band instead of leaving a phase-shifted residual.
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

/// Centre of the presence peak that carries most vocals and snares.
pub const MID_HZ: f32 = 1_200.0;
/// Width of the presence peak. A moderate Q keeps the cut musical instead of
/// turning it into a resonant notch.
pub const MID_Q: f32 = 0.9;
/// How far the outgoing presence band is dipped when the incoming deck takes
/// over. Subtle on purpose: `FadeEq` should clear space, not hollow the mix.
pub const MID_CUT_DB: f32 = -6.0;

/// Corner of the high shelf.
pub const HIGH_SHELF_HZ: f32 = 6_000.0;
/// How far the outgoing highs are dipped. Cymbals and hats are the second
/// biggest source of long-blend clutter after the vocal band.
pub const HIGH_CUT_DB: f32 = -4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BandKind {
    LowShelf,
    Peaking,
    HighShelf,
    HighPass,
}

#[derive(Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    fn unity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

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

    /// RBJ high-shelf, normalised by `a0`.
    fn high_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let rate = sample_rate.max(1.0);
        let freq = freq.clamp(1.0, rate * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 * 0.5 * std::f32::consts::SQRT_2;
        let beta = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + beta;
        Self {
            b0: a * ((a + 1.0) + (a - 1.0) * cos_w0 + beta) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * cos_w0 - beta) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cos_w0 - beta) / a0,
        }
    }

    /// RBJ peaking EQ, normalised by `a0`.
    fn peaking(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let rate = sample_rate.max(1.0);
        let freq = freq.clamp(1.0, rate * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    /// RBJ second-order high-pass, normalised by `a0`.
    ///
    /// Used by the `filterCut` technique: sweeping it up removes the outgoing
    /// track's whole body instead of beat-matching a tempo that is too far away.
    fn high_pass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let rate = sample_rate.max(1.0);
        let freq = freq.clamp(1.0, rate * 0.45);
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos_w0) * 0.5 / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: (1.0 + cos_w0) * 0.5 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn for_kind(kind: BandKind, sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        match kind {
            BandKind::LowShelf => Self::low_shelf(sample_rate, freq, gain_db),
            BandKind::Peaking => Self::peaking(sample_rate, freq, q, gain_db),
            BandKind::HighShelf => Self::high_shelf(sample_rate, freq, gain_db),
            BandKind::HighPass => Self::high_pass(sample_rate, freq, q),
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

/// One filter band with independent state per output channel.
struct Band {
    kind: BandKind,
    sample_rate: f32,
    freq: f32,
    q: f32,
    coeffs: BiquadCoeffs,
    state: [BiquadState; 2],
    gain_db: f32,
    /// Whether the filter actually does anything. A band at 0 dB (or a
    /// high-pass below the audible range) is skipped sample-for-sample.
    active: bool,
}

impl Band {
    fn new(kind: BandKind, sample_rate: f32, freq: f32, q: f32) -> Self {
        Self {
            kind,
            sample_rate,
            freq,
            q,
            coeffs: BiquadCoeffs::unity(),
            state: [BiquadState::default(); 2],
            gain_db: 0.0,
            active: false,
        }
    }

    /// Set the band gain in dB. Recomputes coefficients only when it changed
    /// enough to be audible, so the audio thread does no needless trig.
    fn set_gain_db(&mut self, gain_db: f32) {
        if (gain_db - self.gain_db).abs() < 0.01 {
            return;
        }
        self.gain_db = gain_db;
        self.active = gain_db.abs() >= 0.001;
        self.coeffs = if self.active {
            BiquadCoeffs::for_kind(self.kind, self.sample_rate, self.freq, self.q, gain_db)
        } else {
            BiquadCoeffs::unity()
        };
    }

    /// Move a high-pass cutoff. Values at or below 20 Hz bypass the band.
    ///
    /// Coefficients are only recomputed once the cutoff moved by ~1 % (with a
    /// 5 Hz floor). The sweep is logarithmic, so a fixed dead-band would either
    /// skip audible steps at the bottom or recompute every few samples at the
    /// top; scaling with the frequency keeps the trig rate roughly constant.
    fn set_cutoff_hz(&mut self, freq: f32) {
        let rate = self.sample_rate.max(1.0);
        let freq = freq.clamp(1.0, rate * 0.45);
        if (freq - self.freq).abs() < (self.freq * 0.01).max(5.0) {
            return;
        }
        self.freq = freq;
        self.active = freq > 20.0;
        self.coeffs = if self.active {
            BiquadCoeffs::for_kind(self.kind, self.sample_rate, self.freq, self.q, self.gain_db)
        } else {
            BiquadCoeffs::unity()
        };
    }

    #[inline]
    fn process(&mut self, sample: f32, channel: usize) -> f32 {
        if !self.active {
            return sample;
        }
        self.state[channel.min(1)].process(sample, &self.coeffs)
    }
}

/// One deck's EQ: three tone bands plus the optional filter-cut high-pass.
/// State is kept per output channel so a stereo signal is filtered
/// independently and the two channels stay identical.
pub struct FadeEq {
    low: Band,
    mid: Band,
    high: Band,
    highpass: Band,
}

impl FadeEq {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            low: Band::new(BandKind::LowShelf, rate, BASS_SHELF_HZ, 1.0),
            mid: Band::new(BandKind::Peaking, rate, MID_HZ, MID_Q),
            high: Band::new(BandKind::HighShelf, rate, HIGH_SHELF_HZ, 1.0),
            highpass: Band::new(
                BandKind::HighPass,
                rate,
                20.0,
                std::f32::consts::FRAC_1_SQRT_2,
            ),
        }
    }

    /// Set all three band gains in dB. The engine updates them once per state
    /// block with the values for the current crossfade position.
    pub fn set_gains_db(&mut self, low_db: f32, mid_db: f32, high_db: f32) {
        self.low.set_gain_db(low_db);
        self.mid.set_gain_db(mid_db);
        self.high.set_gain_db(high_db);
    }

    /// Set the filter-cut high-pass cutoff. `<= 20 Hz` restores a clean path.
    pub fn set_highpass_hz(&mut self, hz: f32) {
        self.highpass.set_cutoff_hz(hz);
    }

    /// Convenience for the bass-only case (tests and any caller that does not
    /// need the presence bands).
    #[cfg(test)]
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.set_gains_db(gain_db, 0.0, 0.0);
    }

    /// Apply the bands to one sample of `channel`.
    ///
    /// Output channels beyond stereo share the right-channel state, which is
    /// fine because the engine only ever mixes mono or stereo.
    #[inline]
    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        let channel = channel.min(1);
        let sample = self.highpass.process(sample, channel);
        let sample = self.low.process(sample, channel);
        let sample = self.mid.process(sample, channel);
        self.high.process(sample, channel)
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

    /// Feed one mono tone through the EQ and return the steady-state response.
    fn response(eq: &mut FadeEq, freq: f32) -> f32 {
        const RATE: f32 = 48_000.0;
        let input = tone(RATE, freq, 9_600);
        let mut output = Vec::new();
        for (index, sample) in input.iter().enumerate() {
            let filtered = eq.process(*sample, 0);
            if index >= 2_000 {
                output.push(filtered);
            }
        }
        db(rms(&output), rms(&input[2_000..]))
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
        let mut bass = FadeEq::new(48_000);
        bass.set_gain_db(BASS_CUT_DB);
        assert!(response(&mut bass, 60.0) < -12.0);

        let mut treble = FadeEq::new(48_000);
        treble.set_gain_db(BASS_CUT_DB);
        assert!(response(&mut treble, 4_000.0).abs() < 0.5);
    }

    #[test]
    fn presence_band_clears_the_vocal_range_only() {
        let mut mid = FadeEq::new(48_000);
        mid.set_gains_db(0.0, MID_CUT_DB, 0.0);
        // The peak sits on vocals/snares and leaves the extreme lows and highs.
        assert!(response(&mut mid, MID_HZ) < -4.0);
        assert!(response(&mut mid, 60.0).abs() < 1.0);
        assert!(response(&mut mid, 12_000.0).abs() < 1.5);
    }

    #[test]
    fn high_shelf_clears_cymbals_only() {
        let mut high = FadeEq::new(48_000);
        high.set_gains_db(0.0, 0.0, HIGH_CUT_DB);
        assert!(response(&mut high, 12_000.0) < -2.5);
        assert!(response(&mut high, 300.0).abs() < 1.0);
    }

    #[test]
    fn stacked_bands_are_transparent_when_all_zero() {
        let mut eq = FadeEq::new(48_000);
        eq.set_gains_db(0.0, 0.0, 0.0);
        let input = tone(48_000.0, 1_000.0, 1_024);
        for (index, sample) in input.iter().enumerate() {
            assert_eq!(eq.process(*sample, index % 2), *sample);
        }
    }

    #[test]
    fn filter_cut_high_pass_removes_the_body_but_keeps_the_top() {
        let mut eq = FadeEq::new(48_000);
        eq.set_highpass_hz(300.0);
        assert!(response(&mut eq, 60.0) < -10.0);
        assert!(response(&mut eq, 4_000.0).abs() < 1.5);
        // Back to the bypass range: a low tone passes untouched again.
        eq.set_highpass_hz(20.0);
        assert!(response(&mut eq, 60.0).abs() < 0.5);
    }
}
