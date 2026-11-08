//! Analysis configuration and documented defaults.

use std::collections::BTreeMap;

use nc_core::{MachineConfig, MaterialPreset, ToolTable};
use serde::{Deserialize, Serialize};

use crate::types::{RuleId, Severity};

/// Explicit stock dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StockConfig {
    /// Minimum corner of the stock in machine millimetres.
    pub origin_mm: [f64; 3],
    /// Size of the stock in millimetres.
    pub size_mm: [f64; 3],
}

impl StockConfig {
    /// Maximum corner of the stock.
    #[must_use]
    pub fn max_mm(&self) -> [f64; 3] {
        [
            self.origin_mm[0] + self.size_mm[0],
            self.origin_mm[1] + self.size_mm[1],
            self.origin_mm[2] + self.size_mm[2],
        ]
    }

    /// Builds a stock model from this configuration.
    pub fn to_stock(&self) -> Result<nc_stock::StockModel, nc_stock::StockError> {
        nc_stock::StockModel::explicit(self.origin_mm, self.max_mm())
    }
}

/// Numeric tolerances used by rules.
///
/// Every default is documented in `docs/RULES.md`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerances {
    /// Maximum allowed difference between start and end arc radius, in mm.
    pub arc_radius_mm: f64,
    /// Cutting segments shorter than this are "tiny", in mm.
    pub min_segment_mm: f64,
    /// Two hole centres closer than this in XY count as the same hole, in mm.
    pub hole_xy_mm: f64,
    /// Clearance above the stock top for rapids, in mm.
    pub clearance_mm: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            arc_radius_mm: 0.05,
            min_segment_mm: 0.01,
            hole_xy_mm: 0.05,
            clearance_mm: 2.0,
        }
    }
}

impl Tolerances {
    /// Returns a copy with every non-finite or negative value replaced by its
    /// default, so a malformed config cannot disable a rule silently.
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let defaults = Self::default();
        macro_rules! fix {
            ($field:ident) => {
                if self.$field.is_finite() && self.$field >= 0.0 {
                    self.$field
                } else {
                    defaults.$field
                }
            };
        }
        Self {
            arc_radius_mm: fix!(arc_radius_mm),
            min_segment_mm: fix!(min_segment_mm),
            hole_xy_mm: fix!(hole_xy_mm),
            clearance_mm: fix!(clearance_mm),
        }
    }
}

/// Per-rule settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSettings {
    /// `false` disables the rule; a disabled rule is reported as skipped.
    pub enabled: bool,
    /// Optional severity override.
    pub severity: Option<Severity>,
    /// Rule-specific numeric thresholds.
    pub thresholds: BTreeMap<String, f64>,
}

impl Default for RuleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: None,
            thresholds: BTreeMap::new(),
        }
    }
}

impl RuleSettings {
    /// Returns a threshold or the supplied default.
    #[must_use]
    pub fn threshold(&self, key: &str, default: f64) -> f64 {
        self.thresholds.get(key).copied().unwrap_or(default)
    }
}

/// Sparse per-rule overrides, keyed by stable rule id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleConfig(pub BTreeMap<RuleId, RuleSettings>);

impl RuleConfig {
    /// Settings for a rule, or the defaults when unconfigured.
    #[must_use]
    pub fn settings(&self, rule: &RuleId) -> RuleSettings {
        self.0.get(rule).cloned().unwrap_or_default()
    }

    /// Inserts settings for a rule.
    pub fn set(&mut self, rule: RuleId, settings: RuleSettings) {
        self.0.insert(rule, settings);
    }

    /// Iterates rules in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&RuleId, &RuleSettings)> {
        self.0.iter()
    }
}

/// Everything a rule may read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisConfig {
    /// Machine limits, when the user supplied them.
    pub machine: Option<MachineConfig>,
    /// Explicit stock, or `None` to infer.
    pub stock: Option<StockConfig>,
    /// Sparse tool overlay.
    pub tools: ToolTable,
    /// Material preset, when chosen.
    pub material: Option<MaterialPreset>,
    /// Rule tolerances.
    pub tolerances: Tolerances,
    /// Per-rule overrides.
    pub rules: RuleConfig,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            machine: None,
            stock: None,
            tools: ToolTable::new(),
            material: None,
            tolerances: Tolerances::default(),
            rules: RuleConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tolerances_are_documented_values() {
        let tolerances = Tolerances::default();
        assert_eq!(tolerances.arc_radius_mm, 0.05);
        assert_eq!(tolerances.min_segment_mm, 0.01);
        assert_eq!(tolerances.hole_xy_mm, 0.05);
        assert_eq!(tolerances.clearance_mm, 2.0);
    }

    #[test]
    fn bad_tolerances_fall_back() {
        let tolerances = Tolerances {
            clearance_mm: f64::NAN,
            min_segment_mm: -1.0,
            ..Tolerances::default()
        };
        let fixed = tolerances.sanitized();
        assert_eq!(fixed.clearance_mm, 2.0);
        assert_eq!(fixed.min_segment_mm, 0.01);
    }

    #[test]
    fn unconfigured_rule_is_enabled() {
        let config = RuleConfig::default();
        let settings = config.settings(&RuleId::new("modal.feed_missing"));
        assert!(settings.enabled);
        assert_eq!(settings.severity, None);
        assert_eq!(settings.threshold("clearance_mm", 2.0), 2.0);
    }

    #[test]
    fn stock_config_converts_to_a_model() {
        let config = StockConfig {
            origin_mm: [0.0, 0.0, -20.0],
            size_mm: [100.0, 80.0, 20.0],
        };
        let stock = config.to_stock().expect("stock");
        assert_eq!(stock.top_z_mm, 0.0);
    }
}
