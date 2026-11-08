//! Simulator configuration.

use serde::{Deserialize, Serialize};

/// Configuration for one simulation.
///
/// Defaults are documented in `docs/SIMULATION.md`. A missing tool diameter is
/// the normal case and produces a labeled centerline trace, never a guessed
/// width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimConfig {
    /// Requested heightmap cell size in millimetres.
    pub cell_mm: f64,
    /// Maximum number of heightmap cells.
    pub max_cells: usize,
    /// Rapid rate used by the timeline in mm/min.
    pub rapid_rate_mm_min: f64,
    /// Pause shown at a tool change, in seconds.
    pub tool_change_pause_s: f64,
    /// When true, rapids below the stock top cut and are highlighted.
    pub crash_preview: bool,
    /// Render grid resolution while paused.
    pub render_resolution: usize,
    /// Upper bound on ticks performed by one `advance` call.
    pub max_ticks_per_advance: usize,
    /// Assumed cutter diameter when the tool table has none. `None` keeps the
    /// centerline trace.
    pub assumed_tool_diameter_mm: Option<f64>,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            cell_mm: 0.75,
            max_cells: 4_000_000,
            rapid_rate_mm_min: 5000.0,
            tool_change_pause_s: 2.0,
            crash_preview: false,
            render_resolution: 64,
            max_ticks_per_advance: 8,
            assumed_tool_diameter_mm: None,
        }
    }
}

impl SimConfig {
    /// Returns a copy with a validated cell size and tick budget.
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let mut config = self.clone();
        if !config.cell_mm.is_finite() || config.cell_mm <= 0.0 {
            config.cell_mm = Self::default().cell_mm;
        }
        if !config.rapid_rate_mm_min.is_finite() || config.rapid_rate_mm_min <= 0.0 {
            config.rapid_rate_mm_min = Self::default().rapid_rate_mm_min;
        }
        if !config.tool_change_pause_s.is_finite() || config.tool_change_pause_s < 0.0 {
            config.tool_change_pause_s = Self::default().tool_change_pause_s;
        }
        config.assumed_tool_diameter_mm = match config.assumed_tool_diameter_mm {
            Some(diameter) if diameter.is_finite() && diameter > 0.0 => Some(diameter),
            _ => None,
        };
        config.max_cells = config.max_cells.max(1);
        config.max_ticks_per_advance = config.max_ticks_per_advance.max(1);
        config.render_resolution = config.render_resolution.clamp(8, 512);
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_useful_and_finite() {
        let config = SimConfig::default();
        assert!(config.cell_mm > 0.0);
        assert!(config.rapid_rate_mm_min > 0.0);
        assert!(config.max_ticks_per_advance >= 1);
        assert!(config.assumed_tool_diameter_mm.is_none());
    }

    #[test]
    fn sanitize_repairs_bad_values() {
        let config = SimConfig {
            cell_mm: f64::NAN,
            max_ticks_per_advance: 0,
            ..SimConfig::default()
        };
        let config = config.sanitized();
        assert_eq!(config.cell_mm, SimConfig::default().cell_mm);
        assert_eq!(config.max_ticks_per_advance, 1);
    }
}
