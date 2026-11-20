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

use crate::playback::biquad::{Band, BandKind};

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
    use std::f32::consts::PI;

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
