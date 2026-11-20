//! Digital signal processing utilities x3
#[inline]
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {value.max(min).min(max)}
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {a + t * (b - a)}
#[inline]
pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.0;
    }
    (value - min) / (max - min)
}
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let (dot, norm_a, norm_b) = a
        .iter()
        .zip(b)
        .fold((0.0f32, 0.0f32, 0.0f32), |(dot, na, nb), (&x, &y)| {
            (dot + x * y, na + x * x, nb + y * y)
        });
    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }
    1.0 - clamp(dot / (norm_a.sqrt() * norm_b.sqrt()), -1.0, 1.0)
}
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }
    a.iter()
        .zip(b)
        .map(|(&x, &y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}
