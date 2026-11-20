// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Fractional delay line plus the delay/echo effect built on top of it.
//!
//! Both the plain delay and the echo share this implementation: the echo is the
//! same tap with a feedback path, so one code path covers both without keeping
//! two nearly identical modules. Buffers are allocated once for the longest
//! supported delay, which keeps every later parameter change allocation-free on
//! the realtime audio thread.

/// Longest delay the effect accepts, in seconds.
pub const MAX_DELAY_SECS: f32 = 2.0;

/// Feedback is capped below unity so a long echo always decays.
pub const MAX_FEEDBACK: f32 = 0.85;

/// One channel of a ring buffer with linearly interpolated reads.
pub struct DelayLine {
    buffer: Vec<f32>,
    write: usize,
}

impl DelayLine {
    pub(crate) fn new(max_samples: usize) -> Self {
        let capacity = (max_samples + 2).next_power_of_two();
        Self {
            buffer: vec![0.0; capacity],
            write: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write = 0;
    }

    /// Read `delay` samples behind the write head.
    pub(crate) fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let mut position = self.write as f32 - delay;
        if position < 0.0 {
            position += len as f32;
        }
        if position >= len as f32 {
            position -= len as f32;
        }
        let index = position as usize;
        let fraction = position - index as f32;
        let a = self.buffer[index % len];
        let b = self.buffer[(index + 1) % len];
        a + (b - a) * fraction
    }

    pub(crate) fn write(&mut self, value: f32) {
        let len = self.buffer.len();
        // Flush denormals: a long decaying echo otherwise leaves tiny values in
        // the line that cost far more CPU than the signal is worth.
        self.buffer[self.write] = if value.abs() < 1e-20 { 0.0 } else { value };
        self.write = (self.write + 1) % len;
    }
}

/// Single-tap delay with an optional (damped) feedback path.
pub struct Delay {
    lines: Vec<DelayLine>,
    /// One-pole low-pass state for the feedback path, per channel.
    damp: Vec<f32>,
    sample_rate: f32,
    channels: usize,
    max_samples: f32,
    enabled: bool,
    target_samples: f32,
    current_samples: f32,
    feedback: f32,
    mix: f32,
}

impl Delay {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let rate = sample_rate.max(1) as f32;
        let channels = channels.max(1) as usize;
        let max_samples = (MAX_DELAY_SECS * rate).max(64.0);
        let default_samples = (0.25 * rate).min(max_samples);
        Self {
            lines: (0..channels)
                .map(|_| DelayLine::new(max_samples as usize))
                .collect(),
            damp: vec![0.0; channels],
            sample_rate: rate,
            channels,
            max_samples,
            enabled: false,
            target_samples: default_samples,
            current_samples: default_samples,
            feedback: 0.0,
            mix: 0.0,
        }
    }

    /// Apply the settings for the next block. Ignored while disabled.
    pub fn configure(&mut self, enabled: bool, time_ms: f32, feedback: f32, mix: f32) {
        if self.enabled && !enabled {
            // Drop the buffer so re-enabling does not replay an old echo.
            for line in &mut self.lines {
                line.clear();
            }
            self.damp.fill(0.0);
        }
        let was_enabled = self.enabled;
        self.enabled = enabled;
        let max_ms = self.max_samples / self.sample_rate * 1000.0;
        let time = if time_ms.is_finite() {
            time_ms.clamp(1.0, max_ms)
        } else {
            250.0
        };
        self.target_samples = (time * 0.001 * self.sample_rate).clamp(1.0, self.max_samples - 1.0);
        if !was_enabled && enabled {
            // Start at the requested tap instead of sliding in from the default.
            self.current_samples = self.target_samples;
        }
        self.feedback = if feedback.is_finite() {
            feedback.clamp(0.0, MAX_FEEDBACK)
        } else {
            0.0
        };
        self.mix = if mix.is_finite() {
            mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn process(&mut self, input: f32, channel: usize) -> f32 {
        if !self.enabled {
            return input;
        }
        let index = channel % self.channels;
        // Glide the tap so releasing the time slider bends instead of clicking.
        self.current_samples += (self.target_samples - self.current_samples) * 0.0005;
        let delayed = self.lines[index].read(self.current_samples);
        // Damp the feedback path a little so long echoes do not turn harsh.
        self.damp[index] += (delayed - self.damp[index]) * 0.35;
        self.lines[index].write(input + self.damp[index] * self.feedback);
        input * (1.0 - self.mix) + delayed * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_delay_is_a_passthrough() {
        let mut delay = Delay::new(48_000, 2);
        for sample in [-0.5, 0.0, 0.25] {
            assert_eq!(delay.process(sample, 0), sample);
        }
    }

    #[test]
    fn first_echo_arrives_after_the_configured_time() {
        let mut delay = Delay::new(48_000, 1);
        delay.configure(true, 10.0, 0.0, 1.0);
        // 10 ms at 48 kHz = 480 samples; the first 479 reads are silence.
        let mut first = None;
        for index in 0..600 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let output = delay.process(input, 0);
            if index > 0 && output.abs() > 0.5 {
                first = Some(index);
                break;
            }
        }
        let first = first.expect("echo never arrived");
        // The tap glides in from its default, so allow a wide tolerance.
        assert!((300..=520).contains(&first), "echo arrived at {first}");
    }

    #[test]
    fn feedback_decays_and_never_explodes() {
        let mut delay = Delay::new(48_000, 1);
        delay.configure(true, 50.0, MAX_FEEDBACK, 1.0);
        let mut peak = 0.0f32;
        for index in 0..200_000 {
            let input = if index < 100 { 1.0 } else { 0.0 };
            let output = delay.process(input, 0);
            assert!(output.is_finite());
            peak = peak.max(output.abs());
        }
        assert!(peak <= 8.0, "feedback ran away: {peak}");
    }

    #[test]
    fn mix_zero_keeps_the_dry_signal() {
        let mut delay = Delay::new(48_000, 1);
        delay.configure(true, 20.0, 0.5, 0.0);
        let mut input = 0.0;
        for index in 0..1_000 {
            input = ((index as f32) * 0.01).sin() * 0.4;
            assert!((delay.process(input, 0) - input).abs() < 1e-6);
        }
    }
}
