//! Orthographic top-view camera for the 2D toolpath.

use crate::geom::{Bounds2, Point2};
use crate::viewport::Viewport;

/// An orthographic top-view camera with a Y-up world convention.
///
/// World X increases to the right on screen; world Y increases upward, while
/// screen Y increases downward. The camera shows the world point `offset` at
/// the center of its viewport and scales the world by `scale` pixels per
/// world unit.
///
/// All operations are total: invalid scales, factors, and deltas are ignored
/// and never produce a `NaN` transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2D {
    viewport: Viewport,
    offset: Point2,
    scale: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self::new(Viewport::default())
    }
}

impl Camera2D {
    /// Smallest accepted scale, in pixels per world unit.
    pub const MIN_SCALE: f32 = 1.0e-6;

    /// Largest accepted scale, in pixels per world unit.
    pub const MAX_SCALE: f32 = 1.0e6;

    /// Creates a camera showing the origin at the viewport center at 1:1.
    #[must_use]
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            offset: Point2::ZERO,
            scale: 1.0,
        }
    }

    /// Creates a camera fitted to `bounds` with `padding_px` on every side.
    #[must_use]
    pub fn fitted(viewport: Viewport, bounds: Bounds2, padding_px: f32) -> Self {
        let mut camera = Self::new(viewport);
        camera.fit(bounds, padding_px);
        camera
    }

    /// Current viewport size.
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Changes the viewport size without changing the visible world center or
    /// scale. Call [`Camera2D::fit`] afterwards when a resize should refit.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
    }

    /// World point shown at the viewport center.
    #[must_use]
    pub const fn offset(&self) -> Point2 {
        self.offset
    }

    /// Sets the world point shown at the viewport center.
    ///
    /// Non-finite input is ignored.
    pub fn set_offset(&mut self, offset: Point2) {
        if offset.is_finite() {
            self.offset = offset;
        }
    }

    /// Pixels per world unit.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Sets the scale, clamped to the supported range.
    ///
    /// Non-finite or non-positive values are ignored.
    pub fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale.clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        }
    }

    /// Fits `bounds` into the viewport with `padding_px` on every side.
    ///
    /// The center of the bounds becomes the viewport center. Empty bounds are
    /// ignored. Degenerate bounds keep the current scale when they cannot
    /// determine one; a single point is still centered. Padding is in logical
    /// pixels and is clamped so at least one pixel remains drawable.
    pub fn fit(&mut self, bounds: Bounds2, padding_px: f32) {
        if bounds.is_empty() {
            return;
        }
        self.offset = bounds.center();

        let padding = if padding_px.is_finite() && padding_px > 0.0 {
            padding_px
        } else {
            0.0
        };
        let available = Point2::new(
            (self.viewport.width() - 2.0 * padding).max(1.0),
            (self.viewport.height() - 2.0 * padding).max(1.0),
        );
        let size = bounds.size();
        let scale_x = if size.x > 0.0 {
            available.x / size.x
        } else {
            f32::INFINITY
        };
        let scale_y = if size.y > 0.0 {
            available.y / size.y
        } else {
            f32::INFINITY
        };
        let scale = scale_x.min(scale_y);
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale.clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        }
    }

    /// Pans the view by a screen-space delta in pixels.
    ///
    /// The content follows the pointer: a drag to the right moves the drawing
    /// right, a drag down moves it down.
    pub fn pan(&mut self, delta_px: Point2) {
        if !delta_px.is_finite() {
            return;
        }
        self.offset.x -= delta_px.x / self.scale;
        self.offset.y += delta_px.y / self.scale;
        if !self.offset.is_finite() {
            self.offset = Point2::ZERO;
        }
    }

    /// Zooms by `factor` while keeping the world point under `pivot_px` fixed.
    ///
    /// A factor greater than one zooms in. Non-finite or non-positive factors
    /// are ignored.
    pub fn zoom_at(&mut self, pivot_px: Point2, factor: f32) {
        if !pivot_px.is_finite() || !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let old_offset = self.offset;
        let world_pivot = self.screen_to_world(pivot_px);
        if !world_pivot.is_finite() {
            return;
        }
        let new_scale = (self.scale * factor).clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        self.scale = new_scale;
        let center = self.viewport.center();
        let offset = Point2::new(
            world_pivot.x - (pivot_px.x - center.x) / new_scale,
            world_pivot.y + (pivot_px.y - center.y) / new_scale,
        );
        self.offset = if offset.is_finite() {
            offset
        } else {
            old_offset
        };
    }

    /// Zooms around the viewport center.
    pub fn zoom(&mut self, factor: f32) {
        self.zoom_at(self.viewport.center(), factor);
    }

    /// Converts a world point to screen coordinates.
    #[must_use]
    pub fn world_to_screen(&self, world: Point2) -> Point2 {
        let center = self.viewport.center();
        Point2::new(
            center.x + (world.x - self.offset.x) * self.scale,
            center.y - (world.y - self.offset.y) * self.scale,
        )
    }

    /// Converts a screen point to world coordinates.
    #[must_use]
    pub fn screen_to_world(&self, screen: Point2) -> Point2 {
        let center = self.viewport.center();
        Point2::new(
            self.offset.x + (screen.x - center.x) / self.scale,
            self.offset.y - (screen.y - center.y) / self.scale,
        )
    }

    /// World-space bounds currently visible in the viewport.
    ///
    /// Returns a point-sized bound for a zero-sized viewport and empty bounds
    /// only if the transform is not finite.
    #[must_use]
    pub fn visible_world_bounds(&self) -> Bounds2 {
        let width = self.viewport.width();
        let height = self.viewport.height();
        Bounds2::from_points([
            self.screen_to_world(Point2::ZERO),
            self.screen_to_world(Point2::new(width, 0.0)),
            self.screen_to_world(Point2::new(0.0, height)),
            self.screen_to_world(Point2::new(width, height)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() <= epsilon
    }

    #[test]
    fn fit_centers_and_fits_bounds() {
        let viewport = Viewport::new(800.0, 600.0);
        let bounds = Bounds2::new(Point2::new(-50.0, 10.0), Point2::new(50.0, 60.0));
        let camera = Camera2D::fitted(viewport, bounds, 0.0);

        assert_eq!(camera.offset(), bounds.center());
        assert!(approx(camera.scale(), 8.0, 1.0e-4));
        let bottom_left = camera.world_to_screen(bounds.min());
        let top_right = camera.world_to_screen(bounds.max());
        assert!(approx(bottom_left.x, 0.0, 1.0e-4));
        assert!(approx(top_right.x, 800.0, 1.0e-4));
        // Y is flipped and the narrower axis stays centered.
        assert!(approx(bottom_left.y, 500.0, 1.0e-4));
        assert!(approx(top_right.y, 100.0, 1.0e-4));
        assert_eq!(camera.world_to_screen(bounds.center()), viewport.center());
    }

    #[test]
    fn fit_honors_padding() {
        let viewport = Viewport::new(800.0, 600.0);
        let bounds = Bounds2::new(Point2::ZERO, Point2::new(100.0, 50.0));
        let camera = Camera2D::fitted(viewport, bounds, 20.0);
        assert!(approx(camera.scale(), 760.0 / 100.0, 1.0e-4));
        let bottom_left = camera.world_to_screen(bounds.min());
        let top_right = camera.world_to_screen(bounds.max());
        assert!(approx(bottom_left.x, 20.0, 1.0e-4));
        assert!(approx(bottom_left.y, 490.0, 1.0e-4));
        assert!(approx(top_right.x, 780.0, 1.0e-4));
        assert!(approx(top_right.y, 110.0, 1.0e-4));
    }

    #[test]
    fn fit_ignores_empty_and_handles_degenerate_bounds() {
        let viewport = Viewport::new(800.0, 600.0);
        let mut camera = Camera2D::new(viewport);
        camera.set_scale(3.0);
        camera.fit(Bounds2::empty(), 10.0);
        assert_eq!(camera.scale(), 3.0);
        assert_eq!(camera.offset(), Point2::ZERO);

        camera.fit(Bounds2::from_point(Point2::new(7.0, 9.0)), 10.0);
        assert_eq!(camera.offset(), Point2::new(7.0, 9.0));
        assert_eq!(camera.scale(), 3.0);
        assert_eq!(
            camera.world_to_screen(Point2::new(7.0, 9.0)),
            viewport.center()
        );

        camera.fit(
            Bounds2::new(Point2::new(0.0, 5.0), Point2::new(100.0, 5.0)),
            0.0,
        );
        assert!(approx(camera.scale(), 8.0, 1.0e-4));
    }

    #[test]
    fn zoom_at_keeps_the_pivot_fixed() {
        let viewport = Viewport::new(800.0, 600.0);
        let mut camera = Camera2D::fitted(
            viewport,
            Bounds2::new(Point2::new(-40.0, -20.0), Point2::new(40.0, 20.0)),
            8.0,
        );
        let pivot = Point2::new(137.0, 421.0);
        let world_before = camera.screen_to_world(pivot);
        camera.zoom_at(pivot, 2.5);
        let screen_after = camera.world_to_screen(world_before);
        assert!(approx(screen_after.x, pivot.x, 1.0e-3));
        assert!(approx(screen_after.y, pivot.y, 1.0e-3));

        camera.zoom_at(pivot, 0.1);
        let screen_after = camera.world_to_screen(world_before);
        assert!(approx(screen_after.x, pivot.x, 1.0e-3));
        assert!(approx(screen_after.y, pivot.y, 1.0e-3));
    }

    #[test]
    fn pan_moves_content_with_the_pointer() {
        let viewport = Viewport::new(800.0, 600.0);
        let mut camera = Camera2D::fitted(
            viewport,
            Bounds2::new(Point2::new(-10.0, -10.0), Point2::new(10.0, 10.0)),
            0.0,
        );
        let world = Point2::new(3.0, -4.0);
        let before = camera.world_to_screen(world);
        let delta = Point2::new(12.0, -7.0);
        camera.pan(delta);
        let after = camera.world_to_screen(world);
        assert!(approx(after.x - before.x, delta.x, 1.0e-4));
        assert!(approx(after.y - before.y, delta.y, 1.0e-4));
    }

    #[test]
    fn screen_world_round_trip_and_visible_bounds() {
        let viewport = Viewport::new(1280.0, 800.0);
        let camera = Camera2D::fitted(
            viewport,
            Bounds2::new(Point2::new(-100.0, -50.0), Point2::new(100.0, 50.0)),
            16.0,
        );
        for screen in [
            Point2::new(0.0, 0.0),
            Point2::new(640.0, 400.0),
            Point2::new(1279.0, 799.0),
        ] {
            let world = camera.screen_to_world(screen);
            let back = camera.world_to_screen(world);
            assert!(approx(back.x, screen.x, 1.0e-3));
            assert!(approx(back.y, screen.y, 1.0e-3));
        }
        let visible = camera.visible_world_bounds();
        assert!(visible.contains(Point2::new(-100.0, -50.0)));
        assert!(visible.contains(Point2::new(100.0, 50.0)));
        assert!(visible.width() > 200.0);
        assert!(visible.height() > 100.0);
    }

    #[test]
    fn invalid_input_does_not_break_the_camera() {
        let viewport = Viewport::new(800.0, 600.0);
        let mut camera = Camera2D::new(viewport);
        camera.set_scale(f32::NAN);
        camera.set_scale(0.0);
        assert_eq!(camera.scale(), 1.0);
        camera.pan(Point2::new(f32::NAN, 1.0));
        camera.zoom_at(Point2::new(1.0, 1.0), f32::INFINITY);
        camera.zoom_at(Point2::new(1.0, 1.0), -1.0);
        assert_eq!(camera.offset(), Point2::ZERO);
        assert_eq!(camera.scale(), 1.0);
        assert!(camera.world_to_screen(Point2::ZERO).is_finite());

        // The camera is usable with a zero-sized viewport.
        let mut empty = Camera2D::new(Viewport::default());
        empty.fit(Bounds2::from_point(Point2::new(2.0, 3.0)), 0.0);
        assert_eq!(empty.offset(), Point2::new(2.0, 3.0));
        assert!(empty.world_to_screen(Point2::ZERO).is_finite());
    }
}
