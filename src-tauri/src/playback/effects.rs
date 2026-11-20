// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! The user-facing effects chain: chorus → flanger → delay → echo → reverb.
//!
//! Settings travel as atomics in [`crate::playback::engine::Shared`] and are
//! snapshotted once per state block, exactly like the graphic EQ. A disabled
//! effect returns the sample untouched, so a box with nothing switched on costs
//! a handful of branches per sample.
//!
//! Effects run inside each deck's `RingSource`, after the graphic EQ and before
//! the volume gain. A crossfade therefore fades an effect tail together with its
//! deck, and seeking starts from empty buffers.

use serde::{Deserialize, Serialize};

use crate::playback::delay::{Delay, MAX_FEEDBACK};
use crate::playback::mod_delay::ModDelay;
use crate::playback::reverb::Reverb;

/// Shortest/longest delay and echo tap in milliseconds.
pub const MIN_TIME_MS: f32 = 20.0;
pub const MAX_TIME_MS: f32 = 1000.0;

/// Chorus tuning: a wide, slow shimmer around 15 ms.
const CHORUS_BASE_MS: f32 = 15.0;
const CHORUS_DEPTH_MS: f32 = 8.0;
const CHORUS_RATE_HZ: f32 = 0.6;

/// Flanger tuning: a short, faster sweep with feedback.
const FLANGER_BASE_MS: f32 = 2.5;
const FLANGER_DEPTH_MS: f32 = 2.0;
const FLANGER_RATE_HZ: f32 = 0.25;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DelaySettings {
    pub enabled: bool,
    pub time_ms: f32,
    pub mix: f32,
}

impl Default for DelaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            time_ms: 250.0,
            mix: 0.3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EchoSettings {
    pub enabled: bool,
    pub time_ms: f32,
    pub feedback: f32,
    pub mix: f32,
}

impl Default for EchoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            time_ms: 350.0,
            feedback: 0.35,
            mix: 0.3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChorusSettings {
    pub enabled: bool,
    pub depth: f32,
    pub mix: f32,
}

impl Default for ChorusSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            depth: 0.5,
            mix: 0.4,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FlangerSettings {
    pub enabled: bool,
    pub depth: f32,
    pub feedback: f32,
    pub mix: f32,
}

impl Default for FlangerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            depth: 0.5,
            feedback: 0.3,
            mix: 0.4,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReverbSettings {
    pub enabled: bool,
    pub size: f32,
    pub damping: f32,
    pub mix: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 0.5,
            damping: 0.5,
            mix: 0.25,
        }
    }
}

/// Whole effects box as seen by the frontend.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EffectsSettings {
    pub delay: DelaySettings,
    pub echo: EchoSettings,
    pub chorus: ChorusSettings,
    pub flanger: FlangerSettings,
    pub reverb: ReverbSettings,
}

fn clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

impl EffectsSettings {
    /// Clamp every field to the supported range. Both the command handler and
    /// the engine run this, so a hand-written invoke cannot push a feedback of
    /// 10 into the audio thread.
    pub fn sanitized(mut self) -> Self {
        self.delay.time_ms = clamp(self.delay.time_ms, MIN_TIME_MS, MAX_TIME_MS, 250.0);
        self.delay.mix = clamp(self.delay.mix, 0.0, 1.0, 0.3);

        self.echo.time_ms = clamp(self.echo.time_ms, MIN_TIME_MS, MAX_TIME_MS, 350.0);
        self.echo.feedback = clamp(self.echo.feedback, 0.0, MAX_FEEDBACK, 0.35);
        self.echo.mix = clamp(self.echo.mix, 0.0, 1.0, 0.3);

        self.chorus.depth = clamp(self.chorus.depth, 0.0, 1.0, 0.5);
        self.chorus.mix = clamp(self.chorus.mix, 0.0, 1.0, 0.4);

        self.flanger.depth = clamp(self.flanger.depth, 0.0, 1.0, 0.5);
        self.flanger.feedback = clamp(self.flanger.feedback, 0.0, MAX_FEEDBACK, 0.3);
        self.flanger.mix = clamp(self.flanger.mix, 0.0, 1.0, 0.4);

        self.reverb.size = clamp(self.reverb.size, 0.0, 1.0, 0.5);
        self.reverb.damping = clamp(self.reverb.damping, 0.0, 1.0, 0.5);
        self.reverb.mix = clamp(self.reverb.mix, 0.0, 1.0, 0.25);
        self
    }
}

/// All five effects, in processing order.
pub struct Effects {
    delay: Delay,
    echo: Delay,
    chorus: ModDelay,
    flanger: ModDelay,
    reverb: Reverb,
}

impl Effects {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let channels = channels.max(1);
        Self {
            delay: Delay::new(sample_rate, channels),
            echo: Delay::new(sample_rate, channels),
            chorus: ModDelay::new(
                sample_rate,
                channels,
                CHORUS_BASE_MS,
                CHORUS_DEPTH_MS,
                CHORUS_RATE_HZ,
            ),
            flanger: ModDelay::new(
                sample_rate,
                channels,
                FLANGER_BASE_MS,
                FLANGER_DEPTH_MS,
                FLANGER_RATE_HZ,
            ),
            reverb: Reverb::new(sample_rate, channels),
        }
    }

    /// Apply one block's settings. Called on the audio thread; only writes
    /// coefficients, no allocation.
    pub fn configure(&mut self, settings: &EffectsSettings) {
        self.chorus.configure(
            settings.chorus.enabled,
            settings.chorus.depth,
            0.0,
            settings.chorus.mix,
        );
        self.flanger.configure(
            settings.flanger.enabled,
            settings.flanger.depth,
            settings.flanger.feedback,
            settings.flanger.mix,
        );
        // Plain delay has no feedback; the echo adds it.
        self.delay.configure(
            settings.delay.enabled,
            settings.delay.time_ms,
            0.0,
            settings.delay.mix,
        );
        self.echo.configure(
            settings.echo.enabled,
            settings.echo.time_ms,
            settings.echo.feedback,
            settings.echo.mix,
        );
        self.reverb.configure(
            settings.reverb.enabled,
            settings.reverb.size,
            settings.reverb.damping,
            settings.reverb.mix,
        );
    }

    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        let sample = self.chorus.process(sample, channel);
        let sample = self.flanger.process(sample, channel);
        let sample = self.delay.process(sample, channel);
        let sample = self.echo.process(sample, channel);
        self.reverb.process(sample, channel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_box_is_a_passthrough() {
        let mut effects = Effects::new(48_000, 2);
        effects.configure(&EffectsSettings::default());
        for sample in [-0.75, -0.1, 0.0, 0.3, 0.9] {
            assert_eq!(effects.process(sample, 0), sample);
            assert_eq!(effects.process(sample, 1), sample);
        }
    }

    #[test]
    fn sanitize_clamps_every_field() {
        let settings = EffectsSettings {
            delay: DelaySettings {
                enabled: true,
                time_ms: f32::NAN,
                mix: 5.0,
            },
            echo: EchoSettings {
                enabled: true,
                time_ms: -10.0,
                feedback: 9.0,
                mix: -1.0,
            },
            chorus: ChorusSettings {
                enabled: true,
                depth: 2.0,
                mix: f32::INFINITY,
            },
            flanger: FlangerSettings {
                enabled: true,
                depth: -3.0,
                feedback: 0.99,
                mix: 2.0,
            },
            reverb: ReverbSettings {
                enabled: true,
                size: 4.0,
                damping: -1.0,
                mix: 0.5,
            },
        }
        .sanitized();
        assert_eq!(settings.delay.time_ms, 250.0);
        assert_eq!(settings.delay.mix, 1.0);
        assert_eq!(settings.echo.time_ms, MIN_TIME_MS);
        assert_eq!(settings.echo.feedback, MAX_FEEDBACK);
        assert_eq!(settings.echo.mix, 0.0);
        assert_eq!(settings.chorus.depth, 1.0);
        assert_eq!(settings.chorus.mix, 0.4);
        assert_eq!(settings.flanger.depth, 0.0);
        assert_eq!(settings.flanger.feedback, MAX_FEEDBACK);
        assert_eq!(settings.flanger.mix, 1.0);
        assert_eq!(settings.reverb.size, 1.0);
        assert_eq!(settings.reverb.damping, 0.0);
    }

    #[test]
    fn serialises_with_camel_case_fields() {
        let json = serde_json::to_string(&DelaySettings::default()).unwrap();
        assert!(json.contains("\"timeMs\""), "{json}");
    }

    #[test]
    fn partial_json_falls_back_to_defaults() {
        let settings: EffectsSettings =
            serde_json::from_str(r#"{ "delay": { "enabled": true } }"#).unwrap();
        assert!(settings.delay.enabled);
        assert_eq!(settings.delay.time_ms, 250.0);
        assert!(!settings.reverb.enabled);
    }
}
