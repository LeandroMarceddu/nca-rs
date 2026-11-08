//! Exact arc and helix geometry shared by the stock sweep and the simulator.
//!
//! The parser stores arcs exactly; the sweep and the timeline must not fall
//! back to display chords. This module owns the plane basis, the exact point on
//! an arc at a normalized parameter, and the analytic arc extents used by stock
//! inference and collision checks.

use nc_core::math;
use nc_gcode::{ArcRecord, Plane};

/// Returns the `(first, second, normal)` axis indices of a plane.
///
/// `Xy` is `(X, Y, Z)`, `Zx` is `(Z, X, Y)`, and `Yz` is `(Y, Z, X)`. A
/// positive arc sweep is counter-clockwise about the plane's positive normal.
#[must_use]
pub const fn plane_axes(plane: Plane) -> (usize, usize, usize) {
    match plane {
        Plane::Xy => (0, 1, 2),
        Plane::Zx => (2, 0, 1),
        Plane::Yz => (1, 2, 0),
    }
}

/// Returns the exact point on an arc at `fraction` in `0.0..=1.0`.
///
/// The endpoints are returned verbatim at `0` and `1`, so a slightly
/// inconsistent recorded radius never introduces a step at a span boundary.
/// Interior points use the recorded center, start angle, and radius. The
/// normal-axis coordinate is interpolated between `from_mm` and `to_mm`, which
/// matches `ArcRecord::axial_delta_mm` for a consistent record.
#[must_use]
pub fn arc_point(
    arc: &ArcRecord,
    plane: Plane,
    from_mm: [f64; 3],
    to_mm: [f64; 3],
    fraction: f64,
) -> [f64; 3] {
    if !fraction.is_finite() || fraction <= 0.0 {
        return from_mm;
    }
    if fraction >= 1.0 {
        return to_mm;
    }
    let radius = if arc.radius_mm.is_finite() && arc.radius_mm > 0.0 {
        arc.radius_mm
    } else {
        return lerp_point(from_mm, to_mm, fraction);
    };
    if !arc.start_angle_rad.is_finite() || !arc.sweep_rad.is_finite() {
        return lerp_point(from_mm, to_mm, fraction);
    }
    let (u, v, normal) = plane_axes(plane);
    let angle = arc.start_angle_rad + arc.sweep_rad * fraction;
    let mut point = [0.0; 3];
    point[u] = arc.center_mm[u] + radius * math::cos(angle);
    point[v] = arc.center_mm[v] + radius * math::sin(angle);
    point[normal] = from_mm[normal] + (to_mm[normal] - from_mm[normal]) * fraction;
    point
}

/// Returns the axis-aligned extents of a complete arc including its endpoints.
///
/// The normal axis is linear, so only the two endpoints contribute there. The
/// two in-plane axes are checked at their four cardinal extrema and included
/// only when the corresponding angle lies inside the sweep.
#[must_use]
pub fn arc_bounds(
    arc: &ArcRecord,
    plane: Plane,
    from_mm: [f64; 3],
    to_mm: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for axis in 0..3 {
        min[axis] = from_mm[axis].min(to_mm[axis]);
        max[axis] = from_mm[axis].max(to_mm[axis]);
    }
    let radius = if arc.radius_mm.is_finite() && arc.radius_mm > 0.0 {
        arc.radius_mm
    } else {
        return (min, max);
    };
    if !arc.start_angle_rad.is_finite() || !arc.sweep_rad.is_finite() {
        return (min, max);
    }
    let (u, v, _) = plane_axes(plane);
    let half_pi = math::FRAC_PI_2;
    // (axis, angle, signed radius) for the four in-plane extrema.
    let candidates = [
        (u, 0.0, radius),
        (u, math::PI, -radius),
        (v, half_pi, radius),
        (v, 3.0 * half_pi, -radius),
    ];
    for (axis, angle, signed_radius) in candidates {
        if angle_in_sweep(arc.start_angle_rad, arc.sweep_rad, angle) {
            let value = arc.center_mm[axis] + signed_radius;
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    (min, max)
}

/// Returns `true` when `target_rad` lies on the arc from `start_rad` through
/// `sweep_rad`.
#[must_use]
pub fn angle_in_sweep(start_rad: f64, sweep_rad: f64, target_rad: f64) -> bool {
    if !start_rad.is_finite() || !sweep_rad.is_finite() || !target_rad.is_finite() {
        return false;
    }
    if math::abs(sweep_rad) >= math::TAU - 1.0e-12 {
        return true;
    }
    if sweep_rad > 0.0 {
        let relative = normalize_angle(target_rad - start_rad);
        relative <= sweep_rad + 1.0e-12
    } else if sweep_rad < 0.0 {
        let relative = normalize_angle(start_rad - target_rad);
        relative <= -sweep_rad + 1.0e-12
    } else {
        false
    }
}

fn lerp_point(from_mm: [f64; 3], to_mm: [f64; 3], fraction: f64) -> [f64; 3] {
    [
        from_mm[0] + (to_mm[0] - from_mm[0]) * fraction,
        from_mm[1] + (to_mm[1] - from_mm[1]) * fraction,
        from_mm[2] + (to_mm[2] - from_mm[2]) * fraction,
    ]
}

fn normalize_angle(angle: f64) -> f64 {
    let wrapped = angle - math::TAU * math::floor(angle / math::TAU);
    if wrapped < 0.0 {
        wrapped + math::TAU
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_gcode::ArcFormat;

    fn arc(start_angle_rad: f64, sweep_rad: f64) -> ArcRecord {
        ArcRecord {
            center_mm: [10.0, 10.0, 0.0],
            radius_mm: 5.0,
            end_radius_mm: 5.0,
            start_angle_rad,
            sweep_rad,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        }
    }

    #[test]
    fn quarter_arc_point_is_exact() {
        let record = arc(0.0, math::FRAC_PI_2);
        let from = [15.0, 10.0, 0.0];
        let to = [10.0, 15.0, 0.0];
        let middle = arc_point(&record, Plane::Xy, from, to, 0.5);
        let expected = 5.0 * math::sqrt(0.5);
        assert!((middle[0] - (10.0 + expected)).abs() < 1e-12);
        assert!((middle[1] - (10.0 + expected)).abs() < 1e-12);
        assert_eq!(arc_point(&record, Plane::Xy, from, to, 0.0), from);
        assert_eq!(arc_point(&record, Plane::Xy, from, to, 1.0), to);
    }

    #[test]
    fn full_circle_bounds_include_every_extreme() {
        let record = arc(0.0, math::TAU);
        let point = [15.0, 10.0, 0.0];
        let (min, max) = arc_bounds(&record, Plane::Xy, point, point);
        assert_eq!(min, [5.0, 5.0, 0.0]);
        assert_eq!(max, [15.0, 15.0, 0.0]);
    }

    #[test]
    fn partial_arc_bounds_only_include_angles_inside_the_sweep() {
        let record = arc(0.0, math::FRAC_PI_2);
        let from = [15.0, 10.0, 0.0];
        let to = [10.0, 15.0, 0.0];
        let (min, max) = arc_bounds(&record, Plane::Xy, from, to);
        assert_eq!(min, [10.0, 10.0, 0.0]);
        assert_eq!(max, [15.0, 15.0, 0.0]);
    }

    #[test]
    fn zx_plane_uses_z_then_x() {
        let record = ArcRecord {
            center_mm: [0.0, 2.0, 10.0],
            radius_mm: 2.0,
            end_radius_mm: 2.0,
            start_angle_rad: 0.0,
            sweep_rad: math::FRAC_PI_2,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let from = [0.0, 2.0, 12.0];
        let to = [2.0, 2.0, 10.0];
        let middle = arc_point(&record, Plane::Zx, from, to, 0.5);
        let expected = 2.0 * math::sqrt(0.5);
        assert!((middle[2] - (10.0 + expected)).abs() < 1e-12);
        assert!((middle[0] - expected).abs() < 1e-12);
        assert_eq!(middle[1], 2.0);
    }
}
