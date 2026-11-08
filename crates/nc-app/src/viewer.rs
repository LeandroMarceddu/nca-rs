//! Interactive 2D/3D toolpath canvas for the read-only viewer.
//!
//! The canvas draws the parsed program through `nc-viewer` in one of two
//! modes:
//!
//! - [`ViewerMode::Top2D`] (the default) uses a [`Camera2D`] fitted to the
//!   program bounds, a cached 2D [`RenderPlan`], and a world-space grid.
//! - [`ViewerMode::ThreeD`] uses an orbit [`Camera3D`] with Z up, a cached 3D
//!   [`RenderPlan`] painted far-to-near, a ground grid on Z=0, the stock box
//!   from `nc-stock`, and a dashed work-envelope box.
//!
//! Gestures: in 2D a drag pans, the wheel or a pinch zooms at the cursor, and
//! a double click or FIT refits. In 3D a left drag orbits, a shift drag or
//! middle drag pans, and the wheel or a pinch zooms at the cursor.
//!
//! A plan is cached by program identity, segment count, the active camera
//! transform, and viewport size. Moving the tool marker never touches the
//! cache: the marker is an argument to paint, not part of the key.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    Bounds, Entity, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    PathBuilder, Pixels, Point, ScrollDelta, Size, Styled as _, Window, canvas, div, fill, point,
    px, size,
};
use nc_gcode::Program;
use nc_viewer::{
    Bounds2, Bounds3, Camera2D, Camera3D, MoveKind, MoveStyle, Point2, Point3, RenderOptions,
    RenderPlan, SegmentKind, StockBox, Toolpath, Viewport,
};

use crate::theme::{Palette, move_color};

/// Screen-space padding around the fitted toolpath, in logical pixels.
const VIEW_PADDING_PX: f32 = 48.0;

/// Orbit sensitivity: radians of yaw or pitch per logical drag pixel.
const ORBIT_RAD_PER_PX: f32 = 0.007;

/// Ground-grid margin around the toolpath footprint, as a fraction of its size.
const GRID_MARGIN_FRACTION: f32 = 0.35;

/// Segment count of the synthetic preview used when no program is loaded.
const SYNTHETIC_SEGMENTS: usize = 6_000;

/// Guard against a pathological camera producing an unbounded grid loop.
const MAX_GRID_LINES: i64 = 512;

/// Which projection the viewer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerMode {
    /// Orthographic top view with the world Y axis up.
    #[default]
    Top2D,
    /// Orbiting perspective view with world Z up.
    ThreeD,
}

impl ViewerMode {
    /// Short label for the toolbar chip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Top2D => "2D TOP",
            Self::ThreeD => "3D",
        }
    }

    /// Returns `true` for the orbit view.
    #[must_use]
    pub const fn is_three_d(self) -> bool {
        matches!(self, Self::ThreeD)
    }
}

/// What an in-flight pointer drag is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragKind {
    /// 2D camera pan (top view).
    Pan2D,
    /// 3D orbit around the target.
    Orbit3D,
    /// 3D camera pan in the screen plane.
    Pan3D,
}

/// One in-flight pointer drag.
#[derive(Clone, Copy, Debug)]
struct DragState {
    at: Point<Pixels>,
    kind: DragKind,
}

/// Shared camera and cached plan behind the canvas.
///
/// The handle is cheap to clone (one `Rc`) and is what the paint closure and
/// the pointer handlers both hold, so a repaint never rebuilds the plan just
/// because the programme list or status strip re-rendered.
#[derive(Clone)]
pub struct ViewerHandle {
    inner: Rc<RefCell<Viewer>>,
}

impl Default for ViewerHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewerHandle {
    /// Creates an unfitted viewer. The first paint fits the current program.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Viewer::default())),
        }
    }

    /// Records a primary drag start at a window-space pointer position.
    pub fn begin_drag(&self, at: Point<Pixels>) -> bool {
        let mut viewer = self.inner.borrow_mut();
        let kind = if viewer.mode.is_three_d() {
            DragKind::Orbit3D
        } else {
            DragKind::Pan2D
        };
        viewer.drag = Some(DragState { at, kind });
        false
    }

    /// Records a pan drag start (shift drag or middle drag).
    pub fn begin_pan_drag(&self, at: Point<Pixels>) -> bool {
        let mut viewer = self.inner.borrow_mut();
        let kind = if viewer.mode.is_three_d() {
            DragKind::Pan3D
        } else {
            DragKind::Pan2D
        };
        viewer.drag = Some(DragState { at, kind });
        false
    }

    /// Applies the pointer delta since the last position for the active drag.
    pub fn drag_to(&self, at: Point<Pixels>) -> bool {
        let mut viewer = self.inner.borrow_mut();
        let Some(last) = viewer.drag else {
            return false;
        };
        viewer.drag = Some(DragState {
            at,
            kind: last.kind,
        });
        let delta = Point2::new(f32::from(at.x - last.at.x), f32::from(at.y - last.at.y));
        if !delta.is_finite() || delta.length_squared() <= f32::EPSILON {
            return false;
        }
        match last.kind {
            DragKind::Pan2D => viewer.camera.pan(delta),
            DragKind::Orbit3D => viewer
                .camera3d
                .orbit(-delta.x * ORBIT_RAD_PER_PX, -delta.y * ORBIT_RAD_PER_PX),
            DragKind::Pan3D => viewer.camera3d.pan(delta),
        }
        true
    }

    /// Ends the current drag gesture.
    pub fn end_drag(&self) {
        self.inner.borrow_mut().drag = None;
    }

    /// Zooms around a window-space pointer position.
    pub fn zoom_at(&self, at: Point<Pixels>, factor: f32) -> bool {
        let mut viewer = self.inner.borrow_mut();
        let Some(rect) = viewer.rect else {
            return false;
        };
        let local = local_point(rect, at);
        if viewer.mode.is_three_d() {
            let viewport = viewer.camera3d.viewport();
            if viewport.is_empty() || !local.is_finite() || !factor.is_finite() || factor <= 0.0 {
                return false;
            }
            let before_distance = viewer.camera3d.distance();
            let before_target = viewer.camera3d.target();
            let units_per_pixel = viewer.camera3d.units_per_pixel();
            if units_per_pixel.is_finite() && units_per_pixel > 0.0 {
                let center = viewport.center();
                let offset_x = local.x - center.x;
                let offset_y = center.y - local.y;
                let k = 1.0 - 1.0 / factor;
                viewer
                    .camera3d
                    .pan(Point2::new(-offset_x * k, offset_y * k));
            }
            viewer.camera3d.zoom(factor);
            return viewer.camera3d.distance() != before_distance
                || viewer.camera3d.target() != before_target;
        }
        let before = viewer.camera.scale();
        viewer.camera.zoom_at(local, factor);
        let after = viewer.camera.scale();
        (after - before).abs() > f32::EPSILON * before.max(1.0)
    }

    /// Refits the active camera on the next paint.
    pub fn fit(&self) -> bool {
        let mut viewer = self.inner.borrow_mut();
        if viewer.mode.is_three_d() {
            viewer.pending_fit_3d = true;
        } else {
            viewer.pending_fit = true;
        }
        true
    }

    /// Switches the projection mode and refits the destination camera once.
    pub fn set_mode(&self, mode: ViewerMode) -> bool {
        let mut viewer = self.inner.borrow_mut();
        if viewer.mode == mode {
            return false;
        }
        viewer.mode = mode;
        if mode.is_three_d() {
            viewer.pending_fit_3d = true;
        }
        true
    }

    /// Current projection mode.
    #[must_use]
    pub fn mode(&self) -> ViewerMode {
        self.inner.borrow().mode
    }

    /// Whether the stock box is painted.
    #[must_use]
    pub fn stock_visible(&self) -> bool {
        self.inner.borrow().stock_visible
    }

    /// Shows or hides the stock box.
    pub fn set_stock_visible(&self, visible: bool) -> bool {
        let mut viewer = self.inner.borrow_mut();
        if viewer.stock_visible == visible {
            return false;
        }
        viewer.stock_visible = visible;
        true
    }

    /// Current zoom relative to the fitted scale, as a whole percentage.
    #[must_use]
    pub fn zoom_percent(&self) -> u32 {
        let viewer = self.inner.borrow();
        if viewer.mode.is_three_d() {
            let fit = viewer.fit_distance3d;
            let distance = viewer.camera3d.distance();
            if fit > 0.0 && fit.is_finite() && distance > 0.0 && distance.is_finite() {
                return (fit / distance * 100.0).round().clamp(1.0, 100_000.0) as u32;
            }
            return 100;
        }
        let fit = viewer.fit_scale;
        if fit > 0.0 && fit.is_finite() {
            (viewer.camera.scale() / fit * 100.0)
                .round()
                .clamp(1.0, 100_000.0) as u32
        } else {
            100
        }
    }
}

/// A view that owns a [`ViewerHandle`], so canvas gestures can redraw it.
pub trait HasViewer {
    /// The canvas state shared with the paint closure.
    fn viewer_handle(&self) -> &ViewerHandle;
}

/// Draws the interactive toolpath canvas.
///
/// `program` is the parsed program; `None` draws the deterministic synthetic
/// preview. `stock` is the display box inferred by `nc-stock` or entered by
/// the operator. `marker` is the tool position at the program start. The
/// `visibility` array gates the five display segment groups.
#[allow(clippy::too_many_arguments)]
pub fn view<T: HasViewer + 'static>(
    handle: &ViewerHandle,
    palette: Palette,
    program: Option<Arc<Program>>,
    stock: Option<StockBox>,
    marker: Option<Point3>,
    visibility: [bool; 5],
    preview: bool,
    view: Entity<T>,
) -> impl IntoElement {
    let canvas_element = {
        let measure = handle.clone();
        let paint_handle = handle.clone();
        canvas(
            move |bounds: Bounds<Pixels>, _, _| {
                measure.inner.borrow_mut().rect = Some(bounds);
            },
            move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
                paint(
                    bounds,
                    &paint_handle,
                    program.clone(),
                    stock,
                    marker,
                    visibility,
                    preview,
                    palette,
                    window,
                );
            },
        )
        .size_full()
    };

    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, {
            let view = view.clone();
            move |event, _, cx| {
                let at = event.position;
                let double_click = event.click_count > 1;
                let pan = event.modifiers.shift;
                view.update(cx, |this, cx| {
                    let viewer = this.viewer_handle();
                    let changed = if double_click {
                        viewer.fit()
                    } else if pan {
                        viewer.begin_pan_drag(at)
                    } else {
                        viewer.begin_drag(at)
                    };
                    if changed {
                        cx.notify();
                    }
                });
            }
        })
        .on_mouse_down(MouseButton::Middle, {
            let view = view.clone();
            move |event, _, cx| {
                let at = event.position;
                view.update(cx, |this, cx| {
                    if this.viewer_handle().begin_pan_drag(at) {
                        cx.notify();
                    }
                });
            }
        })
        .on_mouse_move({
            let view = view.clone();
            move |event, _, cx| {
                if !matches!(
                    event.pressed_button,
                    Some(MouseButton::Left | MouseButton::Middle)
                ) {
                    return;
                }
                let at = event.position;
                view.update(cx, |this, cx| {
                    if this.viewer_handle().drag_to(at) {
                        cx.notify();
                    }
                });
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |this, _| this.viewer_handle().end_drag());
            }
        })
        .on_mouse_up(MouseButton::Middle, {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |this, _| this.viewer_handle().end_drag());
            }
        })
        .on_scroll({
            let view = view.clone();
            move |event, _, cx| {
                let factor = scroll_zoom_factor(event.delta);
                let at = event.position;
                if (factor - 1.0).abs() <= 0.001 {
                    return;
                }
                view.update(cx, |this, cx| {
                    if this.viewer_handle().zoom_at(at, factor) {
                        cx.notify();
                    }
                });
            }
        })
        .on_pinch({
            let view = view.clone();
            move |event, _, cx| {
                let factor = (1.0_f32 + event.delta).clamp(0.05, 20.0);
                let at = event.position;
                view.update(cx, |this, cx| {
                    if this.viewer_handle().zoom_at(at, factor) {
                        cx.notify();
                    }
                });
            }
        })
        .child(canvas_element)
}

/// Converts a window-space pointer position into canvas-local coordinates.
fn local_point(rect: Bounds<Pixels>, at: Point<Pixels>) -> Point2 {
    Point2::new(
        f32::from(at.x) - f32::from(rect.origin.x),
        f32::from(at.y) - f32::from(rect.origin.y),
    )
}

/// Maps one wheel or trackpad scroll delta to a zoom factor.
fn scroll_zoom_factor(delta: ScrollDelta) -> f32 {
    match delta {
        ScrollDelta::Lines(delta) => 1.2_f32.powf(delta.y.clamp(-8.0, 8.0)),
        ScrollDelta::Pixels(delta) => {
            let dy = f32::from(delta.y).clamp(-240.0, 240.0);
            (1.0 + dy * 0.0025).clamp(0.2, 5.0)
        }
    }
}

/// Cached render plan and the state it was built for.
#[derive(Default)]
struct PlanCache {
    key: Option<PlanKey>,
    plan: RenderPlan,
}

/// Identity of one render plan: program, geometry, and camera transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlanKey {
    program: usize,
    segments: usize,
    scale: u32,
    offset_x: u32,
    offset_y: u32,
    width: u32,
    height: u32,
}

impl PlanKey {
    fn new(program: usize, segments: usize, camera: &Camera2D) -> Self {
        let offset = camera.offset();
        let viewport = camera.viewport();
        Self {
            program,
            segments,
            scale: camera.scale().to_bits(),
            offset_x: offset.x.to_bits(),
            offset_y: offset.y.to_bits(),
            width: viewport.width().to_bits(),
            height: viewport.height().to_bits(),
        }
    }
}

/// Cached 3D render plan and the camera/viewport it was built for.
#[derive(Default)]
struct PlanCache3D {
    key: Option<PlanKey3D>,
    plan: RenderPlan,
}

/// Identity of one 3D render plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlanKey3D {
    program: usize,
    segments: usize,
    target_x: u32,
    target_y: u32,
    target_z: u32,
    yaw: u32,
    pitch: u32,
    distance: u32,
    width: u32,
    height: u32,
}

impl PlanKey3D {
    fn new(program: usize, segments: usize, camera: &Camera3D) -> Self {
        let target = camera.target();
        let viewport = camera.viewport();
        Self {
            program,
            segments,
            target_x: target.x.to_bits(),
            target_y: target.y.to_bits(),
            target_z: target.z.to_bits(),
            yaw: camera.yaw().to_bits(),
            pitch: camera.pitch().to_bits(),
            distance: camera.distance().to_bits(),
            width: viewport.width().to_bits(),
            height: viewport.height().to_bits(),
        }
    }
}

/// Mutable viewer state shared through [`ViewerHandle`].
struct Viewer {
    mode: ViewerMode,
    camera: Camera2D,
    camera3d: Camera3D,
    path_bounds: Bounds2,
    path_bounds3: Bounds3,
    stock_visible: bool,
    identity: usize,
    /// Keeps the current program alive so a freed allocation cannot reuse its
    /// address while a plan keyed by that address is still cached.
    identity_program: Option<Arc<Program>>,
    fit_scale: f32,
    fit_distance3d: f32,
    pending_fit: bool,
    pending_fit_3d: bool,
    rect: Option<Bounds<Pixels>>,
    drag: Option<DragState>,
    synthetic: Option<Arc<Toolpath>>,
    empty: Option<Arc<Toolpath>>,
    /// Whether an absent program should draw the built-in synthetic preview.
    preview: bool,
    cache: PlanCache,
    cache3d: PlanCache3D,
    #[cfg(test)]
    plan_builds: u32,
    #[cfg(test)]
    plan_builds_3d: u32,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            mode: ViewerMode::default(),
            camera: Camera2D::default(),
            camera3d: Camera3D::default(),
            path_bounds: Bounds2::default(),
            path_bounds3: Bounds3::default(),
            stock_visible: true,
            identity: 0,
            identity_program: None,
            fit_scale: 0.0,
            fit_distance3d: 0.0,
            pending_fit: false,
            pending_fit_3d: false,
            rect: None,
            drag: None,
            synthetic: None,
            empty: None,
            preview: true,
            cache: PlanCache::default(),
            cache3d: PlanCache3D::default(),
            #[cfg(test)]
            plan_builds: 0,
            #[cfg(test)]
            plan_builds_3d: 0,
        }
    }
}

impl Viewer {
    /// Updates the cameras for the current viewport and program, then ensures
    /// the active render plan matches.
    fn prepare(&mut self, size: Size<Pixels>, program: Option<&Arc<Program>>) -> Camera2D {
        let viewport = Viewport::new(f32::from(size.width), f32::from(size.height));
        let identity = program.map_or(0, |program| Arc::as_ptr(program) as usize);
        let synthetic: Arc<Toolpath>;
        let empty: Arc<Toolpath>;
        let path: &Toolpath = match program {
            Some(program) => &program.toolpath,
            None if self.preview => {
                synthetic = Arc::clone(self.synthetic.get_or_insert_with(|| {
                    Arc::new(nc_viewer::synthetic_toolpath(SYNTHETIC_SEGMENTS))
                }));
                synthetic.as_ref()
            }
            None => {
                empty = Arc::clone(self.empty.get_or_insert_with(|| Arc::new(Toolpath::new())));
                empty.as_ref()
            }
        };

        if viewport != self.camera.viewport() {
            self.camera.set_viewport(viewport);
            self.pending_fit = true;
        }
        if viewport != self.camera3d.viewport() {
            self.camera3d.set_viewport(viewport);
            self.pending_fit_3d = true;
        }
        if identity != self.identity {
            self.identity = identity;
            self.identity_program = program.cloned();
            self.pending_fit = true;
            self.pending_fit_3d = true;
        }
        self.path_bounds = path.bounds().xy_bounds();
        self.path_bounds3 = path.bounds();
        if self.pending_fit && !self.path_bounds.is_empty() && !viewport.is_empty() {
            self.camera.fit(self.path_bounds, VIEW_PADDING_PX);
            self.fit_scale = self.camera.scale();
            self.pending_fit = false;
        }
        if self.pending_fit_3d && !self.path_bounds3.is_empty() && !viewport.is_empty() {
            self.camera3d.fit_bounds(self.path_bounds3, VIEW_PADDING_PX);
            self.fit_distance3d = self.camera3d.distance();
            self.pending_fit_3d = false;
        }

        if self.mode.is_three_d() {
            let key3d = PlanKey3D::new(identity, path.segment_count(), &self.camera3d);
            if self.cache3d.key != Some(key3d) {
                self.cache3d.plan =
                    RenderPlan::from_3d(path, &self.camera3d, RenderOptions::default());
                self.cache3d.key = Some(key3d);
                #[cfg(test)]
                {
                    self.plan_builds_3d += 1;
                }
            }
        } else {
            let key = PlanKey::new(identity, path.segment_count(), &self.camera);
            if self.cache.key != Some(key) {
                self.cache.plan = RenderPlan::from_2d(path, &self.camera, RenderOptions::default());
                self.cache.key = Some(key);
                #[cfg(test)]
                {
                    self.plan_builds += 1;
                }
            }
        }
        self.camera
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    bounds: Bounds<Pixels>,
    handle: &ViewerHandle,
    program: Option<Arc<Program>>,
    stock: Option<StockBox>,
    marker: Option<Point3>,
    visibility: [bool; 5],
    preview: bool,
    palette: Palette,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width < 24.0 || height < 24.0 {
        return;
    }

    let mut viewer = handle.inner.borrow_mut();
    viewer.preview = preview;
    let camera = viewer.prepare(bounds.size, program.as_ref());

    if viewer.mode.is_three_d() {
        let camera3d = viewer.camera3d;
        paint_ground_grid_3d(&camera3d, viewer.path_bounds3, bounds, palette, window);
        if viewer.stock_visible
            && let Some(stock) = stock
        {
            paint_stock_3d(&camera3d, &stock, bounds, palette, window);
        }
        if !viewer.path_bounds3.is_empty() {
            paint_envelope_3d(&camera3d, viewer.path_bounds3, bounds, palette, window);
        }
        paint_plan_3d(bounds, &viewer.cache3d.plan, visibility, palette, window);
        if let Some(marker) = marker {
            paint_marker_3d(&camera3d, bounds, marker, palette, window);
        }
        return;
    }

    paint_grid(&camera, bounds, palette, window);
    if !viewer.path_bounds.is_empty() {
        paint_envelope(&camera, viewer.path_bounds, bounds, palette, window);
    }
    paint_plan(bounds, &viewer.cache.plan, visibility, palette, window);
    if let Some(marker) = marker {
        paint_marker(&camera, bounds, marker.xy(), palette, window);
    }
}

/// Paints a world-aligned grid with major lines every fifth step.
fn paint_grid(camera: &Camera2D, bounds: Bounds<Pixels>, palette: Palette, window: &mut Window) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    let visible = camera.visible_world_bounds();
    if visible.is_empty() || !visible.width().is_finite() || !visible.height().is_finite() {
        return;
    }

    let step = grid_step_mm(camera.scale());
    let mut minor = PathBuilder::stroke(px(1.0));
    let mut major = PathBuilder::stroke(px(1.0));

    let first_x = grid_index(visible.min().x, step);
    for index in first_x..=first_x + MAX_GRID_LINES {
        let x = index as f32 * step;
        if x > visible.max().x {
            break;
        }
        let screen = camera.world_to_screen(Point2::new(x, 0.0));
        let line_x = left + screen.x;
        let target = if index.rem_euclid(5) == 0 {
            &mut major
        } else {
            &mut minor
        };
        target.move_to(point(px(line_x), px(top)));
        target.line_to(point(px(line_x), px(bottom)));
    }

    let first_y = grid_index(visible.min().y, step);
    for index in first_y..=first_y + MAX_GRID_LINES {
        let y = index as f32 * step;
        if y > visible.max().y {
            break;
        }
        let screen = camera.world_to_screen(Point2::new(0.0, y));
        let line_y = top + screen.y;
        let target = if index.rem_euclid(5) == 0 {
            &mut major
        } else {
            &mut minor
        };
        target.move_to(point(px(left), px(line_y)));
        target.line_to(point(px(right), px(line_y)));
    }

    paint_path(minor, palette.grid_minor, window);
    paint_path(major, palette.grid_major, window);
    paint_axes(camera, bounds, palette, window);
}

/// Index of the first grid line at or below a world coordinate.
fn grid_index(world: f32, step: f32) -> i64 {
    (world / step)
        .floor()
        .clamp(-1_000_000_000.0, 1_000_000_000.0) as i64
}

/// Paints the world X=0 and Y=0 axes when they are on screen.
fn paint_axes(camera: &Camera2D, bounds: Bounds<Pixels>, palette: Palette, window: &mut Window) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let origin = camera.world_to_screen(Point2::ZERO);
    let mut axes = PathBuilder::stroke(px(1.0));

    if (0.0..=width).contains(&origin.x) {
        axes.move_to(point(px(left + origin.x), px(top)));
        axes.line_to(point(px(left + origin.x), px(top + height)));
    }
    if (0.0..=height).contains(&origin.y) {
        axes.move_to(point(px(left), px(top + origin.y)));
        axes.line_to(point(px(left + width), px(top + origin.y)));
    }
    paint_path(axes, palette.border_strong, window);
}

/// Paints the program's XY work envelope as a dashed outline.
fn paint_envelope(
    camera: &Camera2D,
    world: Bounds2,
    bounds: Bounds<Pixels>,
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let min = camera.world_to_screen(world.min());
    let max = camera.world_to_screen(world.max());
    let x0 = left + min.x;
    let y0 = top + max.y;
    let x1 = left + max.x;
    let y1 = top + min.y;

    let mut outline = PathBuilder::stroke(px(1.0)).dash_array(&[px(7.0), px(5.0)]);
    outline.move_to(point(px(x0), px(y0)));
    outline.line_to(point(px(x1), px(y0)));
    outline.line_to(point(px(x1), px(y1)));
    outline.line_to(point(px(x0), px(y1)));
    outline.close();
    paint_path(outline, palette.border_strong, window);
}

/// Projects one world point to window coordinates.
fn project_window(camera: &Camera3D, bounds: Bounds<Pixels>, world: Point3) -> Option<(f32, f32)> {
    let projected = camera.project(world)?;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    Some((left + projected.screen.x, top + projected.screen.y))
}

/// Appends one projected world-space line to `builder`.
fn push_world_line(
    builder: &mut PathBuilder,
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    start: Point3,
    end: Point3,
) -> bool {
    let (Some((x0, y0)), Some((x1, y1))) = (
        project_window(camera, bounds, start),
        project_window(camera, bounds, end),
    ) else {
        return false;
    };
    builder.move_to(point(px(x0), px(y0)));
    builder.line_to(point(px(x1), px(y1)));
    true
}

/// Paints a subdued ground grid on the work Z=0 plane.
fn paint_ground_grid_3d(
    camera: &Camera3D,
    path: Bounds3,
    bounds: Bounds<Pixels>,
    palette: Palette,
    window: &mut Window,
) {
    let footprint = if path.is_empty() {
        Bounds2::new(Point2::new(-50.0, -50.0), Point2::new(50.0, 50.0))
    } else {
        path.xy_bounds()
    };
    if footprint.is_empty() {
        return;
    }

    let units_per_pixel = camera.units_per_pixel();
    let px_per_unit = if units_per_pixel.is_finite() && units_per_pixel > 0.0 {
        1.0 / units_per_pixel
    } else {
        1.0
    };
    let mut step = grid_step_mm(px_per_unit);
    let margin = (footprint.width().max(footprint.height()) * GRID_MARGIN_FRACTION).max(step * 2.0);
    let mut min_x = footprint.min().x - margin;
    let mut max_x = footprint.max().x + margin;
    let mut min_y = footprint.min().y - margin;
    let mut max_y = footprint.max().y + margin;

    for _ in 0..16 {
        min_x = (min_x / step).floor() * step;
        max_x = (max_x / step).ceil() * step;
        min_y = (min_y / step).floor() * step;
        max_y = (max_y / step).ceil() * step;
        let count_x = ((max_x - min_x) / step).round() as i64 + 1;
        let count_y = ((max_y - min_y) / step).round() as i64 + 1;
        if count_x <= MAX_GRID_LINES && count_y <= MAX_GRID_LINES {
            break;
        }
        step *= 2.0;
        if !step.is_finite() || step <= 0.0 {
            return;
        }
    }

    let mut minor = PathBuilder::stroke(px(1.0));
    let mut major = PathBuilder::stroke(px(1.0));
    let count_x = (((max_x - min_x) / step).round() as i64).clamp(0, MAX_GRID_LINES);
    let count_y = (((max_y - min_y) / step).round() as i64).clamp(0, MAX_GRID_LINES);
    for index in 0..=count_x {
        let x = min_x + index as f32 * step;
        let target = if ((x / step).round() as i64).rem_euclid(5) == 0 {
            &mut major
        } else {
            &mut minor
        };
        push_world_line(
            target,
            camera,
            bounds,
            Point3::new(x, min_y, 0.0),
            Point3::new(x, max_y, 0.0),
        );
    }
    for index in 0..=count_y {
        let y = min_y + index as f32 * step;
        let target = if ((y / step).round() as i64).rem_euclid(5) == 0 {
            &mut major
        } else {
            &mut minor
        };
        push_world_line(
            target,
            camera,
            bounds,
            Point3::new(min_x, y, 0.0),
            Point3::new(max_x, y, 0.0),
        );
    }
    paint_path(minor, palette.grid_minor.alpha(0.8), window);
    paint_path(major, palette.grid_major.alpha(0.9), window);

    let mut axes = PathBuilder::stroke(px(1.0));
    if min_y <= 0.0 && max_y >= 0.0 {
        push_world_line(
            &mut axes,
            camera,
            bounds,
            Point3::new(min_x, 0.0, 0.0),
            Point3::new(max_x, 0.0, 0.0),
        );
    }
    if min_x <= 0.0 && max_x >= 0.0 {
        push_world_line(
            &mut axes,
            camera,
            bounds,
            Point3::new(0.0, min_y, 0.0),
            Point3::new(0.0, max_y, 0.0),
        );
    }
    paint_path(axes, palette.border_strong.alpha(0.9), window);
}

/// Paints the stock box as translucent faces.
fn paint_stock_3d(
    camera: &Camera3D,
    stock: &StockBox,
    bounds: Bounds<Pixels>,
    palette: Palette,
    window: &mut Window,
) {
    let corners = stock.corners();
    let mut screen = [Point2::ZERO; 8];
    for (slot, corner) in screen.iter_mut().zip(corners) {
        let Some((x, y)) = project_window(camera, bounds, corner) else {
            return;
        };
        *slot = Point2::new(x, y);
    }

    for face in visible_stock_faces(stock, camera) {
        let mut fill_path = PathBuilder::fill();
        fill_path.move_to(point(px(screen[face[0]].x), px(screen[face[0]].y)));
        for &index in &face[1..] {
            fill_path.line_to(point(px(screen[index].x), px(screen[index].y)));
        }
        fill_path.close();
        paint_path(fill_path, palette.text_faint.alpha(0.10), window);

        let mut outline = PathBuilder::stroke(px(1.0));
        outline.move_to(point(px(screen[face[0]].x), px(screen[face[0]].y)));
        for &index in &face[1..] {
            outline.line_to(point(px(screen[index].x), px(screen[index].y)));
        }
        outline.close();
        paint_path(outline, palette.border_strong.alpha(0.42), window);
    }
}

/// Face indices whose outward normal points toward the camera eye, sorted
/// far-to-near by camera-space depth.
fn visible_stock_faces(stock: &StockBox, camera: &Camera3D) -> Vec<[usize; 4]> {
    let eye = camera.eye();
    let forward = camera.forward();
    let corners = stock.corners();
    let mut visible: Vec<([usize; 4], f32)> = Vec::with_capacity(3);
    for face in stock.faces() {
        let points = face.map(|index| corners[index]);
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        let center = Point3::new(
            (points[0].x + points[1].x + points[2].x + points[3].x) * 0.25,
            (points[0].y + points[1].y + points[2].y + points[3].y) * 0.25,
            (points[0].z + points[1].z + points[2].z + points[3].z) * 0.25,
        );
        if normal.dot(eye - center) <= 0.0 {
            continue;
        }
        visible.push((face, (center - eye).dot(forward)));
    }
    visible.sort_by(|left, right| right.1.total_cmp(&left.1));
    visible.into_iter().map(|(face, _)| face).collect()
}

/// Paints the toolpath bounds as a dashed 3D box.
fn paint_envelope_3d(
    camera: &Camera3D,
    world: Bounds3,
    bounds: Bounds<Pixels>,
    palette: Palette,
    window: &mut Window,
) {
    if world.is_empty() {
        return;
    }
    let min = world.min();
    let max = world.max();
    let mut outline = PathBuilder::stroke(px(1.0)).dash_array(&[px(7.0), px(5.0)]);

    for z in [min.z, max.z] {
        push_world_line(
            &mut outline,
            camera,
            bounds,
            Point3::new(min.x, min.y, z),
            Point3::new(max.x, min.y, z),
        );
        push_world_line(
            &mut outline,
            camera,
            bounds,
            Point3::new(max.x, min.y, z),
            Point3::new(max.x, max.y, z),
        );
        push_world_line(
            &mut outline,
            camera,
            bounds,
            Point3::new(max.x, max.y, z),
            Point3::new(min.x, max.y, z),
        );
        push_world_line(
            &mut outline,
            camera,
            bounds,
            Point3::new(min.x, max.y, z),
            Point3::new(min.x, min.y, z),
        );
    }
    for (x, y) in [
        (min.x, min.y),
        (max.x, min.y),
        (max.x, max.y),
        (min.x, max.y),
    ] {
        push_world_line(
            &mut outline,
            camera,
            bounds,
            Point3::new(x, y, min.z),
            Point3::new(x, y, max.z),
        );
    }
    paint_path(outline, palette.border_strong.alpha(0.8), window);
}

/// Representative semantic kind for one display segment group.
///
/// The parser still emits the five display kinds; this is the closest stable
/// mapping until the semantic taxonomy reaches the toolpath stream.
#[must_use]
pub fn segment_kind(kind: SegmentKind) -> MoveKind {
    match kind {
        SegmentKind::Rapid => MoveKind::Rapid,
        SegmentKind::Feed => MoveKind::Feed,
        SegmentKind::Plunge => MoveKind::Plunge,
        SegmentKind::Arc => MoveKind::ArcCw,
        SegmentKind::Probe => MoveKind::Probe,
    }
}

/// Style for one display segment group.
#[must_use]
pub fn segment_style(kind: SegmentKind) -> MoveStyle {
    segment_kind(kind).style()
}

/// Colour for one display segment group.
#[must_use]
pub fn segment_color(kind: SegmentKind, palette: Palette) -> Hsla {
    move_color(segment_kind(kind), palette)
}

fn path_builder_for(kind: SegmentKind) -> PathBuilder {
    let style = segment_style(kind);
    let builder = PathBuilder::stroke(px(style.width));
    match style.dash {
        Some([on, off]) => builder.dash_array(&[px(on), px(off)]),
        None => builder,
    }
}

/// Paints the 3D plan in the plan's far-to-near order.
fn paint_plan_3d(
    bounds: Bounds<Pixels>,
    plan: &RenderPlan,
    visibility: [bool; 5],
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut batch: Option<(SegmentKind, PathBuilder)> = None;

    for polyline in plan.polylines() {
        if !visibility[polyline.kind as usize] {
            continue;
        }
        if polyline.points.is_empty() {
            continue;
        }
        if polyline.points.len() == 1 {
            if let Some((kind, builder)) = batch.take() {
                paint_path(builder, segment_color(kind, palette), window);
            }
            let single = polyline.points[0];
            paint_dot(
                window,
                segment_color(polyline.kind, palette),
                left + single.x,
                top + single.y,
                1.8,
            );
            continue;
        }
        let continues = batch
            .as_ref()
            .is_some_and(|(kind, _)| *kind == polyline.kind);
        if !continues {
            if let Some((kind, builder)) = batch.take() {
                paint_path(builder, segment_color(kind, palette), window);
            }
            batch = Some((polyline.kind, path_builder_for(polyline.kind)));
        }
        if let Some((_, builder)) = batch.as_mut() {
            for (index, screen) in polyline.points.iter().enumerate() {
                let world = point(px(left + screen.x), px(top + screen.y));
                if index == 0 {
                    builder.move_to(world);
                } else {
                    builder.line_to(world);
                }
            }
        }
    }
    if let Some((kind, builder)) = batch.take() {
        paint_path(builder, segment_color(kind, palette), window);
    }
}

/// Screen position of the 3D tool marker, or `None` when it is behind the near
/// plane or outside the canvas plus a small margin.
fn marker_screen_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    world: Point3,
) -> Option<(f32, f32)> {
    if !world.is_finite() {
        return None;
    }
    let projected = camera.project(world)?;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    let x = left + projected.screen.x;
    let y = top + projected.screen.y;
    if x < left - 12.0 || x > right + 12.0 || y < top - 12.0 || y > bottom + 12.0 {
        return None;
    }
    Some((x, y))
}

/// Paints the program-start tool marker in 3D.
fn paint_marker_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    world: Point3,
    palette: Palette,
    window: &mut Window,
) {
    let Some((x, y)) = marker_screen_3d(camera, bounds, world) else {
        return;
    };

    if world.z.abs() > 1.0e-3
        && let Some((plane_x, plane_y)) =
            project_window(camera, bounds, Point3::new(world.x, world.y, 0.0))
    {
        let mut dropline = PathBuilder::stroke(px(1.0)).dash_array(&[px(3.0), px(3.0)]);
        dropline.move_to(point(px(x), px(y)));
        dropline.line_to(point(px(plane_x), px(plane_y)));
        paint_path(dropline, palette.accent.alpha(0.35), window);
        paint_dot(window, palette.accent.alpha(0.22), plane_x, plane_y, 4.0);
    }

    paint_dot(window, palette.accent.alpha(0.18), x, y, 12.0);
    paint_dot(window, palette.accent_hover, x, y, 6.5);
    paint_dot(window, palette.bg, x, y, 4.8);
    paint_dot(window, palette.accent_hover, x, y, 2.8);

    let mut cross = PathBuilder::stroke(px(1.0));
    cross.move_to(point(px(x - 13.0), px(y)));
    cross.line_to(point(px(x + 13.0), px(y)));
    cross.move_to(point(px(x), px(y - 13.0)));
    cross.line_to(point(px(x), px(y + 13.0)));
    paint_path(cross, palette.text, window);
}

/// Paints the render plan, one GPUI path per motion style.
fn paint_plan(
    bounds: Bounds<Pixels>,
    plan: &RenderPlan,
    visibility: [bool; 5],
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);

    let mut rapid = path_builder_for(SegmentKind::Rapid);
    let mut feed = path_builder_for(SegmentKind::Feed);
    let mut plunge = path_builder_for(SegmentKind::Plunge);
    let mut arc = path_builder_for(SegmentKind::Arc);
    let mut probe = path_builder_for(SegmentKind::Probe);
    let mut dots: Vec<(f32, f32, Hsla)> = Vec::new();

    for polyline in plan.polylines() {
        if !visibility[polyline.kind as usize] {
            continue;
        }
        let target = match polyline.kind {
            SegmentKind::Rapid => &mut rapid,
            SegmentKind::Feed => &mut feed,
            SegmentKind::Plunge => &mut plunge,
            SegmentKind::Arc => &mut arc,
            SegmentKind::Probe => &mut probe,
        };
        if polyline.points.len() == 1 {
            let single = polyline.points[0];
            dots.push((
                left + single.x,
                top + single.y,
                segment_color(polyline.kind, palette),
            ));
            continue;
        }
        for (index, screen) in polyline.points.iter().enumerate() {
            let world = point(px(left + screen.x), px(top + screen.y));
            if index == 0 {
                target.move_to(world);
            } else {
                target.line_to(world);
            }
        }
    }

    for (kind, builder) in [
        (SegmentKind::Rapid, rapid),
        (SegmentKind::Feed, feed),
        (SegmentKind::Plunge, plunge),
        (SegmentKind::Arc, arc),
        (SegmentKind::Probe, probe),
    ] {
        if visibility[kind as usize] {
            paint_path(builder, segment_color(kind, palette), window);
        }
    }
    for (x, y, color) in dots {
        paint_dot(window, color, x, y, 1.8);
    }
}

/// Screen position of the 2D tool marker, or `None` when it is outside the
/// canvas plus a small margin for the crosshair.
fn marker_screen(camera: &Camera2D, bounds: Bounds<Pixels>, world: Point2) -> Option<(f32, f32)> {
    if !world.is_finite() {
        return None;
    }
    let screen = camera.world_to_screen(world);
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    let x = left + screen.x;
    let y = top + screen.y;
    if x < left - 12.0 || x > right + 12.0 || y < top - 12.0 || y > bottom + 12.0 {
        return None;
    }
    Some((x, y))
}

/// Paints the program-start tool marker: halo, ring, centre dot, crosshair.
fn paint_marker(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    world: Point2,
    palette: Palette,
    window: &mut Window,
) {
    let Some((x, y)) = marker_screen(camera, bounds, world) else {
        return;
    };

    paint_dot(window, palette.accent.alpha(0.18), x, y, 12.0);
    paint_dot(window, palette.accent_hover, x, y, 6.5);
    paint_dot(window, palette.bg, x, y, 4.8);
    paint_dot(window, palette.accent_hover, x, y, 2.8);

    let mut cross = PathBuilder::stroke(px(1.0));
    cross.move_to(point(px(x - 13.0), px(y)));
    cross.line_to(point(px(x + 13.0), px(y)));
    cross.move_to(point(px(x), px(y - 13.0)));
    cross.line_to(point(px(x), px(y + 13.0)));
    paint_path(cross, palette.text, window);
}

/// Paints one filled circle.
fn paint_dot(window: &mut Window, color: Hsla, x: f32, y: f32, radius: f32) {
    let bounds = Bounds::new(
        point(px(x - radius), px(y - radius)),
        size(px(radius * 2.0), px(radius * 2.0)),
    );
    window.paint_quad(fill(bounds, color).corner_radii(px(radius)));
}

fn paint_path(builder: PathBuilder, color: Hsla, window: &mut Window) {
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Chooses a readable world-space grid step for a pixels-per-millimetre scale.
///
/// The result is a 1/2/5 decade step that keeps minor lines between roughly 24
/// and 160 logical pixels apart.
#[must_use]
pub fn grid_step_mm(scale_px_per_mm: f32) -> f32 {
    const TARGET_PX: f32 = 40.0;
    const MIN_STEP_MM: f32 = 0.001;
    if !scale_px_per_mm.is_finite() || scale_px_per_mm <= 0.0 {
        return 1.0;
    }
    let raw = TARGET_PX / scale_px_per_mm;
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let decade = 10.0_f32.powf(raw.log10().floor());
    let normalized = raw / decade;
    let multiple = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    (decade * multiple).max(MIN_STEP_MM)
}

/// The tool marker position at the start of the program.
#[must_use]
pub fn program_start_marker(program: Option<&Program>) -> Option<Point3> {
    let segment = program?.toolpath.segments().first()?;
    (segment.start.is_finite()).then_some(segment.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::DEMO_SOURCE;

    fn demo() -> Arc<Program> {
        Arc::new(nc_gcode::parse_program(DEMO_SOURCE).expect("demo parses"))
    }

    fn test_bounds(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.0), px(0.0)), size(px(width), px(height)))
    }

    #[test]
    fn fit_pads_and_centres_the_toolpath() {
        let handle = ViewerHandle::new();
        let program = Arc::new(
            nc_gcode::parse_program("G21 G90\nG0 X0 Y0\nG1 X40 F800\nG1 Y20\n")
                .expect("test program"),
        );
        let bounds = test_bounds(800.0, 600.0);
        let camera = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program))
        };

        let world = program.toolpath.bounds().xy_bounds();
        assert_eq!(world.min(), Point2::new(0.0, 0.0));
        assert_eq!(world.max(), Point2::new(40.0, 20.0));
        let expected = (800.0 - 2.0 * VIEW_PADDING_PX) / 40.0;
        assert!((camera.scale() - expected).abs() < 1.0e-3);
        let min = camera.world_to_screen(world.min());
        let max = camera.world_to_screen(world.max());
        assert!((min.x - VIEW_PADDING_PX).abs() < 0.01);
        assert!((max.x - (800.0 - VIEW_PADDING_PX)).abs() < 0.01);
    }

    #[test]
    fn marker_only_repaints_do_not_rebuild_the_plan() {
        let handle = ViewerHandle::new();
        let program = demo();
        let bounds = test_bounds(1000.0, 700.0);
        {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program));
            assert_eq!(viewer.plan_builds, 1);
            // Every marker-only paint calls `prepare` with the same program
            // and camera; the plan key is unchanged, so no rebuild happens.
            for _ in 0..25 {
                let _ = viewer.prepare(bounds.size, Some(&program));
            }
            assert_eq!(viewer.plan_builds, 1);
        }

        let other = Arc::new(nc_gcode::parse_program("G0 X0\nG1 X5 F100\n").expect("program"));
        let mut viewer = handle.inner.borrow_mut();
        viewer.prepare(bounds.size, Some(&other));
        assert_eq!(viewer.plan_builds, 2);
    }

    #[test]
    fn a_missing_program_without_preview_stays_empty() {
        let handle = ViewerHandle::new();
        let bounds = test_bounds(800.0, 600.0);
        let mut viewer = handle.inner.borrow_mut();
        viewer.preview = false;
        let _ = viewer.prepare(bounds.size, None);
        assert!(viewer.cache.plan.is_empty());
        assert_eq!(viewer.plan_builds, 1);
    }

    #[test]
    fn demo_builds_2d_and_3d_plans() {
        let handle = ViewerHandle::new();
        let program = demo();
        assert!(!program.is_empty());
        let bounds = test_bounds(1280.0, 760.0);
        let mut viewer = handle.inner.borrow_mut();
        let _ = viewer.prepare(bounds.size, Some(&program));
        assert!(!viewer.cache.plan.is_empty());
        assert!(viewer.cache.plan.point_count() > 0);

        viewer.mode = ViewerMode::ThreeD;
        let _ = viewer.prepare(bounds.size, Some(&program));
        assert!(!viewer.cache3d.plan.is_empty());
        assert!(viewer.cache3d.plan.point_count() > 0);
    }

    #[test]
    fn switching_mode_fits_only_the_destination_camera() {
        let handle = ViewerHandle::new();
        let program = demo();
        let bounds = test_bounds(900.0, 600.0);
        let two_d = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program))
        };
        assert!(handle.set_mode(ViewerMode::ThreeD));
        let three_d = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program));
            viewer.camera3d
        };
        assert!(three_d.distance() > 0.0);
        let back = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program))
        };
        assert_eq!(two_d.scale(), back.scale());
        assert!(handle.set_mode(ViewerMode::Top2D));
    }

    #[test]
    fn grid_step_is_a_one_two_five_decade() {
        for scale in [0.1_f32, 1.0, 3.7, 40.0, 500.0] {
            let step = grid_step_mm(scale);
            let normalized = step / 10.0_f32.powf(step.log10().floor());
            assert!(
                [1.0, 2.0, 5.0]
                    .iter()
                    .any(|value| (normalized - value).abs() < 1.0e-4),
                "scale {scale} gave step {step} (normalized {normalized})"
            );
        }
        assert_eq!(grid_step_mm(0.0), 1.0);
        assert_eq!(grid_step_mm(f32::NAN), 1.0);
    }

    #[test]
    fn a_camera_outside_the_stock_sees_three_faces() {
        let stock = StockBox::from_xy(100.0, 60.0, 0.0, 20.0).expect("stock");
        let camera = Camera3D::new(Viewport::new(800.0, 600.0));
        let faces = visible_stock_faces(&stock, &camera);
        assert_eq!(faces.len(), 3);
        assert!(faces.iter().all(|face| face.iter().all(|index| *index < 8)));
    }

    #[test]
    fn program_start_marker_is_deterministic() {
        let program = demo();
        let marker = program_start_marker(Some(&program)).expect("marker");
        let again = program_start_marker(Some(&program)).expect("marker");
        assert_eq!(marker, again);
        assert!(marker.is_finite());
        assert_eq!(program_start_marker(None), None);
    }
}
