//! The compact per-segment toolpath representation.

use crate::geom::{Bounds3, Point3};
use crate::move_kind::MoveKind;

/// One straight motion segment in world space.
///
/// The layout is deliberately compact: three `f32` coordinates per endpoint,
/// then `u16` layer/tool ids and the one-byte semantic [`MoveKind`]. On common
/// targets the struct is 32 bytes, so a million segments occupy about 32 MB.
/// No segment owns heap data; the whole path is one flat [`Vec`] inside
/// [`Toolpath`](crate::Toolpath).
///
/// `kind` is the exact semantic kind assigned by the parser, never a coarse
/// display projection: every chord of an arc keeps its arc or helix kind and a
/// canned-cycle cutting move keeps Drill, Peck, or Bore.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Start point in millimetres.
    pub start: Point3,
    /// End point in millimetres.
    pub end: Point3,
    /// Layer id used by the g-code layer for grouping and visibility.
    pub layer: u16,
    /// Tool number that produced this segment.
    pub tool: u16,
    /// Semantic motion kind.
    pub kind: MoveKind,
}

impl Segment {
    /// Creates a segment on layer 0 with tool 0.
    #[must_use]
    pub const fn new(start: Point3, end: Point3, kind: MoveKind) -> Self {
        Self {
            start,
            end,
            layer: 0,
            tool: 0,
            kind,
        }
    }

    /// Sets the layer id.
    #[must_use]
    pub const fn with_layer(mut self, layer: u16) -> Self {
        self.layer = layer;
        self
    }

    /// Sets the tool number.
    #[must_use]
    pub const fn with_tool(mut self, tool: u16) -> Self {
        self.tool = tool;
        self
    }

    /// Euclidean length in world units.
    #[must_use]
    pub fn length(&self) -> f32 {
        self.start.distance(self.end)
    }

    /// Midpoint of the segment.
    #[must_use]
    pub fn midpoint(&self) -> Point3 {
        (self.start + self.end) * 0.5
    }

    /// Returns `true` when both endpoints are identical.
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.start == self.end
    }

    /// Returns `true` when both endpoints are finite.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.start.is_finite() && self.end.is_finite()
    }

    /// Axis-aligned bounds of the segment.
    #[must_use]
    pub fn bounds(&self) -> Bounds3 {
        Bounds3::new(self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_is_compact() {
        assert!(
            std::mem::size_of::<Segment>() <= 32,
            "Segment grew to {} bytes",
            std::mem::size_of::<Segment>()
        );
        assert_eq!(std::mem::size_of::<MoveKind>(), 1);
    }

    #[test]
    fn segment_helpers() {
        let segment = Segment::new(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(3.0, 4.0, 0.0),
            MoveKind::Feed,
        )
        .with_layer(7)
        .with_tool(2);
        assert_eq!(segment.layer, 7);
        assert_eq!(segment.tool, 2);
        assert_eq!(segment.length(), 5.0);
        assert_eq!(segment.midpoint(), Point3::new(1.5, 2.0, 0.0));
        assert!(!segment.is_degenerate());
        assert!(segment.is_valid());
        assert_eq!(
            segment.bounds(),
            Bounds3::new(Point3::ZERO, Point3::new(3.0, 4.0, 0.0))
        );
    }

    #[test]
    fn every_move_kind_fits_in_a_segment() {
        for kind in MoveKind::ALL {
            let segment = Segment::new(Point3::ZERO, Point3::new(1.0, 0.0, 0.0), kind);
            assert_eq!(segment.kind, kind);
            assert!(segment.is_valid());
        }
    }
}
