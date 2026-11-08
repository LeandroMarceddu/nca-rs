//! Axis-aligned stock-box geometry for the 3D viewer.
//!
//! A [`StockBox`] is a pure geometric description of rectangular stock in world
//! millimetres with +Z up. It has no renderer, camera, or GPUI dependency: the
//! painter projects its corners and draws whichever edges or faces it needs.

use crate::geom::{Bounds2, Bounds3, Point3};

/// An axis-aligned rectangular stock block in world millimetres, +Z up.
///
/// Create one with [`StockBox::new`], [`StockBox::from_xy`], or
/// [`StockBox::from_bounds`]. Every constructor returns `None` for non-finite
/// values or a degenerate axis. The corners are public so a painter can read
/// them directly; after a direct struct construction, keeping `min < max` on
/// every axis is the caller's responsibility, and [`StockBox::is_valid`]
/// reports whether that invariant holds.
///
/// # Corner order
///
/// [`StockBox::corners`] returns a stable array:
///
/// ```text
/// 0 = (min.x, min.y, min.z)   1 = (max.x, min.y, min.z)
/// 2 = (max.x, max.y, min.z)   3 = (min.x, max.y, min.z)
/// 4 = (min.x, min.y, max.z)   5 = (max.x, min.y, max.z)
/// 6 = (max.x, max.y, max.z)   7 = (min.x, max.y, max.z)
/// ```
///
/// Corners 0-3 form the bottom face and 4-7 the top face; corner `i + 4` sits
/// directly above corner `i`. Each ring runs counter-clockwise when viewed from
/// +Z (above).
///
/// # Example
///
/// ```
/// use nc_viewer::{Point3, StockBox};
///
/// let stock = StockBox::from_xy(120.0, 80.0, 0.0, 25.0).expect("valid stock");
/// assert_eq!(stock.min, Point3::new(-60.0, -40.0, -25.0));
/// assert_eq!(stock.max, Point3::new(60.0, 40.0, 0.0));
/// assert_eq!(stock.corners()[0], stock.min);
/// assert_eq!(stock.height_mm(), 25.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StockBox {
    /// Minimum corner: smallest X, Y, and Z, in world millimetres.
    pub min: Point3,
    /// Maximum corner: largest X, Y, and Z, in world millimetres.
    pub max: Point3,
}

impl StockBox {
    /// Creates a stock box from its minimum and maximum corners in world
    /// millimetres.
    ///
    /// Returns `None` unless every component is finite and the box has positive
    /// extent on all three axes.
    #[must_use]
    pub fn new(min: Point3, max: Point3) -> Option<Self> {
        if !min.is_finite() || !max.is_finite() {
            return None;
        }
        if min.x >= max.x || min.y >= max.y || min.z >= max.z {
            return None;
        }
        Some(Self { min, max })
    }

    /// Creates a stock box centered on the world XY origin from its width and
    /// depth, with its top face at `top_z_mm` and the given height below it.
    ///
    /// The footprint spans `-width_mm / 2 ..= width_mm / 2` on X and
    /// `-depth_mm / 2 ..= depth_mm / 2` on Y. Values are accepted as `f64` and
    /// stored as `f32` world coordinates. Returns `None` unless every value is
    /// finite, width, depth, and height are strictly positive, and the result
    /// has a finite positive extent on every axis at `f32` precision.
    #[must_use]
    pub fn from_xy(width_mm: f64, depth_mm: f64, top_z_mm: f64, height_mm: f64) -> Option<Self> {
        if !width_mm.is_finite()
            || !depth_mm.is_finite()
            || !top_z_mm.is_finite()
            || !height_mm.is_finite()
        {
            return None;
        }
        if width_mm <= 0.0 || depth_mm <= 0.0 || height_mm <= 0.0 {
            return None;
        }
        let half_width_mm = (width_mm * 0.5) as f32;
        let half_depth_mm = (depth_mm * 0.5) as f32;
        let bottom_z_mm = (top_z_mm - height_mm) as f32;
        let top_z_mm = top_z_mm as f32;
        Self::new(
            Point3::new(-half_width_mm, -half_depth_mm, bottom_z_mm),
            Point3::new(half_width_mm, half_depth_mm, top_z_mm),
        )
    }

    /// Creates a stock box from the XY footprint of `bounds`, with its top face
    /// at `top_z_mm` and the given height below it.
    ///
    /// The Z range of `bounds` is ignored. `top_z_mm` and `height_mm` are
    /// accepted as `f64` and stored as `f32` world coordinates. Returns `None`
    /// unless both are finite, the height is strictly positive, `bounds` is not
    /// empty, and the result has a finite positive extent on every axis.
    #[must_use]
    pub fn from_bounds(bounds: Bounds3, top_z_mm: f64, height_mm: f64) -> Option<Self> {
        if !top_z_mm.is_finite() || !height_mm.is_finite() || height_mm <= 0.0 {
            return None;
        }
        if bounds.is_empty() {
            return None;
        }
        let xy_min = bounds.min();
        let xy_max = bounds.max();
        let bottom_z_mm = (top_z_mm - height_mm) as f32;
        let top_z_mm = top_z_mm as f32;
        Self::new(
            Point3::new(xy_min.x, xy_min.y, bottom_z_mm),
            Point3::new(xy_max.x, xy_max.y, top_z_mm),
        )
    }

    /// Returns the eight corners in the stable order documented on
    /// [`StockBox`].
    ///
    /// Corners 0-3 are the bottom face and 4-7 the top face; corner `i + 4` is
    /// directly above corner `i`.
    #[must_use]
    pub const fn corners(&self) -> [Point3; 8] {
        let min = self.min;
        let max = self.max;
        [
            Point3::new(min.x, min.y, min.z),
            Point3::new(max.x, min.y, min.z),
            Point3::new(max.x, max.y, min.z),
            Point3::new(min.x, max.y, min.z),
            Point3::new(min.x, min.y, max.z),
            Point3::new(max.x, min.y, max.z),
            Point3::new(max.x, max.y, max.z),
            Point3::new(min.x, max.y, max.z),
        ]
    }

    /// Returns the twelve edges as corner-index pairs.
    ///
    /// Entries 0-3 are the bottom ring `(0,1) (1,2) (2,3) (3,0)`, entries 4-7
    /// the top ring `(4,5) (5,6) (6,7) (7,4)`, and entries 8-11 the verticals
    /// `(0,4) (1,5) (2,6) (3,7)`.
    #[must_use]
    pub const fn edges(&self) -> [(usize, usize); 12] {
        [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ]
    }

    /// Returns the six faces as outward-facing corner-index quads.
    ///
    /// Each quad runs counter-clockwise when seen from outside the box:
    ///
    /// ```text
    /// 0 bottom [3, 2, 1, 0] (-Z)   1 top    [4, 5, 6, 7] (+Z)
    /// 2 -Y     [0, 1, 5, 4]       3 +X     [1, 2, 6, 5]
    /// 4 +Y     [2, 3, 7, 6]       5 -X     [3, 0, 4, 7]
    /// ```
    #[must_use]
    pub const fn faces(&self) -> [[usize; 4]; 6] {
        [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ]
    }

    /// Returns the top-face corners, counter-clockwise seen from above.
    ///
    /// The points are corners 4, 5, 6, and 7 of [`StockBox::corners`], and all
    /// share `max.z`.
    #[must_use]
    pub fn top_face(&self) -> [Point3; 4] {
        let corners = self.corners();
        [corners[4], corners[5], corners[6], corners[7]]
    }

    /// Returns the bottom-face corners, counter-clockwise seen from below.
    ///
    /// The points are corners 3, 2, 1, and 0 of [`StockBox::corners`], and all
    /// share `min.z`.
    #[must_use]
    pub fn bottom_face(&self) -> [Point3; 4] {
        let corners = self.corners();
        [corners[3], corners[2], corners[1], corners[0]]
    }

    /// Returns `true` when `point` is inside the box or on any of its faces.
    ///
    /// Non-finite points, and every point tested against an invalid box, are
    /// outside.
    #[must_use]
    pub fn contains(&self, point: Point3) -> bool {
        self.is_valid()
            && point.is_finite()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Returns the XY footprint. Empty for an invalid box.
    #[must_use]
    pub fn xy_bounds(&self) -> Bounds2 {
        if self.is_valid() {
            Bounds2::new(self.min.xy(), self.max.xy())
        } else {
            Bounds2::empty()
        }
    }

    /// Returns the full box. Empty for an invalid box.
    #[must_use]
    pub fn bounds(&self) -> Bounds3 {
        if self.is_valid() {
            Bounds3::new(self.min, self.max)
        } else {
            Bounds3::empty()
        }
    }

    /// Returns the height along Z in millimetres, or `0` for an invalid box.
    #[must_use]
    pub fn height_mm(&self) -> f32 {
        if self.is_valid() {
            self.max.z - self.min.z
        } else {
            0.0
        }
    }

    /// Returns `true` when all six coordinates are finite and `min < max` on
    /// every axis.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min.x < self.max.x
            && self.min.y < self.max.y
            && self.min.z < self.max.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point2;

    fn sample() -> StockBox {
        StockBox::from_xy(20.0, 10.0, 0.0, 5.0).expect("valid stock")
    }

    #[test]
    fn new_accepts_valid_bounds_and_rejects_degenerate_or_non_finite() {
        let min = Point3::new(-10.0, -20.0, -30.0);
        let max = Point3::new(10.0, 20.0, 30.0);
        let stock = StockBox::new(min, max).expect("valid stock");
        assert_eq!(stock.min, min);
        assert_eq!(stock.max, max);
        assert!(stock.is_valid());

        let invalid = [
            (Point3::ZERO, Point3::ZERO),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 1.0, 1.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(1.0, 0.0, 1.0)),
            (Point3::new(0.0, 0.0, 1.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 1.0, 1.0)),
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 1.0)),
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(f32::NAN, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0)),
            (
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, f32::INFINITY, 1.0),
            ),
            (
                Point3::new(0.0, f32::NEG_INFINITY, 0.0),
                Point3::new(1.0, 1.0, 1.0),
            ),
        ];
        for (min, max) in invalid {
            assert!(
                StockBox::new(min, max).is_none(),
                "accepted {min:?}..{max:?}"
            );
        }
    }

    #[test]
    fn from_xy_centers_footprint_and_builds_down_from_top() {
        let stock = StockBox::from_xy(100.0, 60.0, 5.0, 20.0).expect("valid stock");
        assert_eq!(stock.min, Point3::new(-50.0, -30.0, -15.0));
        assert_eq!(stock.max, Point3::new(50.0, 30.0, 5.0));
        assert_eq!(stock.height_mm(), 20.0);
        assert_eq!(stock.corners()[0], stock.min);
        assert_eq!(stock.corners()[7], Point3::new(-50.0, 30.0, 5.0));

        let square = StockBox::from_xy(12.5, 12.5, 0.0, 2.5).expect("valid stock");
        assert_eq!(square.min.x, -6.25);
        assert_eq!(square.max.x, 6.25);
        assert_eq!(square.min.y, -6.25);
        assert_eq!(square.max.y, 6.25);
    }

    #[test]
    fn from_xy_rejects_non_positive_or_non_finite_values() {
        let invalid = [
            (0.0, 10.0, 0.0, 10.0),
            (-1.0, 10.0, 0.0, 10.0),
            (10.0, 0.0, 0.0, 10.0),
            (10.0, -1.0, 0.0, 10.0),
            (10.0, 10.0, 0.0, 0.0),
            (10.0, 10.0, 0.0, -1.0),
            (f64::NAN, 10.0, 0.0, 10.0),
            (10.0, f64::INFINITY, 0.0, 10.0),
            (10.0, 10.0, f64::NAN, 10.0),
            (10.0, 10.0, 0.0, f64::INFINITY),
            (f64::MAX, 10.0, 0.0, 10.0),
            (1.0e-300, 10.0, 0.0, 10.0),
        ];
        for (width, depth, top, height) in invalid {
            assert!(
                StockBox::from_xy(width, depth, top, height).is_none(),
                "accepted w={width} d={depth} top={top} h={height}"
            );
        }
    }

    #[test]
    fn from_bounds_uses_xy_footprint_and_requested_top_and_height() {
        let bounds = Bounds3::new(
            Point3::new(-10.0, -5.0, -100.0),
            Point3::new(30.0, 15.0, 100.0),
        );
        let stock = StockBox::from_bounds(bounds, 2.5, 10.0).expect("valid stock");
        assert_eq!(stock.min, Point3::new(-10.0, -5.0, -7.5));
        assert_eq!(stock.max, Point3::new(30.0, 15.0, 2.5));
        assert_eq!(stock.xy_bounds(), bounds.xy_bounds());
        assert_eq!(stock.height_mm(), 10.0);

        assert!(StockBox::from_bounds(Bounds3::empty(), 0.0, 10.0).is_none());
        assert!(
            StockBox::from_bounds(Bounds3::from_point(Point3::new(1.0, 2.0, 3.0)), 0.0, 10.0)
                .is_none()
        );
        let footprint = Bounds3::new(Point3::new(-1.0, -1.0, 0.0), Point3::new(1.0, 1.0, 0.0));
        for (top, height) in [
            (0.0, 0.0),
            (0.0, -1.0),
            (f64::NAN, 1.0),
            (f64::INFINITY, 1.0),
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
            (f64::MAX, 1.0),
        ] {
            assert!(
                StockBox::from_bounds(footprint, top, height).is_none(),
                "accepted top={top} h={height}"
            );
        }
    }

    #[test]
    fn corner_order_is_stable_and_top_bottom_faces_match_min_max() {
        let stock = sample();
        let expected = [
            Point3::new(-10.0, -5.0, -5.0),
            Point3::new(10.0, -5.0, -5.0),
            Point3::new(10.0, 5.0, -5.0),
            Point3::new(-10.0, 5.0, -5.0),
            Point3::new(-10.0, -5.0, 0.0),
            Point3::new(10.0, -5.0, 0.0),
            Point3::new(10.0, 5.0, 0.0),
            Point3::new(-10.0, 5.0, 0.0),
        ];
        let corners = stock.corners();
        assert_eq!(corners, expected);
        assert_eq!(
            stock.top_face(),
            [corners[4], corners[5], corners[6], corners[7]]
        );
        assert_eq!(
            stock.bottom_face(),
            [corners[3], corners[2], corners[1], corners[0]]
        );
        for (bottom, top) in corners[..4].iter().zip(&corners[4..]) {
            assert_eq!(bottom.x, top.x);
            assert_eq!(bottom.y, top.y);
            assert_eq!(bottom.z, stock.min.z);
            assert_eq!(top.z, stock.max.z);
        }
    }

    #[test]
    fn edge_table_is_stable_and_references_distinct_corners() {
        let stock = sample();
        let edges = stock.edges();
        assert_eq!(
            edges,
            [
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ]
        );
        let corners = stock.corners();
        for (index, &(a, b)) in edges.iter().enumerate() {
            assert!(a < corners.len() && b < corners.len());
            assert_ne!(a, b);
            for &(other_a, other_b) in edges.iter().skip(index + 1) {
                assert_ne!(
                    (a.min(b), a.max(b)),
                    (other_a.min(other_b), other_a.max(other_b))
                );
            }
            let delta = corners[b] - corners[a];
            assert!(delta.length() > 0.0);
            assert_eq!(
                [delta.x != 0.0, delta.y != 0.0, delta.z != 0.0]
                    .iter()
                    .filter(|changed| **changed)
                    .count(),
                1,
                "edge {a}-{b} is not axis-aligned"
            );
        }
    }

    #[test]
    fn face_table_is_stable_and_outward_facing() {
        let stock = sample();
        let faces = stock.faces();
        assert_eq!(
            faces,
            [
                [3, 2, 1, 0],
                [4, 5, 6, 7],
                [0, 1, 5, 4],
                [1, 2, 6, 5],
                [2, 3, 7, 6],
                [3, 0, 4, 7],
            ]
        );
        let corners = stock.corners();
        let center = stock.bounds().center();
        for face in faces {
            for &index in &face {
                assert!(index < corners.len());
            }
            let points = face.map(|index| corners[index]);
            let normal = (points[1] - points[0]).cross(points[2] - points[0]);
            assert!(normal.length() > 0.0);
            let face_center = (points[0] + points[1] + points[2] + points[3]) * 0.25;
            assert!(
                normal.dot(face_center - center) > 0.0,
                "face {face:?} points inward"
            );
        }
    }

    #[test]
    fn contains_inside_on_faces_and_outside() {
        let stock = sample();
        assert!(stock.contains(Point3::new(0.0, 0.0, -2.5)));
        assert!(stock.contains(stock.min));
        assert!(stock.contains(stock.max));
        assert!(stock.contains(Point3::new(-10.0, 0.0, -2.5)));
        assert!(stock.contains(Point3::new(10.0, 0.0, -2.5)));
        assert!(stock.contains(Point3::new(0.0, -5.0, -2.5)));
        assert!(stock.contains(Point3::new(0.0, 5.0, -2.5)));
        assert!(stock.contains(Point3::new(0.0, 0.0, -5.0)));
        assert!(stock.contains(Point3::new(0.0, 0.0, 0.0)));

        assert!(!stock.contains(Point3::new(-10.001, 0.0, -2.5)));
        assert!(!stock.contains(Point3::new(10.001, 0.0, -2.5)));
        assert!(!stock.contains(Point3::new(0.0, -5.001, -2.5)));
        assert!(!stock.contains(Point3::new(0.0, 5.001, -2.5)));
        assert!(!stock.contains(Point3::new(0.0, 0.0, -5.001)));
        assert!(!stock.contains(Point3::new(0.0, 0.0, 0.001)));
        assert!(!stock.contains(Point3::new(f32::NAN, 0.0, 0.0)));
        assert!(!stock.contains(Point3::new(0.0, f32::INFINITY, 0.0)));
    }

    #[test]
    fn bounds_xy_bounds_and_height_match_the_construction() {
        let stock = StockBox::from_xy(100.0, 60.0, 5.0, 20.0).expect("valid stock");
        assert_eq!(
            stock.bounds(),
            Bounds3::new(
                Point3::new(-50.0, -30.0, -15.0),
                Point3::new(50.0, 30.0, 5.0)
            )
        );
        assert_eq!(
            stock.xy_bounds(),
            Bounds2::new(Point2::new(-50.0, -30.0), Point2::new(50.0, 30.0))
        );
        assert_eq!(stock.xy_bounds().width(), 100.0);
        assert_eq!(stock.xy_bounds().height(), 60.0);
        assert_eq!(stock.height_mm(), 20.0);
        assert_eq!(stock.bounds().size(), Point3::new(100.0, 60.0, 20.0));
    }

    #[test]
    fn extreme_finite_values_are_total() {
        let huge = StockBox::new(
            Point3::new(f32::MIN, f32::MIN, f32::MIN),
            Point3::new(f32::MAX, f32::MAX, f32::MAX),
        )
        .expect("finite ordered bounds");
        assert!(huge.is_valid());
        assert!(huge.corners().iter().all(|corner| corner.is_finite()));
        assert_eq!(huge.corners().len(), 8);
        assert_eq!(huge.edges().len(), 12);
        assert_eq!(huge.faces().len(), 6);
        assert_eq!(huge.top_face().len(), 4);
        assert_eq!(huge.bottom_face().len(), 4);
        assert!(huge.contains(Point3::ZERO));
        assert!(huge.contains(Point3::new(f32::MAX, f32::MAX, f32::MAX)));
        assert!(huge.xy_bounds().contains(Point2::ZERO));
        assert!(!huge.bounds().is_empty());
        let _ = huge.height_mm();

        let tall = StockBox::from_xy(1.0e30, 2.0e30, 1.0e30, 5.0e29);
        assert!(tall.is_some());
        let tall = tall.expect("representable stock");
        assert!(tall.height_mm().is_finite() && tall.height_mm() > 0.0);

        let wide = StockBox::from_bounds(
            Bounds3::new(
                Point3::new(f32::MIN, f32::MIN, f32::MIN),
                Point3::new(f32::MAX, f32::MAX, f32::MAX),
            ),
            0.0,
            1.0,
        )
        .expect("finite footprint");
        assert!(wide.max.z > wide.min.z);
    }

    #[test]
    fn invalid_struct_literals_do_not_panic() {
        let invalid = StockBox {
            min: Point3::new(5.0, 0.0, 0.0),
            max: Point3::new(-5.0, 1.0, 1.0),
        };
        assert!(!invalid.is_valid());
        assert!(!invalid.contains(Point3::ZERO));
        assert!(invalid.xy_bounds().is_empty());
        assert!(invalid.bounds().is_empty());
        assert_eq!(invalid.height_mm(), 0.0);
        assert_eq!(invalid.corners().len(), 8);
        assert_eq!(invalid.edges().len(), 12);
        assert_eq!(invalid.faces().len(), 6);
        assert_eq!(invalid.top_face().len(), 4);
        assert_eq!(invalid.bottom_face().len(), 4);
    }
}
