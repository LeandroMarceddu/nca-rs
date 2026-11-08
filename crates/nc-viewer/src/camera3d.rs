//! Orbit camera and software 3D projection.

use std::f32::consts::{FRAC_PI_4, PI, TAU};

use crate::geom::{Bounds3, Point2, Point3};
use crate::viewport::Viewport;

/// Projection model used by [`Camera3D`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Perspective projection with a vertical field of view in radians.
    Perspective {
        /// Vertical field of view in radians.
        fov_y_rad: f32,
    },
    /// Orthographic projection with a fixed screen scale.
    Orthographic {
        /// Pixels per world unit.
        scale_px_per_unit: f32,
    },
}

impl Default for Projection {
    fn default() -> Self {
        Self::Perspective {
            fov_y_rad: FRAC_PI_4,
        }
    }
}

impl Projection {
    /// Creates a perspective projection from a vertical field of view in
    /// degrees. Invalid values are sanitized when the camera projects.
    #[must_use]
    pub fn perspective_degrees(degrees: f32) -> Self {
        Self::Perspective {
            fov_y_rad: degrees.to_radians(),
        }
    }

    /// Creates an orthographic projection.
    #[must_use]
    pub fn orthographic(scale_px_per_unit: f32) -> Self {
        Self::Orthographic { scale_px_per_unit }
    }

    /// Returns `true` for an orthographic projection.
    #[must_use]
    pub const fn is_orthographic(self) -> bool {
        matches!(self, Self::Orthographic { .. })
    }
}

/// A point projected into screen space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedPoint {
    /// Screen position in logical pixels.
    pub screen: Point2,
    /// Camera-space depth in world units; larger is farther from the eye.
    pub depth: f32,
}

/// An orbit camera looking at a target point, with world Z up.
///
/// `yaw` rotates the eye around +Z (zero is the +X side) and `pitch` lifts it
/// above the XY plane. The eye sits `distance` world units from `target` and
/// always looks at the target. Pitch is clamped just short of vertical so the
/// view basis never degenerates.
///
/// Depth is camera-space distance along the view direction: larger values are
/// farther away. A painter should draw larger depths first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3D {
    viewport: Viewport,
    target: Point3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    projection: Projection,
    z_near: f32,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self::new(Viewport::default())
    }
}

impl Camera3D {
    /// Default yaw in radians: the eye is on the +X/-Y side.
    pub const DEFAULT_YAW: f32 = -FRAC_PI_4;
    /// Default pitch in radians above the XY plane.
    pub const DEFAULT_PITCH: f32 = PI / 6.0;
    /// Default orbit distance in world units.
    pub const DEFAULT_DISTANCE: f32 = 300.0;
    /// Smallest accepted orbit distance.
    pub const MIN_DISTANCE: f32 = 0.01;
    /// Largest accepted orbit distance.
    pub const MAX_DISTANCE: f32 = 1.0e7;
    /// Smallest accepted orthographic scale in pixels per world unit.
    pub const MIN_ORTHO_SCALE: f32 = 1.0e-4;
    /// Largest accepted orthographic scale in pixels per world unit.
    pub const MAX_ORTHO_SCALE: f32 = 1.0e6;
    /// Smallest accepted near-plane distance.
    pub const MIN_Z_NEAR: f32 = 1.0e-4;
    /// Largest accepted near-plane distance.
    pub const MAX_Z_NEAR: f32 = 1.0e5;
    /// Pitch is clamped to this magnitude, just short of vertical.
    pub const PITCH_LIMIT: f32 = PI * 0.5 - 1.0e-3;
    /// Default vertical field of view in radians.
    pub const DEFAULT_FOV_Y: f32 = FRAC_PI_4;
    /// Minimum accepted vertical field of view in radians (1 degree).
    pub const MIN_FOV_Y: f32 = PI / 180.0;
    /// Maximum accepted vertical field of view in radians (170 degrees).
    pub const MAX_FOV_Y: f32 = PI * 170.0 / 180.0;
    /// Default near-plane distance in world units.
    pub const DEFAULT_Z_NEAR: f32 = 0.01;

    /// Creates a camera with the default view and a perspective projection.
    #[must_use]
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            target: Point3::ZERO,
            yaw: Self::DEFAULT_YAW,
            pitch: Self::DEFAULT_PITCH,
            distance: Self::DEFAULT_DISTANCE,
            projection: Projection::default(),
            z_near: Self::DEFAULT_Z_NEAR,
        }
    }

    /// Creates a camera with a specific projection model.
    #[must_use]
    pub fn with_projection(viewport: Viewport, projection: Projection) -> Self {
        let mut camera = Self::new(viewport);
        camera.set_projection(projection);
        camera
    }

    /// Creates a camera fitted to `bounds` with `padding_px` on every side.
    #[must_use]
    pub fn fitted(viewport: Viewport, bounds: Bounds3, padding_px: f32) -> Self {
        let mut camera = Self::new(viewport);
        camera.fit_bounds(bounds, padding_px);
        camera
    }

    /// Current viewport size.
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Changes the viewport size; the target, angles, and projection scale are
    /// unchanged.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
    }

    /// Point the camera orbits around and looks at.
    #[must_use]
    pub const fn target(&self) -> Point3 {
        self.target
    }

    /// Sets the orbit target. Non-finite input is ignored.
    pub fn set_target(&mut self, target: Point3) {
        if target.is_finite() {
            self.target = target;
        }
    }

    /// Yaw in radians, wrapped to `[-pi, pi]`.
    #[must_use]
    pub const fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Sets yaw in radians; finite values are wrapped.
    pub fn set_yaw(&mut self, yaw: f32) {
        if yaw.is_finite() {
            self.yaw = wrap_angle(yaw);
        }
    }

    /// Pitch in radians, clamped to [`Camera3D::PITCH_LIMIT`].
    #[must_use]
    pub const fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Sets pitch in radians; finite values are clamped.
    pub fn set_pitch(&mut self, pitch: f32) {
        if pitch.is_finite() {
            self.pitch = pitch.clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
        }
    }

    /// Orbit distance in world units.
    #[must_use]
    pub const fn distance(&self) -> f32 {
        self.distance
    }

    /// Sets the orbit distance, clamped to the supported range.
    pub fn set_distance(&mut self, distance: f32) {
        if distance.is_finite() && distance > 0.0 {
            self.distance = distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
        }
    }

    /// Current projection model.
    #[must_use]
    pub const fn projection(&self) -> Projection {
        self.projection
    }

    /// Sets the projection model. Invalid fields are sanitized at projection
    /// time rather than rejected here.
    pub fn set_projection(&mut self, projection: Projection) {
        self.projection = projection;
    }

    /// Near-plane distance used for culling and perspective division.
    #[must_use]
    pub const fn z_near(&self) -> f32 {
        self.z_near
    }

    /// Sets the near-plane distance, clamped to a positive finite range.
    pub fn set_z_near(&mut self, z_near: f32) {
        if z_near.is_finite() && z_near > 0.0 {
            self.z_near = z_near.clamp(Self::MIN_Z_NEAR, Self::MAX_Z_NEAR);
        }
    }

    /// Orbit around the target by angle deltas in radians.
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        if delta_yaw.is_finite() {
            self.yaw = wrap_angle(self.yaw + delta_yaw);
        }
        if delta_pitch.is_finite() {
            self.pitch = (self.pitch + delta_pitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
        }
    }

    /// Moves the target in the camera's screen plane by a pixel delta.
    ///
    /// Content follows the pointer: a drag to the right moves the drawing
    /// right, a drag down moves it down. The world-units-per-pixel scale is
    /// evaluated at the target's depth.
    pub fn pan(&mut self, delta_px: Point2) {
        if !delta_px.is_finite() {
            return;
        }
        let units_per_pixel = self.units_per_pixel();
        if !units_per_pixel.is_finite() || units_per_pixel <= 0.0 {
            return;
        }
        let right = self.right();
        let up = self.up();
        let next = self.target - right * (delta_px.x * units_per_pixel)
            + up * (delta_px.y * units_per_pixel);
        if next.is_finite() {
            self.target = next;
        }
    }

    /// Zooms in with a factor greater than one and out below one.
    ///
    /// Perspective cameras reduce their orbit distance; orthographic cameras
    /// multiply their pixel scale. Non-finite or non-positive factors are
    /// ignored.
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        match self.projection {
            Projection::Perspective { .. } => {
                self.distance =
                    (self.distance / factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
            }
            Projection::Orthographic { .. } => {
                let scale = self.ortho_scale() * factor;
                if scale.is_finite() {
                    self.projection = Projection::Orthographic {
                        scale_px_per_unit: scale
                            .clamp(Self::MIN_ORTHO_SCALE, Self::MAX_ORTHO_SCALE),
                    };
                }
            }
        }
    }

    /// Fits `bounds` into the viewport with `padding_px` on every side.
    ///
    /// The target moves to the bounds center and the distance or orthographic
    /// scale is chosen so the bounds' bounding sphere fits. Empty bounds are
    /// ignored; point-sized bounds only recenter.
    pub fn fit_bounds(&mut self, bounds: Bounds3, padding_px: f32) {
        if bounds.is_empty() {
            return;
        }
        self.target = bounds.center();
        let radius = bounds.size().length() * 0.5;
        if !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let padding = if padding_px.is_finite() && padding_px > 0.0 {
            padding_px
        } else {
            0.0
        };
        let width = (self.viewport.width() - 2.0 * padding).max(1.0);
        let height = (self.viewport.height() - 2.0 * padding).max(1.0);
        let aspect = width / height;

        match self.projection {
            Projection::Perspective { .. } => {
                let fov_y = self.fov_y();
                let fov_x = 2.0 * ((fov_y * 0.5).tan() * aspect).atan();
                let limiting_fov = fov_y.min(fov_x).clamp(Self::MIN_FOV_Y, Self::MAX_FOV_Y);
                let distance = radius / (limiting_fov * 0.5).sin();
                if distance.is_finite() && distance > 0.0 {
                    self.distance = distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
                }
            }
            Projection::Orthographic { .. } => {
                let scale = width.min(height) / (radius * 2.0);
                if scale.is_finite() && scale > 0.0 {
                    self.projection = Projection::Orthographic {
                        scale_px_per_unit: scale
                            .clamp(Self::MIN_ORTHO_SCALE, Self::MAX_ORTHO_SCALE),
                    };
                    // Orthographic projection itself ignores distance, but the
                    // near plane still culls. Put the eye outside the bounding
                    // sphere so no fitted geometry is dropped.
                    self.distance =
                        (radius * 2.0 + self.z_near).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
                }
            }
        }
    }

    /// Eye position in world space.
    #[must_use]
    pub fn eye(&self) -> Point3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        self.target
            + Point3::new(cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch) * self.distance
    }

    /// Unit vector from the eye toward the target.
    #[must_use]
    pub fn forward(&self) -> Point3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        -Point3::new(cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch)
    }

    /// Unit screen-right vector in world space.
    #[must_use]
    pub fn right(&self) -> Point3 {
        self.forward()
            .cross(Point3::new(0.0, 0.0, 1.0))
            .normalized()
            .unwrap_or(Point3::new(1.0, 0.0, 0.0))
    }

    /// Unit screen-up vector in world space.
    #[must_use]
    pub fn up(&self) -> Point3 {
        self.right()
            .cross(self.forward())
            .normalized()
            .unwrap_or(Point3::new(0.0, 0.0, 1.0))
    }

    /// Transforms a world point into camera space.
    ///
    /// X is screen right, Y is screen up, and Z is depth along the view
    /// direction (positive in front of the eye).
    #[must_use]
    pub fn view_point(&self, world: Point3) -> Point3 {
        let relative = world - self.eye();
        Point3::new(
            relative.dot(self.right()),
            relative.dot(self.up()),
            relative.dot(self.forward()),
        )
    }

    /// Projects a world point to screen space.
    ///
    /// Returns `None` when the point is at or behind the near plane or when
    /// the projection is not finite.
    ///
    /// Repeated projection of many points uses an internal precomputed
    /// projector so the view basis and projection constants are computed only
    /// once per plan.
    #[must_use]
    pub fn project(&self, world: Point3) -> Option<ProjectedPoint> {
        self.projector().project(world)
    }

    /// Captures the current view basis and projection constants for fast
    /// batched projection.
    ///
    /// The projector is a snapshot: later camera changes do not affect it.
    #[must_use]
    pub(crate) fn projector(&self) -> ViewProjector {
        let forward = self.forward();
        let right = forward
            .cross(Point3::new(0.0, 0.0, 1.0))
            .normalized()
            .unwrap_or(Point3::new(1.0, 0.0, 0.0));
        let up = right
            .cross(forward)
            .normalized()
            .unwrap_or(Point3::new(0.0, 0.0, 1.0));
        let mode = match self.projection {
            Projection::Perspective { .. } => {
                let focal = (self.viewport.height() * 0.5) / (self.fov_y() * 0.5).tan();
                ProjectorMode::Perspective { focal }
            }
            Projection::Orthographic { .. } => ProjectorMode::Orthographic {
                scale: self.ortho_scale(),
            },
        };
        ViewProjector {
            eye: self.eye(),
            right,
            up,
            forward,
            center: self.viewport.center(),
            mode,
            z_near: self.z_near,
        }
    }

    /// World units represented by one screen pixel at the target depth.
    #[must_use]
    pub fn units_per_pixel(&self) -> f32 {
        match self.projection {
            Projection::Perspective { .. } => {
                let height = self.viewport.height().max(1.0);
                2.0 * self.distance * (self.fov_y() * 0.5).tan() / height
            }
            Projection::Orthographic { .. } => 1.0 / self.ortho_scale(),
        }
    }

    fn fov_y(&self) -> f32 {
        match self.projection {
            Projection::Perspective { fov_y_rad } => {
                if fov_y_rad.is_finite() {
                    fov_y_rad.clamp(Self::MIN_FOV_Y, Self::MAX_FOV_Y)
                } else {
                    Self::DEFAULT_FOV_Y
                }
            }
            Projection::Orthographic { .. } => Self::DEFAULT_FOV_Y,
        }
    }

    fn ortho_scale(&self) -> f32 {
        match self.projection {
            Projection::Orthographic { scale_px_per_unit } => {
                if scale_px_per_unit.is_finite() && scale_px_per_unit > 0.0 {
                    scale_px_per_unit.clamp(Self::MIN_ORTHO_SCALE, Self::MAX_ORTHO_SCALE)
                } else {
                    1.0
                }
            }
            Projection::Perspective { .. } => 1.0,
        }
    }
}

/// Precomputed basis for projecting many points with one camera snapshot.
pub(crate) struct ViewProjector {
    eye: Point3,
    right: Point3,
    up: Point3,
    forward: Point3,
    center: Point2,
    mode: ProjectorMode,
    z_near: f32,
}

enum ProjectorMode {
    Perspective { focal: f32 },
    Orthographic { scale: f32 },
}

impl ViewProjector {
    /// Projects one world point. See [`Camera3D::project`].
    pub(crate) fn project(&self, world: Point3) -> Option<ProjectedPoint> {
        if !world.is_finite() {
            return None;
        }
        let relative = world - self.eye;
        let x = relative.dot(self.right);
        let y = relative.dot(self.up);
        let z = relative.dot(self.forward);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() || z <= self.z_near {
            return None;
        }
        let screen = match self.mode {
            ProjectorMode::Perspective { focal } => {
                Point2::new(self.center.x + x * focal / z, self.center.y - y * focal / z)
            }
            ProjectorMode::Orthographic { scale } => {
                Point2::new(self.center.x + x * scale, self.center.y - y * scale)
            }
        };
        if !screen.is_finite() {
            return None;
        }
        Some(ProjectedPoint { screen, depth: z })
    }
}

/// Wraps an angle to `[-pi, pi]`.
fn wrap_angle(angle: f32) -> f32 {
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI { wrapped - TAU } else { wrapped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() <= epsilon
    }

    #[test]
    fn target_projects_to_viewport_center() {
        let camera = Camera3D::new(Viewport::new(800.0, 600.0));
        let projected = camera.project(Point3::ZERO).expect("target visible");
        assert!(approx(projected.screen.x, 400.0, 1.0e-4));
        assert!(approx(projected.screen.y, 300.0, 1.0e-4));
        assert!(approx(projected.depth, camera.distance(), 1.0e-4));
    }

    #[test]
    fn view_basis_is_orthonormal() {
        let camera = Camera3D::new(Viewport::new(800.0, 600.0));
        let forward = camera.forward();
        let right = camera.right();
        let up = camera.up();
        assert!(approx(forward.length(), 1.0, 1.0e-5));
        assert!(approx(right.length(), 1.0, 1.0e-5));
        assert!(approx(up.length(), 1.0, 1.0e-5));
        assert!(approx(forward.dot(right), 0.0, 1.0e-5));
        assert!(approx(forward.dot(up), 0.0, 1.0e-5));
        assert!(approx(right.dot(up), 0.0, 1.0e-5));
    }

    #[test]
    fn orbit_and_pitch_clamp() {
        let mut camera = Camera3D::new(Viewport::new(800.0, 600.0));
        let eye_before = camera.eye();
        camera.orbit(PI * 0.5, 0.0);
        assert!(camera.eye().distance(eye_before) > 1.0);

        camera.set_pitch(10.0);
        assert!(camera.pitch() <= Camera3D::PITCH_LIMIT);
        camera.set_pitch(-10.0);
        assert!(camera.pitch() >= -Camera3D::PITCH_LIMIT);
        let projected = camera.project(Point3::ZERO).expect("target visible");
        assert!(projected.screen.is_finite());
        assert!(camera.right().is_finite());
        assert!(camera.up().is_finite());
    }

    #[test]
    fn zoom_changes_distance_or_scale() {
        let mut perspective = Camera3D::new(Viewport::new(800.0, 600.0));
        let distance = perspective.distance();
        perspective.zoom(2.0);
        assert!(approx(perspective.distance(), distance / 2.0, 1.0e-4));

        let mut ortho =
            Camera3D::with_projection(Viewport::new(800.0, 600.0), Projection::orthographic(2.0));
        ortho.zoom(3.0);
        assert_eq!(
            ortho.projection(),
            Projection::Orthographic {
                scale_px_per_unit: 6.0
            }
        );
        ortho.set_projection(Projection::orthographic(0.0));
        assert!(ortho.project(Point3::ZERO).is_some());
        assert_eq!(ortho.units_per_pixel(), 1.0);
    }

    #[test]
    fn orthographic_projection_maps_axes() {
        let mut camera =
            Camera3D::with_projection(Viewport::new(800.0, 600.0), Projection::orthographic(2.0));
        camera.set_target(Point3::ZERO);
        camera.set_yaw(-PI * 0.5);
        camera.set_pitch(0.0);
        camera.set_distance(100.0);

        let point = Point3::new(10.0, 20.0, 20.0);
        let projected = camera.project(point).expect("visible");
        assert!(approx(projected.screen.x, 400.0 + 20.0, 1.0e-4));
        assert!(approx(projected.screen.y, 300.0 - 40.0, 1.0e-4));
        assert!(approx(projected.depth, 120.0, 1.0e-4));
    }

    #[test]
    fn near_plane_culling() {
        let mut camera = Camera3D::new(Viewport::new(800.0, 600.0));
        camera.set_yaw(-PI * 0.5);
        camera.set_pitch(0.0);
        camera.set_distance(100.0);
        let eye = camera.eye();
        let forward = camera.forward();
        assert!(camera.project(eye).is_none());
        assert!(camera.project(eye - forward * 10.0).is_none());
        assert!(camera.project(eye + forward * 1.0).is_some());
        assert!(camera.project(Point3::new(f32::NAN, 0.0, 0.0)).is_none());
    }

    #[test]
    fn pan_moves_content_with_the_pointer() {
        let mut camera = Camera3D::new(Viewport::new(800.0, 600.0));
        let before = camera.project(Point3::ZERO).expect("visible").screen;
        let delta = Point2::new(15.0, -9.0);
        camera.pan(delta);
        let after = camera.project(Point3::ZERO).expect("visible").screen;
        assert!(approx(after.x - before.x, delta.x, 1.0e-3));
        assert!(approx(after.y - before.y, delta.y, 1.0e-3));
    }

    #[test]
    fn fit_bounds_recenters_and_keeps_geometry_visible() {
        let bounds = Bounds3::new(
            Point3::new(-50.0, -30.0, -5.0),
            Point3::new(50.0, 30.0, 5.0),
        );
        let camera = Camera3D::fitted(Viewport::new(800.0, 600.0), bounds, 10.0);
        assert_eq!(camera.target(), bounds.center());
        assert!(camera.distance() > 0.0);
        for corner in [
            bounds.min(),
            bounds.max(),
            Point3::new(bounds.min().x, bounds.max().y, bounds.min().z),
            Point3::new(bounds.max().x, bounds.min().y, bounds.max().z),
        ] {
            let projected = camera.project(corner).expect("corner visible");
            assert!(projected.screen.is_finite());
        }

        let mut point_camera = Camera3D::new(Viewport::new(800.0, 600.0));
        point_camera.fit_bounds(Bounds3::from_point(Point3::new(1.0, 2.0, 3.0)), 0.0);
        assert_eq!(point_camera.target(), Point3::new(1.0, 2.0, 3.0));
        point_camera.fit_bounds(Bounds3::empty(), 0.0);
        assert_eq!(point_camera.target(), Point3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn orthographic_fit_keeps_large_geometry_in_front_of_the_near_plane() {
        let bounds = Bounds3::new(
            Point3::new(-5000.0, -3000.0, -100.0),
            Point3::new(5000.0, 3000.0, 100.0),
        );
        let mut camera =
            Camera3D::with_projection(Viewport::new(800.0, 600.0), Projection::orthographic(0.1));
        camera.fit_bounds(bounds, 20.0);
        assert_eq!(camera.target(), bounds.center());
        assert!(camera.distance() > 3000.0);
        for corner in [
            bounds.min(),
            bounds.max(),
            Point3::new(bounds.min().x, bounds.max().y, bounds.min().z),
            Point3::new(bounds.max().x, bounds.min().y, bounds.max().z),
        ] {
            assert!(camera.project(corner).is_some());
        }
    }

    #[test]
    fn invalid_camera_input_is_ignored() {
        let mut camera = Camera3D::new(Viewport::new(800.0, 600.0));
        camera.set_target(Point3::new(f32::NAN, 0.0, 0.0));
        assert_eq!(camera.target(), Point3::ZERO);
        camera.set_distance(-1.0);
        assert_eq!(camera.distance(), Camera3D::DEFAULT_DISTANCE);
        camera.set_z_near(0.0);
        assert_eq!(camera.z_near(), Camera3D::DEFAULT_Z_NEAR);
        camera.zoom(f32::NAN);
        camera.zoom(0.0);
        camera.orbit(f32::NAN, f32::NAN);
        assert!(camera.project(Point3::ZERO).is_some());
        assert!(camera.eye().is_finite());
    }
}
