//! Optional material presets used by process rules.
//!
//! A preset carries published starting ranges, not machine-specific truth.
//! Every value is optional; an unknown value skips the rule that needs it.
//! The built-in presets are documented in `docs/RULES.md`.

use serde::{Deserialize, Serialize};

/// A material preset with cutting-data ranges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialPreset {
    /// Human-readable preset name.
    pub name: String,
    /// Minimum recommended chipload in millimetres per tooth.
    pub chipload_min_mm: Option<f64>,
    /// Maximum recommended chipload in millimetres per tooth.
    pub chipload_max_mm: Option<f64>,
    /// Minimum surface speed in metres per minute.
    pub surface_speed_min_mpm: Option<f64>,
    /// Maximum surface speed in metres per minute.
    pub surface_speed_max_mpm: Option<f64>,
    /// Maximum material removal rate in cubic millimetres per minute.
    pub max_mrr_mm3_min: Option<f64>,
    /// `true` when the preset assumes flood or mist coolant.
    pub requires_coolant: bool,
    /// Free-form notes shown in the UI and report.
    pub notes: String,
}

impl MaterialPreset {
    /// Creates an empty preset with only a name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            chipload_min_mm: None,
            chipload_max_mm: None,
            surface_speed_min_mpm: None,
            surface_speed_max_mpm: None,
            max_mrr_mm3_min: None,
            requires_coolant: false,
            notes: String::new(),
        }
    }

    /// Names of the built-in presets.
    #[must_use]
    pub fn builtin_names() -> &'static [&'static str] {
        &["aluminum-6061", "brass", "mild-steel", "plywood"]
    }

    /// Looks up a built-in preset by name.
    ///
    /// These are conservative starting ranges for a small 3-axis mill, not a
    /// guarantee. They are documented in `docs/RULES.md`.
    #[must_use]
    pub fn builtin(name: &str) -> Option<Self> {
        let key = name.trim().to_ascii_lowercase();
        let mut preset = match key.as_str() {
            "aluminum-6061" | "aluminium-6061" | "6061" => {
                let mut preset = Self::new("aluminum-6061");
                preset.chipload_min_mm = Some(0.02);
                preset.chipload_max_mm = Some(0.10);
                preset.surface_speed_min_mpm = Some(100.0);
                preset.surface_speed_max_mpm = Some(300.0);
                preset.max_mrr_mm3_min = Some(50_000.0);
                preset.notes = "Conservative 6061-T6 starting range for a small mill.".into();
                preset
            }
            "brass" => {
                let mut preset = Self::new("brass");
                preset.chipload_min_mm = Some(0.02);
                preset.chipload_max_mm = Some(0.08);
                preset.surface_speed_min_mpm = Some(80.0);
                preset.surface_speed_max_mpm = Some(200.0);
                preset.max_mrr_mm3_min = Some(30_000.0);
                preset.notes = "Free-machining brass starting range.".into();
                preset
            }
            "mild-steel" | "steel" => {
                let mut preset = Self::new("mild-steel");
                preset.chipload_min_mm = Some(0.01);
                preset.chipload_max_mm = Some(0.05);
                preset.surface_speed_min_mpm = Some(30.0);
                preset.surface_speed_max_mpm = Some(80.0);
                preset.max_mrr_mm3_min = Some(10_000.0);
                preset.requires_coolant = true;
                preset.notes = "Mild steel starting range; coolant assumed.".into();
                preset
            }
            "plywood" | "wood" => {
                let mut preset = Self::new("plywood");
                preset.chipload_min_mm = Some(0.05);
                preset.chipload_max_mm = Some(0.25);
                preset.surface_speed_min_mpm = Some(100.0);
                preset.surface_speed_max_mpm = Some(300.0);
                preset.max_mrr_mm3_min = Some(60_000.0);
                preset.notes =
                    "Plywood/softwood starting range; dust extraction is not modelled.".into();
                preset
            }
            _ => return None,
        };
        if preset.name.is_empty() {
            preset.name = key;
        }
        Some(preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_available_and_finite() {
        for name in MaterialPreset::builtin_names() {
            let preset = MaterialPreset::builtin(name).expect("builtin");
            assert_eq!(&preset.name, name);
            for value in [
                preset.chipload_min_mm,
                preset.chipload_max_mm,
                preset.surface_speed_min_mpm,
                preset.surface_speed_max_mpm,
                preset.max_mrr_mm3_min,
            ]
            .into_iter()
            .flatten()
            {
                assert!(value.is_finite() && value > 0.0);
            }
        }
    }

    #[test]
    fn unknown_builtin_is_none() {
        assert!(MaterialPreset::builtin("unobtainium").is_none());
    }
}
