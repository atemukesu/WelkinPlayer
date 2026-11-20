// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Twelve-band graphic equaliser for the whole output bus.
//!
//! Unlike [`crate::playback::fade_eq`], which only acts while two decks overlap,
//! this EQ is part of normal playback: the user shapes the sound once and both
//! the active deck and a crossfade's incoming deck pass through the same bands.
//!
//! The bands are RBJ peaking filters spaced roughly one octave apart, so a
//! slider really emphasises (or removes) its neighbourhood. A band at 0 dB is
//! skipped sample-for-sample, which keeps a flat EQ at zero cost on the realtime
//! audio thread. Gain changes are applied per state block by the caller, exactly
//! like the crossfade bands, so no allocation or locking happens in the callback.

use serde::Serialize;

use crate::playback::biquad::{Band, BandKind};

/// Number of graphic bands. The frontend mirrors this list.
pub const BAND_COUNT: usize = 12;

/// Centre frequencies, low to high. Roughly one octave apart.
pub const BANDS_HZ: [f32; BAND_COUNT] = [
    31.0, 63.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 12_000.0, 16_000.0,
    20_000.0,
];

/// Widest boost/cut a single band accepts, in dB.
pub const MAX_GAIN_DB: f32 = 12.0;

/// Widest pre-amp attenuation/boost, in dB. Negative values leave headroom for
/// boosted bands so a hot preset does not clip the output.
pub const MIN_PREAMP_DB: f32 = -24.0;
pub const MAX_PREAMP_DB: f32 = 12.0;

/// Bandwidth of each peaking section. `1.4` gives a smooth overlap between
/// neighbouring octave bands without turning the curve into a comb.
const BAND_Q: f32 = 1.4;

/// Band gains (dB) and pre-amp as seen by the frontend after a reload.
#[derive(Clone, Debug, Serialize)]
pub struct EqualizerState {
    pub enabled: bool,
    pub gains: Vec<f32>,
    pub preamp: f32,
}

pub struct GraphicEq {
    enabled: bool,
    preamp_db: f32,
    preamp_gain: f32,
    bands: [Band; BAND_COUNT],
}

impl GraphicEq {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            enabled: false,
            preamp_db: 0.0,
            preamp_gain: 1.0,
            bands: std::array::from_fn(|index| {
                Band::new(BandKind::Peaking, rate, BANDS_HZ[index], BAND_Q)
            }),
        }
    }

    /// Apply the current settings. Called once per state block on the audio
    /// thread; `Band::set_gain_db` ignores sub-dead-band changes, so a settled
    /// slider costs nothing.
    pub fn configure(&mut self, enabled: bool, gains: &[f32], preamp_db: f32) {
        if self.enabled && !enabled {
            // Drop the filter memory so re-enabling starts from silence instead
            // of replaying a stale tail from the last time it was on.
            for band in &mut self.bands {
                band.reset();
            }
        }
        self.enabled = enabled;
        let preamp_db = if preamp_db.is_finite() {
            preamp_db.clamp(MIN_PREAMP_DB, MAX_PREAMP_DB)
        } else {
            0.0
        };
        if (preamp_db - self.preamp_db).abs() >= 0.01 {
            self.preamp_db = preamp_db;
            self.preamp_gain = 10f32.powf(preamp_db / 20.0);
        }
        for (index, band) in self.bands.iter_mut().enumerate() {
            let gain = gains
                .get(index)
                .copied()
                .filter(|value| value.is_finite())
                .unwrap_or(0.0)
                .clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
            band.set_gain_db(gain);
        }
    }

    /// Filter one sample of `channel`. A bypassed or flat EQ returns the input
    /// unchanged.
    #[inline]
    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        if !self.enabled {
            return sample;
        }
        let mut value = sample;
        for band in &mut self.bands {
            value = band.process(value, channel);
        }
        value * self.preamp_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const RATE: f32 = 48_000.0;

    fn tone(freq: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| (2.0 * PI * freq * index as f32 / RATE).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn db(a: f32, b: f32) -> f32 {
        20.0 * (a / b).log10()
    }

    fn response(eq: &mut GraphicEq, freq: f32) -> f32 {
        let input = tone(freq, 9_600);
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
    fn flat_and_enabled_is_transparent() {
        let mut eq = GraphicEq::new(48_000);
        eq.configure(true, &[0.0; BAND_COUNT], 0.0);
        for (index, sample) in tone(1_000.0, 1_024).iter().enumerate() {
            assert_eq!(eq.process(*sample, index % 2), *sample);
        }
    }

    #[test]
    fn disabled_bypasses_even_with_boosted_gains() {
        let mut eq = GraphicEq::new(48_000);
        eq.configure(false, &[MAX_GAIN_DB; BAND_COUNT], 0.0);
        for (index, sample) in tone(1_000.0, 512).iter().enumerate() {
            assert_eq!(eq.process(*sample, index % 2), *sample);
        }
    }

    #[test]
    fn boosting_one_band_only_lifts_its_neighbourhood() {
        let mut gains = [0.0; BAND_COUNT];
        // Index 5 is the 1 kHz band.
        gains[5] = 12.0;
        let mut eq = GraphicEq::new(48_000);
        eq.configure(true, &gains, 0.0);
        assert!(response(&mut eq, 1_000.0) > 6.0);

        let mut low = GraphicEq::new(48_000);
        low.configure(true, &gains, 0.0);
        assert!(response(&mut low, 60.0).abs() < 1.5);
    }

    #[test]
    fn cutting_one_band_only_dips_its_neighbourhood() {
        let mut gains = [0.0; BAND_COUNT];
        // Index 8 is the 8 kHz band.
        gains[8] = -12.0;
        let mut eq = GraphicEq::new(48_000);
        eq.configure(true, &gains, 0.0);
        assert!(response(&mut eq, 8_000.0) < -5.0);

        let mut low = GraphicEq::new(48_000);
        low.configure(true, &gains, 0.0);
        assert!(response(&mut low, 250.0).abs() < 1.5);
    }

    #[test]
    fn preamp_scales_the_whole_bus() {
        let mut eq = GraphicEq::new(48_000);
        eq.configure(true, &[0.0; BAND_COUNT], -6.0);
        let level = response(&mut eq, 1_000.0);
        assert!((level + 6.0).abs() < 0.5, "expected -6 dB, got {level}");
    }

    #[test]
    fn gains_are_clamped_to_the_supported_range() {
        let mut eq = GraphicEq::new(48_000);
        eq.configure(true, &[1_000.0; BAND_COUNT], 0.0);
        // A clamp keeps the filter stable; just verify it still produces a
        // finite, boosted response instead of exploding.
        let level = response(&mut eq, 1_000.0);
        assert!(level.is_finite());
        assert!(level > 6.0);
    }
}
