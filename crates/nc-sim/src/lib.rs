//! Deterministic stock-removal playback.
//!
//! The simulator is a pure engine: the app owns the wall clock and calls
//! [`Simulation::advance`]. The internal tick is exactly 1/120 s of simulated
//! time, so the heightmap and the exact pose are a pure function of the tick
//! count regardless of frame rate. [`Simulation::stats`] exposes cached
//! UI-facing volume, depth, and time totals; the volume scan is keyed by the
//! heightmap version.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod error;
mod simulation;
mod timeline;

pub use config::SimConfig;
pub use error::SimError;
pub use simulation::{SimSnapshot, SimStats, Simulation};
pub use timeline::{MotionSpan, Timeline};

/// The fixed simulation tick in seconds of simulated time.
pub const TICK_S: f64 = 1.0 / 120.0;
