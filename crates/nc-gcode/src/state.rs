//! Modal parser state for the grblHAL G-code dialect.
//!
//! [`ParserState`] mirrors the modal groups a controller reports in its `$G`
//! parser-state response. Distances are canonical millimetres; the current
//! [`Units`] only affects how program words are interpreted when they are
//! read.

use nc_core::{Units, Wcs};
use nc_viewer::ArcPlane;
use serde::{Deserialize, Serialize};

/// Active motion mode.
///
/// Canned cycles live in the same modal group as `G0`..`G3`, matching grbl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MotionMode {
    /// `G0`, rapid positioning.
    #[default]
    Rapid,
    /// `G1`, linear feed.
    Linear,
    /// `G2`, clockwise arc.
    ArcCw,
    /// `G3`, counter-clockwise arc.
    ArcCcw,
    /// `G38.2`..`G38.5`, probing move.
    Probe(ProbeKind),
    /// One of the supported canned cycles.
    Canned(CannedCycleKind),
    /// `G80`, no motion mode.
    None,
}

impl MotionMode {
    /// The G-code that selects this mode.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Rapid => "G0",
            Self::Linear => "G1",
            Self::ArcCw => "G2",
            Self::ArcCcw => "G3",
            Self::Probe(kind) => kind.g_code(),
            Self::Canned(kind) => kind.g_code(),
            Self::None => "G80",
        }
    }

    /// Returns `true` for `G2` and `G3`.
    #[must_use]
    pub const fn is_arc(self) -> bool {
        matches!(self, Self::ArcCw | Self::ArcCcw)
    }

    /// Returns `true` for probing moves.
    #[must_use]
    pub const fn is_probe(self) -> bool {
        matches!(self, Self::Probe(_))
    }
}

/// Direction and error handling of a `G38.x` probing move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeKind {
    /// `G38.2`, probe toward the workpiece and error on no contact.
    Toward,
    /// `G38.3`, probe toward the workpiece and ignore no contact.
    TowardNoError,
    /// `G38.4`, probe away from the workpiece and error on no contact.
    Away,
    /// `G38.5`, probe away from the workpiece and ignore no contact.
    AwayNoError,
}

impl ProbeKind {
    /// The G-code that selects this probe kind.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Toward => "G38.2",
            Self::TowardNoError => "G38.3",
            Self::Away => "G38.4",
            Self::AwayNoError => "G38.5",
        }
    }
}

/// Canned cycles supported by grblHAL and this parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CannedCycleKind {
    /// `G73`, chip-breaking peck drilling.
    ChipBreak,
    /// `G81`, drilling.
    Drill,
    /// `G82`, drilling with dwell.
    DrillDwell,
    /// `G83`, peck drilling with full retract.
    PeckDrill,
    /// `G85`, boring with feed out.
    BoreFeedOut,
    /// `G86`, boring with spindle stop and rapid out.
    BoreStop,
    /// `G89`, boring with dwell and feed out.
    BoreDwell,
}

impl CannedCycleKind {
    /// The G-code that selects this cycle.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::ChipBreak => "G73",
            Self::Drill => "G81",
            Self::DrillDwell => "G82",
            Self::PeckDrill => "G83",
            Self::BoreFeedOut => "G85",
            Self::BoreStop => "G86",
            Self::BoreDwell => "G89",
        }
    }

    /// Returns `true` for cycles that take a `Q` peck increment.
    #[must_use]
    pub const fn needs_peck(self) -> bool {
        matches!(self, Self::ChipBreak | Self::PeckDrill)
    }

    /// Returns `true` for cycles that accept a `P` dwell.
    #[must_use]
    pub const fn has_dwell(self) -> bool {
        matches!(self, Self::DrillDwell | Self::BoreStop | Self::BoreDwell)
    }
}

/// Cutter compensation state.
///
/// v1 records the mode and never applies the offset: the toolpath stays on the
/// programmed centerline. Rules that depend on the tool radius must be skipped
/// while compensation is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutterComp {
    /// `G40`.
    #[default]
    Off,
    /// `G41`.
    Left,
    /// `G42`.
    Right,
}

impl CutterComp {
    /// The G-code that selects this state.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Off => "G40",
            Self::Left => "G41",
            Self::Right => "G42",
        }
    }

    /// Returns `true` when compensation is active.
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Active arc plane.
///
/// G18 is the ZX plane, so its in-plane axes are ordered `(Z, X)` to keep
/// clockwise/counter-clockwise direction consistent with the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Plane {
    /// `G17`, XY plane with +Z as its normal.
    #[default]
    Xy,
    /// `G18`, ZX plane with +Y as its normal.
    Zx,
    /// `G19`, YZ plane with +X as its normal.
    Yz,
}

impl Plane {
    /// The G-code that selects this plane.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Xy => "G17",
            Self::Zx => "G18",
            Self::Yz => "G19",
        }
    }

    /// The matching viewer plane.
    #[must_use]
    pub const fn arc_plane(self) -> ArcPlane {
        match self {
            Self::Xy => ArcPlane::Xy,
            Self::Zx => ArcPlane::Xz,
            Self::Yz => ArcPlane::Yz,
        }
    }

    /// Axis indices `(u, v)` of the plane's right-handed frame.
    pub(crate) const fn axes(self) -> (usize, usize) {
        match self {
            Self::Xy => (0, 1),
            Self::Zx => (2, 0),
            Self::Yz => (1, 2),
        }
    }

    /// Axis index of the plane normal (`Z`, `Y`, or `X`).
    pub(crate) const fn normal_axis(self) -> usize {
        match self {
            Self::Xy => 2,
            Self::Zx => 1,
            Self::Yz => 0,
        }
    }
}

/// Distance mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DistanceMode {
    /// `G90`, absolute coordinates.
    #[default]
    Absolute,
    /// `G91`, incremental coordinates.
    Incremental,
}

impl DistanceMode {
    /// The G-code that selects this mode.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Absolute => "G90",
            Self::Incremental => "G91",
        }
    }
}

/// Feed-rate mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FeedMode {
    /// `G94`, feed in units per minute.
    #[default]
    UnitsPerMinute,
    /// `G93`, inverse-time feed (one minute divided by the move time).
    InverseTime,
}

impl FeedMode {
    /// The G-code that selects this mode.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::UnitsPerMinute => "G94",
            Self::InverseTime => "G93",
        }
    }
}

/// Spindle direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SpindleMode {
    /// `M5`, spindle stopped.
    #[default]
    Off,
    /// `M3`, clockwise.
    Clockwise,
    /// `M4`, counter-clockwise.
    CounterClockwise,
}

impl SpindleMode {
    /// The M-code that selects this mode.
    #[must_use]
    pub const fn m_code(self) -> &'static str {
        match self {
            Self::Off => "M5",
            Self::Clockwise => "M3",
            Self::CounterClockwise => "M4",
        }
    }
}

/// Coolant state.
///
/// grblHAL keeps mist and flood as independent bits: `M7` sets mist, `M8` sets
/// flood, and `M9` clears both. Both can be on at the same time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CoolantMode {
    /// `M9`, coolant off.
    #[default]
    Off,
    /// `M7`, mist.
    Mist,
    /// `M8`, flood.
    Flood,
    /// `M7` and `M8` both active.
    MistAndFlood,
}

impl CoolantMode {
    /// The M-code words that describe this state.
    #[must_use]
    pub const fn m_code(self) -> &'static str {
        match self {
            Self::Off => "M9",
            Self::Mist => "M7",
            Self::Flood => "M8",
            Self::MistAndFlood => "M7 M8",
        }
    }

    /// Returns `true` when mist is on.
    #[must_use]
    pub const fn mist(self) -> bool {
        matches!(self, Self::Mist | Self::MistAndFlood)
    }

    /// Returns `true` when flood is on.
    #[must_use]
    pub const fn flood(self) -> bool {
        matches!(self, Self::Flood | Self::MistAndFlood)
    }
}

/// Path-control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PathControl {
    /// `G61`, exact path.
    ExactPath,
    /// `G64`, continuous path.
    #[default]
    Continuous,
}

impl PathControl {
    /// The G-code that selects this mode.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::ExactPath => "G61",
            Self::Continuous => "G64",
        }
    }
}

/// Canned-cycle retract mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RetractMode {
    /// `G98`, retract to the initial Z level.
    #[default]
    Initial,
    /// `G99`, retract to the R plane.
    RPlane,
}

impl RetractMode {
    /// The G-code that selects this mode.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Initial => "G98",
            Self::RPlane => "G99",
        }
    }
}

/// Tool-length compensation state (`G43.1`/`G49`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ToolLength {
    /// `G49`, no dynamic tool-length offset.
    #[default]
    Cancel,
    /// `G43.1`, offset applied to the machine-coordinate transform.
    Dynamic {
        /// Per-axis offset in millimetres.
        offset_mm: [f64; 3],
    },
}

impl ToolLength {
    /// The per-axis offset in millimetres, zero when cancelled.
    #[must_use]
    pub const fn offset_mm(self) -> [f64; 3] {
        match self {
            Self::Cancel => [0.0; 3],
            Self::Dynamic { offset_mm } => offset_mm,
        }
    }

    /// The G-code that selects this state.
    #[must_use]
    pub const fn g_code(self) -> &'static str {
        match self {
            Self::Cancel => "G49",
            Self::Dynamic { .. } => "G43.1",
        }
    }
}

/// `G51` uniform coordinate scaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    /// Positive scale factor applied about `pivot_mm`.
    pub factor: f64,
    /// Pivot point in millimetres, in program coordinates.
    pub pivot_mm: [f64; 3],
}

/// XY coordinate rotation, seeded by the caller from a controller setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    /// Counter-clockwise angle in radians.
    pub angle_rad: f64,
    /// Pivot point in the XY plane, in program coordinates.
    pub pivot_mm: [f64; 2],
}

/// Parameters of the active canned cycle.
///
/// `z_mm` and `r_mm` are absolute program coordinates. They are resolved from
/// `G90`/`G91` when the word is read, so a cycle repeat does not depend on the
/// distance mode of the first block. `initial_z_mm` is the program Z level at
/// the start of the cycle and is the reference for `G98` and for incremental
/// `R` words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CannedCycleParams {
    /// Hole depth in millimetres, in program coordinates.
    pub z_mm: f64,
    /// R (clearance) plane in millimetres, in program coordinates.
    pub r_mm: f64,
    /// Peck increment in millimetres, for `G73`/`G83`.
    pub q_mm: Option<f64>,
    /// Dwell at the bottom in seconds, for `G82`/`G86`/`G89`.
    pub p_seconds: Option<f64>,
    /// Program Z at the start of the cycle, in millimetres.
    pub initial_z_mm: f64,
}

/// Complete modal parser state.
///
/// The default is the state a freshly booted grblHAL controller reports:
/// `G0 G17 G21 G54 G90 G94 G98`, no tool, no feed, no spindle.
///
/// Parameter and expression state is not modal: it lives in the owning
/// [`Parser`](crate::Parser) as a bounded
/// [`Parameters`](crate::Parameters) table, so this snapshot stays `Copy` and
/// can be replaced from a controller `$G` report.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParserState {
    /// Active motion mode (the `G0`/`G1`/`G2`/`G3`/cycle group).
    pub motion: MotionMode,
    /// Active arc plane.
    pub plane: Plane,
    /// Program units for future words.
    pub units: Units,
    /// Absolute or incremental distance mode.
    pub distance: DistanceMode,
    /// Feed-rate interpretation.
    pub feed_mode: FeedMode,
    /// Active work coordinate system.
    pub wcs: Wcs,
    /// Spindle direction.
    pub spindle: SpindleMode,
    /// Coolant state.
    pub coolant: CoolantMode,
    /// Selected tool number from a `T` word or `M6`.
    pub tool: u32,
    /// Programmed feed in millimetres per minute for `G94`, or inverse minutes
    /// for `G93`. `None` until an `F` word is seen.
    pub feed: Option<f64>,
    /// Programmed spindle speed in revolutions per minute.
    pub spindle_rpm: f64,
    /// Dynamic tool-length offset.
    pub tool_length: ToolLength,
    /// `G92` offset in millimetres, applied in addition to the work offset.
    pub g92: [f64; 3],
    /// Offset saved by `G92.2` and restored by `G92.3`, in millimetres.
    pub suspended_g92: [f64; 3],
    /// `G51` scaling, when active.
    pub scaling: Option<Scaling>,
    /// XY rotation, when active.
    ///
    /// grblHAL does not have a `G68` G-code: rotation is a per-work-offset
    /// setting configured with `G10 L2 R`. This parser applies a rotation
    /// seeded by the caller through [`Parser::set_state`](crate::Parser::set_state);
    /// reading `G10 L2 R` is a documented follow-up.
    pub rotation: Option<Rotation>,
    /// `G61`/`G64` path control.
    pub path_control: PathControl,
    /// Cutter radius compensation state (`G40`/`G41`/`G42`).
    pub cutter_comp: CutterComp,
    /// Canned-cycle retract mode.
    pub retract: RetractMode,
    /// Parameters of the active canned cycle, when one is active.
    pub canned: Option<CannedCycleParams>,
    /// Current position in program (work) coordinates, in millimetres.
    pub position: [f64; 3],
}

impl ParserState {
    /// Feed converted to millimetres per minute, when the feed mode allows it.
    ///
    /// Returns `None` for `G93` inverse-time feed, where the programmed value
    /// is not a speed.
    #[must_use]
    pub fn feed_mm_per_min(&self) -> Option<f64> {
        match self.feed_mode {
            FeedMode::UnitsPerMinute => self.feed,
            FeedMode::InverseTime => None,
        }
    }

    /// The G- and M-codes that describe the modal state, in `$G` report order.
    ///
    /// This is a display helper for parity with grblHAL's parser-state
    /// report; it is not used by the parser itself.
    #[must_use]
    pub fn g_code_summary(&self) -> String {
        let mut words = vec![
            self.motion.g_code().to_string(),
            self.wcs.as_str().to_string(),
        ];
        if self.g92 != [0.0; 3] {
            words.push("G92".to_string());
        }
        words.extend([
            self.plane.g_code().to_string(),
            if self.units.is_metric() { "G21" } else { "G20" }.to_string(),
            self.distance.g_code().to_string(),
            self.feed_mode.g_code().to_string(),
            self.cutter_comp.g_code().to_string(),
            self.tool_length.g_code().to_string(),
            self.retract.g_code().to_string(),
            if self.scaling.is_some() { "G51" } else { "G50" }.to_string(),
            self.spindle.m_code().to_string(),
            self.coolant.m_code().to_string(),
        ]);
        words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_a_fresh_controller() {
        let state = ParserState::default();
        assert_eq!(state.motion, MotionMode::Rapid);
        assert_eq!(state.plane, Plane::Xy);
        assert_eq!(state.units, Units::Mm);
        assert_eq!(state.distance, DistanceMode::Absolute);
        assert_eq!(state.feed_mode, FeedMode::UnitsPerMinute);
        assert_eq!(state.wcs, Wcs::G54);
        assert_eq!(state.spindle, SpindleMode::Off);
        assert_eq!(state.coolant, CoolantMode::Off);
        assert_eq!(state.feed, None);
        assert_eq!(state.position, [0.0, 0.0, 0.0]);
        assert_eq!(state.tool_length, ToolLength::Cancel);
        assert_eq!(
            state.g_code_summary(),
            "G0 G54 G17 G21 G90 G94 G40 G49 G98 G50 M5 M9"
        );
    }

    #[test]
    fn plane_mapping_matches_the_viewer() {
        assert_eq!(Plane::Xy.arc_plane(), ArcPlane::Xy);
        assert_eq!(Plane::Zx.arc_plane(), ArcPlane::Xz);
        assert_eq!(Plane::Yz.arc_plane(), ArcPlane::Yz);
        assert_eq!(Plane::Xy.axes(), (0, 1));
        assert_eq!(Plane::Zx.axes(), (2, 0));
        assert_eq!(Plane::Yz.axes(), (1, 2));
        assert_eq!(Plane::Xy.normal_axis(), 2);
        assert_eq!(Plane::Zx.normal_axis(), 1);
        assert_eq!(Plane::Yz.normal_axis(), 0);
    }

    #[test]
    fn feed_conversion_respects_inverse_time() {
        let mut state = ParserState {
            feed: Some(100.0),
            ..ParserState::default()
        };
        assert_eq!(state.feed_mm_per_min(), Some(100.0));
        state.feed_mode = FeedMode::InverseTime;
        assert_eq!(state.feed_mm_per_min(), None);
    }
}
