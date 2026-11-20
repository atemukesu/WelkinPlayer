//! AutoMix Engine - Key Detector

use crate::types::{AudioBuffer, Result};
use std::f32::consts::PI;

#[derive(Default)]
pub struct KeyDetector;

// Krumhansl-Kessler key profiles (normalized).
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

fn goertzel_power(samples: &[f32], frequency: f32, sample_rate: f32) -> f32 {
    let count = samples.len();
    if count == 0 || frequency <= 0.0 || frequency >= sample_rate * 0.5 {
        return 0.0;
    }
    let n = count as f32;
    let k = (0.5 + n * frequency / sample_rate).floor();
    let omega = 2.0 * PI * k / n;
    let coeff = 2.0 * omega.cos();
    let mut q1 = 0.0f32;
    let mut q2 = 0.0f32;
    for &s in samples {
        let q0 = coeff * q1 - q2 + s;
        q2 = q1;
        q1 = q0;
    }
    q1 * q1 + q2 * q2 - coeff * q1 * q2
}

impl KeyDetector {
    pub fn detect(&self, audio: &AudioBuffer) -> Result<String> {
        let chroma = self.compute_chroma(audio)?;
        if chroma.len() != 12 {
            return Err("Invalid chroma vector".into());
        }

        let mut best_correlation = -1.0f32;
        let mut best_pitch_class = 0i32;
        let mut best_is_major = true;

        for shift in 0..12 {
            let major_corr = correlate_with_profile(&chroma, &MAJOR_PROFILE, shift);
            if major_corr > best_correlation {
                best_correlation = major_corr;
                best_pitch_class = shift;
                best_is_major = true;
            }
            let minor_corr = correlate_with_profile(&chroma, &MINOR_PROFILE, shift);
            if minor_corr > best_correlation {
                best_correlation = minor_corr;
                best_pitch_class = shift;
                best_is_major = false;
            }
        }
        Ok(pitch_class_to_camelot(best_pitch_class, best_is_major))
    }

    pub fn compute_chroma(&self, audio: &AudioBuffer) -> Result<Vec<f32>> {
        if audio.samples.is_empty() {
            return Err("Empty audio buffer".into());
        }

        let mono = audio.to_mono();

        let frame_size: usize = (4096).min(mono.len());
        let hop_size: usize = (frame_size / 2).max(512);
        if frame_size < 1024 {
            return Ok(vec![1.0 / 12.0; 12]);
        }

        let mut chroma = vec![0.0f32; 12];
        let mut frame_count = 0i32;

        let window: Vec<f32> = (0..frame_size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (frame_size - 1) as f32).cos()))
            .collect();

        let mut frame = vec![0.0f32; frame_size];

        let mut start = 0usize;
        while start + frame_size <= mono.len() {
            for i in 0..frame_size {
                frame[i] = mono[start + i] * window[i];
            }
            for midi in 48..84 {
                let freq = 440.0 * 2.0f32.powf((midi as f32 - 69.0) / 12.0);
                let power = goertzel_power(&frame, freq, audio.sample_rate as f32);
                chroma[(midi as usize) % 12] += power;
            }
            frame_count += 1;
            if frame_count >= 256 {
                break;
            }
            start += hop_size;
        }

        let sum: f32 = chroma.iter().sum();
        if sum > 0.0 {
            for v in chroma.iter_mut() {
                *v /= sum;
            }
        } else {
            for v in chroma.iter_mut() {
                *v = 1.0 / 12.0;
            }
        }
        Ok(chroma)
    }
}

fn correlate_with_profile(chroma: &[f32], profile: &[f32; 12], shift: i32) -> f32 {
    let chroma_mean: f32 = chroma.iter().sum::<f32>() / 12.0;
    let profile_sum: f32 = profile.iter().sum();
    let profile_mean = profile_sum / 12.0;

    let mut numerator = 0.0f32;
    let mut chroma_var = 0.0f32;
    let mut profile_var = 0.0f32;

    for (i, &profile_v) in profile.iter().enumerate() {
        let shifted_i = ((i as i32 + shift) % 12) as usize;
        let c = chroma[shifted_i] - chroma_mean;
        let p = profile_v - profile_mean;
        numerator += c * p;
        chroma_var += c * c;
        profile_var += p * p;
    }
    let denominator = (chroma_var * profile_var).sqrt();
    if denominator < 1e-10 {
        return 0.0;
    }
    numerator / denominator
}

fn pitch_class_to_camelot(pitch_class: i32, is_major: bool) -> String {
    static MAJOR_CAMELOT: [i32; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
    static MINOR_CAMELOT: [i32; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];

    let (camelot_num, mode_char) = if is_major {
        (MAJOR_CAMELOT[pitch_class as usize], 'B')
    } else {
        (MINOR_CAMELOT[pitch_class as usize], 'A')
    };
    format!("{}{}", camelot_num, mode_char)
}
