//! Music theory utilities: Camelot wheel and BPM helpers.

pub fn parse_camelot_number(key: &str) -> i32 {
    if key.is_empty() {
        return 0;
    }
    let mut chars = key.chars();
    chars.next_back();
    let prefix: String = chars.collect();
    prefix.parse::<i32>().unwrap_or(0)
}

pub fn parse_camelot_mode(key: &str) -> char {
    key.chars().last().unwrap_or('A')
}

/// Distance on the Camelot wheel between two keys.
pub fn camelot_distance(key1: &str, key2: &str) -> i32 {
    if key1.is_empty() || key2.is_empty() {
        return 0;
    }
    let num1 = parse_camelot_number(key1);
    let num2 = parse_camelot_number(key2);
    let mode1 = parse_camelot_mode(key1);
    let mode2 = parse_camelot_mode(key2);

    if num1 == 0 || num2 == 0 {
        return 0;
    }

    let diff = (num1 - num2).abs();
    let wheel_dist = diff.min(12 - diff);

    if mode1 == mode2 {
        return wheel_dist;
    }
    if num1 == num2 {
        return 0;
    }
    wheel_dist + 1
}

pub fn keys_compatible(key1: &str, key2: &str) -> bool {
    camelot_distance(key1, key2) <= 1
}

pub fn bpm_distance(bpm1: f32, bpm2: f32) -> f32 {
    if bpm1 <= 0.0 || bpm2 <= 0.0 {
        return 0.0;
    }
    let ratio = bpm1 / bpm2;
    [
        (1.0 - ratio).abs(),
        (2.0 - ratio).abs(),
        (0.5 - ratio).abs(),
    ]
    .iter()
    .cloned()
    .fold(f32::INFINITY, f32::min)
}

/// Stretch ratio to apply to source to match target BPM, clamped to one octave.
pub fn calculate_stretch_ratio(target_bpm: f32, source_bpm: f32) -> f32 {
    if source_bpm <= 0.0 || target_bpm <= 0.0 {
        return 1.0;
    }
    let mut ratio = target_bpm / source_bpm;
    if (1.0 - ratio).abs() < 0.01 {
        return 1.0;
    }
    if ratio > 1.5 {
        ratio /= 2.0;
    }
    if ratio < 0.67 {
        ratio *= 2.0;
    }
    ratio
}
