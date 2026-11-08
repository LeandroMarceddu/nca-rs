//! Basic points and axis-aligned bounds used by the viewer.
//!
//! Coordinates are millimetres in world space and logical pixels in screen
//! space. The viewer deliberately uses `f32`: rendering precision is well
//! below what a display can resolve, and compact points keep the toolpath
//! stream small. The g-code/domain layer owns `f64` machine coordinates.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A point or vector in two dimensions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point2 {
    /// X coordinate.
    pub x: f32,
    /// Y coordinate.
    pub y: f32,
}

impl Point2 {
    /// The origin, `(0, 0)`.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// Creates a point from its coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Returns `true` when both coordinates are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Squared Euclidean length.
    #[must_use]
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// Squared distance to `other`.
    #[must_use]
    pub fn distance_squared(self, other: Self) -> f32 {
        (self - other).length_squared()
    }

    /// Distance to `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> f32 {
        self.distance_squared(other).sqrt()
    }

    /// Returns a unit-length vector, or `None` for zero/non-finite input.
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        let length = self.length();
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let normalized = self / length;
        normalized.is_finite().then_some(normalized)
    }
}

impl Add for Point2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Point2 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Point2 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for Point2 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl Mul<f32> for Point2 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<f32> for Point2 {
    type Output = Self;

    fn div(self, rhs: f32) -> Self::Output {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl Neg for Point2 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y)
    }
}

/// A point or vector in three dimensions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point3 {
    /// X coordinate in millimetres.
    pub x: f32,
    /// Y coordinate in millimetres.
    pub y: f32,
    /// Z coordinate in millimetres.
    pub z: f32,
}

impl Point3 {
    /// The origin, `(0, 0, 0)`.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Creates a point from its coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Returns `true` when all coordinates are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Drops Z, returning the XY projection.
    #[must_use]
    pub const fn xy(self) -> Point2 {
        Point2::new(self.x, self.y)
    }

    /// Squared Euclidean length.
    #[must_use]
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// Squared distance to `other`.
    #[must_use]
    pub fn distance_squared(self, other: Self) -> f32 {
        (self - other).length_squared()
    }

    /// Distance to `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> f32 {
        self.distance_squared(other).sqrt()
    }

    /// Returns a unit-length vector, or `None` for zero/non-finite input.
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        let length = self.length();
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let normalized = self / length;
        normalized.is_finite().then_some(normalized)
    }
}

impl Add for Point3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for Point3 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl Sub for Point3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign for Point3 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl Mul<f32> for Point3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f32> for Point3 {
    type Output = Self;

    fn div(self, rhs: f32) -> Self::Output {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for Point3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// An axis-aligned rectangle in two dimensions.
///
/// The empty bounds use `min = +inf` and `max = -inf`, so `expand` is the
/// natural way to grow them. Non-finite points are ignored rather than
/// poisoning the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds2 {
    min: Point2,
    max: Point2,
}

impl Default for Bounds2 {
    fn default() -> Self {
        Self::empty()
    }
}

impl Bounds2 {
    /// Returns empty bounds that contain no point.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            min: Point2::new(f32::INFINITY, f32::INFINITY),
            max: Point2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }

    /// Creates bounds from two opposite corners.
    ///
    /// Corners may be supplied in any order. If either corner is non-finite,
    /// empty bounds are returned.
    #[must_use]
    pub fn new(a: Point2, b: Point2) -> Self {
        if !a.is_finite() || !b.is_finite() {
            return Self::empty();
        }
        Self {
            min: Point2::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point2::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    /// Creates the smallest bounds containing a single point.
    #[must_use]
    pub fn from_point(point: Point2) -> Self {
        if !point.is_finite() {
            return Self::empty();
        }
        Self {
            min: point,
            max: point,
        }
    }

    /// Creates bounds containing every finite point in `points`.
    #[must_use]
    pub fn from_points<I>(points: I) -> Self
    where
        I: IntoIterator<Item = Point2>,
    {
        let mut bounds = Self::empty();
        for point in points {
            bounds.expand(point);
        }
        bounds
    }

    /// Lower-left corner. Meaningful only when the bounds are not empty.
    #[must_use]
    pub const fn min(&self) -> Point2 {
        self.min
    }

    /// Upper-right corner. Meaningful only when the bounds are not empty.
    #[must_use]
    pub const fn max(&self) -> Point2 {
        self.max
    }

    /// Returns `true` when the bounds contain no point.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    /// Width, or `0` for empty bounds.
    #[must_use]
    pub fn width(&self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            self.max.x - self.min.x
        }
    }

    /// Height, or `0` for empty bounds.
    #[must_use]
    pub fn height(&self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            self.max.y - self.min.y
        }
    }

    /// Width and height as a point-like vector.
    #[must_use]
    pub fn size(&self) -> Point2 {
        Point2::new(self.width(), self.height())
    }

    /// Center, or the origin for empty bounds.
    #[must_use]
    pub fn center(&self) -> Point2 {
        if self.is_empty() {
            Point2::ZERO
        } else {
            Point2::new(
                self.min.x * 0.5 + self.max.x * 0.5,
                self.min.y * 0.5 + self.max.y * 0.5,
            )
        }
    }

    /// Grows the bounds to include `point`. Non-finite points are ignored.
    pub fn expand(&mut self, point: Point2) {
        if !point.is_finite() {
            return;
        }
        self.min.x = self.min.x.min(point.x);
        self.min.y = self.min.y.min(point.y);
        self.max.x = self.max.x.max(point.x);
        self.max.y = self.max.y.max(point.y);
    }

    /// Grows the bounds to include `other`.
    pub fn union(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = *other;
            return;
        }
        self.expand(other.min);
        self.expand(other.max);
    }

    /// Returns `true` when `point` lies inside or on the boundary.
    #[must_use]
    pub fn contains(&self, point: Point2) -> bool {
        !self.is_empty()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Returns `true` when `other` is fully contained.
    #[must_use]
    pub fn contains_bounds(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && other.min.x >= self.min.x
            && other.max.x <= self.max.x
            && other.min.y >= self.min.y
            && other.max.y <= self.max.y
    }

    /// Returns `true` when the two rectangles overlap or touch.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    /// Returns bounds grown by `amount` on every side.
    ///
    /// Negative or non-finite amounts return the original bounds.
    #[must_use]
    pub fn expanded(&self, amount: f32) -> Self {
        if self.is_empty() || !amount.is_finite() || amount < 0.0 {
            return *self;
        }
        Self {
            min: Point2::new(self.min.x - amount, self.min.y - amount),
            max: Point2::new(self.max.x + amount, self.max.y + amount),
        }
    }
}

/// An axis-aligned box in three dimensions.
///
/// Like [`Bounds2`], empty bounds use inverted infinities and ignore
/// non-finite points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds3 {
    min: Point3,
    max: Point3,
}

impl Default for Bounds3 {
    fn default() -> Self {
        Self::empty()
    }
}

impl Bounds3 {
    /// Returns empty bounds that contain no point.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            min: Point3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
            max: Point3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }

    /// Creates bounds from two opposite corners.
    ///
    /// Corners may be supplied in any order. If either corner is non-finite,
    /// empty bounds are returned.
    #[must_use]
    pub fn new(a: Point3, b: Point3) -> Self {
        if !a.is_finite() || !b.is_finite() {
            return Self::empty();
        }
        Self {
            min: Point3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
            max: Point3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
        }
    }

    /// Creates the smallest bounds containing a single point.
    #[must_use]
    pub fn from_point(point: Point3) -> Self {
        if !point.is_finite() {
            return Self::empty();
        }
        Self {
            min: point,
            max: point,
        }
    }

    /// Creates bounds containing every finite point in `points`.
    #[must_use]
    pub fn from_points<I>(points: I) -> Self
    where
        I: IntoIterator<Item = Point3>,
    {
        let mut bounds = Self::empty();
        for point in points {
            bounds.expand(point);
        }
        bounds
    }

    /// Minimum corner. Meaningful only when the bounds are not empty.
    #[must_use]
    pub const fn min(&self) -> Point3 {
        self.min
    }

    /// Maximum corner. Meaningful only when the bounds are not empty.
    #[must_use]
    pub const fn max(&self) -> Point3 {
        self.max
    }

    /// Returns `true` when the bounds contain no point.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    /// Size along each axis, or zero for empty bounds.
    #[must_use]
    pub fn size(&self) -> Point3 {
        if self.is_empty() {
            Point3::ZERO
        } else {
            self.max - self.min
        }
    }

    /// Center, or the origin for empty bounds.
    #[must_use]
    pub fn center(&self) -> Point3 {
        if self.is_empty() {
            Point3::ZERO
        } else {
            Point3::new(
                self.min.x * 0.5 + self.max.x * 0.5,
                self.min.y * 0.5 + self.max.y * 0.5,
                self.min.z * 0.5 + self.max.z * 0.5,
            )
        }
    }

    /// Grows the bounds to include `point`. Non-finite points are ignored.
    pub fn expand(&mut self, point: Point3) {
        if !point.is_finite() {
            return;
        }
        self.min.x = self.min.x.min(point.x);
        self.min.y = self.min.y.min(point.y);
        self.min.z = self.min.z.min(point.z);
        self.max.x = self.max.x.max(point.x);
        self.max.y = self.max.y.max(point.y);
        self.max.z = self.max.z.max(point.z);
    }

    /// Grows the bounds to include `other`.
    pub fn union(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = *other;
            return;
        }
        self.expand(other.min);
        self.expand(other.max);
    }

    /// Returns `true` when `point` lies inside or on the boundary.
    #[must_use]
    pub fn contains(&self, point: Point3) -> bool {
        !self.is_empty()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Returns `true` when `other` is fully contained.
    #[must_use]
    pub fn contains_bounds(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && other.min.x >= self.min.x
            && other.max.x <= self.max.x
            && other.min.y >= self.min.y
            && other.max.y <= self.max.y
            && other.min.z >= self.min.z
            && other.max.z <= self.max.z
    }

    /// Returns `true` when the two boxes overlap or touch.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    /// The XY footprint of the box, used by the top view.
    #[must_use]
    pub fn xy_bounds(&self) -> Bounds2 {
        if self.is_empty() {
            Bounds2::empty()
        } else {
            Bounds2::new(self.min.xy(), self.max.xy())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bounds_contain_nothing() {
        let bounds = Bounds2::empty();
        assert!(bounds.is_empty());
        assert!(!bounds.contains(Point2::ZERO));
        assert_eq!(bounds.size(), Point2::ZERO);
        assert_eq!(bounds.center(), Point2::ZERO);
        assert_eq!(bounds.width(), 0.0);
        assert_eq!(bounds.height(), 0.0);
    }

    #[test]
    fn bounds2_expand_union_and_queries() {
        let mut bounds = Bounds2::from_point(Point2::new(2.0, 3.0));
        assert!(bounds.contains(Point2::new(2.0, 3.0)));
        bounds.expand(Point2::new(-1.0, 5.0));
        assert_eq!(bounds.min(), Point2::new(-1.0, 3.0));
        assert_eq!(bounds.max(), Point2::new(2.0, 5.0));
        assert_eq!(bounds.size(), Point2::new(3.0, 2.0));
        assert_eq!(bounds.center(), Point2::new(0.5, 4.0));

        let other = Bounds2::new(Point2::new(4.0, -2.0), Point2::new(6.0, 1.0));
        bounds.union(&other);
        assert!(bounds.contains_bounds(&other));
        assert!(bounds.intersects(&other));
        assert_eq!(bounds.max(), Point2::new(6.0, 5.0));

        let mut empty = Bounds2::empty();
        empty.union(&other);
        assert_eq!(empty, other);
    }

    #[test]
    fn bounds2_normalizes_corners_and_ignores_non_finite() {
        let bounds = Bounds2::new(Point2::new(4.0, -2.0), Point2::new(1.0, 3.0));
        assert_eq!(bounds.min(), Point2::new(1.0, -2.0));
        assert_eq!(bounds.max(), Point2::new(4.0, 3.0));

        let mut bounds = bounds;
        bounds.expand(Point2::new(f32::NAN, 0.0));
        bounds.expand(Point2::new(f32::INFINITY, 0.0));
        assert_eq!(bounds.min(), Point2::new(1.0, -2.0));
        assert_eq!(bounds.max(), Point2::new(4.0, 3.0));

        assert!(Bounds2::new(Point2::new(f32::NAN, 0.0), Point2::ZERO).is_empty());
    }

    #[test]
    fn bounds3_expand_union_and_queries() {
        let mut bounds = Bounds3::from_point(Point3::new(1.0, 2.0, 3.0));
        bounds.expand(Point3::new(-1.0, 5.0, 0.0));
        assert_eq!(bounds.min(), Point3::new(-1.0, 2.0, 0.0));
        assert_eq!(bounds.max(), Point3::new(1.0, 5.0, 3.0));
        assert_eq!(bounds.size(), Point3::new(2.0, 3.0, 3.0));
        assert_eq!(bounds.center(), Point3::new(0.0, 3.5, 1.5));
        assert!(bounds.contains(Point3::new(0.0, 3.0, 1.0)));
        assert!(!bounds.contains(Point3::new(0.0, 6.0, 1.0)));

        let other = Bounds3::new(Point3::new(2.0, 2.0, 2.0), Point3::new(4.0, 4.0, 4.0));
        assert!(!bounds.intersects(&other));
        bounds.union(&other);
        assert!(bounds.intersects(&other));
        assert!(bounds.contains_bounds(&other));
    }

    #[test]
    fn point_math_is_finite_aware() {
        let p = Point3::new(3.0, 4.0, 0.0);
        assert_eq!(p.length(), 5.0);
        assert_eq!(p.xy(), Point2::new(3.0, 4.0));
        assert_eq!(p.normalized().map(|n| n.length()), Some(1.0));
        assert!(Point3::new(f32::NAN, 0.0, 0.0).normalized().is_none());
        assert!(Point2::ZERO.normalized().is_none());
        assert_eq!(
            Point3::new(1.0, 0.0, 0.0).cross(Point3::new(0.0, 1.0, 0.0)),
            Point3::new(0.0, 0.0, 1.0)
        );
    }
}
