//! Core domain types for `nca-rs`.
//!
//! This crate has no internal dependencies and no controller, clock, or UI
//! coupling. It owns coordinates, units, work coordinate systems, tool data,
//! machine limits, material presets, dialect identifiers, and the deterministic
//! math facade every other crate uses.
//!
//! All stored coordinates are millimetres. Inch mode is a display and parsing
//! concern handled by [`Units`] at the edges.
//!
//! ```
//! use nc_core::{Axis, Position, Units};
//!
//! let position = Position::xyz(25.4, 0.0, -2.0);
//! assert_eq!(position.get(Axis::X), 25.4);
//! assert_eq!(Units::Inch.from_mm(25.4), 1.0);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod axis;
mod dialect;
mod error;
mod machine;
mod material;
pub mod math;
mod position;
mod tool;
mod units;
mod wcs;

pub use axis::{AXIS_COUNT, Axes, Axis, InvalidAxisLetter};
pub use dialect::Dialect;
pub use error::CoreError;
pub use machine::MachineConfig;
pub use material::MaterialPreset;
pub use position::Position;
pub use tool::{FieldSource, Tool, ToolField, ToolHint, ToolShape, ToolTable};
pub use units::Units;
pub use wcs::{CoordinateSystem, Wcs};

/// Version of the core crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
