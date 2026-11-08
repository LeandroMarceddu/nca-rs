//! Simulator errors.

use thiserror::Error;

/// Errors raised while constructing or running a simulation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimError {
    /// The stock extent is invalid.
    #[error("invalid stock: {0}")]
    InvalidStock(String),
    /// The heightmap grid could not be allocated within the cell cap.
    #[error("simulation grid too large: {cells} cells > {max_cells}")]
    GridTooLarge {
        /// Requested cell count.
        cells: usize,
        /// Configured maximum.
        max_cells: usize,
    },
    /// The program has no motion to simulate.
    #[error("program has no motion")]
    EmptyProgram,
    /// The requested time or line is outside the timeline.
    #[error("seek target out of range")]
    SeekOutOfRange,
}
