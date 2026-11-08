//! Circular arcs and chord-tolerance tessellation.
//!
//! An [`Arc`] is planar: it lies in a coordinate plane and its normal
//! coordinate is taken from the center point. Helical arcs (a simultaneous
//! Z move on an XY arc) are not represented yet; the g-code layer must split
//! those into small planar arcs or line segments.

use std::f32::consts::{PI, TAU};

use crate::geom::{Bounds3, Point2, Point3};

/// The chord tolerance used when a caller does not supply a positive one.
///
/// This is in world units (millimetres), matching the toolpath geometry.
pub const DEFAULT_CHORD_TOLERANCE: f32 = 0.02;

/// Upper bound on the number of chords generated for one arc.
///
/// The cap protects the viewer from an accidentally tiny tolerance turning a
/// single arc into millions of segments. It is far above any useful display
/// resolution: 65,536 chords around a full circle is a 0.000096 radian step.
pub const MAX_ARC_SEGMENTS: usize = 65_536;

/// The plane containing a circular arc.
///
/// The `(u, v)` basis is right-handed with the plane normal, and matches
/// grblHAL's arc axis ordering:
///
/// | Plane | u | v | Normal |
/// |---|---|---|---|
/// | [`ArcPlane::Xy`] | X | Y | +Z |
/// | [`ArcPlane::Xz`] | Z | X | +Y |
/// | [`ArcPlane::Yz`] | Y | Z | +X |
///
/// Note that G18 is the ZX plane: its first in-plane axis is Z, not X. This
/// keeps clockwise/counter-clockwise direction consistent with the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArcPlane {
    /// G17, the XY plane, with +Z as its normal.
    Xy,
    /// G18, the ZX plane, with +Y as its normal.
    Xz,
    /// G19, the YZ plane, with +X as its normal.
    Yz,
}

impl ArcPlane {
    /// Projects a 3D point onto this plane's `(u, v)` coordinates.
    #[must_use]
    pub fn project(self, point: Point3) -> Point2 {
        match self {
            Self::Xy => Point2::new(point.x, point.y),
            Self::Xz => Point2::new(point.z, point.x),
            Self::Yz => Point2::new(point.y, point.z),
        }
    }

    /// Builds a 3D point from in-plane coordinates plus a normal coordinate.
    #[must_use]
    pub fn unproject(self, uv: Point2, normal: f32) -> Point3 {
        match self {
            Self::Xy => Point3::new(uv.x, uv.y, normal),
            Self::Xz => Point3::new(uv.y, normal, uv.x),
            Self::Yz => Point3::new(normal, uv.x, uv.y),
        }
    }

    /// Returns the coordinate of `point` along the plane normal.
    #[must_use]
    pub fn normal_coordinate(self, point: Point3) -> f32 {
        match self {
            Self::Xy => point.z,
            Self::Xz => point.y,
            Self::Yz => point.x,
        }
    }
}

/// A circular arc in a coordinate plane.
///
/// Angles are radians measured from the plane's `u` axis toward its `v` axis.
/// The stored sweep is signed: positive is counter-clockwise in the `(u, v)`
/// frame, negative is clockwise. A sweep of `+/- 2*pi` is a full circle.
///
/// Constructors reject zero, negative, and non-finite radii, so an `Arc` that
/// exists always has a usable radius. Tessellation never panics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arc {
    plane: ArcPlane,
    center: Point3,
    radius: f32,
    start_angle: f32,
    sweep: f32,
    clockwise: bool,
}

impl Arc {
    /// Creates an arc from start and end angles.
    ///
    /// `start_angle` and `end_angle` may be any finite radians and are wrapped
    /// into one revolution. If the directed sweep is zero (for example
    /// `start_angle == end_angle`, the G-code full-circle convention), a full
    /// circle is created. `clockwise` selects the direction of travel.
    ///
    /// Returns `None` when the radius is not finite and positive, when the
    /// center or angles are non-finite, or when the resulting coordinates
    /// would overflow `f32`.
    #[must_use]
    pub fn new(
        plane: ArcPlane,
        center: Point3,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        clockwise: bool,
    ) -> Option<Self> {
        if !center.is_finite()
            || !radius.is_finite()
            || radius <= 0.0
            || !start_angle.is_finite()
            || !end_angle.is_finite()
        {
            return None;
        }
        if !extent_is_finite(center, radius) {
            return None;
        }
        let start = start_angle.rem_euclid(TAU);
        let end = end_angle.rem_euclid(TAU);
        let sweep = directed_sweep(start, end, clockwise);
        if !sweep.is_finite() {
            return None;
        }
        Some(Self {
            plane,
            center,
            radius,
            start_angle: start,
            sweep,
            clockwise,
        })
    }

    /// Creates a full circle starting at `start_angle`.
    ///
    /// Returns `None` for the same invalid geometry as [`Arc::new`].
    #[must_use]
    pub fn full_circle(
        plane: ArcPlane,
        center: Point3,
        radius: f32,
        start_angle: f32,
        clockwise: bool,
    ) -> Option<Self> {
        Self::new(plane, center, radius, start_angle, start_angle, clockwise)
    }

    /// Creates an arc from its endpoints and center.
    ///
    /// The radius is the in-plane distance from `center` to `start`. The end
    /// angle comes from `end`; a slightly mismatched end radius is tolerated
    /// (common with rounded G-code coordinates) and the generated arc ends on
    /// the circle through `start`, not necessarily exactly on `end`.
    ///
    /// If the planar projections of `start` and `end` coincide, a full circle
    /// is created, matching the G-code convention. The normal-axis coordinate
    /// is taken from `center`, so endpoints with a different Z are projected
    /// onto the plane.
    ///
    /// Returns `None` when the center or endpoints are non-finite, when the
    /// start coincides with the center, or when the endpoint lands on the
    /// center without being a full circle.
    #[must_use]
    pub fn from_endpoints(
        plane: ArcPlane,
        start: Point3,
        end: Point3,
        center: Point3,
        clockwise: bool,
    ) -> Option<Self> {
        if !start.is_finite() || !end.is_finite() || !center.is_finite() {
            return None;
        }
        let center_uv = plane.project(center);
        let start_rel = plane.project(start) - center_uv;
        let end_rel = plane.project(end) - center_uv;
        let radius = start_rel.length();
        if !radius.is_finite() || radius <= 0.0 || !extent_is_finite(center, radius) {
            return None;
        }

        let start_angle = start_rel.y.atan2(start_rel.x);
        let full_epsilon = (radius * 1.0e-5).max(1.0e-6);
        if start_rel.distance(end_rel) <= full_epsilon {
            return Self::full_circle(plane, center, radius, start_angle, clockwise);
        }
        if end_rel.length() <= full_epsilon {
            return None;
        }
        let end_angle = end_rel.y.atan2(end_rel.x);
        Self::new(plane, center, radius, start_angle, end_angle, clockwise)
    }

    /// The plane containing the arc.
    #[must_use]
    pub const fn plane(&self) -> ArcPlane {
        self.plane
    }

    /// The arc center. Its normal-axis coordinate is the arc's normal position.
    #[must_use]
    pub const fn center(&self) -> Point3 {
        self.center
    }

    /// The arc radius in world units.
    #[must_use]
    pub const fn radius(&self) -> f32 {
        self.radius
    }

    /// Whether travel is clockwise in the plane's `(u, v)` frame.
    #[must_use]
    pub const fn clockwise(&self) -> bool {
        self.clockwise
    }

    /// Start angle in radians, wrapped to `[0, 2*pi)`.
    #[must_use]
    pub const fn start_angle(&self) -> f32 {
        self.start_angle
    }

    /// Signed sweep in radians. Negative is clockwise.
    #[must_use]
    pub const fn sweep(&self) -> f32 {
        self.sweep
    }

    /// End angle in radians. This is `start_angle + sweep`, not re-wrapped.
    #[must_use]
    pub fn end_angle(&self) -> f32 {
        self.start_angle + self.sweep
    }

    /// Arc length in world units.
    #[must_use]
    pub fn length(&self) -> f32 {
        self.radius * self.sweep.abs()
    }

    /// Returns `true` for a sweep of one full revolution.
    #[must_use]
    pub fn is_full_circle(&self) -> bool {
        (self.sweep.abs() - TAU).abs() <= 1.0e-4
    }

    /// Returns the 3D point at the start of the arc.
    #[must_use]
    pub fn start_point(&self) -> Point3 {
        self.point_at_angle(self.start_angle)
    }

    /// Returns the 3D point at the end of the arc.
    #[must_use]
    pub fn end_point(&self) -> Point3 {
        self.point_at_angle(self.start_angle + self.sweep)
    }

    /// Returns the 3D point at an arbitrary angle in the arc's plane.
    #[must_use]
    pub fn point_at_angle(&self, angle: f32) -> Point3 {
        let (sin, cos) = angle.sin_cos();
        let center = self.plane.project(self.center);
        let uv = Point2::new(center.x + cos * self.radius, center.y + sin * self.radius);
        self.plane
            .unproject(uv, self.plane.normal_coordinate(self.center))
    }

    /// Returns the 3D point at `fraction` of the sweep, clamped to `0..=1`.
    ///
    /// A non-finite fraction is treated as zero.
    #[must_use]
    pub fn point_at(&self, fraction: f32) -> Point3 {
        let t = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.point_at_angle(self.start_angle + self.sweep * t)
    }

    /// Returns the point at chord vertex `index` of `segments` chords.
    ///
    /// `segments == 0` returns the start point, and `index` is clamped to
    /// `0..=segments`. The last vertex is exactly the arc's end angle. This is
    /// the allocation-free building block used by
    /// [`Toolpath::append_arc`](crate::Toolpath::append_arc).
    #[must_use]
    pub fn point_at_index(&self, index: usize, segments: usize) -> Point3 {
        if segments == 0 {
            return self.start_point();
        }
        let index = index.min(segments);
        let angle = if index == segments {
            self.start_angle + self.sweep
        } else {
            self.start_angle + self.sweep * (index as f32 / segments as f32)
        };
        self.point_at_angle(angle)
    }

    /// Number of chords needed so the sagitta stays within `tolerance`.
    ///
    /// `tolerance` is in world units. Non-finite or non-positive values use
    /// [`DEFAULT_CHORD_TOLERANCE`]. The step angle is capped at 60 degrees so
    /// coarse tolerances still preserve a recognizable shape, and the result
    /// is capped at [`MAX_ARC_SEGMENTS`].
    #[must_use]
    pub fn segment_count(&self, tolerance: f32) -> usize {
        let sweep_abs = self.sweep.abs();
        if !sweep_abs.is_finite() || sweep_abs <= 0.0 {
            return 0;
        }
        let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
            tolerance
        } else {
            DEFAULT_CHORD_TOLERANCE
        };
        let ratio = (tolerance / self.radius).clamp(0.0, 1.0);
        let step = (2.0 * (1.0 - ratio).acos()).min(PI / 3.0);
        if !step.is_finite() || step <= 0.0 {
            return MAX_ARC_SEGMENTS;
        }
        let count = (sweep_abs / step).ceil();
        if !count.is_finite() || count <= 0.0 {
            return 1;
        }
        (count as usize).clamp(1, MAX_ARC_SEGMENTS)
    }

    /// Tessellates the arc into points whose consecutive pairs are chords.
    ///
    /// The returned vector includes both endpoints. Its length is
    /// `segment_count(tolerance) + 1` for valid geometry.
    #[must_use]
    pub fn tessellate(&self, tolerance: f32) -> Vec<Point3> {
        let mut points = Vec::new();
        self.tessellate_into(tolerance, &mut points);
        points
    }

    /// Appends tessellated points to `out` and returns how many were added.
    ///
    /// Existing contents are preserved. Points that would be non-finite are
    /// skipped, which can happen only for extremely large but finite geometry.
    pub fn tessellate_into(&self, tolerance: f32, out: &mut Vec<Point3>) -> usize {
        let segments = self.segment_count(tolerance);
        if segments == 0 {
            return 0;
        }
        let before = out.len();
        out.reserve(segments.saturating_add(1));
        for index in 0..=segments {
            let point = self.point_at_index(index, segments);
            if point.is_finite() {
                out.push(point);
            }
        }
        out.len() - before
    }

    /// Exact axis-aligned bounds of the arc.
    ///
    /// Cardinal points are included when the sweep passes them, so the bounds
    /// are not limited by the current tessellation tolerance.
    #[must_use]
    pub fn bounds(&self) -> Bounds3 {
        let center = self.plane.project(self.center);
        let start = self.plane.project(self.start_point());
        let end = self.plane.project(self.end_point());
        let mut min_u = start.x.min(end.x);
        let mut max_u = start.x.max(end.x);
        let mut min_v = start.y.min(end.y);
        let mut max_v = start.y.max(end.y);

        let cardinals = [
            (0.0, 1.0, 0.0),
            (PI * 0.5, 0.0, 1.0),
            (PI, -1.0, 0.0),
            (PI * 1.5, 0.0, -1.0),
        ];
        for (angle, u, v) in cardinals {
            if self.contains_angle(angle) {
                let u = center.x + u * self.radius;
                let v = center.y + v * self.radius;
                min_u = min_u.min(u);
                max_u = max_u.max(u);
                min_v = min_v.min(v);
                max_v = max_v.max(v);
            }
        }

        let normal = self.plane.normal_coordinate(self.center);
        let low = self.plane.unproject(Point2::new(min_u, min_v), normal);
        let high = self.plane.unproject(Point2::new(max_u, max_v), normal);
        Bounds3::new(low, high)
    }

    fn contains_angle(&self, angle: f32) -> bool {
        let sweep_abs = self.sweep.abs();
        if sweep_abs >= TAU - 1.0e-5 {
            return true;
        }
        let delta = if self.sweep >= 0.0 {
            angle - self.start_angle
        } else {
            self.start_angle - angle
        };
        let delta = delta.rem_euclid(TAU);
        delta <= sweep_abs + 1.0e-5
    }
}

/// Returns the signed directed sweep from `start` to `end`.
///
/// Both inputs are expected to be wrapped to `[0, 2*pi)`. A zero directed
/// difference is treated as a full circle.
fn directed_sweep(start: f32, end: f32, clockwise: bool) -> f32 {
    let mut delta = if clockwise { start - end } else { end - start };
    delta = delta.rem_euclid(TAU);
    if !delta.is_finite() {
        return 0.0;
    }
    if delta <= 1.0e-6 {
        delta = TAU;
    }
    if clockwise { -delta } else { delta }
}

/// Checks that center plus radius cannot overflow an `f32` coordinate.
fn extent_is_finite(center: Point3, radius: f32) -> bool {
    center.x.abs() + radius < f32::MAX
        && center.y.abs() + radius < f32::MAX
        && center.z.abs() + radius < f32::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() <= epsilon
    }

    fn assert_point_near(actual: Point3, expected: Point3, epsilon: f32) {
        assert!(
            actual.distance(expected) <= epsilon,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn quarter_arc_endpoints_and_tolerance() {
        let arc =
            Arc::new(ArcPlane::Xy, Point3::ZERO, 10.0, 0.0, PI * 0.5, false).expect("valid arc");
        assert_point_near(arc.start_point(), Point3::new(10.0, 0.0, 0.0), 1.0e-5);
        assert_point_near(arc.end_point(), Point3::new(0.0, 10.0, 0.0), 1.0e-5);
        assert!(approx(arc.sweep(), PI * 0.5, 1.0e-5));

        let tolerance = 0.05;
        let points = arc.tessellate(tolerance);
        assert_eq!(points.len(), arc.segment_count(tolerance) + 1);
        assert_point_near(points[0], arc.start_point(), 1.0e-5);
        assert_point_near(*points.last().expect("end point"), arc.end_point(), 1.0e-5);

        for point in &points {
            let radius = point.xy().length();
            assert!(approx(radius, 10.0, 1.0e-4));
        }
        for pair in points.windows(2) {
            let chord_midpoint = (pair[0] + pair[1]) * 0.5;
            let sagitta = 10.0 - chord_midpoint.xy().length();
            assert!(
                sagitta <= tolerance + 1.0e-4,
                "sagitta {sagitta} exceeds tolerance {tolerance}"
            );
        }
    }

    #[test]
    fn full_circle_has_closed_endpoints_and_exact_bounds() {
        let arc = Arc::full_circle(ArcPlane::Xy, Point3::new(1.0, 2.0, -3.0), 4.0, 0.3, false)
            .expect("valid circle");
        assert!(arc.is_full_circle());
        assert!(approx(arc.sweep(), TAU, 1.0e-4));
        assert_point_near(arc.start_point(), arc.end_point(), 1.0e-4);

        let points = arc.tessellate(0.01);
        assert!(points.len() >= 7);
        assert_point_near(points[0], *points.last().expect("end point"), 1.0e-4);

        let bounds = arc.bounds();
        assert_point_near(bounds.min(), Point3::new(-3.0, -2.0, -3.0), 1.0e-4);
        assert_point_near(bounds.max(), Point3::new(5.0, 6.0, -3.0), 1.0e-4);
    }

    #[test]
    fn point_at_angle_includes_the_arc_center() {
        let xy = Arc::new(
            ArcPlane::Xy,
            Point3::new(10.0, -5.0, 2.0),
            4.0,
            0.0,
            PI * 0.5,
            false,
        )
        .expect("xy arc");
        assert_point_near(xy.start_point(), Point3::new(14.0, -5.0, 2.0), 1.0e-5);
        assert_point_near(xy.end_point(), Point3::new(10.0, -1.0, 2.0), 1.0e-5);
        for point in xy.tessellate(0.01) {
            let distance = point.xy().distance(Point2::new(10.0, -5.0));
            assert!(approx(distance, 4.0, 1.0e-4), "point {point:?} off radius");
        }

        let zx = Arc::new(
            ArcPlane::Xz,
            Point3::new(1.0, 2.0, 3.0),
            2.0,
            0.0,
            PI * 0.5,
            false,
        )
        .expect("zx arc");
        assert_point_near(zx.start_point(), Point3::new(1.0, 2.0, 5.0), 1.0e-5);
        assert_point_near(zx.end_point(), Point3::new(3.0, 2.0, 3.0), 1.0e-5);

        let yz = Arc::new(
            ArcPlane::Yz,
            Point3::new(1.0, 2.0, 3.0),
            2.0,
            0.0,
            PI * 0.5,
            false,
        )
        .expect("yz arc");
        assert_point_near(yz.start_point(), Point3::new(1.0, 4.0, 3.0), 1.0e-5);
        assert_point_near(yz.end_point(), Point3::new(1.0, 2.0, 5.0), 1.0e-5);
    }

    #[test]
    fn clockwise_and_wrapping_sweeps() {
        let ccw = Arc::new(
            ArcPlane::Xy,
            Point3::ZERO,
            1.0,
            350.0_f32.to_radians(),
            10.0_f32.to_radians(),
            false,
        )
        .expect("valid arc");
        assert!(approx(ccw.sweep(), 20.0_f32.to_radians(), 1.0e-5));

        let cw = Arc::new(
            ArcPlane::Xy,
            Point3::ZERO,
            1.0,
            350.0_f32.to_radians(),
            10.0_f32.to_radians(),
            true,
        )
        .expect("valid arc");
        assert!(approx(cw.sweep(), -340.0_f32.to_radians(), 1.0e-4));

        let quarter_cw =
            Arc::new(ArcPlane::Xy, Point3::ZERO, 1.0, 0.0, PI * 0.5, true).expect("valid arc");
        assert!(approx(quarter_cw.sweep(), -PI * 1.5, 1.0e-5));
    }

    #[test]
    fn from_endpoints_detects_full_circle_and_bad_geometry() {
        let start = Point3::new(5.0, 0.0, 2.0);
        let circle = Arc::from_endpoints(
            ArcPlane::Xy,
            start,
            start,
            Point3::new(0.0, 0.0, 2.0),
            false,
        )
        .expect("full circle");
        assert!(circle.is_full_circle());
        assert!(approx(circle.radius(), 5.0, 1.0e-5));

        let quarter = Arc::from_endpoints(
            ArcPlane::Xy,
            start,
            Point3::new(0.0, 5.0, 2.0),
            Point3::new(0.0, 0.0, 2.0),
            false,
        )
        .expect("quarter arc");
        assert!(!quarter.is_full_circle());
        assert!(approx(quarter.sweep(), PI * 0.5, 1.0e-5));

        assert!(
            Arc::from_endpoints(
                ArcPlane::Xy,
                Point3::ZERO,
                Point3::new(1.0, 0.0, 0.0),
                Point3::ZERO,
                false,
            )
            .is_none()
        );
        assert!(Arc::new(ArcPlane::Xy, Point3::ZERO, 0.0, 0.0, 1.0, false).is_none());
        assert!(Arc::new(ArcPlane::Xy, Point3::ZERO, -1.0, 0.0, 1.0, false).is_none());
        assert!(Arc::new(ArcPlane::Xy, Point3::ZERO, f32::INFINITY, 0.0, 1.0, false).is_none());
    }

    #[test]
    fn plane_mapping_matches_grbl_axis_order() {
        let xz =
            Arc::new(ArcPlane::Xz, Point3::ZERO, 5.0, 0.0, PI * 0.5, false).expect("valid arc");
        // XZ is the ZX plane: angle 0 points along +Z.
        assert_point_near(xz.start_point(), Point3::new(0.0, 0.0, 5.0), 1.0e-5);
        assert_point_near(xz.end_point(), Point3::new(5.0, 0.0, 0.0), 1.0e-5);
        assert_eq!(
            ArcPlane::Xz.project(Point3::new(2.0, 3.0, 4.0)),
            Point2::new(4.0, 2.0)
        );
        assert_eq!(
            ArcPlane::Xz.unproject(Point2::new(4.0, 2.0), 3.0),
            Point3::new(2.0, 3.0, 4.0)
        );

        let yz = ArcPlane::Yz.unproject(Point2::new(2.0, 3.0), 1.0);
        assert_eq!(yz, Point3::new(1.0, 2.0, 3.0));
        assert_eq!(ArcPlane::Yz.normal_coordinate(yz), 1.0);
    }

    #[test]
    fn coarse_tolerance_still_keeps_a_recognizable_circle() {
        for tolerance in [0.0, -1.0, f32::NAN, 1000.0] {
            let arc = Arc::full_circle(ArcPlane::Xy, Point3::ZERO, 1.0, 0.0, false)
                .expect("valid circle");
            let count = arc.segment_count(tolerance);
            assert!(count >= 6, "tolerance {tolerance} produced {count} chords");
            assert!(count <= MAX_ARC_SEGMENTS);
            assert_eq!(arc.tessellate(tolerance).len(), count + 1);
        }
    }

    #[test]
    fn arc_bounds_include_cardinal_extremes() {
        let arc = Arc::new(
            ArcPlane::Xy,
            Point3::new(2.0, -1.0, 0.0),
            3.0,
            PI * 0.25,
            PI * 1.25,
            false,
        )
        .expect("valid arc");
        let bounds = arc.bounds();
        assert!(bounds.contains(Point3::new(2.0, 2.0, 0.0)));
        assert!(bounds.contains(Point3::new(-1.0, -1.0, 0.0)));
        assert!(bounds.min().x <= 2.0 - 3.0 / 2.0_f32.sqrt() + 1.0e-5);
    }
}
