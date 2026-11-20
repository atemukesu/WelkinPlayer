//!  Energy Analyzer

use crate::types::{AudioBuffer, Result};

#[derive(Default)]
pub struct EnergyAnalyzer;

impl EnergyAnalyzer {
    pub fn compute_curve(&self, audio: &AudioBuffer, resolution: f32) -> Result<Vec<f32>> {
        if audio.samples.is_empty() {
            return Err("Empty audio buffer".into());
        }

        // Mirror C++: int(resolution * sample_rate) * channels, both casts truncate.
        let window_samples =
            ((resolution * audio.sample_rate as f32) as i32 * audio.channels) as usize;
        if window_samples == 0 {
            return Err("Invalid resolution".into());
        }

        let mut energy_curve: Vec<f32> =
            Vec::with_capacity(audio.samples.len() / window_samples + 1);

        let mut start = 0usize;
        while start < audio.samples.len() {
            let end = (start + window_samples).min(audio.samples.len());
            let rms = compute_rms(&audio.samples[start..end]);
            energy_curve.push(rms);
            start += window_samples;
        }

        if energy_curve.is_empty() {
            return Err("No energy data computed".into());
        }

        let max_energy = energy_curve.iter().cloned().fold(0.0f32, f32::max);
        if max_energy > 0.0 {
            for e in energy_curve.iter_mut() {
                *e /= max_energy;
            }
        }

        // Smoothing: simple moving average, half-window = 3.
        let smooth_window: i32 = 3;
        let mut smoothed = vec![0.0f32; energy_curve.len()];
        for (i, out) in smoothed.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            let mut count = 0i32;
            for j in -smooth_window..=smooth_window {
                let idx = i as i32 + j;
                if idx >= 0 && (idx as usize) < energy_curve.len() {
                    sum += energy_curve[idx as usize];
                    count += 1;
                }
            }
            *out = sum / count as f32;
        }

        Ok(smoothed)
    }

    // real dB this time :3
    pub fn compute_db_curve(&self, audio: &AudioBuffer, resolution: f32) -> Result<Vec<f32>> {
        if audio.samples.is_empty() {
            return Err("Empty audio buffer".into());
        }

        let window_samples =
            ((resolution * audio.sample_rate as f32) as i32 * audio.channels) as usize;
        if window_samples == 0 {
            return Err("Invalid resolution".into());
        }

        const FLOOR_DB: f32 = -120.0;
        let mut curve: Vec<f32> = Vec::with_capacity(audio.samples.len() / window_samples + 1);
        let mut start = 0usize;
        while start < audio.samples.len() {
            let end = (start + window_samples).min(audio.samples.len());
            let rms = compute_rms(&audio.samples[start..end]);
            let db = if rms > 0.0 {
                (20.0 * rms.log10()).max(FLOOR_DB)
            } else {
                FLOOR_DB
            };
            curve.push(db);
            start += window_samples;
        }

        if curve.is_empty() {
            return Err("No loudness data computed".into());
        }

        let smoothed: Vec<f32> = (0..curve.len())
            .map(|i| {
                let lo = i.saturating_sub(1);
                let hi = (i + 1).min(curve.len() - 1);
                let slice = &curve[lo..=hi];
                slice.iter().sum::<f32>() / slice.len() as f32
            })
            .collect();

        Ok(smoothed)
    }

    pub fn find_valleys(&self, energy_curve: &[f32], threshold: f32) -> Vec<i32> {
        let mut valleys: Vec<i32> = Vec::new();
        if energy_curve.len() < 3 {
            return valleys;
        }
        for i in 1..energy_curve.len() - 1 {
            if energy_curve[i] < energy_curve[i - 1]
                && energy_curve[i] <= energy_curve[i + 1]
                && energy_curve[i] < threshold
            {
                valleys.push(i as i32);
            }
        }
        valleys
    }

    pub fn find_peaks(&self, energy_curve: &[f32], threshold: f32) -> Vec<i32> {
        let mut peaks: Vec<i32> = Vec::new();
        if energy_curve.len() < 3 {
            return peaks;
        }
        for i in 1..energy_curve.len() - 1 {
            if energy_curve[i] > energy_curve[i - 1]
                && energy_curve[i] >= energy_curve[i + 1]
                && energy_curve[i] > threshold
            {
                peaks.push(i as i32);
            }
        }
        peaks
    }
}

pub fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&v| v * v).sum();
    (sum_sq / samples.len() as f32).sqrt()
}
