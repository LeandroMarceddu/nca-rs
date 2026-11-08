//! Stock model, 2.5D heightmap, cutter shapes, and deterministic sweeping.
//!
//! The stock is either inferred from the program's cutting bounds or supplied
//! explicitly by the user. Inferred stock is fitted to the cuts, so it is a
//! model, not a fact; rules that need a real stock declare that requirement and
//! are skipped when the stock is inferred.
//!
//! The sweep is the single material-removal implementation shared by the
//! simulator and the headless analysis engagement pass. `Heightmap::cut_motion`
//! subdivides long straight moves so a ramp follows the tool Z; arcs and
//! helixes stay on their exact recorded curve.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod arc;
mod cutter;
mod heightmap;
mod stock;

pub use arc::{angle_in_sweep, arc_bounds, arc_point, plane_axes};
pub use cutter::Cutter;
pub use heightmap::{Heightmap, SweepParams};
pub use stock::{InferenceBasis, StockModel, StockSource};

/// Errors raised while constructing stock or a simulation grid.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StockError {
    /// The requested grid would exceed the configured cell cap.
    #[error("stock grid would exceed the cell cap: {cells} cells > {max_cells}")]
    GridTooLarge {
        /// Requested cell count.
        cells: usize,
        /// Configured maximum.
        max_cells: usize,
    },
    /// A stock extent was non-finite or degenerate.
    #[error("invalid stock extents: min {min_mm:?} max {max_mm:?}")]
    InvalidExtent {
        /// Minimum corner in millimetres.
        min_mm: [f64; 3],
        /// Maximum corner in millimetres.
        max_mm: [f64; 3],
    },
}
