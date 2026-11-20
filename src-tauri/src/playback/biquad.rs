// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! RBJ biquad primitives shared by the crossfade EQ and the graphic EQ.
//!
//! Every filter is normalised by `a0` and processed with a transposed direct
//! form II section, so a band can be skipped sample-for-sample when its gain is
//! effectively zero. Keeping the primitives here means the crossfade's three
//! tone bands and the user's twelve graphic bands use the exact same maths.

use std::f32::consts::PI;

/// Gain change (dB) below which a band is considered unchanged. The dead band
/// is inaudible and avoids recomputing coefficients while a slider settles.
const GAIN_DEADBAND_DB: f32 = 0.01;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BandKind {
    LowShelf,
    Peaking,
    HighShelf,
    HighPass,
}

#[derive(Clone, Copy)]
pub struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    pub fn unity() -> Self {
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

    pub fn for_kind(kind: BandKind, sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        match kind {
            BandKind::LowShelf => Self::low_shelf(sample_rate, freq, gain_db),
            BandKind::Peaking => Self::peaking(sample_rate, freq, q, gain_db),
            BandKind::HighShelf => Self::high_shelf(sample_rate, freq, gain_db),
            BandKind::HighPass => Self::high_pass(sample_rate, freq, q),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    /// Transposed direct form II: one multiply-add chain per sample.
    #[inline]
    pub fn process(&mut self, x: f32, c: &BiquadCoeffs) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// One filter band with independent state per output channel.
pub struct Band {
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
    pub fn new(kind: BandKind, sample_rate: f32, freq: f32, q: f32) -> Self {
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
    pub fn set_gain_db(&mut self, gain_db: f32) {
        if (gain_db - self.gain_db).abs() < GAIN_DEADBAND_DB {
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
    pub fn set_cutoff_hz(&mut self, freq: f32) {
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

    /// Drop the filter memory. Used when a band is bypassed so re-enabling it
    /// starts from silence instead of replaying a stale tail.
    pub fn reset(&mut self) {
        self.state = [BiquadState::default(); 2];
    }

    #[inline]
    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        if !self.active {
            return sample;
        }
        self.state[channel.min(1)].process(sample, &self.coeffs)
    }
}
