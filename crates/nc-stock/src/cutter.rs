//! Cutter geometry derived from the sparse tool table.

use nc_core::{Tool, ToolShape};
use serde::{Deserialize, Serialize};

/// A cutter as the stock model and simulator see it.
///
/// A missing diameter is represented by `diameter_mm == None` and
/// `assumed == true`; the simulator then draws a centerline trace one cell
/// wide. There is never a zero-diameter cutter.
///
/// An unknown shape is kept as [`ToolShape::Unknown`] so the UI can label it;
/// the sweep treats it as a flat cylinder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cutter {
    /// Cutter shape; [`ToolShape::Unknown`] when not known.
    pub shape: ToolShape,
    /// Cutting diameter in millimetres, when known or explicitly assumed.
    pub diameter_mm: Option<f64>,
    /// Bull-nose corner radius in millimetres.
    pub corner_radius_mm: Option<f64>,
    /// Drill/V-bit tip angle in degrees.
    pub tip_angle_deg: Option<f64>,
    /// Tip-to-holder-face length in millimetres.
    pub stickout_mm: Option<f64>,
    /// Fluted length in millimetres.
    pub flute_length_mm: Option<f64>,
    /// Shank diameter in millimetres.
    pub shank_diameter_mm: Option<f64>,
    /// Holder diameter in millimetres.
    pub holder_diameter_mm: Option<f64>,
    /// `true` when the diameter is not real tool data. The UI must label it.
    pub assumed: bool,
}

impl Cutter {
    /// Builds a cutter from a resolved tool entry.
    ///
    /// `assumed_diameter_mm` is the simulator's optional fallback; `None`
    /// keeps the centerline trace.
    #[must_use]
    pub fn from_tool(tool: &Tool, assumed_diameter_mm: Option<f64>) -> Self {
        let (diameter_mm, assumed) = match tool.diameter_mm {
            Some(diameter) if diameter.is_finite() && diameter > 0.0 => (Some(diameter), false),
            _ => match assumed_diameter_mm {
                Some(diameter) if diameter.is_finite() && diameter > 0.0 => (Some(diameter), true),
                _ => (None, true),
            },
        };
        Self {
            shape: tool.shape,
            diameter_mm,
            corner_radius_mm: tool.corner_radius_mm,
            tip_angle_deg: tool.tip_angle_deg,
            stickout_mm: tool.stickout_mm,
            flute_length_mm: tool.flute_length_mm,
            shank_diameter_mm: tool.effective_shank_diameter_mm(),
            holder_diameter_mm: tool.holder_diameter_mm,
            assumed,
        }
    }

    /// Builds a centerline cutter with no known dimensions.
    #[must_use]
    pub fn centerline() -> Self {
        Self {
            shape: ToolShape::Unknown,
            diameter_mm: None,
            corner_radius_mm: None,
            tip_angle_deg: None,
            stickout_mm: None,
            flute_length_mm: None,
            shank_diameter_mm: None,
            holder_diameter_mm: None,
            assumed: true,
        }
    }

    /// Radius in millimetres, or `None` for a centerline trace.
    #[must_use]
    pub fn radius_mm(&self) -> Option<f64> {
        self.diameter_mm.map(|diameter| diameter * 0.5)
    }

    /// Sketch radius for the display: zero for the centerline trace.
    #[must_use]
    pub fn display_radius_mm(&self) -> f64 {
        self.radius_mm().unwrap_or(0.0)
    }

    /// `true` when the cutter is a centerline trace of unknown width.
    #[must_use]
    pub fn is_centerline(&self) -> bool {
        self.diameter_mm.is_none()
    }

    /// The shape used for material removal.
    ///
    /// An unknown shape is a flat cylinder. Callers that need to label the
    /// assumption inspect [`Cutter::shape`] or [`Cutter::is_shape_assumed`].
    #[must_use]
    pub fn effective_shape(&self) -> ToolShape {
        match self.shape {
            ToolShape::Unknown => ToolShape::Flat,
            shape => shape,
        }
    }

    /// `true` when the cutter shape is unknown and treated as a flat cylinder.
    #[must_use]
    pub fn is_shape_assumed(&self) -> bool {
        matches!(self.shape, ToolShape::Unknown)
    }

    /// `true` when the shape is a drill or V-bit with a conical tip.
    #[must_use]
    pub fn has_conical_tip(&self) -> bool {
        matches!(self.shape, ToolShape::Drill | ToolShape::VBit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_diameter_is_not_assumed() {
        let mut tool = Tool::new(1);
        tool.diameter_mm = Some(6.0);
        let cutter = Cutter::from_tool(&tool, Some(3.0));
        assert_eq!(cutter.diameter_mm, Some(6.0));
        assert!(!cutter.assumed);
        assert!(!cutter.is_centerline());
    }

    #[test]
    fn missing_diameter_is_centerline_by_default() {
        let tool = Tool::new(2);
        let cutter = Cutter::from_tool(&tool, None);
        assert!(cutter.is_centerline());
        assert!(cutter.assumed);
        assert_eq!(cutter.display_radius_mm(), 0.0);
    }

    #[test]
    fn assumed_diameter_is_flagged() {
        let tool = Tool::new(3);
        let cutter = Cutter::from_tool(&tool, Some(3.175));
        assert_eq!(cutter.diameter_mm, Some(3.175));
        assert!(cutter.assumed);
        assert!(!cutter.is_centerline());
    }

    #[test]
    fn unknown_shape_is_preserved_and_cuts_as_flat() {
        let mut tool = Tool::new(4);
        tool.diameter_mm = Some(4.0);
        let cutter = Cutter::from_tool(&tool, None);
        assert_eq!(cutter.shape, ToolShape::Unknown);
        assert_eq!(cutter.effective_shape(), ToolShape::Flat);
        assert!(cutter.is_shape_assumed());
    }
}
