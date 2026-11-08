//! Pure geometry and camera foundation for the nca-rs 2D/3D toolpath viewer.
//!
//! This crate has no GPUI, windowing, serial, or clock dependencies. It turns a
//! flat toolpath into screen-space polylines that any painter can draw, so the
//! renderer can be replaced (software painter, wgpu texture, or a future GPU
//! path) without touching the geometry layer.
//!
//! # Conventions
//!
//! - World coordinates are millimetres with +Z up. The g-code/domain layer owns
//!   `f64` machine precision; the viewer uses `f32` because rendering cannot
//!   resolve the difference.
//! - Screen coordinates are logical pixels with +Y down, matching GPUI canvas
//!   coordinates. [`Camera2D`] flips the world Y axis for a conventional
//!   top view.
//! - Arcs are planar. G18 is the ZX plane and G19 is the YZ plane, matching
//!   grblHAL's arc axis ordering. Helical arcs are not represented yet.
//! - Every public operation is total: invalid scales, factors, and geometry are
//!   ignored or clamped instead of panicking.
//!
//! # Example
//!
//! ```
//! use nc_viewer::{
//!     Camera2D, MoveKind, Point3, RenderOptions, RenderPlan, Toolpath, Viewport,
//! };
//!
//! let mut path = Toolpath::new();
//! path.append_line(
//!     Point3::new(0.0, 0.0, 0.0),
//!     Point3::new(40.0, 20.0, -1.0),
//!     MoveKind::Feed,
//!     0,
//!     0,
//! );
//!
//! let camera = Camera2D::fitted(
//!     Viewport::new(800.0, 600.0),
//!     path.bounds().xy_bounds(),
//!     16.0,
//! );
//! let plan = RenderPlan::from_2d(&path, &camera, RenderOptions::default());
//! assert_eq!(plan.len(), 1);
//! assert_eq!(plan.polylines[0].point_count(), 2);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod arc;
pub mod camera2d;
pub mod camera3d;
pub mod geom;
pub mod move_kind;
pub mod render;
pub mod segment;
pub mod stock;
pub mod toolpath;
pub mod viewport;

pub use arc::{Arc, ArcPlane, DEFAULT_CHORD_TOLERANCE, MAX_ARC_SEGMENTS};
pub use camera2d::Camera2D;
pub use camera3d::{Camera3D, ProjectedPoint, Projection};
pub use geom::{Bounds2, Bounds3, Point2, Point3};
pub use move_kind::{AnnotationKind, HoleMarker, MoveFamily, MoveKind, MoveStyle};
pub use render::{Polyline, RenderOptions, RenderPlan};
pub use segment::Segment;
pub use stock::StockBox;
pub use toolpath::{Toolpath, synthetic_toolpath};
pub use viewport::Viewport;
