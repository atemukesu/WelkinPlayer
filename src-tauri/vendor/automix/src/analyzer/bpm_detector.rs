//! AutoMix Engine - BPM Detector

use crate::types::{AudioBuffer, Result};
use std::f32::consts::PI;

#[derive(Default)]
pub struct BPMDetector;

impl BPMDetector {
    pub fn detect(&self, audio: &AudioBuffer) -> Result<f32> {
        if audio.samples.is_empty() {
            return Err("Empty audio buffer".into());
        }

        let onset = self.compute_onset_envelope(audio);
        if onset.is_empty() {
            return Err("Failed to compute onset envelope".into());
        }

        let onset_sr = audio.sample_rate / 512;
        let mut bpm = self.estimate_bpm_autocorr(&onset, onset_sr);

        if bpm < 40.0 {
            bpm *= 2.0;
        }
        if bpm > 220.0 {
            bpm /= 2.0;
        }
        Ok(bpm)
    }

    pub fn detect_beats(&self, audio: &AudioBuffer) -> Result<Vec<f32>> {
        if audio.samples.is_empty() {
            return Err("Empty audio buffer".into());
        }

        let onset = self.compute_onset_envelope(audio);
        if onset.is_empty() {
            return Err("Failed to compute onset envelope".into());
        }

        let onset_sr = audio.sample_rate / 512;
        let mut bpm = self.estimate_bpm_autocorr(&onset, onset_sr);
        if bpm < 40.0 {
            bpm *= 2.0;
        }
        if bpm > 220.0 {
            bpm /= 2.0;
        }

        let beat_period_samples = (60.0 / bpm) * onset_sr as f32;
        let min_distance = (beat_period_samples * 0.7) as i32;

        let mean: f32 = onset.iter().sum::<f32>() / onset.len() as f32;
        let variance: f32 =
            onset.iter().map(|&v| (v - mean) * (v - mean)).sum::<f32>() / onset.len() as f32;
        let std_dev = variance.sqrt();
        let threshold = mean + 0.5 * std_dev;

        let peak_indices = self.pick_peaks(&onset, threshold, min_distance);

        let hop_duration = 512.0 / audio.sample_rate as f32;
        let mut beat_times = Vec::with_capacity(peak_indices.len());
        for idx in peak_indices {
            beat_times.push(idx * hop_duration);
        }
        Ok(beat_times)
    }

    fn compute_onset_envelope(&self, audio: &AudioBuffer) -> Vec<f32> {
        const FRAME_SIZE: usize = 1024;
        const HOP_SIZE: usize = 512;

        let mono = audio.to_mono();
        if mono.len() < FRAME_SIZE {
            return Vec::new();
        }

        let mut envelope: Vec<f32> = Vec::with_capacity(mono.len() / HOP_SIZE);

        let window: Vec<f32> = (0..FRAME_SIZE)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (FRAME_SIZE - 1) as f32).cos()))
            .collect();

        let mut prev_spectrum = [0.0f32; 3];

        let mut start = 0usize;
        while start + FRAME_SIZE <= mono.len() {
            let mut low_energy = 0.0f32;
            let mut mid_energy = 0.0f32;
            let mut high_energy = 0.0f32;

            for i in 0..FRAME_SIZE {
                let sample = mono[start + i] * window[i];
                let energy = sample * sample;
                if i < FRAME_SIZE / 8 {
                    low_energy += energy;
                } else if i < FRAME_SIZE / 2 {
                    mid_energy += energy;
                } else {
                    high_energy += energy;
                }
            }

            let spectrum = [low_energy, mid_energy, high_energy];

            let mut flux = 0.0f32;
            for i in 0..3 {
                let diff = spectrum[i] - prev_spectrum[i];
                if diff > 0.0 {
                    flux += diff;
                }
            }
            envelope.push(flux);
            prev_spectrum = spectrum;

            start += HOP_SIZE;
        }

        let max_val = envelope.iter().cloned().fold(0.0f32, f32::max);
        if max_val > 0.0 {
            for v in envelope.iter_mut() {
                *v /= max_val;
            }
        }

        envelope
    }

    fn estimate_bpm_autocorr(&self, onset_envelope: &[f32], sample_rate: i32) -> f32 {
        if onset_envelope.len() < 100 {
            return 120.0;
        }

        let mut min_lag = (sample_rate as f32 * 60.0 / 200.0) as i32;
        let mut max_lag = (sample_rate as f32 * 60.0 / 60.0) as i32;

        max_lag = max_lag.min((onset_envelope.len() / 2) as i32);
        min_lag = min_lag.max(1);

        let mut best_corr = -1.0f32;
        let mut best_lag = min_lag;

        for lag in min_lag..=max_lag {
            let mut corr = 0.0f32;
            let mut count = 0i32;
            let lag_us = lag as usize;

            if onset_envelope.len() <= lag_us {
                continue;
            }
            for i in 0..(onset_envelope.len() - lag_us) {
                corr += onset_envelope[i] * onset_envelope[i + lag_us];
                count += 1;
            }
            if count > 0 {
                corr /= count as f32;
                if corr > best_corr {
                    best_corr = corr;
                    best_lag = lag;
                }
            }
        }

        let mut bpm = (sample_rate as f32 * 60.0) / best_lag as f32;

        if bpm > 140.0 {
            let half_tempo_lag = best_lag * 2;
            if half_tempo_lag <= max_lag {
                let mut half_corr = 0.0f32;
                let mut count = 0i32;
                let lag_us = half_tempo_lag as usize;
                if onset_envelope.len() > lag_us {
                    for i in 0..(onset_envelope.len() - lag_us) {
                        half_corr += onset_envelope[i] * onset_envelope[i + lag_us];
                        count += 1;
                    }
                }
                if count > 0 {
                    half_corr /= count as f32;
                }
                if half_corr > best_corr * 0.55 {
                    bpm /= 2.0;
                }
            } else if bpm > 175.0 {
                bpm /= 2.0;
            }
        } else if bpm < 100.0 {
            // same octave check, other direction: a slow reading can be a half-tempo lock
            let double_tempo_lag = best_lag / 2;
            if double_tempo_lag >= min_lag {
                let mut double_corr = 0.0f32;
                let mut count = 0i32;
                let lag_us = double_tempo_lag as usize;
                if lag_us > 0 && onset_envelope.len() > lag_us {
                    for i in 0..(onset_envelope.len() - lag_us) {
                        double_corr += onset_envelope[i] * onset_envelope[i + lag_us];
                        count += 1;
                    }
                }
                if count > 0 {
                    double_corr /= count as f32;
                }
                if double_corr > best_corr * 0.55 {
                    bpm *= 2.0;
                }
            }
        }

        bpm
    }

    fn pick_peaks(&self, envelope: &[f32], threshold: f32, min_distance: i32) -> Vec<f32> {
        let mut peaks: Vec<f32> = Vec::new();

        if envelope.len() < 3 {
            return peaks;
        }

        for i in 1..envelope.len() - 1 {
            if envelope[i] > threshold
                && envelope[i] > envelope[i - 1]
                && envelope[i] >= envelope[i + 1]
            {
                let i_f = i as f32;
                if peaks.is_empty() || (i as i32 - *peaks.last().unwrap() as i32) >= min_distance {
                    peaks.push(i_f);
                } else {
                    let last_idx = *peaks.last().unwrap() as usize;
                    if envelope[i] > envelope[last_idx] {
                        *peaks.last_mut().unwrap() = i_f;
                    }
                }
            }
        }

        peaks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrects_halftempo_lock() {
        // strong/weak alternating pulses, like a backbeat: raw autocorr locks
        // onto the weaker half-tempo period unless the doubling check catches it
        let period = 24usize;
        let cycles = 40usize;
        let mut envelope = vec![0.0f32; period * cycles];
        for k in 0..cycles {
            envelope[k * period] = if k % 2 == 0 { 1.0 } else { 0.5 };
        }
        let detector = BPMDetector;
        let bpm = detector.estimate_bpm_autocorr(&envelope, 50);
        assert!((bpm - 125.0).abs() < 1.0, "bpm={bpm}");
    }

    #[test]
    fn keeps_genuine_slow_tempo() {
        let period = 48usize;
        let cycles = 20usize;
        let mut envelope = vec![0.0f32; period * cycles];
        for k in 0..cycles {
            envelope[k * period] = 1.0;
        }
        let detector = BPMDetector;
        let bpm = detector.estimate_bpm_autocorr(&envelope, 50);
        assert!((bpm - 62.5).abs() < 1.0, "bpm={bpm}");
    }
}
