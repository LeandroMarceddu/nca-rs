//! Typed errors for the core domain.

use thiserror::Error;

/// Errors raised while validating core configuration values.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CoreError {
    /// A dialect name was not recognized.
    #[error("unknown G-code dialect: {0}")]
    UnknownDialect(String),
    /// A machine travel limit was inverted or non-finite.
    #[error("invalid machine travel limits: min {min:?} max {max:?}")]
    InvalidTravel {
        /// Declared minimum in millimetres.
        min: [f64; 3],
        /// Declared maximum in millimetres.
        max: [f64; 3],
    },
}
