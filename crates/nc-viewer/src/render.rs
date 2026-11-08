//! Render plans: screen-space polylines ready for a GPUI canvas painter.

use crate::camera2d::Camera2D;
use crate::camera3d::Camera3D;
use crate::geom::{Bounds2, Point2};
use crate::segment::{Segment, SegmentKind};
use crate::toolpath::Toolpath;

/// Screen-space distance under which two projected endpoints are treated as
/// connected. A larger gap starts a new polyline, so disjoint toolpath moves
/// never draw an accidental bridge.
const JOIN_TOLERANCE_PX: f32 = 0.01;

/// Squared join tolerance, used on the hot path to avoid a square root.
const JOIN_TOLERANCE_SQ: f32 = JOIN_TOLERANCE_PX * JOIN_TOLERANCE_PX;

/// A single screen-space polyline with its style and painter depth.
///
/// Consecutive segments of the same kind, layer, and tool that share an
/// endpoint are batched into one polyline. A polyline never contains a
/// discontinuity; the renderer can paint each one as an open path.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    /// Screen-space points in logical pixels.
    pub points: Vec<Point2>,
    /// Painter depth. Larger values are farther away.
    ///
    /// Always `0.0` for the 2D plan; camera-space depth for the 3D plan.
    pub depth: f32,
    /// Motion style of every segment in this polyline.
    pub kind: SegmentKind,
    /// Layer id of every segment in this polyline.
    pub layer: u16,
    /// Tool number of every segment in this polyline.
    pub tool: u16,
}

impl Polyline {
    /// Creates a polyline. A non-finite depth is stored as zero.
    #[must_use]
    pub fn new(points: Vec<Point2>, depth: f32, kind: SegmentKind, layer: u16, tool: u16) -> Self {
        Self {
            points,
            depth: if depth.is_finite() { depth } else { 0.0 },
            kind,
            layer,
            tool,
        }
    }

    /// Number of points in the polyline.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// Returns `true` when the polyline has no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// Controls for building a [`RenderPlan`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// Maximum screen-space deviation, in logical pixels, allowed when
    /// simplifying a 2D polyline. Values at or below zero disable decimation.
    pub decimation_tolerance_px: f32,
    /// Drop polylines whose screen bounds do not intersect the viewport.
    pub cull: bool,
    /// Extra margin, in logical pixels, added to the viewport for culling.
    pub cull_margin_px: f32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            decimation_tolerance_px: 0.35,
            cull: true,
            cull_margin_px: 0.0,
        }
    }
}

/// A frame's worth of screen-space polylines.
///
/// The plan owns its point buffers. For the 2D view, polylines are in toolpath
/// order and pre-simplified for the current zoom. For the 3D view, they are
/// sorted far to near so a painter can draw them in order. Rebuild a plan when
/// the camera or toolpath changes; the toolpath itself is never copied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderPlan {
    /// Polylines in painter order.
    pub polylines: Vec<Polyline>,
}

impl RenderPlan {
    /// Creates an empty plan.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an orthographic top-view plan.
    ///
    /// Consecutive segments of the same style and layer/tool are batched,
    /// projected, optionally simplified in screen space (RDP, so both
    /// endpoints of every polyline are preserved), culled, and returned in
    /// toolpath order.
    #[must_use]
    pub fn from_2d(toolpath: &Toolpath, camera: &Camera2D, options: RenderOptions) -> Self {
        let mut plan = Self::new();
        plan.polylines.reserve(toolpath.segment_count() / 32 + 1);
        let cull_bounds = camera.viewport().bounds().expanded(options.cull_margin_px);
        let center = camera.viewport().center();
        let offset = camera.offset();
        let scale = camera.scale();
        let to_screen = |world: Point2| {
            Point2::new(
                center.x + (world.x - offset.x) * scale,
                center.y - (world.y - offset.y) * scale,
            )
        };
        let mut decimator = Decimator::default();
        let mut run: Option<RunBuilder> = None;

        for segment in toolpath.segments() {
            let start = to_screen(segment.start.xy());
            let end = to_screen(segment.end.xy());
            if !start.is_finite() || !end.is_finite() {
                finish_run(
                    &mut plan,
                    run.take(),
                    &options,
                    cull_bounds,
                    Some(&mut decimator),
                );
                continue;
            }
            let continues = run
                .as_ref()
                .is_some_and(|builder| builder.can_continue(segment, start));
            if !continues {
                finish_run(
                    &mut plan,
                    run.take(),
                    &options,
                    cull_bounds,
                    Some(&mut decimator),
                );
                let mut builder = RunBuilder::new(segment, start);
                builder.push_end(end);
                run = Some(builder);
            } else if let Some(builder) = run.as_mut() {
                builder.push_end(end);
            }
        }
        finish_run(&mut plan, run, &options, cull_bounds, Some(&mut decimator));
        plan
    }

    /// Builds a projected, culled, depth-sorted 3D plan.
    ///
    /// Segments clipped by the near plane are dropped, which can split a
    /// batch. Consecutive visible segments stay batched; each polyline's depth
    /// is the average depth of its segments. Polylines are sorted farthest
    /// first for a painter's algorithm.
    #[must_use]
    pub fn from_3d(toolpath: &Toolpath, camera: &Camera3D, options: RenderOptions) -> Self {
        let mut plan = Self::new();
        plan.polylines.reserve(toolpath.segment_count() / 32 + 1);
        let cull_bounds = camera.viewport().bounds().expanded(options.cull_margin_px);
        let projector = camera.projector();
        let mut run: Option<RunBuilder> = None;

        for segment in toolpath.segments() {
            let (Some(start), Some(end)) = (
                projector.project(segment.start),
                projector.project(segment.end),
            ) else {
                finish_run(&mut plan, run.take(), &options, cull_bounds, None);
                continue;
            };
            let midpoint_depth = (start.depth + end.depth) * 0.5;
            if !midpoint_depth.is_finite() {
                finish_run(&mut plan, run.take(), &options, cull_bounds, None);
                continue;
            }
            let continues = run
                .as_ref()
                .is_some_and(|builder| builder.can_continue(segment, start.screen));
            if !continues {
                finish_run(&mut plan, run.take(), &options, cull_bounds, None);
                let mut builder = RunBuilder::new(segment, start.screen);
                builder.add_depth(midpoint_depth);
                builder.push_end(end.screen);
                run = Some(builder);
            } else if let Some(builder) = run.as_mut() {
                builder.add_depth(midpoint_depth);
                builder.push_end(end.screen);
            }
        }
        finish_run(&mut plan, run, &options, cull_bounds, None);
        plan.sort_far_to_near();
        plan
    }

    /// Number of polylines in the plan.
    #[must_use]
    pub fn len(&self) -> usize {
        self.polylines.len()
    }

    /// Returns `true` when the plan has no polylines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.polylines.is_empty()
    }

    /// Total number of screen points in the plan.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.polylines.iter().map(Polyline::point_count).sum()
    }

    /// Read-only access to the polylines.
    #[must_use]
    pub fn polylines(&self) -> &[Polyline] {
        &self.polylines
    }

    /// Takes ownership of the polylines.
    #[must_use]
    pub fn into_polylines(self) -> Vec<Polyline> {
        self.polylines
    }

    /// Removes every polyline, retaining the plan's outer allocation.
    pub fn clear(&mut self) {
        self.polylines.clear();
    }

    /// Sorts polylines farthest first for a painter's algorithm.
    pub fn sort_far_to_near(&mut self) {
        self.polylines.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    }
}

/// Incrementally builds one batched polyline.
struct RunBuilder {
    points: Vec<Point2>,
    kind: SegmentKind,
    layer: u16,
    tool: u16,
    depth_sum: f64,
    depth_count: u32,
}

impl RunBuilder {
    fn new(segment: &Segment, start: Point2) -> Self {
        let mut points = Vec::with_capacity(16);
        points.push(start);
        Self {
            points,
            kind: segment.kind,
            layer: segment.layer,
            tool: segment.tool,
            depth_sum: 0.0,
            depth_count: 0,
        }
    }

    fn can_continue(&self, segment: &Segment, start: Point2) -> bool {
        self.kind == segment.kind
            && self.layer == segment.layer
            && self.tool == segment.tool
            && self
                .points
                .last()
                .is_some_and(|last| last.distance_squared(start) <= JOIN_TOLERANCE_SQ)
    }

    fn push_end(&mut self, end: Point2) {
        if self
            .points
            .last()
            .is_some_and(|last| last.distance_squared(end) <= 1.0e-12)
        {
            return;
        }
        self.points.push(end);
    }

    fn add_depth(&mut self, depth: f32) {
        if depth.is_finite() {
            self.depth_sum += f64::from(depth);
            self.depth_count = self.depth_count.saturating_add(1);
        }
    }

    fn finish(self) -> Polyline {
        let depth = if self.depth_count > 0 {
            (self.depth_sum / f64::from(self.depth_count)) as f32
        } else {
            0.0
        };
        Polyline::new(self.points, depth, self.kind, self.layer, self.tool)
    }
}

fn finish_run(
    plan: &mut RenderPlan,
    run: Option<RunBuilder>,
    options: &RenderOptions,
    cull_bounds: Bounds2,
    decimator: Option<&mut Decimator>,
) {
    let Some(builder) = run else {
        return;
    };
    let mut polyline = builder.finish();
    if polyline.points.is_empty() {
        return;
    }
    if options.cull && !is_visible(&polyline.points, cull_bounds) {
        return;
    }
    if let Some(decimator) = decimator
        && options.decimation_tolerance_px.is_finite()
        && options.decimation_tolerance_px > 0.0
    {
        decimator.simplify(&mut polyline.points, options.decimation_tolerance_px);
    }
    if !polyline.points.is_empty() {
        plan.polylines.push(polyline);
    }
}

fn is_visible(points: &[Point2], cull_bounds: Bounds2) -> bool {
    let bounds = Bounds2::from_points(points.iter().copied());
    bounds.intersects(&cull_bounds)
}

/// Reusable Ramer-Douglas-Peucker workspace.
///
/// The keep mask and traversal stack survive between polylines, so a frame
/// allocates only the polyline point buffers themselves.
#[derive(Default)]
struct Decimator {
    keep: Vec<bool>,
    stack: Vec<(usize, usize)>,
}

impl Decimator {
    /// Simplifies `points` in place. First and last points are always kept,
    /// and every removed point is within `tolerance_px` of the result.
    fn simplify(&mut self, points: &mut Vec<Point2>, tolerance_px: f32) {
        let count = points.len();
        if count <= 2 || !tolerance_px.is_finite() || tolerance_px <= 0.0 {
            return;
        }
        let tolerance_sq = tolerance_px * tolerance_px;

        self.keep.clear();
        self.keep.resize(count, false);
        self.keep[0] = true;
        self.keep[count - 1] = true;

        self.stack.clear();
        self.stack.push((0, count - 1));
        while let Some((first, last)) = self.stack.pop() {
            if last <= first + 1 {
                continue;
            }
            let mut farthest_index = first + 1;
            let mut farthest_distance = -1.0;
            for index in (first + 1)..last {
                let distance =
                    point_segment_distance_squared(points[index], points[first], points[last]);
                if distance > farthest_distance {
                    farthest_distance = distance;
                    farthest_index = index;
                }
            }
            if farthest_distance > tolerance_sq {
                self.keep[farthest_index] = true;
                self.stack.push((first, farthest_index));
                self.stack.push((farthest_index, last));
            }
        }

        let mut write = 0;
        for read in 0..count {
            if self.keep[read] {
                if write != read {
                    points[write] = points[read];
                }
                write += 1;
            }
        }
        points.truncate(write);
    }
}

fn point_segment_distance_squared(point: Point2, start: Point2, end: Point2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if !length_squared.is_finite() || length_squared <= 0.0 {
        return point.distance_squared(start);
    }
    let t = (point - start).dot(segment) / length_squared;
    let t = t.clamp(0.0, 1.0);
    point.distance_squared(start + segment * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera3d::Projection;
    use crate::geom::Point3;
    use crate::toolpath::Toolpath;
    use std::f32::consts::PI;

    fn approx(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() <= epsilon
    }

    fn dense_path(count: usize) -> Toolpath {
        let mut toolpath = Toolpath::with_capacity(count);
        let mut previous = Point3::new(0.0, 0.0, 0.0);
        for index in 0..count {
            let x = index as f32 * 0.05;
            let y = (index as f32 * 0.37).sin() * 0.001;
            let next = Point3::new(x + 0.05, y, 0.0);
            assert!(toolpath.append_line(previous, next, SegmentKind::Feed, 0, 0));
            previous = next;
        }
        toolpath
    }

    #[test]
    fn empty_toolpath_produces_empty_plans() {
        let toolpath = Toolpath::new();
        let camera2d = Camera2D::new(crate::viewport::Viewport::new(800.0, 600.0));
        let camera3d = Camera3D::new(crate::viewport::Viewport::new(800.0, 600.0));
        let plan2d = RenderPlan::from_2d(&toolpath, &camera2d, RenderOptions::default());
        let plan3d = RenderPlan::from_3d(&toolpath, &camera3d, RenderOptions::default());
        assert!(plan2d.is_empty());
        assert!(plan3d.is_empty());
        assert_eq!(plan2d.point_count(), 0);
        assert_eq!(plan3d.point_count(), 0);
    }

    #[test]
    fn single_point_is_safe() {
        let mut toolpath = Toolpath::new();
        let point = Point3::new(1.0, 2.0, 0.0);
        toolpath.append_line(point, point, SegmentKind::Feed, 0, 0);

        let mut camera2d = Camera2D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera2d.fit(toolpath.bounds().xy_bounds(), 0.0);
        let plan = RenderPlan::from_2d(&toolpath, &camera2d, RenderOptions::default());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.point_count(), 1);

        let mut camera3d = Camera3D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera3d.set_target(point);
        let plan = RenderPlan::from_3d(&toolpath, &camera3d, RenderOptions::default());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.point_count(), 1);
    }

    #[test]
    fn decimation_preserves_endpoints_and_reduces_points() {
        let toolpath = dense_path(5000);
        let mut camera = Camera2D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.fit(toolpath.bounds().xy_bounds(), 0.0);

        let plan = RenderPlan::from_2d(&toolpath, &camera, RenderOptions::default());
        assert_eq!(plan.len(), 1);
        let polyline = &plan.polylines[0];
        assert!(
            polyline.points.len() < 20,
            "decimation kept {} points",
            polyline.points.len()
        );
        let first = toolpath.segments()[0].start.xy();
        let last = toolpath.segments().last().expect("last").end.xy();
        assert_eq!(polyline.points[0], camera.world_to_screen(first));
        assert_eq!(
            *polyline.points.last().expect("last"),
            camera.world_to_screen(last)
        );

        let raw = RenderPlan::from_2d(
            &toolpath,
            &camera,
            RenderOptions {
                decimation_tolerance_px: 0.0,
                ..RenderOptions::default()
            },
        );
        assert_eq!(raw.point_count(), 5001);
        assert!(polyline.points.len() < raw.point_count() / 10);
    }

    #[test]
    fn decimation_stays_within_the_screen_tolerance() {
        let mut toolpath = Toolpath::with_capacity(1000);
        let mut previous = Point3::new(0.0, 0.0, 0.0);
        for index in 0..1000 {
            let x = index as f32 * 0.1;
            let y = (index as f32 * 0.2).sin() * 5.0;
            let next = Point3::new(x + 0.1, y, 0.0);
            toolpath.append_line(previous, next, SegmentKind::Feed, 0, 0);
            previous = next;
        }
        let mut camera = Camera2D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.fit(toolpath.bounds().xy_bounds(), 0.0);
        let tolerance = 0.35;
        let plan = RenderPlan::from_2d(
            &toolpath,
            &camera,
            RenderOptions {
                decimation_tolerance_px: tolerance,
                ..RenderOptions::default()
            },
        );
        assert_eq!(plan.len(), 1);
        let simplified = &plan.polylines[0].points;
        assert!(simplified.len() < 1001);

        let mut original: Vec<Point2> = Vec::with_capacity(1001);
        original.push(camera.world_to_screen(toolpath.segments()[0].start.xy()));
        for segment in toolpath.segments() {
            original.push(camera.world_to_screen(segment.end.xy()));
        }
        for point in original {
            let distance = distance_to_polyline(point, simplified);
            assert!(
                distance <= tolerance + 1.0e-3,
                "removed point is {distance} px from the simplified path"
            );
        }
    }

    fn distance_to_polyline(point: Point2, polyline: &[Point2]) -> f32 {
        let mut best = f32::INFINITY;
        for pair in polyline.windows(2) {
            let segment = pair[1] - pair[0];
            let length_squared = segment.length_squared();
            let t = if length_squared > 0.0 {
                ((point - pair[0]).dot(segment) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            best = best.min(point.distance(pair[0] + segment * t));
        }
        best
    }

    #[test]
    fn styles_and_layers_batch_separately() {
        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        toolpath.append_line(
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        toolpath.append_line(
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(3.0, 0.0, 0.0),
            SegmentKind::Rapid,
            0,
            0,
        );
        toolpath.append_line(
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            SegmentKind::Rapid,
            1,
            0,
        );
        let camera = Camera2D::fitted(
            crate::viewport::Viewport::new(800.0, 600.0),
            toolpath.bounds().xy_bounds(),
            10.0,
        );
        let plan = RenderPlan::from_2d(
            &toolpath,
            &camera,
            RenderOptions {
                decimation_tolerance_px: 0.0,
                ..RenderOptions::default()
            },
        );
        assert_eq!(plan.len(), 3);
        assert_eq!(plan.polylines[0].kind, SegmentKind::Feed);
        assert_eq!(plan.polylines[0].point_count(), 3);
        assert_eq!(plan.polylines[1].kind, SegmentKind::Rapid);
        assert_eq!(plan.polylines[2].layer, 1);
    }

    #[test]
    fn disjoint_moves_do_not_join() {
        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        toolpath.append_line(
            Point3::new(10.0, 0.0, 0.0),
            Point3::new(11.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        let camera = Camera2D::fitted(
            crate::viewport::Viewport::new(800.0, 600.0),
            toolpath.bounds().xy_bounds(),
            0.0,
        );
        let plan = RenderPlan::from_2d(&toolpath, &camera, RenderOptions::default());
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn culling_drops_offscreen_geometry() {
        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        let mut camera = Camera2D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.set_offset(Point2::new(10_000.0, 10_000.0));
        let plan = RenderPlan::from_2d(&toolpath, &camera, RenderOptions::default());
        assert!(plan.is_empty());

        let plan = RenderPlan::from_2d(
            &toolpath,
            &camera,
            RenderOptions {
                cull: false,
                ..RenderOptions::default()
            },
        );
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn three_d_plan_sorts_far_to_near() {
        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            Point3::new(-20.0, -50.0, 0.0),
            Point3::new(-19.0, -40.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        toolpath.append_line(
            Point3::new(20.0, 50.0, 0.0),
            Point3::new(21.0, 60.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        let mut camera = Camera3D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.set_yaw(-PI * 0.5);
        camera.set_pitch(0.0);
        camera.set_distance(100.0);
        camera.set_target(Point3::ZERO);

        let plan = RenderPlan::from_3d(&toolpath, &camera, RenderOptions::default());
        assert_eq!(plan.len(), 2);
        assert!(plan.polylines[0].depth > plan.polylines[1].depth);
        assert_eq!(plan.polylines[0].kind, SegmentKind::Feed);
    }

    #[test]
    fn three_d_culls_geometry_behind_the_camera() {
        let mut camera = Camera3D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.set_yaw(-PI * 0.5);
        camera.set_pitch(0.0);
        camera.set_distance(100.0);
        let eye = camera.eye();
        let forward = camera.forward();
        let behind = eye - forward * 10.0;

        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            behind,
            behind + Point3::new(5.0, 0.0, 0.0),
            SegmentKind::Rapid,
            0,
            0,
        );
        let plan = RenderPlan::from_3d(&toolpath, &camera, RenderOptions::default());
        assert!(plan.is_empty());
    }

    #[test]
    fn three_d_preserves_style_and_metadata() {
        let mut toolpath = Toolpath::new();
        let point = Point3::new(0.0, 0.0, 0.0);
        toolpath.append_line(point, point, SegmentKind::Probe, 4, 9);
        let mut camera = Camera3D::new(crate::viewport::Viewport::new(800.0, 600.0));
        camera.set_target(point);
        let plan = RenderPlan::from_3d(
            &toolpath,
            &camera,
            RenderOptions {
                cull: false,
                ..RenderOptions::default()
            },
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.polylines[0].kind, SegmentKind::Probe);
        assert_eq!(plan.polylines[0].layer, 4);
        assert_eq!(plan.polylines[0].tool, 9);
    }

    #[test]
    fn orthographic_three_d_plan_uses_screen_scale() {
        let mut toolpath = Toolpath::new();
        toolpath.append_line(
            Point3::new(-10.0, 0.0, 0.0),
            Point3::new(10.0, 0.0, 0.0),
            SegmentKind::Feed,
            0,
            0,
        );
        let mut camera = Camera3D::with_projection(
            crate::viewport::Viewport::new(800.0, 600.0),
            Projection::orthographic(10.0),
        );
        camera.set_yaw(-PI * 0.5);
        camera.set_pitch(0.0);
        camera.set_target(Point3::ZERO);
        camera.set_distance(100.0);
        let plan = RenderPlan::from_3d(&toolpath, &camera, RenderOptions::default());
        assert_eq!(plan.len(), 1);
        let polyline = &plan.polylines[0];
        assert!(approx(polyline.points[0].x, 400.0 - 100.0, 1.0e-3));
        assert!(approx(
            polyline.points.last().expect("end").x,
            400.0 + 100.0,
            1.0e-3
        ));
    }

    #[test]
    fn synthetic_path_produces_a_plan() {
        let toolpath = crate::toolpath::synthetic_toolpath(10_000);
        let camera = Camera2D::fitted(
            crate::viewport::Viewport::new(1280.0, 800.0),
            toolpath.bounds().xy_bounds(),
            20.0,
        );
        let plan = RenderPlan::from_2d(&toolpath, &camera, RenderOptions::default());
        assert!(!plan.is_empty());
        assert!(plan.point_count() > 0);
        assert!(plan.len() <= toolpath.len());

        let camera3d = Camera3D::fitted(
            crate::viewport::Viewport::new(1280.0, 800.0),
            toolpath.bounds(),
            20.0,
        );
        let plan3d = RenderPlan::from_3d(&toolpath, &camera3d, RenderOptions::default());
        assert!(!plan3d.is_empty());
        assert!(
            plan3d
                .polylines
                .windows(2)
                .all(|pair| pair[0].depth >= pair[1].depth)
        );
    }
}
