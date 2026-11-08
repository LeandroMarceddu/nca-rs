//! Machine travel and limit configuration.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Optional machine limits used by travel and feed rules.
///
/// Every field is optional because a bare NC file has no machine data. A rule
/// that needs a value is skipped, with the missing input named, when it is
/// absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineConfig {
    /// Minimum travel in millimetres, per axis.
    pub travel_min_mm: [f64; 3],
    /// Maximum travel in millimetres, per axis.
    pub travel_max_mm: [f64; 3],
    /// Maximum programmed feed in mm/min.
    pub max_feed_mm_min: Option<f64>,
    /// Maximum spindle speed in rpm.
    pub max_spindle_rpm: Option<f64>,
    /// Rapid rate in mm/min used by timeline estimates.
    pub rapid_rate_mm_min: Option<f64>,
    /// `true` when the spindle is started by hand and no `M3`/`M4` is expected.
    pub spindle_manual: bool,
}

impl MachineConfig {
    /// Builds a configuration, validating that every limit is finite and that
    /// each minimum is below its maximum.
    pub fn new(travel_min_mm: [f64; 3], travel_max_mm: [f64; 3]) -> Result<Self, CoreError> {
        let config = Self {
            travel_min_mm,
            travel_max_mm,
            max_feed_mm_min: None,
            max_spindle_rpm: None,
            rapid_rate_mm_min: None,
            spindle_manual: false,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validates the travel envelope.
    pub fn validate(&self) -> Result<(), CoreError> {
        let finite = self
            .travel_min_mm
            .iter()
            .chain(self.travel_max_mm.iter())
            .all(|value| value.is_finite());
        let ordered = self
            .travel_min_mm
            .iter()
            .zip(self.travel_max_mm.iter())
            .all(|(min, max)| min <= max);
        if !finite || !ordered {
            return Err(CoreError::InvalidTravel {
                min: self.travel_min_mm,
                max: self.travel_max_mm,
            });
        }
        Ok(())
    }

    /// Returns `true` when `point_mm` is inside the configured travel box.
    #[must_use]
    pub fn contains(&self, point_mm: [f64; 3]) -> bool {
        (0..3).all(|axis| {
            point_mm[axis] >= self.travel_min_mm[axis] && point_mm[axis] <= self.travel_max_mm[axis]
        })
    }

    /// Effective rapid rate, or the documented default of 5000 mm/min.
    #[must_use]
    pub fn rapid_rate_mm_min(&self) -> f64 {
        self.rapid_rate_mm_min.unwrap_or(5000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_is_validated() {
        assert!(MachineConfig::new([0.0; 3], [100.0, 100.0, 0.0]).is_ok());
        assert!(MachineConfig::new([0.0; 3], [-1.0, 100.0, 0.0]).is_err());
        assert!(MachineConfig::new([0.0; 3], [f64::NAN, 100.0, 0.0]).is_err());
    }

    #[test]
    fn contains_checks_every_axis() {
        let config = MachineConfig::new([0.0, 0.0, -100.0], [100.0, 100.0, 0.0]).expect("valid");
        assert!(config.contains([50.0, 50.0, -10.0]));
        assert!(!config.contains([50.0, 101.0, -10.0]));
        assert!(!config.contains([50.0, 50.0, 1.0]));
    }
}
