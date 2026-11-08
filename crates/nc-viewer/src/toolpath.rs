//! The flat toolpath segment stream and a deterministic synthetic generator.

use crate::arc::{Arc, ArcPlane, DEFAULT_CHORD_TOLERANCE};
use crate::geom::{Bounds3, Point3};
use crate::segment::{Segment, SegmentKind};

/// A flat, append-only stream of toolpath segments.
///
/// Segments live in one `Vec`; there is no per-segment heap allocation. Bounds
/// are maintained incrementally and include the exact circle bounds of arcs,
/// even when their stored tessellation is coarser.
///
/// Segments with non-finite coordinates are rejected at ingestion, so bounds
/// and render plans never see `NaN` or infinity from this type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Toolpath {
    segments: Vec<Segment>,
    bounds: Bounds3,
}

impl Toolpath {
    /// Creates an empty toolpath.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty toolpath with room for `capacity` segments.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            segments: Vec::with_capacity(capacity),
            bounds: Bounds3::empty(),
        }
    }

    /// Builds a toolpath from existing segments, dropping invalid geometry.
    #[must_use]
    pub fn from_segments(segments: Vec<Segment>) -> Self {
        let mut toolpath = Self::with_capacity(segments.len());
        for segment in segments {
            toolpath.push(segment);
        }
        toolpath
    }

    /// Number of stored segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Number of stored segments; same as [`Toolpath::len`].
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Returns `true` when no segment has been appended.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Read-only access to the flat segment stream.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// World-space bounds of the path.
    ///
    /// Empty for an empty toolpath.
    #[must_use]
    pub fn bounds(&self) -> Bounds3 {
        self.bounds
    }

    /// Approximate memory held by the toolpath in bytes.
    ///
    /// This includes the struct itself and the segment vector's capacity. It
    /// excludes any caller-owned copies of the data.
    #[must_use]
    pub fn memory_estimate(&self) -> usize {
        std::mem::size_of::<Self>() + self.segments.capacity() * std::mem::size_of::<Segment>()
    }

    /// Removes all segments and resets the bounds.
    pub fn clear(&mut self) {
        self.segments.clear();
        self.bounds = Bounds3::empty();
    }

    /// Reserves room for at least `additional` more segments.
    pub fn reserve(&mut self, additional: usize) {
        self.segments.reserve(additional);
    }

    /// Appends one segment.
    ///
    /// Returns `false` and leaves the path unchanged when either endpoint is
    /// non-finite. Zero-length segments are accepted; a plunge viewed from
    /// above is such a segment and the render plan can draw it as a dot.
    pub fn push(&mut self, segment: Segment) -> bool {
        if !segment.is_valid() {
            return false;
        }
        self.bounds.union(&segment.bounds());
        self.segments.push(segment);
        true
    }

    /// Appends one straight segment.
    ///
    /// Returns `false` for non-finite input. `kind` is caller-controlled; the
    /// parser decides whether a line is a rapid, feed, plunge, or probe move.
    pub fn append_line(
        &mut self,
        start: Point3,
        end: Point3,
        kind: SegmentKind,
        layer: u16,
        tool: u16,
    ) -> bool {
        self.push(Segment {
            start,
            end,
            kind,
            layer,
            tool,
        })
    }

    /// Tessellates `arc` and appends its chords as [`SegmentKind::Arc`].
    ///
    /// `chord_tolerance` is the maximum world-space sagitta; non-positive
    /// values use [`DEFAULT_CHORD_TOLERANCE`]. The exact arc bounds are folded
    /// into the path bounds, so program limits are not weakened by a coarse
    /// display tessellation. Returns the number of segments appended.
    pub fn append_arc(&mut self, arc: &Arc, layer: u16, tool: u16, chord_tolerance: f32) -> usize {
        let count = arc.segment_count(chord_tolerance);
        if count == 0 {
            return 0;
        }
        let arc_bounds = arc.bounds();
        if arc_bounds.is_empty() {
            return 0;
        }

        self.segments.reserve(count);
        let mut start = arc.point_at_index(0, count);
        for index in 1..=count {
            let end = arc.point_at_index(index, count);
            self.segments.push(Segment {
                start,
                end,
                kind: SegmentKind::Arc,
                layer,
                tool,
            });
            start = end;
        }
        self.bounds.union(&arc_bounds);
        count
    }

    /// Adds every valid segment from `other` to this path.
    pub fn extend(&mut self, other: &Toolpath) {
        self.segments.reserve(other.segments.len());
        for segment in &other.segments {
            self.push(*segment);
        }
    }
}

/// Builds a deterministic, realistic mix of rapids, feeds, plunges, and arcs.
///
/// The generator models a serpentine pocket: material is cut at a constant Z,
/// the cutter retracts to a safe Z every few hundred segments, rapids to the
/// next row, plunges, and occasionally cuts a quarter, half, or full circle.
/// Coordinates stay in a bounded region suitable for a demo viewport.
///
/// The returned path has exactly `count` segments for every input, and the
/// same sequence on every call and platform. It is intended for benchmarks,
/// UI demos, and tests, not for controller output.
#[must_use]
pub fn synthetic_toolpath(count: usize) -> Toolpath {
    let mut generator = Synthetic::new(count);
    generator.run();
    generator.toolpath
}

const SAFE_Z: f32 = 5.0;
const CUT_Z: f32 = -1.5;
const ROW_LENGTH: f32 = 80.0;
const ROW_STEP: f32 = 3.0;
const ROW_WRAP: u32 = 20;
const FEED_STEP: f32 = 4.0;

/// Small deterministic PRNG; avoids a dependency in a rendering crate.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }

    fn unit(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

struct Synthetic {
    toolpath: Toolpath,
    count: usize,
    rng: Lcg,
    position: Point3,
    direction: f32,
    row: u32,
    next_rapid: usize,
    next_arc: usize,
}

impl Synthetic {
    fn new(count: usize) -> Self {
        Self {
            toolpath: Toolpath::with_capacity(count),
            count,
            rng: Lcg::new(0x5EED_1234_ABCD_0001),
            position: Point3::new(0.0, 0.0, CUT_Z),
            direction: 1.0,
            row: 0,
            next_rapid: 400,
            next_arc: 60,
        }
    }

    fn run(&mut self) {
        while self.toolpath.len() < self.count {
            if self.toolpath.len() >= self.next_rapid && self.try_rapid_row() {
                continue;
            }
            if self.toolpath.len() >= self.next_arc && self.try_arc() {
                continue;
            }
            self.feed_step();
        }
    }

    fn try_rapid_row(&mut self) -> bool {
        let remaining = self.count - self.toolpath.len();
        if remaining < 2 {
            return false;
        }

        self.row = self.row.wrapping_add(1);
        let y = (self.row % ROW_WRAP) as f32 * ROW_STEP;
        let (x, direction) = if self.row.is_multiple_of(2) {
            (0.0, 1.0)
        } else {
            (ROW_LENGTH, -1.0)
        };

        if self.position.z != SAFE_Z {
            self.rapid_to(Point3::new(self.position.x, self.position.y, SAFE_Z));
        }
        self.rapid_to(Point3::new(x, y, SAFE_Z));
        if self.toolpath.len() < self.count {
            self.plunge_to(Point3::new(x, y, CUT_Z));
        }
        self.direction = direction;
        self.next_rapid = self.toolpath.len() + 320 + (self.rng.next_u32() % 160) as usize;
        self.next_arc = self.toolpath.len() + 30 + (self.rng.next_u32() % 50) as usize;
        true
    }

    fn try_arc(&mut self) -> bool {
        let remaining = self.count - self.toolpath.len();
        if remaining < 8 || self.position.z != CUT_Z {
            return false;
        }

        let radius = 2.0 + self.rng.unit() * 8.0;
        let side = if self.rng.next_u32().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let center = Point3::new(self.position.x, self.position.y + side * radius, CUT_Z);
        let start_angle = (self.position.y - center.y).atan2(self.position.x - center.x);
        let clockwise = self.rng.next_u32().is_multiple_of(2);
        let full = self.rng.next_u32().is_multiple_of(5);
        let sweep = if self.rng.next_u32().is_multiple_of(2) {
            std::f32::consts::PI * 0.5
        } else {
            std::f32::consts::PI
        };

        let arc = if full {
            Arc::full_circle(ArcPlane::Xy, center, radius, start_angle, clockwise)
        } else {
            let end_angle = if clockwise {
                start_angle - sweep
            } else {
                start_angle + sweep
            };
            Arc::new(
                ArcPlane::Xy,
                center,
                radius,
                start_angle,
                end_angle,
                clockwise,
            )
        };
        let Some(arc) = arc else {
            return false;
        };
        let chord_count = arc.segment_count(DEFAULT_CHORD_TOLERANCE);
        if chord_count == 0 || chord_count > remaining {
            return false;
        }
        let appended = self
            .toolpath
            .append_arc(&arc, 0, 0, DEFAULT_CHORD_TOLERANCE);
        if appended == 0 {
            return false;
        }
        self.position = arc.end_point();
        self.next_arc = self.toolpath.len() + 40 + (self.rng.next_u32() % 80) as usize;
        true
    }

    fn feed_step(&mut self) {
        let step = FEED_STEP + self.rng.unit() * FEED_STEP;
        let mut end_x = self.position.x + self.direction * step;
        let mut hit_boundary = false;
        if end_x >= ROW_LENGTH {
            end_x = ROW_LENGTH;
            hit_boundary = true;
        } else if end_x <= 0.0 {
            end_x = 0.0;
            hit_boundary = true;
        }
        let end = Point3::new(end_x, self.position.y, CUT_Z);
        let _ = self
            .toolpath
            .append_line(self.position, end, SegmentKind::Feed, 0, 0);
        self.position = end;

        if hit_boundary && self.toolpath.len() < self.count {
            let stepover = Point3::new(end_x, self.position.y + ROW_STEP, CUT_Z);
            let _ = self
                .toolpath
                .append_line(self.position, stepover, SegmentKind::Feed, 0, 0);
            self.position = stepover;
            self.direction = -self.direction;
        }
    }

    fn rapid_to(&mut self, end: Point3) {
        if self.toolpath.len() >= self.count {
            return;
        }
        if self
            .toolpath
            .append_line(self.position, end, SegmentKind::Rapid, 0, 0)
        {
            self.position = end;
        }
    }

    fn plunge_to(&mut self, end: Point3) {
        if self.toolpath.len() >= self.count {
            return;
        }
        if self
            .toolpath
            .append_line(self.position, end, SegmentKind::Plunge, 0, 0)
        {
            self.position = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn empty_toolpath() {
        let toolpath = Toolpath::new();
        assert!(toolpath.is_empty());
        assert_eq!(toolpath.len(), 0);
        assert_eq!(toolpath.segment_count(), 0);
        assert!(toolpath.bounds().is_empty());
        assert!(toolpath.memory_estimate() >= std::mem::size_of::<Toolpath>());
    }

    #[test]
    fn append_line_updates_bounds() {
        let mut toolpath = Toolpath::new();
        assert!(toolpath.append_line(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 5.0, -1.0),
            SegmentKind::Feed,
            3,
            1,
        ));
        assert_eq!(toolpath.len(), 1);
        assert_eq!(toolpath.segments()[0].layer, 3);
        assert_eq!(toolpath.segments()[0].tool, 1);
        assert_eq!(toolpath.bounds().min(), Point3::new(0.0, 0.0, -1.0));
        assert_eq!(toolpath.bounds().max(), Point3::new(10.0, 5.0, 0.0));

        assert!(!toolpath.append_line(
            Point3::new(f32::NAN, 0.0, 0.0),
            Point3::new(1.0, 1.0, 1.0),
            SegmentKind::Feed,
            0,
            0,
        ));
        assert_eq!(toolpath.len(), 1);
    }

    #[test]
    fn append_arc_uses_arc_kind_and_exact_bounds() {
        let mut toolpath = Toolpath::new();
        let arc =
            Arc::full_circle(ArcPlane::Xy, Point3::ZERO, 10.0, 0.0, false).expect("valid circle");
        let count = toolpath.append_arc(&arc, 2, 4, 0.1);
        assert!(count >= 6);
        assert_eq!(toolpath.len(), count);
        assert!(
            toolpath
                .segments()
                .iter()
                .all(|segment| segment.kind == SegmentKind::Arc)
        );
        assert_eq!(toolpath.bounds().min().x, -10.0);
        assert_eq!(toolpath.bounds().max().x, 10.0);
        assert_eq!(toolpath.bounds().min().z, 0.0);
        assert_eq!(toolpath.bounds().max().z, 0.0);
    }

    #[test]
    fn arc_endpoints_are_preserved_in_the_path() {
        let mut toolpath = Toolpath::new();
        let arc = Arc::new(
            ArcPlane::Xy,
            Point3::new(5.0, 5.0, 0.0),
            3.0,
            0.0,
            PI * 0.5,
            false,
        )
        .expect("valid arc");
        toolpath.append_arc(&arc, 0, 0, 0.01);
        let first = toolpath.segments().first().expect("first");
        let last = toolpath.segments().last().expect("last");
        assert!(first.start.distance(arc.start_point()) <= 1.0e-4);
        assert!(last.end.distance(arc.end_point()) <= 1.0e-4);
    }

    #[test]
    fn synthetic_path_is_deterministic_and_exact() {
        for count in [0, 1, 2, 3, 10, 999, 5000] {
            let a = synthetic_toolpath(count);
            let b = synthetic_toolpath(count);
            assert_eq!(a.len(), count);
            assert_eq!(a.segments(), b.segments());
            assert!(a.bounds().is_empty() == (count == 0));
        }
    }

    #[test]
    fn synthetic_path_has_a_realistic_kind_mix() {
        let toolpath = synthetic_toolpath(5000);
        let mut rapid = 0;
        let mut feed = 0;
        let mut plunge = 0;
        let mut arc = 0;
        for segment in toolpath.segments() {
            match segment.kind {
                SegmentKind::Rapid => rapid += 1,
                SegmentKind::Feed => feed += 1,
                SegmentKind::Plunge => plunge += 1,
                SegmentKind::Arc => arc += 1,
                SegmentKind::Probe => {}
            }
        }
        assert!(rapid > 0);
        assert!(feed > 0);
        assert!(plunge > 0);
        assert!(arc > 0);
        assert!(toolpath.bounds().contains(Point3::new(0.0, 0.0, CUT_Z)));
        assert!(toolpath.bounds().size().x <= ROW_LENGTH + 20.0);
        assert!(toolpath.bounds().size().y <= 200.0);
    }

    #[test]
    fn synthetic_path_has_no_non_finite_points() {
        let toolpath = synthetic_toolpath(20_000);
        for segment in toolpath.segments() {
            assert!(segment.start.is_finite());
            assert!(segment.end.is_finite());
        }
        assert!(toolpath.bounds().size().x.is_finite());
        assert!(toolpath.bounds().size().y.is_finite());
    }
}
