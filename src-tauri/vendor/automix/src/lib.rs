//! Pure-DSP library: BPM/key/energy analysis, transition planning and
//! crossfade-point selection. No audio I/O, no decoding, no persistence.
//!
//! This vendored copy removes upstream's `mixer` module: it was the only user
//! of the native RubberBand library, which WelkinPlayer never links. See
//! Cargo.toml for details.

pub mod dsp;
pub mod music_theory;
pub mod types;

pub mod analyzer;
pub mod matcher;

pub use analyzer::Analyzer;
pub use matcher::TransitionPointFinder;
pub use types::{
    AudioBuffer, CrossfadeParameters, CurveType, Error, RampPoint, Result, SmartCrossfadeAnalysis,
    SmartMixSearchMode, TrackFeatures,
};
