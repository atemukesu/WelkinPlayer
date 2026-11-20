//! Audio Analyzer section, bite me if broken
mod bpm_detector;
mod energy_analyzer;
mod key_detector;

pub use bpm_detector::BPMDetector;
pub use energy_analyzer::EnergyAnalyzer;
pub use key_detector::KeyDetector;

use crate::types::{AudioBuffer, Result, TrackFeatures};

pub struct Analyzer {
    bpm_detector: BPMDetector,
    key_detector: KeyDetector,
    energy_analyzer: EnergyAnalyzer,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            bpm_detector: BPMDetector,
            key_detector: KeyDetector,
            energy_analyzer: EnergyAnalyzer,
        }
    }

    pub fn analyze(&mut self, audio: &AudioBuffer) -> Result<TrackFeatures> {
        let mut features = TrackFeatures {
            duration: audio.duration_seconds(),
            ..Default::default()
        };
        if let Ok(v) = self.detect_bpm(audio) {features.bpm = v;}
        if let Ok(v) = self.detect_beats(audio) {features.beats = v;}
        if let Ok(v) = self.detect_key(audio) {features.key = v;}
        if let Ok(v) = self.compute_mfcc(audio) {features.mfcc = v;}
        if let Ok(v) = self.compute_chroma(audio) {features.chroma = v;}
        if let Ok(v) = self.compute_energy_curve(audio) {features.energy_curve = v;}
        if let Ok(v) = self.compute_loudness_curve_db(audio) {
            features.loudness_curve_db = v;
            features.loudness_curve_resolution_seconds = LOUDNESS_CURVE_RESOLUTION_S;
        }
        Ok(features)
    }
    pub fn detect_bpm(&mut self, audio: &AudioBuffer) -> Result<f32> {self.bpm_detector.detect(audio)}
    pub fn detect_beats(&mut self, audio: &AudioBuffer) -> Result<Vec<f32>> {self.bpm_detector.detect_beats(audio)}
    pub fn detect_key(&mut self, audio: &AudioBuffer)-> Result<String> {self.key_detector.detect(audio)}
    pub fn compute_mfcc(&mut self, audio: &AudioBuffer) -> Result<Vec<f32>> {compute_mfcc_simple(audio)}
    pub fn compute_chroma(&mut self, audio: &AudioBuffer) -> Result<Vec<f32>> {self.key_detector.compute_chroma(audio)}
    pub fn compute_energy_curve(&mut self, audio: &AudioBuffer) -> Result<Vec<f32>> {self.energy_analyzer.compute_curve(audio, 0.5)}
    pub fn compute_loudness_curve_db(&mut self, audio: &AudioBuffer) -> Result<Vec<f32>> {
        self.energy_analyzer.compute_db_curve(audio, LOUDNESS_CURVE_RESOLUTION_S)
    }
}

const LOUDNESS_CURVE_RESOLUTION_S: f32 = 0.1;

/// Simplified MFCC (not real mel filterbanks, IT DOESNT SOUND THAT DIFFERENT)
fn compute_mfcc_simple(audio: &AudioBuffer) -> Result<Vec<f32>> {
    const FRAME_SIZE: usize = 2048;
    if audio.samples.len() < FRAME_SIZE*2 {return Ok(vec![0.0; 13]);}

    let mono = audio.to_mono();
    let mut mfcc = vec![0.0f32; 13];

    let mut total_energy = 0.0f32;
    for &v in &mono {total_energy += v * v;}

    if total_energy > 0.0 {mfcc[0] = (total_energy / mono.len() as f32 + 1e-10).ln();}
    let mut sum = 0.0f32;
    let mut sum_sq = 0.0f32;
    for &v in &mono {
        sum += v;
        sum_sq += v * v;
    }
    let mean = sum / mono.len() as f32;
    let variance = sum_sq / mono.len() as f32 - mean * mean;

    mfcc[1] = mean;
    mfcc[2] = variance.max(0.0).sqrt();
    Ok(mfcc)
}
