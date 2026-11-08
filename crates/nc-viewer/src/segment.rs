//! The compact per-segment toolpath representation.

use crate::geom::{Bounds3, Point3};

/// The motion style of one toolpath segment.
///
/// `Probe` marks probing moves so the renderer can use a distinct style; it is
/// not a cutting motion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SegmentKind {
    /// Rapid (G0) travel.
    #[default]
    Rapid = 0,
    /// Feed (G1) cutting move.
    Feed = 1,
    /// Vertical entry move into material.
    Plunge = 2,
    /// One chord of a tessellated arc.
    Arc = 3,
    /// Probing move.
    Probe = 4,
}

impl SegmentKind {
    /// Every kind, in display/legend order.
    pub const ALL: [Self; 5] = [
        Self::Rapid,
        Self::Feed,
        Self::Plunge,
        Self::Arc,
        Self::Probe,
    ];

    /// Returns `true` for moves that remove material.
    #[must_use]
    pub const fn is_cutting(self) -> bool {
        matches!(self, Self::Feed | Self::Plunge | Self::Arc)
    }

    /// Returns `true` for rapid travel.
    #[must_use]
    pub const fn is_rapid(self) -> bool {
        matches!(self, Self::Rapid)
    }

    /// A short ASCII label suitable for a legend or debug output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rapid => "rapid",
            Self::Feed => "feed",
            Self::Plunge => "plunge",
            Self::Arc => "arc",
            Self::Probe => "probe",
        }
    }
}

/// One straight motion segment in world space.
///
/// The layout is deliberately compact: three `f32` coordinates per endpoint,
/// then `u16` layer/tool ids and a `u8` kind. On common targets the struct is
/// 32 bytes, so a million segments occupy about 32 MB. No segment owns heap
/// data; the whole path is one flat [`Vec`] inside
/// [`Toolpath`](crate::Toolpath).
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
    /// Motion style.
    pub kind: SegmentKind,
}

impl Segment {
    /// Creates a segment on layer 0 with tool 0.
    #[must_use]
    pub const fn new(start: Point3, end: Point3, kind: SegmentKind) -> Self {
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
        assert_eq!(std::mem::size_of::<SegmentKind>(), 1);
    }

    #[test]
    fn segment_helpers() {
        let segment = Segment::new(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(3.0, 4.0, 0.0),
            SegmentKind::Feed,
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
    fn kind_properties() {
        assert!(SegmentKind::Feed.is_cutting());
        assert!(SegmentKind::Plunge.is_cutting());
        assert!(SegmentKind::Arc.is_cutting());
        assert!(!SegmentKind::Rapid.is_cutting());
        assert!(!SegmentKind::Probe.is_cutting());
        assert!(SegmentKind::Rapid.is_rapid());
        assert_eq!(SegmentKind::default(), SegmentKind::Rapid);
        assert_eq!(SegmentKind::ALL.len(), 5);
        for kind in SegmentKind::ALL {
            assert!(!kind.label().is_empty());
        }
    }
}
