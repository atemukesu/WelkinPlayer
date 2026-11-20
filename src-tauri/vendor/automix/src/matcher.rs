//! smart crossfade point selection, loudness only :3

use crate::types::{
    CrossfadeParameters, CurveType, RampPoint, SmartCrossfadeAnalysis, SmartMixSearchMode,
    TrackFeatures,
};

const SEARCH_CENTER_DB: f32 = -24.0;

struct PassParams {
    window_s: f32,
    tolerance_db: f32,
    max_volume_delta_db: f32,
    max_slope: f32,
    mode: SmartMixSearchMode,
}

const STRICT: PassParams = PassParams {
    window_s: 30.0,
    tolerance_db: 4.0,
    max_volume_delta_db: 4.0,
    max_slope: 10.0,
    mode: SmartMixSearchMode::Strict,
};

const RELAXED: PassParams = PassParams {
    window_s: 37.0,
    tolerance_db: 6.5,
    max_volume_delta_db: 6.0,
    max_slope: 12.0,
    mode: SmartMixSearchMode::Relaxed,
};

const FALLBACK_WINDOW_S: f32 = 40.0;
const FULL_FADE_LOW_DB: f32 = -37.0;
const FULL_FADE_HIGH_DB: f32 = -23.0;
const CURVE_FAST_SLOPE: f32 = 4.0;
const CURVE_MEDIUM_SLOPE: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeClass {
    Partial,
    HardCut,
    FadeToSilence,
    FullFade,
}

#[derive(Default)]
pub struct TransitionPointFinder;

impl TransitionPointFinder {
    pub fn analyze(
        &self,
        out_track: &TrackFeatures,
        in_track: &TrackFeatures,
    ) -> SmartCrossfadeAnalysis {
        if out_track.loudness_curve_db.is_empty() || in_track.loudness_curve_db.is_empty() {
            return degenerate_analysis(out_track);
        }

        if let Some((op, ip, cost)) = self.best_pair(out_track, in_track, &STRICT)
            && validate(&op, &ip, &STRICT)
        {
            return build_analysis(out_track, op, ip, cost, STRICT.mode);
        }

        if let Some((op, ip, cost)) = self.best_pair(out_track, in_track, &RELAXED)
            && validate(&op, &ip, &RELAXED)
        {
            return build_analysis(out_track, op, ip, cost, RELAXED.mode);
        }

        // welp, wing it :D
        self.fallback(out_track, in_track)
    }

    fn best_pair(
        &self,
        out_track: &TrackFeatures,
        in_track: &TrackFeatures,
        pass: &PassParams,
    ) -> Option<(RampPoint, RampPoint, f32)> {
        let out_window = tail_window(out_track, pass.window_s);
        let in_window = head_window(in_track, pass.window_s);
        if out_window.is_empty() || in_window.is_empty() {
            return None;
        }

        let tolerance_db = pass.tolerance_db;
        let out_points: Vec<RampPoint> = ramp_points(
            out_window,
            resolution(out_track),
            SEARCH_CENTER_DB,
            tolerance_db,
            false,
        )
        .into_iter()
        .filter(|p| (p.db - SEARCH_CENTER_DB).abs() <= tolerance_db)
        .collect();
        let in_points: Vec<RampPoint> = ramp_points(
            in_window,
            resolution(in_track),
            SEARCH_CENTER_DB,
            tolerance_db,
            true,
        )
        .into_iter()
        .filter(|p| (p.db - SEARCH_CENTER_DB).abs() <= tolerance_db)
        .collect();

        let mut best: Option<(RampPoint, RampPoint, f32)> = None;
        for &op in &out_points {
            for &ip in &in_points {
                let cost = candidate_cost(&op, &ip);
                if best.is_none_or(|(_, _, best_cost)| cost < best_cost) {
                    best = Some((op, ip, cost));
                }
            }
        }
        best
    }

    fn fallback(
        &self,
        out_track: &TrackFeatures,
        in_track: &TrackFeatures,
    ) -> SmartCrossfadeAnalysis {
        let out_window = tail_window(out_track, FALLBACK_WINDOW_S);
        let in_window = head_window(in_track, FALLBACK_WINDOW_S);

        let out_class = classify_edge(out_window, false);
        let in_class = classify_edge(in_window, true);

        let (out_db, out_time_from_edge_ms, out_slope) =
            locate_ramp(out_window, resolution(out_track), out_class, false);
        let (in_db, in_time_from_edge_ms, in_slope) =
            locate_ramp(in_window, resolution(in_track), in_class, true);

        let ramp_out_start_ms = seconds_to_ms(out_track.duration) - out_time_from_edge_ms;
        let ramp_in_start_ms = in_time_from_edge_ms;

        SmartCrossfadeAnalysis {
            parameters: CrossfadeParameters {
                ramp_out_start_ms,
                ramp_in_start_ms,
                fade_out_curve: pick_curve(out_class, out_slope),
                fade_in_curve: pick_curve(in_class, in_slope),
            },
            search_mode: SmartMixSearchMode::Fallback,
            score: 0.0,
            volume_delta_db: (out_db - in_db).abs(),
        }
    }
}

fn build_analysis(
    out_track: &TrackFeatures,
    op: RampPoint,
    ip: RampPoint,
    cost: f32,
    mode: SmartMixSearchMode,
) -> SmartCrossfadeAnalysis {
    SmartCrossfadeAnalysis {
        parameters: CrossfadeParameters {
            ramp_out_start_ms: seconds_to_ms(out_track.duration) - op.time_from_edge_ms,
            ramp_in_start_ms: ip.time_from_edge_ms,
            fade_out_curve: CurveType::LinearFast,
            fade_in_curve: CurveType::LinearFast,
        },
        search_mode: mode,
        score: cost,
        volume_delta_db: (op.db - ip.db).abs(),
    }
}

fn degenerate_analysis(out_track: &TrackFeatures) -> SmartCrossfadeAnalysis {
    SmartCrossfadeAnalysis {
        parameters: CrossfadeParameters {
            ramp_out_start_ms: seconds_to_ms(out_track.duration),
            ramp_in_start_ms: 0,
            fade_out_curve: CurveType::LinearMedium,
            fade_in_curve: CurveType::LinearMedium,
        },
        search_mode: SmartMixSearchMode::Fallback,
        score: 0.0,
        volume_delta_db: 0.0,
    }
}

fn candidate_cost(op: &RampPoint, ip: &RampPoint) -> f32 {
    let delta_time_ms = (op.time_from_edge_ms - ip.time_from_edge_ms).unsigned_abs() as f32;
    delta_time_ms / 5000.0
        + 0.35 * (op.slope.abs() + ip.slope.abs())
        + 0.45 * ((op.db - SEARCH_CENTER_DB).abs() + (ip.db - SEARCH_CENTER_DB).abs())
        + 4.0 * (op.db - ip.db).abs()
}

fn validate(op: &RampPoint, ip: &RampPoint, pass: &PassParams) -> bool {
    let volume_delta = (op.db - ip.db).abs();
    volume_delta <= pass.max_volume_delta_db
        && op.slope.abs() <= pass.max_slope
        && ip.slope.abs() <= pass.max_slope
}

fn ramp_points(
    window: &[f32],
    resolution_s: f32,
    center_db: f32,
    tolerance_db: f32,
    from_start: bool,
) -> Vec<RampPoint> {
    threshold_ladder(center_db, tolerance_db)
        .into_iter()
        .filter_map(|threshold_db| {
            let idx = find_edge_index(window, threshold_db, from_start)?;
            Some(RampPoint {
                db: window[idx],
                time_from_edge_ms: index_to_time_from_edge_ms(
                    window.len(),
                    idx,
                    resolution_s,
                    from_start,
                ),
                slope: local_slope(window, idx),
            })
        })
        .collect()
}

fn threshold_ladder(center_db: f32, tolerance_db: f32) -> Vec<f32> {
    const COARSE_MIN_DB: i32 = -120;
    const COARSE_MAX_DB: i32 = 6;
    const COARSE_STEP_DB: i32 = 5;

    let mut milli_db: Vec<i32> = (0..)
        .map(|i| COARSE_MIN_DB + i * COARSE_STEP_DB)
        .take_while(|&db| db <= COARSE_MAX_DB)
        .map(|db| db * 100)
        .collect();

    let fine_lo = (center_db - tolerance_db).floor() as i32;
    let fine_hi = (center_db + tolerance_db).ceil() as i32;
    milli_db.extend((fine_lo..=fine_hi).map(|db| db * 100));

    milli_db.sort_unstable();
    milli_db.dedup();
    milli_db.into_iter().map(|m| m as f32 / 100.0).collect()
}

fn find_edge_index(window: &[f32], threshold_db: f32, from_start: bool) -> Option<usize> {
    let n = window.len();
    if n == 0 {
        return None;
    }
    if from_start {
        (0..n).find(|&i| window[i] >= threshold_db)
    } else {
        (0..n).rev().find(|&i| window[i] >= threshold_db)
    }
}

fn index_to_time_from_edge_ms(n: usize, idx: usize, resolution_s: f32, from_start: bool) -> i64 {
    let steps = if from_start { idx } else { n - 1 - idx };
    (steps as f32 * resolution_s * 1000.0) as i64
}

fn local_slope(window: &[f32], idx: usize) -> f32 {
    let n = window.len();
    if n == 0 {
        return 0.0;
    }
    let prev = window[idx.saturating_sub(1)];
    let next = window[(idx + 1).min(n - 1)];
    (next - prev) / 2.0
}

fn classify_edge(window: &[f32], from_start: bool) -> EdgeClass {
    if window.is_empty() {
        return EdgeClass::Partial;
    }
    let boundary_db = if from_start {
        window[0]
    } else {
        *window.last().unwrap()
    };
    let min_db = window.iter().cloned().fold(f32::MAX, f32::min);
    let max_db = window.iter().cloned().fold(f32::MIN, f32::max);

    if min_db < FULL_FADE_LOW_DB && max_db > FULL_FADE_HIGH_DB {
        EdgeClass::FullFade
    } else if boundary_db > FULL_FADE_HIGH_DB {
        EdgeClass::HardCut
    } else if boundary_db < FULL_FADE_LOW_DB {
        EdgeClass::FadeToSilence
    } else {
        EdgeClass::Partial
    }
}

fn locate_ramp(
    window: &[f32],
    resolution_s: f32,
    class: EdgeClass,
    from_start: bool,
) -> (f32, i64, f32) {
    if window.is_empty() {
        return (FULL_FADE_LOW_DB, 0, 0.0);
    }
    let n = window.len();
    let boundary_idx = if from_start { 0 } else { n - 1 };
    let idx = match class {
        EdgeClass::HardCut => boundary_idx,
        EdgeClass::FadeToSilence => {
            find_edge_index(window, FULL_FADE_LOW_DB, from_start).unwrap_or(boundary_idx)
        }
        EdgeClass::FullFade => {
            find_edge_index(window, FULL_FADE_HIGH_DB, from_start).unwrap_or(boundary_idx)
        }
        EdgeClass::Partial => n / 2,
    };

    (
        window[idx],
        index_to_time_from_edge_ms(n, idx, resolution_s, from_start),
        local_slope(window, idx),
    )
}

fn pick_curve(class: EdgeClass, slope: f32) -> CurveType {
    let bungee = class == EdgeClass::HardCut;
    let s = slope.abs();
    match (bungee, s) {
        (true, s) if s >= CURVE_FAST_SLOPE => CurveType::BungeeFast,
        (true, s) if s >= CURVE_MEDIUM_SLOPE => CurveType::BungeeMedium,
        (true, _) => CurveType::BungeeSlow,
        (false, s) if s >= CURVE_FAST_SLOPE => CurveType::LinearFast,
        (false, s) if s >= CURVE_MEDIUM_SLOPE => CurveType::LinearMedium,
        (false, _) => CurveType::LinearSlow,
    }
}

fn resolution(track: &TrackFeatures) -> f32 {
    track.loudness_curve_resolution_seconds.max(0.001)
}

fn tail_window(track: &TrackFeatures, window_s: f32) -> &[f32] {
    let want = (window_s / resolution(track)).ceil() as usize;
    let total = track.loudness_curve_db.len();
    let start = total.saturating_sub(want.max(1));
    &track.loudness_curve_db[start..]
}

fn head_window(track: &TrackFeatures, window_s: f32) -> &[f32] {
    let want = (window_s / resolution(track)).ceil() as usize;
    let end = want.min(track.loudness_curve_db.len());
    &track.loudness_curve_db[..end]
}

fn seconds_to_ms(seconds: f32) -> i64 {
    (seconds.max(0.0) * 1000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge_track(
        duration_s: f32,
        floor_db: f32,
        peak_db: f32,
        ramp_s: f32,
        fade_in_at_start: bool,
    ) -> TrackFeatures {
        const RES: f32 = 0.1;
        let n = (duration_s / RES) as usize;
        let ramp_steps = ((ramp_s / RES) as usize).max(1);
        let mut curve = vec![peak_db; n];

        let steps = ramp_steps.min(n);
        let ramped = |i: usize| floor_db + (i as f32 / ramp_steps as f32) * (peak_db - floor_db);
        if fade_in_at_start {
            for (i, c) in curve.iter_mut().enumerate().take(steps) {
                *c = ramped(i);
            }
        } else {
            for (i, c) in curve.iter_mut().rev().enumerate().take(steps) {
                *c = ramped(i);
            }
        }

        TrackFeatures {
            duration: duration_s,
            loudness_curve_db: curve,
            loudness_curve_resolution_seconds: RES,
            ..Default::default()
        }
    }

    #[test]
    fn matched_fades_validate_strict() {
        let finder = TransitionPointFinder;
        let out_track = edge_track(60.0, -60.0, -6.0, 10.0, false);
        let in_track = edge_track(60.0, -60.0, -6.0, 10.0, true);

        let analysis = finder.analyze(&out_track, &in_track);

        assert_eq!(analysis.search_mode, SmartMixSearchMode::Strict);
        assert_eq!(analysis.parameters.fade_out_curve, CurveType::LinearFast);
        assert_eq!(analysis.parameters.fade_in_curve, CurveType::LinearFast);
        assert!(analysis.volume_delta_db <= STRICT.max_volume_delta_db);
        assert!(analysis.parameters.ramp_out_start_ms < seconds_to_ms(out_track.duration));
        assert!(analysis.parameters.ramp_in_start_ms > 0);
    }

    #[test]
    fn hard_cut_both_sides_falls_back_to_bungee() {
        let finder = TransitionPointFinder;
        let out_track = edge_track(60.0, -6.0, -6.0, 0.1, false);
        let in_track = edge_track(60.0, -6.0, -6.0, 0.1, true);

        let analysis = finder.analyze(&out_track, &in_track);

        assert_eq!(analysis.search_mode, SmartMixSearchMode::Fallback);
        assert!(matches!(
            analysis.parameters.fade_out_curve,
            CurveType::BungeeFast | CurveType::BungeeMedium | CurveType::BungeeSlow
        ));
        assert!(matches!(
            analysis.parameters.fade_in_curve,
            CurveType::BungeeFast | CurveType::BungeeMedium | CurveType::BungeeSlow
        ));
        assert_eq!(
            analysis.parameters.ramp_out_start_ms,
            seconds_to_ms(out_track.duration)
        );
        assert_eq!(analysis.parameters.ramp_in_start_ms, 0);
    }

    #[test]
    fn full_fade_both_sides_falls_back_to_linear() {
        let finder = TransitionPointFinder;
        let out_track = edge_track(60.0, -90.0, -3.0, 0.5, false);
        let in_track = edge_track(60.0, -90.0, -3.0, 0.5, true);

        let analysis = finder.analyze(&out_track, &in_track);

        assert_eq!(analysis.search_mode, SmartMixSearchMode::Fallback);
        assert!(matches!(
            analysis.parameters.fade_out_curve,
            CurveType::LinearFast | CurveType::LinearMedium | CurveType::LinearSlow
        ));
        assert!(analysis.parameters.ramp_out_start_ms > 0);
        assert!(analysis.parameters.ramp_out_start_ms < seconds_to_ms(out_track.duration));
    }

    #[test]
    fn missing_loudness_data_is_a_safe_default() {
        let finder = TransitionPointFinder;
        let out_track = TrackFeatures {
            duration: 30.0,
            ..Default::default()
        };
        let in_track = TrackFeatures {
            duration: 30.0,
            ..Default::default()
        };

        let analysis = finder.analyze(&out_track, &in_track);

        assert_eq!(analysis.search_mode, SmartMixSearchMode::Fallback);
        assert_eq!(
            analysis.parameters.ramp_out_start_ms,
            seconds_to_ms(out_track.duration)
        );
        assert_eq!(analysis.parameters.ramp_in_start_ms, 0);
    }

    #[test]
    fn threshold_ladder_covers_coarse_and_fine_bands() {
        let ladder = threshold_ladder(-24.0, 4.0);
        assert!(ladder.contains(&-120.0));
        assert!(ladder.contains(&-20.0));
        assert!(ladder.contains(&-24.0));
        assert!(ladder.contains(&-28.0));
        assert!(ladder.windows(2).all(|w| w[0] < w[1]));
    }
}
