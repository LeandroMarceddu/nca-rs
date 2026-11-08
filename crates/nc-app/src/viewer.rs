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
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    Bounds, Entity, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    PathBuilder, Pixels, Point, ScrollDelta, Size, Styled as _, Window, canvas, div, fill, point,
    px, size,
};
use nc_gcode::Program;
use nc_viewer::{
    Bounds2, Bounds3, Camera2D, Camera3D, HoleMarker, MoveKind, Point2, Point3, RenderOptions,
    RenderPlan, StockBox, Toolpath, Viewport,
};

use crate::sim::{PathPhase, SimCanvas};
use crate::theme::{Palette, cut_depth_color, move_color, stock_depth_color};

/// Screen-space padding around the fitted toolpath, in logical pixels.
const VIEW_PADDING_PX: f32 = 48.0;

/// Screen-space padding around a focused diagnostic line, in logical pixels.
const FOCUS_PADDING_PX: f32 = 72.0;

/// Upper bound on warning-overlay segments painted in one frame.
///
/// A single source line can expand into thousands of arc chords. The first
/// segments in report line order are drawn; the count is deterministic.
const MAX_OVERLAY_SEGMENTS: usize = 20_000;

/// Analysis overlays passed to the canvas for this frame.
#[derive(Clone, Debug, Default)]
pub struct ViewerOverlay {
    /// Source lines with an error finding, sorted.
    pub error_lines: Arc<[usize]>,
    /// Source lines with a warning finding and no error, sorted.
    pub warning_lines: Arc<[usize]>,
    /// Source line to emphasize after overlays (diagnostic selection or the
    /// SIMULATE current line).
    pub highlight_line: Option<usize>,
}

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
            // Vertical orbit is intentionally inverted: dragging down raises
            // the eye, dragging up lowers it. Horizontal yaw follows the drag.
            DragKind::Orbit3D => viewer
                .camera3d
                .orbit(-delta.x * ORBIT_RAD_PER_PX, delta.y * ORBIT_RAD_PER_PX),
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

    /// Frames a world-space XY region in the top view on the next paint.
    pub fn focus_2d(&self, bounds: Bounds2) -> bool {
        if bounds.is_empty() {
            return false;
        }
        let mut viewer = self.inner.borrow_mut();
        viewer.pending_focus = Some(bounds);
        true
    }

    /// Frames a world-space region in the orbit view on the next paint.
    pub fn focus_3d(&self, bounds: Bounds3) -> bool {
        if bounds.is_empty() {
            return false;
        }
        let mut viewer = self.inner.borrow_mut();
        viewer.pending_focus3d = Some(bounds);
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

    /// Projects a world point to canvas-local screen coordinates.
    ///
    /// This uses the camera from the last paint. The shell uses it to place the
    /// unknown-diameter label beside the tool without painting text in the canvas.
    #[must_use]
    pub fn project_pose(&self, pose_mm: [f64; 3], mode: ViewerMode) -> Option<(f32, f32)> {
        if !pose_mm.iter().all(|value| value.is_finite()) {
            return None;
        }
        let viewer = self.inner.borrow();
        let rect = viewer.rect?;
        let left = f32::from(rect.origin.x);
        let top = f32::from(rect.origin.y);
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        let (x, y) = if mode.is_three_d() && viewer.mode.is_three_d() {
            let world = Point3::new(pose_mm[0] as f32, pose_mm[1] as f32, pose_mm[2] as f32);
            let projected = viewer.camera3d.project(world)?;
            (left + projected.screen.x, top + projected.screen.y)
        } else if !mode.is_three_d() && !viewer.mode.is_three_d() {
            let world = Point2::new(pose_mm[0] as f32, pose_mm[1] as f32);
            let screen = viewer.camera.world_to_screen(world);
            (left + screen.x, top + screen.y)
        } else {
            return None;
        };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let local_x = x - left;
        let local_y = y - top;
        if local_x < -8.0 || local_y < -8.0 || local_x > width + 8.0 || local_y > height + 8.0 {
            return None;
        }
        Some((local_x, local_y))
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
/// `visibility` array gates exactly one semantic [`MoveKind`] per slot.
/// `overlay` carries analysis warning/error lines and the current highlight.
#[allow(clippy::too_many_arguments)]
pub fn view<T: HasViewer + 'static>(
    handle: &ViewerHandle,
    palette: Palette,
    program: Option<Arc<Program>>,
    stock: Option<StockBox>,
    marker: Option<Point3>,
    visibility: [bool; MoveKind::ALL.len()],
    preview: bool,
    sim: Option<SimCanvas>,
    overlay: Option<ViewerOverlay>,
    view: Entity<T>,
) -> impl IntoElement {
    let canvas_element = {
        let measure = handle.clone();
        let paint_handle = handle.clone();
        let paint_sim = sim.clone();
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
                    paint_sim.clone(),
                    overlay.clone(),
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

/// Cached SIMULATE path plan and the camera it was built for.
#[derive(Default)]
struct SimPlanCache {
    key: Option<SimPlanKey2D>,
    plan: SimPathPlan,
}

/// Identity of one SIMULATE 2D plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SimPlanKey2D {
    program: usize,
    segments: usize,
    scale: u32,
    offset_x: u32,
    offset_y: u32,
    width: u32,
    height: u32,
    air_top_z: u32,
}

impl SimPlanKey2D {
    fn new(program: usize, segments: usize, camera: &Camera2D, air_top_z: f32) -> Self {
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
            air_top_z: air_top_z.to_bits(),
        }
    }
}

/// Cached SIMULATE 3D plan and the camera it was built for.
#[derive(Default)]
struct SimPlanCache3D {
    key: Option<SimPlanKey3D>,
    plan: SimPathPlan,
}

/// Identity of one SIMULATE 3D plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SimPlanKey3D {
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
    air_top_z: u32,
}

impl SimPlanKey3D {
    fn new(program: usize, segments: usize, camera: &Camera3D, air_top_z: f32) -> Self {
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
            air_top_z: air_top_z.to_bits(),
        }
    }
}

/// One projected run of toolpath segments in the SIMULATE plan.
///
/// Unlike the generic [`RenderPlan`], this keeps the source segment index of
/// every screen segment, so the current-line split happens at paint time and a
/// pose-only or line-only update never rebuilds the plan.
#[derive(Clone, Debug)]
struct SimPolyline {
    points: Vec<Point2>,
    segments: Vec<usize>,
    air: Vec<bool>,
    depth: f32,
    kind: MoveKind,
}

/// A projected toolpath for SIMULATE.
#[derive(Clone, Debug, Default)]
struct SimPathPlan {
    polylines: Vec<SimPolyline>,
}

impl SimPathPlan {
    fn from_2d(toolpath: &Toolpath, camera: &Camera2D, air_top_z: f32) -> Self {
        let viewport = camera.viewport().bounds();
        let center = camera.viewport().center();
        let offset = camera.offset();
        let scale = camera.scale();
        let to_screen = |world: Point2| {
            Point2::new(
                center.x + (world.x - offset.x) * scale,
                center.y - (world.y - offset.y) * scale,
            )
        };
        let mut plan = Self::default();
        let mut run: Option<SimRun> = None;
        for (index, segment) in toolpath.segments().iter().enumerate() {
            let start = to_screen(segment.start.xy());
            let end = to_screen(segment.end.xy());
            if !start.is_finite() || !end.is_finite() {
                finish_sim_run(&mut plan, run.take(), Some(viewport));
                continue;
            }
            let continues = run
                .as_ref()
                .is_some_and(|builder| builder.can_continue(segment, start));
            if !continues {
                finish_sim_run(&mut plan, run.take(), Some(viewport));
                run = Some(SimRun::new(segment, start));
            }
            if let Some(builder) = run.as_mut() {
                builder.push(index, segment, end, air_top_z);
            }
        }
        finish_sim_run(&mut plan, run, Some(viewport));
        plan
    }

    fn from_3d(toolpath: &Toolpath, camera: &Camera3D, air_top_z: f32) -> Self {
        let viewport = camera.viewport().bounds();
        let mut plan = Self::default();
        let mut run: Option<SimRun> = None;
        for (index, segment) in toolpath.segments().iter().enumerate() {
            let (Some(start), Some(end)) =
                (camera.project(segment.start), camera.project(segment.end))
            else {
                finish_sim_run(&mut plan, run.take(), Some(viewport));
                continue;
            };
            let depth = (start.depth + end.depth) * 0.5;
            if !depth.is_finite() {
                finish_sim_run(&mut plan, run.take(), Some(viewport));
                continue;
            }
            let continues = run
                .as_ref()
                .is_some_and(|builder| builder.can_continue(segment, start.screen));
            if !continues {
                finish_sim_run(&mut plan, run.take(), Some(viewport));
                run = Some(SimRun::new(segment, start.screen));
            }
            if let Some(builder) = run.as_mut() {
                builder.depth += depth;
                builder.push(index, segment, end.screen, air_top_z);
            }
        }
        finish_sim_run(&mut plan, run, Some(viewport));
        plan.polylines.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        plan
    }
}

/// Builder for one connected SIMULATE run.
struct SimRun {
    points: Vec<Point2>,
    segments: Vec<usize>,
    air: Vec<bool>,
    depth: f32,
    kind: MoveKind,
    layer: u16,
    tool: u16,
}

impl SimRun {
    fn new(segment: &nc_viewer::Segment, start: Point2) -> Self {
        Self {
            points: vec![start],
            segments: Vec::new(),
            air: Vec::new(),
            depth: 0.0,
            kind: segment.kind,
            layer: segment.layer,
            tool: segment.tool,
        }
    }

    fn can_continue(&self, segment: &nc_viewer::Segment, start: Point2) -> bool {
        if self.kind != segment.kind || self.layer != segment.layer || self.tool != segment.tool {
            return false;
        }
        self.points.last().is_some_and(|last| {
            let dx = last.x - start.x;
            let dy = last.y - start.y;
            dx * dx + dy * dy <= 1.0e-4
        })
    }

    fn push(&mut self, index: usize, segment: &nc_viewer::Segment, end: Point2, air_top_z: f32) {
        self.points.push(end);
        self.segments.push(index);
        let mid_z = (f64::from(segment.start.z) + f64::from(segment.end.z)) * 0.5;
        self.air
            .push(segment.kind.is_cutting() && mid_z > f64::from(air_top_z) + 1.0e-6);
    }
}

fn finish_sim_run(plan: &mut SimPathPlan, run: Option<SimRun>, viewport: Option<Bounds2>) {
    let Some(run) = run else {
        return;
    };
    if run.segments.is_empty() || run.points.len() < 2 {
        return;
    }
    if let Some(viewport) = viewport {
        let mut min = Point2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Point2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for point in &run.points {
            min = Point2::new(min.x.min(point.x), min.y.min(point.y));
            max = Point2::new(max.x.max(point.x), max.y.max(point.y));
        }
        let cull = Bounds2::new(min, max);
        if !cull.intersects(&viewport.expanded(8.0)) {
            return;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let depth = if run.segments.is_empty() {
        0.0
    } else {
        run.depth / run.segments.len() as f32
    };
    plan.polylines.push(SimPolyline {
        points: run.points,
        segments: run.segments,
        air: run.air,
        depth,
        kind: run.kind,
    });
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
    pending_focus: Option<Bounds2>,
    pending_focus3d: Option<Bounds3>,
    rect: Option<Bounds<Pixels>>,
    drag: Option<DragState>,
    synthetic: Option<Arc<Toolpath>>,
    empty: Option<Arc<Toolpath>>,
    /// Whether an absent program should draw the built-in synthetic preview.
    preview: bool,
    cache: PlanCache,
    cache3d: PlanCache3D,
    sim_cache: SimPlanCache,
    sim_cache3d: SimPlanCache3D,
    #[cfg(test)]
    plan_builds: u32,
    #[cfg(test)]
    plan_builds_3d: u32,
    #[cfg(test)]
    sim_plan_builds: u32,
    #[cfg(test)]
    sim_plan_builds_3d: u32,
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
            pending_focus: None,
            pending_focus3d: None,
            rect: None,
            drag: None,
            synthetic: None,
            empty: None,
            preview: true,
            cache: PlanCache::default(),
            cache3d: PlanCache3D::default(),
            sim_cache: SimPlanCache::default(),
            sim_cache3d: SimPlanCache3D::default(),
            #[cfg(test)]
            plan_builds: 0,
            #[cfg(test)]
            plan_builds_3d: 0,
            #[cfg(test)]
            sim_plan_builds: 0,
            #[cfg(test)]
            sim_plan_builds_3d: 0,
        }
    }
}

impl Viewer {
    /// Updates the cameras for the current viewport and program, then ensures
    /// the active render plan matches.
    ///
    /// A marker or current-line update passes the same camera and program, so
    /// it never rebuilds a plan. SIMULATE has its own plan type because it must
    /// keep the source segment index of every screen segment.
    fn prepare(
        &mut self,
        size: Size<Pixels>,
        program: Option<&Arc<Program>>,
        sim: Option<&SimCanvas>,
    ) -> Camera2D {
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
            self.pending_focus = None;
            self.pending_focus3d = None;
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
        // An explicit diagnostic focus wins over the program-fit request.
        if let Some(bounds) = self.pending_focus.take() {
            self.camera.fit(bounds, FOCUS_PADDING_PX);
            self.pending_fit = false;
        }
        if let Some(bounds) = self.pending_focus3d.take() {
            self.camera3d.fit_bounds(bounds, FOCUS_PADDING_PX);
            self.pending_fit_3d = false;
        }

        if self.mode.is_three_d() {
            if let Some(sim) = sim {
                let key3d = SimPlanKey3D::new(
                    identity,
                    path.segment_count(),
                    &self.camera3d,
                    sim.stock.max.z,
                );
                if self.sim_cache3d.key != Some(key3d) {
                    self.sim_cache3d.plan =
                        SimPathPlan::from_3d(path, &self.camera3d, sim.stock.max.z);
                    self.sim_cache3d.key = Some(key3d);
                    #[cfg(test)]
                    {
                        self.sim_plan_builds_3d += 1;
                    }
                }
            } else {
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
            }
        } else if let Some(sim) = sim {
            let key = SimPlanKey2D::new(
                identity,
                path.segment_count(),
                &self.camera,
                sim.stock.max.z,
            );
            if self.sim_cache.key != Some(key) {
                self.sim_cache.plan = SimPathPlan::from_2d(path, &self.camera, sim.stock.max.z);
                self.sim_cache.key = Some(key);
                #[cfg(test)]
                {
                    self.sim_plan_builds += 1;
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
    visibility: [bool; MoveKind::ALL.len()],
    preview: bool,
    sim: Option<SimCanvas>,
    overlay: Option<ViewerOverlay>,
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
    let camera = viewer.prepare(bounds.size, program.as_ref(), sim.as_ref());
    let stock_on = viewer.stock_visible && sim.as_ref().is_none_or(|sim| sim.toggles.stock);

    if viewer.mode.is_three_d() {
        let camera3d = viewer.camera3d;
        paint_ground_grid_3d(&camera3d, viewer.path_bounds3, bounds, palette, window);
        if stock_on && let Some(stock) = stock {
            paint_stock_3d(&camera3d, &stock, bounds, palette, window);
        }
        if let Some(sim) = &sim
            && stock_on
        {
            paint_heightfield_3d(&camera3d, bounds, sim, palette, window);
        }
        if !viewer.path_bounds3.is_empty() {
            paint_envelope_3d(&camera3d, viewer.path_bounds3, bounds, palette, window);
        }
        if let Some(sim) = &sim {
            paint_sim_path(
                bounds,
                &viewer.sim_cache3d.plan,
                sim,
                visibility,
                palette,
                window,
            );
        } else {
            paint_plan_3d(bounds, &viewer.cache3d.plan, visibility, palette, window);
        }
        if let (Some(program), Some(overlay)) = (program.as_deref(), overlay.as_ref()) {
            paint_overlay_3d(&camera3d, bounds, program, overlay, palette, window);
            paint_highlight_3d(&camera3d, bounds, program, overlay, palette, window);
        }
        if let Some(sim) = &sim {
            if sim.toggles.tool {
                paint_sim_tool_3d(&camera3d, bounds, sim, palette, window);
            }
        } else if let Some(marker) = marker {
            paint_marker_3d(&camera3d, bounds, marker, palette, window);
        }
        return;
    }

    paint_grid(&camera, bounds, palette, window);
    if !viewer.path_bounds.is_empty() {
        paint_envelope(&camera, viewer.path_bounds, bounds, palette, window);
    }
    if let Some(sim) = &sim
        && stock_on
    {
        paint_sim_stock_2d(&camera, bounds, sim, palette, window);
    }
    if let Some(sim) = &sim {
        paint_sim_path(
            bounds,
            &viewer.sim_cache.plan,
            sim,
            visibility,
            palette,
            window,
        );
    } else {
        paint_plan(bounds, &viewer.cache.plan, visibility, palette, window);
    }
    if let (Some(program), Some(overlay)) = (program.as_deref(), overlay.as_ref()) {
        paint_overlay_2d(&camera, bounds, program, overlay, palette, window);
        paint_highlight_2d(&camera, bounds, program, overlay, palette, window);
    }
    if let Some(sim) = &sim {
        if sim.toggles.tool {
            paint_sim_tool_2d(&camera, bounds, sim, palette, window);
        }
    } else if let Some(marker) = marker {
        paint_marker(&camera, bounds, marker.xy(), palette, window);
    }
}

/// Paints analysis warning and error segments in the top view.
///
/// The overlay is a dashed halo, deliberately distinct from the semantic move
/// colours and from any solid red simulation marking. Errors paint before
/// warnings and the segment budget is shared, so the visible set is stable.
fn paint_overlay_2d(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    program: &Program,
    overlay: &ViewerOverlay,
    palette: Palette,
    window: &mut Window,
) {
    let mut remaining = MAX_OVERLAY_SEGMENTS;
    remaining = remaining.saturating_sub(paint_alert_lines_2d(
        camera,
        bounds,
        program,
        &overlay.error_lines,
        palette.danger,
        remaining,
        window,
    ));
    let _ = paint_alert_lines_2d(
        camera,
        bounds,
        program,
        &overlay.warning_lines,
        palette.warning,
        remaining,
        window,
    );
}

/// Paints analysis warning and error segments in the orbit view.
fn paint_overlay_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    program: &Program,
    overlay: &ViewerOverlay,
    palette: Palette,
    window: &mut Window,
) {
    let mut remaining = MAX_OVERLAY_SEGMENTS;
    remaining = remaining.saturating_sub(paint_alert_lines_3d(
        camera,
        bounds,
        program,
        &overlay.error_lines,
        palette.danger,
        remaining,
        window,
    ));
    let _ = paint_alert_lines_3d(
        camera,
        bounds,
        program,
        &overlay.warning_lines,
        palette.warning,
        remaining,
        window,
    );
}

/// Paints dashed line halos for one severity in the top view.
fn paint_alert_lines_2d(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    program: &Program,
    lines: &[usize],
    color: Hsla,
    limit: usize,
    window: &mut Window,
) -> usize {
    if lines.is_empty() || limit == 0 {
        return 0;
    }
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut halo = PathBuilder::stroke(px(12.0));
    let mut dash = PathBuilder::stroke(px(2.0)).dash_array(&[px(6.0), px(4.0)]);
    let mut drawn = 0usize;
    for line in lines {
        for range in crate::analysis_ui::segment_ranges(program, *line, *line) {
            for segment in program
                .toolpath
                .segments()
                .get(range.start.min(program.toolpath.len())..range.end.min(program.toolpath.len()))
                .unwrap_or_default()
            {
                if drawn >= limit {
                    paint_path(halo, color.alpha(0.16), window);
                    paint_path(dash, color.alpha(0.92), window);
                    return drawn;
                }
                let start = camera.world_to_screen(segment.start.xy());
                let end = camera.world_to_screen(segment.end.xy());
                halo.move_to(point(px(left + start.x), px(top + start.y)));
                halo.line_to(point(px(left + end.x), px(top + end.y)));
                dash.move_to(point(px(left + start.x), px(top + start.y)));
                dash.line_to(point(px(left + end.x), px(top + end.y)));
                drawn += 1;
            }
        }
    }
    paint_path(halo, color.alpha(0.16), window);
    paint_path(dash, color.alpha(0.92), window);
    drawn
}

/// Paints dashed line halos for one severity in the orbit view.
fn paint_alert_lines_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    program: &Program,
    lines: &[usize],
    color: Hsla,
    limit: usize,
    window: &mut Window,
) -> usize {
    if lines.is_empty() || limit == 0 {
        return 0;
    }
    let mut halo = PathBuilder::stroke(px(12.0));
    let mut dash = PathBuilder::stroke(px(2.0)).dash_array(&[px(6.0), px(4.0)]);
    let mut drawn = 0usize;
    for line in lines {
        for range in crate::analysis_ui::segment_ranges(program, *line, *line) {
            let start = range.start.min(program.toolpath.len());
            let end = range.end.min(program.toolpath.len());
            for segment in &program.toolpath.segments()[start..end] {
                let (Some((x0, y0)), Some((x1, y1))) = (
                    project_window(camera, bounds, segment.start),
                    project_window(camera, bounds, segment.end),
                ) else {
                    continue;
                };
                if drawn >= limit {
                    paint_path(halo, color.alpha(0.16), window);
                    paint_path(dash, color.alpha(0.92), window);
                    return drawn;
                }
                halo.move_to(point(px(x0), px(y0)));
                halo.line_to(point(px(x1), px(y1)));
                dash.move_to(point(px(x0), px(y0)));
                dash.line_to(point(px(x1), px(y1)));
                drawn += 1;
            }
        }
    }
    paint_path(halo, color.alpha(0.16), window);
    paint_path(dash, color.alpha(0.92), window);
    drawn
}

/// Paints the selected diagnostic line or the SIMULATE current line last, so
/// the current-line highlight wins over the warning overlay.
fn paint_highlight_2d(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    program: &Program,
    overlay: &ViewerOverlay,
    palette: Palette,
    window: &mut Window,
) {
    let Some(line) = overlay.highlight_line else {
        return;
    };
    let ranges = crate::analysis_ui::segment_ranges_with_fallback(program, line, line);
    if ranges.is_empty() {
        return;
    }
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut halo = PathBuilder::stroke(px(7.0));
    let mut line_path = PathBuilder::stroke(px(2.0));
    let mut drawn = 0usize;
    for range in ranges {
        let start = range.start.min(program.toolpath.len());
        let end = range.end.min(program.toolpath.len());
        for segment in &program.toolpath.segments()[start..end] {
            if drawn >= MAX_OVERLAY_SEGMENTS {
                break;
            }
            let start = camera.world_to_screen(segment.start.xy());
            let end = camera.world_to_screen(segment.end.xy());
            halo.move_to(point(px(left + start.x), px(top + start.y)));
            halo.line_to(point(px(left + end.x), px(top + end.y)));
            line_path.move_to(point(px(left + start.x), px(top + start.y)));
            line_path.line_to(point(px(left + end.x), px(top + end.y)));
            drawn += 1;
        }
    }
    paint_path(halo, palette.accent.alpha(0.18), window);
    paint_path(line_path, palette.accent_hover, window);
}

/// Paints the selected diagnostic line or the SIMULATE current line in 3D.
fn paint_highlight_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    program: &Program,
    overlay: &ViewerOverlay,
    palette: Palette,
    window: &mut Window,
) {
    let Some(line) = overlay.highlight_line else {
        return;
    };
    let ranges = crate::analysis_ui::segment_ranges_with_fallback(program, line, line);
    if ranges.is_empty() {
        return;
    }
    let mut halo = PathBuilder::stroke(px(7.0));
    let mut line_path = PathBuilder::stroke(px(2.0));
    let mut drawn = 0usize;
    for range in ranges {
        let start = range.start.min(program.toolpath.len());
        let end = range.end.min(program.toolpath.len());
        for segment in &program.toolpath.segments()[start..end] {
            if drawn >= MAX_OVERLAY_SEGMENTS {
                break;
            }
            let (Some((x0, y0)), Some((x1, y1))) = (
                project_window(camera, bounds, segment.start),
                project_window(camera, bounds, segment.end),
            ) else {
                continue;
            };
            halo.move_to(point(px(x0), px(y0)));
            halo.line_to(point(px(x1), px(y1)));
            line_path.move_to(point(px(x0), px(y0)));
            line_path.line_to(point(px(x1), px(y1)));
            drawn += 1;
        }
    }
    paint_path(halo, palette.accent.alpha(0.18), window);
    paint_path(line_path, palette.accent_hover, window);
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

/// Projects one world point to window coordinates plus camera-space depth.
fn project_window_depth(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    world: Point3,
) -> Option<(f32, f32, f32)> {
    let projected = camera.project(world)?;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    Some((
        left + projected.screen.x,
        top + projected.screen.y,
        projected.depth,
    ))
}

/// Expands a projected quad around its centroid so a per-cell fill overlaps its
/// neighbours without the even-odd cancellation that one multi-subpath fill
/// would produce.
fn inflate_quad(corners: &mut [Point2; 4], amount_px: f32) {
    if !amount_px.is_finite() || amount_px <= 0.0 {
        return;
    }
    let cx = (corners[0].x + corners[1].x + corners[2].x + corners[3].x) * 0.25;
    let cy = (corners[0].y + corners[1].y + corners[2].y + corners[3].y) * 0.25;
    for corner in corners {
        let dx = corner.x - cx;
        let dy = corner.y - cy;
        let length = (dx * dx + dy * dy).sqrt();
        if length > 1.0e-4 {
            corner.x += dx / length * amount_px;
            corner.y += dy / length * amount_px;
        }
    }
}

/// Paints the sampled cut-stock heightfield in 3D.
///
/// Quads are sorted far-to-near and consecutive quads that fall in the same
/// depth-colour bucket share one GPUI path. At the playing resolution this is
/// 4096 quads; the paused resolution is 16384.
fn paint_heightfield_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    sim: &SimCanvas,
    palette: Palette,
    window: &mut Window,
) {
    const BUCKETS: usize = 8;
    let mesh = &sim.mesh;
    let resolution = mesh.resolution;
    if resolution == 0 || mesh.heights_mm.len() < (resolution + 1) * (resolution + 1) {
        return;
    }
    // The uncut top is one flat quad. Only cells below the top need their own
    // projected heightfield patch, which keeps the surface free of tessellation
    // seams and cuts the paint count for an untouched block to one path.
    let stock = sim.stock;
    let top_corners = [
        Point3::new(stock.min.x, stock.min.y, stock.max.z),
        Point3::new(stock.max.x, stock.min.y, stock.max.z),
        Point3::new(stock.max.x, stock.max.y, stock.max.z),
        Point3::new(stock.min.x, stock.max.y, stock.max.z),
    ];
    let mut top_screen = [Point2::ZERO; 4];
    let mut top_valid = true;
    for (slot, world) in top_screen.iter_mut().zip(top_corners) {
        let Some((x, y, _)) = project_window_depth(camera, bounds, world) else {
            top_valid = false;
            break;
        };
        *slot = Point2::new(x, y);
    }
    if top_valid {
        let mut top = PathBuilder::fill();
        top.move_to(point(px(top_screen[0].x), px(top_screen[0].y)));
        for corner in &top_screen[1..] {
            top.line_to(point(px(corner.x), px(corner.y)));
        }
        top.close();
        paint_path(top, stock_depth_color(0.0, palette), window);
    }
    let mut quads: Vec<(f32, [Point2; 4], usize)> = Vec::with_capacity(resolution * resolution);
    for row in 0..resolution {
        for col in 0..resolution {
            let Some(h00) = mesh.height_at(col, row) else {
                continue;
            };
            let Some(h10) = mesh.height_at(col + 1, row) else {
                continue;
            };
            let Some(h01) = mesh.height_at(col, row + 1) else {
                continue;
            };
            let Some(h11) = mesh.height_at(col + 1, row + 1) else {
                continue;
            };
            let average =
                (f64::from(h00) + f64::from(h10) + f64::from(h01) + f64::from(h11)) * 0.25;
            if mesh.top_z_mm - average <= 1.0e-4 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let fraction = mesh.depth_fraction(average as f32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let bucket = (fraction * (BUCKETS - 1) as f32).round() as usize;
            let bucket = bucket.min(BUCKETS - 1);

            let xy00 = mesh.vertex_xy_mm(col, row);
            let xy10 = mesh.vertex_xy_mm(col + 1, row);
            let xy11 = mesh.vertex_xy_mm(col + 1, row + 1);
            let xy01 = mesh.vertex_xy_mm(col, row + 1);
            let corners = [(xy00, h00), (xy10, h10), (xy11, h11), (xy01, h01)];
            let mut screen = [Point2::ZERO; 4];
            let mut depth = 0.0_f32;
            let mut valid = true;
            for (slot, (xy, height)) in screen.iter_mut().zip(corners) {
                let world = Point3::new(xy[0] as f32, xy[1] as f32, height);
                let Some((x, y, z)) = project_window_depth(camera, bounds, world) else {
                    valid = false;
                    break;
                };
                *slot = Point2::new(x, y);
                depth += z;
            }
            if !valid {
                continue;
            }
            quads.push((depth * 0.25, screen, bucket));
        }
    }
    quads.sort_by(|left, right| right.0.total_cmp(&left.0));

    // Each cut cell is its own fill path. Lower cells project slightly away
    // from their neighbours, so batching them into one path makes the even-odd
    // fill rule cancel the overlaps and produces a checkerboard.
    for (_, mut corners, bucket) in quads {
        inflate_quad(&mut corners, 0.45);
        let mut path = PathBuilder::fill();
        path.move_to(point(px(corners[0].x), px(corners[0].y)));
        for corner in &corners[1..] {
            path.line_to(point(px(corner.x), px(corner.y)));
        }
        path.close();
        #[allow(clippy::cast_precision_loss)]
        let fraction = bucket as f32 / (BUCKETS - 1) as f32;
        paint_path(path, stock_depth_color(fraction, palette), window);
    }
}

/// Paints the cut stock footprint and depth-shaded cells in 2D.
fn paint_sim_stock_2d(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    sim: &SimCanvas,
    palette: Palette,
    window: &mut Window,
) {
    const BUCKETS: usize = 8;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mesh = &sim.mesh;
    let stock = sim.stock;
    let min = camera.world_to_screen(Point2::new(stock.min.x, stock.min.y));
    let max = camera.world_to_screen(Point2::new(stock.max.x, stock.max.y));
    let x0 = left + min.x;
    let x1 = left + max.x;
    let y0 = top + max.y;
    let y1 = top + min.y;
    let mut panel = PathBuilder::fill();
    panel.move_to(point(px(x0), px(y0)));
    panel.line_to(point(px(x1), px(y0)));
    panel.line_to(point(px(x1), px(y1)));
    panel.line_to(point(px(x0), px(y1)));
    panel.close();
    paint_path(panel, stock_depth_color(0.0, palette).alpha(0.92), window);

    let resolution = mesh.resolution;
    if resolution == 0 {
        return;
    }
    let mut cells: BTreeMap<usize, PathBuilder> = BTreeMap::new();
    for row in 0..resolution {
        for col in 0..resolution {
            let Some(h00) = mesh.height_at(col, row) else {
                continue;
            };
            let Some(h10) = mesh.height_at(col + 1, row) else {
                continue;
            };
            let Some(h01) = mesh.height_at(col, row + 1) else {
                continue;
            };
            let Some(h11) = mesh.height_at(col + 1, row + 1) else {
                continue;
            };
            let average =
                (f64::from(h00) + f64::from(h10) + f64::from(h01) + f64::from(h11)) * 0.25;
            let depth = mesh.top_z_mm - average;
            if depth <= 1.0e-6 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let fraction = mesh.depth_fraction(average as f32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let bucket = (fraction * (BUCKETS - 1) as f32).round() as usize;
            let bucket = bucket.min(BUCKETS - 1);
            let xy00 = mesh.vertex_xy_mm(col, row);
            let xy10 = mesh.vertex_xy_mm(col + 1, row);
            let xy11 = mesh.vertex_xy_mm(col + 1, row + 1);
            let xy01 = mesh.vertex_xy_mm(col, row + 1);
            let p00 = camera.world_to_screen(Point2::new(xy00[0] as f32, xy00[1] as f32));
            let p10 = camera.world_to_screen(Point2::new(xy10[0] as f32, xy10[1] as f32));
            let p11 = camera.world_to_screen(Point2::new(xy11[0] as f32, xy11[1] as f32));
            let p01 = camera.world_to_screen(Point2::new(xy01[0] as f32, xy01[1] as f32));
            let cell = [
                Point2::new(left + p00.x, top + p00.y),
                Point2::new(left + p10.x, top + p10.y),
                Point2::new(left + p11.x, top + p11.y),
                Point2::new(left + p01.x, top + p01.y),
            ];
            let path = cells.entry(bucket).or_insert_with(PathBuilder::fill);
            path.move_to(point(px(cell[0].x), px(cell[0].y)));
            path.line_to(point(px(cell[1].x), px(cell[1].y)));
            path.line_to(point(px(cell[2].x), px(cell[2].y)));
            path.line_to(point(px(cell[3].x), px(cell[3].y)));
            path.close();
        }
    }
    for (bucket, path) in cells {
        #[allow(clippy::cast_precision_loss)]
        let fraction = bucket as f32 / (BUCKETS - 1) as f32;
        paint_path(path, cut_depth_color(fraction, palette), window);
    }

    let mut outline = PathBuilder::stroke(px(1.0));
    outline.move_to(point(px(x0), px(y0)));
    outline.line_to(point(px(x1), px(y0)));
    outline.line_to(point(px(x1), px(y1)));
    outline.line_to(point(px(x0), px(y1)));
    outline.close();
    paint_path(outline, palette.border_strong.alpha(0.7), window);
}

fn phase_index(phase: PathPhase) -> u16 {
    match phase {
        PathPhase::Future => 0,
        PathPhase::Traversed => 1,
        PathPhase::Current => 2,
    }
}

fn sim_key(phase: PathPhase, kind: MoveKind, air: bool) -> u16 {
    (phase_index(phase) << 6) | (u16::from(air) << 5) | kind as u16
}

fn sim_key_style(key: u16) -> (PathPhase, MoveKind, bool) {
    let phase = match key >> 6 {
        0 => PathPhase::Future,
        2 => PathPhase::Current,
        _ => PathPhase::Traversed,
    };
    let air = (key >> 5) & 1 == 1;
    let kind = MoveKind::ALL
        .into_iter()
        .find(|kind| *kind as u16 == key & 0x1f)
        .unwrap_or(MoveKind::Feed);
    (phase, kind, air)
}

fn sim_path_builder(kind: MoveKind, phase: PathPhase, air: bool) -> PathBuilder {
    let style = kind.style();
    let width = style.width
        + if phase == PathPhase::Current {
            1.0
        } else {
            0.0
        };
    let builder = PathBuilder::stroke(px(width));
    if air {
        return builder.dash_array(&[px(2.0), px(3.0)]);
    }
    match style.dash {
        Some([on, off]) => builder.dash_array(&[px(on), px(off)]),
        None => builder,
    }
}

fn paint_sim_path(
    bounds: Bounds<Pixels>,
    plan: &SimPathPlan,
    sim: &SimCanvas,
    visibility: [bool; MoveKind::ALL.len()],
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut builders: BTreeMap<u16, PathBuilder> = BTreeMap::new();
    let mut last_points: BTreeMap<u16, Point2> = BTreeMap::new();
    let mut dots: Vec<(f32, f32, Hsla)> = Vec::new();

    for polyline in &plan.polylines {
        if !visibility[polyline.kind as usize] {
            continue;
        }
        for (index, segment) in polyline.segments.iter().enumerate() {
            let Some(start) = polyline.points.get(index).copied() else {
                continue;
            };
            let Some(end) = polyline.points.get(index + 1).copied() else {
                continue;
            };
            let phase = sim.segment_lines.phase(*segment, sim.snapshot.current_line);
            if phase == PathPhase::Future && !sim.toggles.future_path {
                continue;
            }
            if matches!(polyline.kind, MoveKind::Rapid | MoveKind::Retract) && !sim.toggles.rapids {
                continue;
            }
            let air = polyline.air.get(index).copied().unwrap_or(false);
            if air && !sim.toggles.air_cuts {
                continue;
            }
            let key = sim_key(phase, polyline.kind, air);
            let color = if air {
                palette.info.alpha(match phase {
                    PathPhase::Future => 0.2,
                    _ => 0.5,
                })
            } else {
                let color = kind_color(polyline.kind, palette);
                match phase {
                    PathPhase::Future => color.alpha(0.22),
                    PathPhase::Traversed | PathPhase::Current => color,
                }
            };
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            if dx * dx + dy * dy <= 0.36 {
                dots.push((left + end.x, top + end.y, color));
                continue;
            }
            let builder = builders
                .entry(key)
                .or_insert_with(|| sim_path_builder(polyline.kind, phase, air));
            let continues = last_points.get(&key).is_some_and(|last| {
                let dx = last.x - start.x;
                let dy = last.y - start.y;
                dx * dx + dy * dy <= 1.0e-4
            });
            if !continues {
                builder.move_to(point(px(left + start.x), px(top + start.y)));
            }
            builder.line_to(point(px(left + end.x), px(top + end.y)));
            last_points.insert(key, end);
        }
    }

    for (key, builder) in builders {
        let (phase, kind, air) = sim_key_style(key);
        let color = if air {
            palette
                .info
                .alpha(if phase == PathPhase::Future { 0.2 } else { 0.5 })
        } else {
            let color = kind_color(kind, palette);
            match phase {
                PathPhase::Future => color.alpha(0.22),
                PathPhase::Traversed | PathPhase::Current => color,
            }
        };
        paint_path(builder, color, window);
    }
    for (x, y, color) in dots {
        paint_dot(window, color, x, y, 1.8);
    }
}

/// Paints the 2D tool circle and crosshair at the current pose.
fn paint_sim_tool_2d(
    camera: &Camera2D,
    bounds: Bounds<Pixels>,
    sim: &SimCanvas,
    palette: Palette,
    window: &mut Window,
) {
    let pose = sim.snapshot.pose_mm;
    let Some((x, y)) = marker_screen(camera, bounds, Point2::new(pose[0] as f32, pose[1] as f32))
    else {
        return;
    };
    let radius_mm = sim
        .cutter
        .as_ref()
        .map_or(0.0, nc_stock::Cutter::display_radius_mm);
    #[allow(clippy::cast_possible_truncation)]
    let radius_px = if radius_mm > 0.0 {
        (radius_mm as f32 * camera.scale()).max(2.5)
    } else {
        4.0
    };
    let color = if sim.tool_assumed {
        palette.warning
    } else {
        palette.accent_hover
    };
    let mut circle = PathBuilder::stroke(px(1.6));
    let segments = 28;
    for index in 0..=segments {
        #[allow(clippy::cast_precision_loss)]
        let angle = std::f64::consts::TAU * index as f64 / segments as f64;
        #[allow(clippy::cast_possible_truncation)]
        let px_x = x + radius_px * nc_core::math::cos(angle) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let px_y = y - radius_px * nc_core::math::sin(angle) as f32;
        if index == 0 {
            circle.move_to(point(px(px_x), px(px_y)));
        } else {
            circle.line_to(point(px(px_x), px(px_y)));
        }
    }
    circle.close();
    paint_path(circle, color, window);

    let mut cross = PathBuilder::stroke(px(1.0));
    let arm = (radius_px + 8.0).max(12.0);
    cross.move_to(point(px(x - arm), px(y)));
    cross.line_to(point(px(x + arm), px(y)));
    cross.move_to(point(px(x), px(y - arm)));
    cross.line_to(point(px(x), px(y + arm)));
    paint_path(cross, palette.text.alpha(0.8), window);
    paint_dot(window, color, x, y, 2.2);
}

/// Paints the 3D tool cylinder, shank, and holder at the current pose.
fn paint_sim_tool_3d(
    camera: &Camera3D,
    bounds: Bounds<Pixels>,
    sim: &SimCanvas,
    palette: Palette,
    window: &mut Window,
) {
    const SEGMENTS: usize = 16;
    let pose = sim.snapshot.pose_mm;
    if !pose.iter().all(|value| value.is_finite()) {
        return;
    }
    let cutter = sim.cutter.as_ref();
    let radius_mm = cutter.map_or(0.0, nc_stock::Cutter::display_radius_mm);
    let thin_mm = if radius_mm > 0.0 {
        radius_mm
    } else {
        f64::from((camera.units_per_pixel() * 3.0).max(0.6))
    };
    let shank_mm = cutter
        .and_then(|cutter| cutter.shank_diameter_mm)
        .map_or(thin_mm, |diameter| (diameter * 0.5).max(thin_mm));
    let flute_mm = cutter
        .and_then(|cutter| cutter.flute_length_mm)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(10.0);
    let stickout_mm = cutter
        .and_then(|cutter| cutter.stickout_mm)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(flute_mm + 8.0)
        .max(flute_mm);
    let holder_mm = cutter
        .and_then(|cutter| cutter.holder_diameter_mm)
        .map_or(shank_mm * 1.5, |diameter| (diameter * 0.5).max(shank_mm));

    let tip = Point3::new(pose[0] as f32, pose[1] as f32, pose[2] as f32);
    let top = Point3::new(tip.x, tip.y, tip.z + stickout_mm as f32);
    let color = if sim.tool_assumed {
        palette.warning
    } else {
        palette.accent_hover
    };

    let ring = |z: f32, radius: f32| -> Vec<Option<(f32, f32)>> {
        (0..=SEGMENTS)
            .map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let angle = std::f64::consts::TAU * index as f64 / SEGMENTS as f64;
                #[allow(clippy::cast_possible_truncation)]
                let x = tip.x + radius * nc_core::math::cos(angle) as f32;
                #[allow(clippy::cast_possible_truncation)]
                let y = tip.y + radius * nc_core::math::sin(angle) as f32;
                project_window(camera, bounds, Point3::new(x, y, z))
            })
            .collect()
    };
    let bottom = ring(tip.z, thin_mm as f32);
    let mid = ring(tip.z + flute_mm as f32, shank_mm as f32);
    let holder = ring(top.z, holder_mm as f32);

    let mut sides = PathBuilder::stroke(px(1.0));
    for index in 0..SEGMENTS {
        let (Some(bottom_a), Some(bottom_b), Some(mid_a), Some(mid_b)) = (
            bottom.get(index).copied().flatten(),
            bottom.get(index + 1).copied().flatten(),
            mid.get(index).copied().flatten(),
            mid.get(index + 1).copied().flatten(),
        ) else {
            continue;
        };
        sides.move_to(point(px(bottom_a.0), px(bottom_a.1)));
        sides.line_to(point(px(mid_a.0), px(mid_a.1)));
        sides.line_to(point(px(mid_b.0), px(mid_b.1)));
        sides.line_to(point(px(bottom_b.0), px(bottom_b.1)));
    }
    paint_path(sides, color.alpha(0.55), window);

    let mut rings = PathBuilder::stroke(px(1.2));
    for ring in [&bottom, &mid, &holder] {
        let mut started = false;
        for screen in ring.iter().flatten() {
            if !started {
                rings.move_to(point(px(screen.0), px(screen.1)));
                started = true;
            } else {
                rings.line_to(point(px(screen.0), px(screen.1)));
            }
        }
    }
    paint_path(rings, color, window);

    if let Some((x, y)) = marker_screen_3d(camera, bounds, tip) {
        let mut cross = PathBuilder::stroke(px(1.0));
        cross.move_to(point(px(x - 7.0), px(y)));
        cross.line_to(point(px(x + 7.0), px(y)));
        cross.move_to(point(px(x), px(y - 7.0)));
        cross.line_to(point(px(x), px(y + 7.0)));
        paint_path(cross, palette.text.alpha(0.85), window);
    }
    if tip.z.abs() > 1.0e-3
        && let Some((plane_x, plane_y)) =
            project_window(camera, bounds, Point3::new(tip.x, tip.y, 0.0))
    {
        let mut dropline = PathBuilder::stroke(px(1.0)).dash_array(&[px(3.0), px(3.0)]);
        if let Some((x, y)) = marker_screen_3d(camera, bounds, tip) {
            dropline.move_to(point(px(x), px(y)));
            dropline.line_to(point(px(plane_x), px(plane_y)));
            paint_path(dropline, color.alpha(0.35), window);
        }
    }
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

/// Colour for one semantic move kind.
#[must_use]
pub fn kind_color(kind: MoveKind, palette: Palette) -> Hsla {
    move_color(kind, palette)
}

/// Builds a stroke with the kind's width and dash pattern.
fn path_builder_for(kind: MoveKind) -> PathBuilder {
    let style = kind.style();
    let builder = PathBuilder::stroke(px(style.width));
    match style.dash {
        Some([on, off]) => builder.dash_array(&[px(on), px(off)]),
        None => builder,
    }
}

/// Screen-space radius of the base hole marker ring, in logical pixels.
const HOLE_MARKER_RADIUS_PX: f32 = 4.2;

/// Paints the hole marker that belongs to a canned-cycle kind.
fn paint_hole_marker(window: &mut Window, marker: HoleMarker, color: Hsla, x: f32, y: f32) {
    match marker {
        HoleMarker::Ring => paint_ring(window, color, x, y, HOLE_MARKER_RADIUS_PX, None),
        HoleMarker::DashedRing => {
            paint_ring(window, color, x, y, HOLE_MARKER_RADIUS_PX, Some([2.0, 2.0]))
        }
        HoleMarker::DoubleRing => {
            paint_ring(window, color, x, y, HOLE_MARKER_RADIUS_PX * 0.68, None);
            paint_ring(window, color, x, y, HOLE_MARKER_RADIUS_PX * 1.25, None);
        }
    }
}

/// Paints a screen-space ring, optionally dashed.
fn paint_ring(
    window: &mut Window,
    color: Hsla,
    x: f32,
    y: f32,
    radius: f32,
    dash: Option<[f32; 2]>,
) {
    const SEGMENTS: usize = 20;
    if !x.is_finite() || !y.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(1.2));
    if let Some([on, off]) = dash {
        builder = builder.dash_array(&[px(on), px(off)]);
    }
    for index in 0..=SEGMENTS {
        #[allow(clippy::cast_precision_loss)]
        let angle = nc_core::math::TAU * index as f64 / SEGMENTS as f64;
        #[allow(clippy::cast_possible_truncation)]
        let point_x = x + radius * nc_core::math::cos(angle) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let point_y = y + radius * nc_core::math::sin(angle) as f32;
        if index == 0 {
            builder.move_to(point(px(point_x), px(point_y)));
        } else {
            builder.line_to(point(px(point_x), px(point_y)));
        }
    }
    builder.close();
    paint_path(builder, color, window);
}

/// Collects and paints one hole marker per canned-cycle polyline.
fn paint_hole_markers(
    window: &mut Window,
    plan: &RenderPlan,
    visibility: [bool; MoveKind::ALL.len()],
    palette: Palette,
    left: f32,
    top: f32,
) {
    let mut seen = std::collections::BTreeSet::new();
    for polyline in plan.polylines() {
        if !visibility[polyline.kind as usize] {
            continue;
        }
        let Some(marker) = polyline.kind.style().marker else {
            continue;
        };
        let Some(first) = polyline.points.first() else {
            continue;
        };
        let x = left + first.x;
        let y = top + first.y;
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        if !seen.insert((polyline.kind as u8, x.to_bits(), y.to_bits())) {
            continue;
        }
        paint_hole_marker(window, marker, kind_color(polyline.kind, palette), x, y);
    }
}

/// Paints the 3D plan in the plan's far-to-near order.
fn paint_plan_3d(
    bounds: Bounds<Pixels>,
    plan: &RenderPlan,
    visibility: [bool; MoveKind::ALL.len()],
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut batch: Option<(MoveKind, PathBuilder)> = None;

    for polyline in plan.polylines() {
        if !visibility[polyline.kind as usize] {
            continue;
        }
        if polyline.points.is_empty() {
            continue;
        }
        if polyline.points.len() == 1 {
            if let Some((kind, builder)) = batch.take() {
                paint_path(builder, kind_color(kind, palette), window);
            }
            let single = polyline.points[0];
            paint_dot(
                window,
                kind_color(polyline.kind, palette),
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
                paint_path(builder, kind_color(kind, palette), window);
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
        paint_path(builder, kind_color(kind, palette), window);
    }
    paint_hole_markers(window, plan, visibility, palette, left, top);
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
    visibility: [bool; MoveKind::ALL.len()],
    palette: Palette,
    window: &mut Window,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);

    let mut builders: [PathBuilder; MoveKind::ALL.len()] =
        std::array::from_fn(|index| path_builder_for(MoveKind::ALL[index]));
    let mut used = [false; MoveKind::ALL.len()];
    let mut dots: Vec<(f32, f32, Hsla)> = Vec::new();

    for polyline in plan.polylines() {
        let index = polyline.kind as usize;
        if !visibility[index] {
            continue;
        }
        if polyline.points.len() == 1 {
            let single = polyline.points[0];
            dots.push((
                left + single.x,
                top + single.y,
                kind_color(polyline.kind, palette),
            ));
            continue;
        }
        used[index] = true;
        let target = &mut builders[index];
        for (point_index, screen) in polyline.points.iter().enumerate() {
            let world = point(px(left + screen.x), px(top + screen.y));
            if point_index == 0 {
                target.move_to(world);
            } else {
                target.line_to(world);
            }
        }
    }

    for (index, builder) in builders.into_iter().enumerate() {
        if visibility[index] && used[index] {
            paint_path(builder, kind_color(MoveKind::ALL[index], palette), window);
        }
    }
    for (x, y, color) in dots {
        paint_dot(window, color, x, y, 1.8);
    }
    paint_hole_markers(window, plan, visibility, palette, left, top);
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
            viewer.prepare(bounds.size, Some(&program), None)
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
            viewer.prepare(bounds.size, Some(&program), None);
            assert_eq!(viewer.plan_builds, 1);
            // Every marker-only paint calls `prepare` with the same program
            // and camera; the plan key is unchanged, so no rebuild happens.
            for _ in 0..25 {
                let _ = viewer.prepare(bounds.size, Some(&program), None);
            }
            assert_eq!(viewer.plan_builds, 1);
        }

        let other = Arc::new(nc_gcode::parse_program("G0 X0\nG1 X5 F100\n").expect("program"));
        let mut viewer = handle.inner.borrow_mut();
        viewer.prepare(bounds.size, Some(&other), None);
        assert_eq!(viewer.plan_builds, 2);
    }

    #[test]
    fn sim_plan_rebuild_ignores_current_line_changes() {
        use crate::sim::{HeightfieldMesh, SegmentLines, SimCanvas, SimToggles};
        use nc_core::ToolTable;
        use nc_stock::StockModel;

        let handle = ViewerHandle::new();
        let program = demo();
        let stock = StockModel::infer(&program);
        let simulation = nc_sim::Simulation::new(
            &program,
            stock,
            &ToolTable::default(),
            &nc_sim::SimConfig::default(),
        )
        .expect("simulation");
        let mesh = Arc::new(HeightfieldMesh::sample(simulation.heightmap(), &stock, 8));
        let stock_box = stock.to_stock_box().expect("stock box");
        let canvas = SimCanvas {
            snapshot: simulation.snapshot(),
            mesh,
            stock: stock_box,
            cutter: None,
            segment_lines: Arc::new(SegmentLines::from_program(&program)),
            toggles: SimToggles::default(),
            tool_assumed: true,
        };
        let bounds = test_bounds(900.0, 650.0);
        {
            let mut viewer = handle.inner.borrow_mut();
            let _ = viewer.prepare(bounds.size, Some(&program), Some(&canvas));
            assert_eq!(viewer.sim_plan_builds, 1);
            let mut moved_line = canvas.clone();
            moved_line.snapshot.current_line = moved_line.snapshot.current_line.saturating_add(3);
            for _ in 0..10 {
                let _ = viewer.prepare(bounds.size, Some(&program), Some(&moved_line));
            }
            assert_eq!(viewer.sim_plan_builds, 1, "line changes rebuilt the plan");
            viewer.camera.pan(Point2::new(25.0, 0.0));
            let _ = viewer.prepare(bounds.size, Some(&program), Some(&moved_line));
            assert_eq!(viewer.sim_plan_builds, 2, "camera change did not rebuild");
        }
    }

    #[test]
    fn a_missing_program_without_preview_stays_empty() {
        let handle = ViewerHandle::new();
        let bounds = test_bounds(800.0, 600.0);
        let mut viewer = handle.inner.borrow_mut();
        viewer.preview = false;
        let _ = viewer.prepare(bounds.size, None, None);
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
        let _ = viewer.prepare(bounds.size, Some(&program), None);
        assert!(!viewer.cache.plan.is_empty());
        assert!(viewer.cache.plan.point_count() > 0);

        viewer.mode = ViewerMode::ThreeD;
        let _ = viewer.prepare(bounds.size, Some(&program), None);
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
            viewer.prepare(bounds.size, Some(&program), None)
        };
        assert!(handle.set_mode(ViewerMode::ThreeD));
        let three_d = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program), None);
            viewer.camera3d
        };
        assert!(three_d.distance() > 0.0);
        let back = {
            let mut viewer = handle.inner.borrow_mut();
            viewer.prepare(bounds.size, Some(&program), None)
        };
        assert_eq!(two_d.scale(), back.scale());
        assert!(handle.set_mode(ViewerMode::Top2D));
    }

    #[test]
    fn vertical_orbit_drag_is_inverted() {
        let handle = ViewerHandle::new();
        assert!(handle.set_mode(ViewerMode::ThreeD));
        let (start_pitch, start_eye_z) = {
            let viewer = handle.inner.borrow();
            (viewer.camera3d.pitch(), viewer.camera3d.eye().z)
        };

        handle.begin_drag(point(px(100.0), px(100.0)));
        assert!(handle.drag_to(point(px(100.0), px(130.0))));
        handle.end_drag();
        let (after_down_pitch, after_down_eye_z) = {
            let viewer = handle.inner.borrow();
            (viewer.camera3d.pitch(), viewer.camera3d.eye().z)
        };
        assert!(
            after_down_pitch > start_pitch,
            "dragging down must raise the pitch ({start_pitch} -> {after_down_pitch})"
        );
        assert!(
            after_down_eye_z > start_eye_z,
            "dragging down must raise the eye ({start_eye_z} -> {after_down_eye_z})"
        );

        handle.begin_drag(point(px(100.0), px(130.0)));
        assert!(handle.drag_to(point(px(100.0), px(100.0))));
        handle.end_drag();
        let (after_up_pitch, after_up_eye_z) = {
            let viewer = handle.inner.borrow();
            (viewer.camera3d.pitch(), viewer.camera3d.eye().z)
        };
        assert!(
            after_up_pitch < after_down_pitch,
            "dragging up must lower the pitch ({after_down_pitch} -> {after_up_pitch})"
        );
        assert!(
            (after_up_pitch - start_pitch).abs() < 1.0e-5,
            "an equal upward drag must return to the starting pitch \
             ({start_pitch} -> {after_up_pitch})"
        );
        assert!(
            (after_up_eye_z - start_eye_z).abs() < 1.0e-4,
            "an equal upward drag must return to the starting eye height \
             ({start_eye_z} -> {after_up_eye_z})"
        );
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
    fn every_move_kind_has_its_own_colour_and_stroke() {
        for palette in [Palette::dark(), Palette::light(), Palette::high_contrast()] {
            let mut colors: Vec<Hsla> = Vec::with_capacity(MoveKind::ALL.len());
            for kind in MoveKind::ALL {
                let color = kind_color(kind, palette);
                assert!(
                    !colors.contains(&color),
                    "{kind:?} repeats a colour in this palette"
                );
                colors.push(color);
                let style = kind.style();
                assert!(style.width > 0.0, "{kind:?} width");
                assert!(!style.label.is_empty(), "{kind:?} label");
            }
        }
    }

    #[test]
    fn hole_kinds_have_distinct_markers() {
        let markers: Vec<HoleMarker> = MoveKind::ALL
            .into_iter()
            .filter_map(|kind| kind.style().marker)
            .collect();
        assert_eq!(
            markers,
            vec![
                HoleMarker::Ring,
                HoleMarker::DashedRing,
                HoleMarker::DoubleRing
            ]
        );
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
