//! Core types

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self(s)
    }
}
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Interleaved L/R (or mono if channels == 1).
    pub samples: Vec<f32>,
    pub sample_rate: i32,
    pub channels: i32,
}

impl Default for AudioBuffer {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: 44100,
            channels: 2,
        }
    }
}

impl AudioBuffer {
    pub fn frame_count(&self) -> usize {
        if self.channels > 0 {
            self.samples.len() / self.channels as usize
        } else {
            0
        }
    }

    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate > 0 {
            self.frame_count() as f32 / self.sample_rate as f32
        } else {
            0.0
        }
    }

    /// Downmix to mono; copies if already mono.
    pub fn to_mono(&self) -> Vec<f32> {
        if self.channels == 1 {
            return self.samples.clone();
        }
        let frames = self.frame_count();
        let mut mono = Vec::with_capacity(frames);
        for i in 0..frames {
            mono.push((self.samples[i * 2] + self.samples[i * 2 + 1]) / 2.0);
        }
        mono
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrackFeatures {
    pub bpm: f32,
    /// Beat positions in seconds.
    pub beats: Vec<f32>,
    /// Camelot notation (e.g. "8A").
    pub key: String,
    /// 13-dimensional MFCC mean.
    pub mfcc: Vec<f32>,
    /// 12-dimensional chroma.
    pub chroma: Vec<f32>,
    /// Energy over time (normalized to [0, 1]).
    pub energy_curve: Vec<f32>,
    /// Total duration in seconds.
    pub duration: f32,
    pub loudness_lufs: Option<f32>,
    pub genre: Option<String>,
    pub loudness_curve_db: Vec<f32>,
    pub loudness_curve_resolution_seconds: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RampPoint {
    pub db: f32,
    pub time_from_edge_ms: i64,
    pub slope: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartMixSearchMode {
    Strict,
    Relaxed,
    Fallback,
}

// six flavors, that's it :D
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveType {
    LinearFast,
    LinearMedium,
    LinearSlow,
    BungeeFast,
    BungeeMedium,
    BungeeSlow,
}

#[derive(Debug, Clone, Copy)]
pub struct CrossfadeParameters {
    pub ramp_out_start_ms: i64,
    pub ramp_in_start_ms: i64,
    pub fade_out_curve: CurveType,
    pub fade_in_curve: CurveType,
}

#[derive(Debug, Clone, Copy)]
pub struct SmartCrossfadeAnalysis {
    pub parameters: CrossfadeParameters,
    pub search_mode: SmartMixSearchMode,
    pub score: f32,
    pub volume_delta_db: f32,
}
