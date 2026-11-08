//! Screen-space viewport geometry shared by both cameras.

use crate::geom::{Bounds2, Point2};

/// The drawable area in logical pixels.
///
/// GPUI canvas coordinates start at zero inside the element, so the viewport
/// has no origin of its own. Negative or non-finite sizes are clamped to zero;
/// every camera operation remains well-defined for a zero-sized viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Viewport {
    width: f32,
    height: f32,
}

impl Viewport {
    /// Creates a viewport, clamping invalid sizes to zero.
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: if width.is_finite() && width > 0.0 {
                width
            } else {
                0.0
            },
            height: if height.is_finite() && height > 0.0 {
                height
            } else {
                0.0
            },
        }
    }

    /// Width in logical pixels.
    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Height in logical pixels.
    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    /// Width and height as a point-like vector.
    #[must_use]
    pub const fn size(self) -> Point2 {
        Point2::new(self.width, self.height)
    }

    /// Center in screen coordinates.
    #[must_use]
    pub const fn center(self) -> Point2 {
        Point2::new(self.width * 0.5, self.height * 0.5)
    }

    /// Returns `true` when either dimension is zero.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// The viewport rectangle in screen coordinates.
    #[must_use]
    pub fn bounds(self) -> Bounds2 {
        Bounds2::new(Point2::ZERO, Point2::new(self.width, self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_sanitizes_invalid_sizes() {
        let viewport = Viewport::new(-10.0, f32::NAN);
        assert_eq!(viewport.width(), 0.0);
        assert_eq!(viewport.height(), 0.0);
        assert!(viewport.is_empty());
        assert_eq!(viewport.center(), Point2::ZERO);

        let viewport = Viewport::new(800.0, 600.0);
        assert_eq!(viewport.size(), Point2::new(800.0, 600.0));
        assert_eq!(viewport.center(), Point2::new(400.0, 300.0));
        assert_eq!(
            viewport.bounds(),
            Bounds2::new(Point2::ZERO, Point2::new(800.0, 600.0))
        );
        assert!(!viewport.is_empty());
    }
}
