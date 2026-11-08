//! Machine-coordinate motion records: the analysis and simulation source of
//! truth.
//!
//! The display [`Toolpath`](nc_viewer::Toolpath) is still produced for the
//! viewer. `MotionRecord` is what rules and the simulator consume; it preserves
//! arcs and helixes exactly and stores canned cycles once per hole.

use std::fmt;

use nc_core::{ToolHint, Units, Wcs};
use nc_viewer::{AnnotationKind, MoveKind};
use serde::{Deserialize, Serialize};

pub use crate::state::{CannedCycleKind as CannedCycle, CoolantMode as CoolantState, CutterComp};
use crate::state::{CoolantMode, DistanceMode, FeedMode, Plane, RetractMode};

/// An interning handle into [`Program::modal_table`](crate::Program::modal_table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ModalId(pub u32);

/// The modal state in effect for a motion record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalState {
    /// Active units, or `None` before `G20`/`G21`.
    pub units: Option<Units>,
    /// Distance mode, or `None` before `G90`/`G91`.
    pub distance: Option<DistanceMode>,
    /// Feed-rate interpretation.
    pub feed_mode: FeedMode,
    /// Work coordinate system.
    pub wcs: Wcs,
    /// Coolant state.
    pub coolant: CoolantState,
    /// Cutter compensation state.
    pub cutter_comp: CutterComp,
    /// `true` while a `G92` offset is active.
    pub g92_active: bool,
    /// Canned-cycle retract mode.
    pub retract_mode: RetractMode,
}

impl ModalState {
    /// A conservative state used when an id is out of range.
    pub const UNKNOWN: Self = Self {
        units: None,
        distance: None,
        feed_mode: FeedMode::UnitsPerMinute,
        wcs: Wcs::G54,
        coolant: CoolantMode::Off,
        cutter_comp: CutterComp::Off,
        g92_active: false,
        retract_mode: RetractMode::Initial,
    };
}

impl Default for ModalState {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// How an arc's center was programmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArcFormat {
    /// `I`/`J`/`K` center offsets.
    Center,
    /// `R` radius.
    Radius,
}

/// Exact arc/helix geometry carried by a [`MotionRecord`].
#[derive(Debug, Clone, PartialEq)]
pub struct ArcRecord {
    /// Center in machine millimetres; the normal-axis value equals the start
    /// point's normal-axis value.
    pub center_mm: [f64; 3],
    /// Center-to-start radius in the arc plane.
    pub radius_mm: f64,
    /// Center-to-end radius in the arc plane.
    pub end_radius_mm: f64,
    /// Start angle about the center in the plane basis.
    pub start_angle_rad: f64,
    /// Signed sweep: positive counter-clockwise, negative clockwise.
    /// `|sweep|` is in `(0, 2*pi]`.
    pub sweep_rad: f64,
    /// Signed travel along the plane normal.
    pub axial_delta_mm: f64,
    /// Center or radius programming.
    pub format: ArcFormat,
}

/// One canned-cycle hole.
#[derive(Debug, Clone, PartialEq)]
pub struct CannedRecord {
    /// The active cycle.
    pub cycle: CannedCycle,
    /// Order of this hole within the cycle's run.
    pub hole_index: usize,
    /// R plane in millimetres.
    pub r_plane_mm: f64,
    /// Bottom Z in millimetres.
    pub bottom_z_mm: f64,
    /// Peck increment, when the cycle takes one.
    pub peck_q_mm: Option<f64>,
    /// Dwell in seconds, when the cycle takes one.
    pub dwell_s: Option<f64>,
    /// G98/G99 retract behavior.
    pub retract_mode: RetractMode,
}

/// One real motion in machine coordinates.
///
/// The layout is capped at 128 bytes; arcs and canned data are boxed so the
/// common record stays small. A test enforces the size.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionRecord {
    /// 1-based source line.
    pub line: usize,
    /// Motion index in program order.
    pub index: usize,
    /// Semantic motion kind.
    pub kind: MoveKind,
    /// Start point in machine millimetres.
    pub start_mm: [f64; 3],
    /// End point in machine millimetres.
    pub end_mm: [f64; 3],
    /// Programmed feed in mm/min, when applicable.
    pub feed_mm_min: Option<f64>,
    /// Programmed spindle speed in rpm.
    pub spindle_rpm: Option<f64>,
    /// Spindle direction, when on.
    pub spindle_cw: Option<bool>,
    /// Active tool number.
    pub tool: u32,
    /// Arc plane.
    pub plane: Plane,
    /// Arc/helix geometry.
    pub arc: Option<Box<ArcRecord>>,
    /// Canned-cycle hole geometry.
    pub canned: Option<Box<CannedRecord>>,
    /// Interned modal state.
    pub modal: ModalId,
}

impl MotionRecord {
    /// Creates a straight motion with no arc or canned data.
    #[must_use]
    pub fn linear(
        line: usize,
        index: usize,
        kind: MoveKind,
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    ) -> Self {
        Self {
            line,
            index,
            kind,
            start_mm,
            end_mm,
            feed_mm_min: None,
            spindle_rpm: None,
            spindle_cw: None,
            tool: 0,
            plane: Plane::Xy,
            arc: None,
            canned: None,
            modal: ModalId(0),
        }
    }

    /// Euclidean chord length in millimetres.
    #[must_use]
    pub fn chord_length_mm(&self) -> f64 {
        let dx = self.end_mm[0] - self.start_mm[0];
        let dy = self.end_mm[1] - self.start_mm[1];
        let dz = self.end_mm[2] - self.start_mm[2];
        nc_core::math::sqrt(dx * dx + dy * dy + dz * dz)
    }

    /// Exact path length: arc length plus axial travel for arcs, chord
    /// otherwise.
    #[must_use]
    pub fn path_length_mm(&self) -> f64 {
        match &self.arc {
            Some(arc) => nc_core::math::hypot(
                arc.radius_mm * nc_core::math::abs(arc.sweep_rad),
                arc.axial_delta_mm,
            ),
            None => self.chord_length_mm(),
        }
    }

    /// Returns `true` when this record removes material by default.
    #[must_use]
    pub fn is_cutting(&self) -> bool {
        self.kind.is_cutting()
    }
}

/// A non-motion event at a program position.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// 1-based source line.
    pub line: usize,
    /// Index of the next motion record.
    pub next_motion_index: usize,
    /// Machine position at the event.
    pub pos_mm: [f64; 3],
    /// Event kind.
    pub kind: AnnotationKind,
}

/// A source comment with its line.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    /// 1-based source line.
    pub line: usize,
    /// Comment text without delimiters.
    pub text: String,
}

/// A block the lossy parser skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedBlock {
    /// 1-based source line.
    pub line: usize,
    /// Stable reason key.
    pub reason: String,
    /// Raw source text.
    pub source: String,
}

/// The result of a lossy parse.
#[derive(Debug, Clone)]
pub struct LossyProgram {
    /// The parsed program, possibly partial.
    pub program: crate::Program,
    /// Per-block errors in source order.
    pub errors: Vec<crate::ParseError>,
    /// Blocks that were skipped.
    pub skipped_blocks: Vec<SkippedBlock>,
    /// First line after which new geometry is not trusted.
    pub trusted_geometry_until: Option<usize>,
}

impl fmt::Display for MotionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "L{} {} [{:.3}, {:.3}, {:.3}] -> [{:.3}, {:.3}, {:.3}]",
            self.line,
            self.kind,
            self.start_mm[0],
            self.start_mm[1],
            self.start_mm[2],
            self.end_mm[0],
            self.end_mm[1],
            self.end_mm[2]
        )
    }
}

/// Re-export so downstream crates reach the hint type through `nc-gcode`.
pub type ProgramToolHint = ToolHint;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_record_stays_compact() {
        let size = std::mem::size_of::<MotionRecord>();
        assert!(size <= 128, "MotionRecord grew to {size} bytes");
    }

    #[test]
    fn chord_and_arc_lengths() {
        let motion = MotionRecord::linear(1, 0, MoveKind::Feed, [0.0; 3], [3.0, 4.0, 0.0]);
        assert_eq!(motion.chord_length_mm(), 5.0);
        assert_eq!(motion.path_length_mm(), 5.0);

        let mut arc = motion.clone();
        arc.arc = Some(Box::new(ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::PI,
            axial_delta_mm: 2.0,
            format: ArcFormat::Center,
        }));
        let expected = nc_core::math::hypot(nc_core::math::PI * 10.0, 2.0);
        assert!((arc.path_length_mm() - expected).abs() < 1e-12);
    }

    #[test]
    fn unknown_modal_state_is_conservative() {
        let state = ModalState::default();
        assert_eq!(state.units, None);
        assert_eq!(state.distance, None);
        assert_eq!(state.cutter_comp, CutterComp::Off);
    }
}
