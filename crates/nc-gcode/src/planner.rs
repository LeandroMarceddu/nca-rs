//! Block planning: turn typed words into validated modal updates and moves.
//!
//! Nothing is mutated while planning. A failed plan leaves the parser exactly
//! where it was, which is how a controller treats a block that returns an
//! error. The parser then executes the plan: it applies the modal state and
//! emits the prepared moves.

use nc_core::{Axis, Position, Wcs, math};
use nc_viewer::SegmentKind;

use crate::block::{Block, GCode, GWord, MCode, Value};
use crate::config::{MIN_ARC_RADIUS_MM, ParserConfig};
use crate::error::{ParseError, ParseErrorKind, WarningKind};
use crate::motion::{ArcFormat, CutterComp};
use crate::state::ParserState;
use crate::state::{
    CannedCycleKind, CannedCycleParams, CoolantMode, DistanceMode, FeedMode, MotionMode, Plane,
    RetractMode, Rotation, Scaling, SpindleMode, ToolLength,
};

/// Maximum pecks generated for one `G73`/`G83` block.
const MAX_CANNED_PECKS: u64 = 10_000;
/// Distance below which two points are considered the same for display.
const EPSILON_MM: f64 = 1.0e-9;
/// Distance below which an arc endpoint counts as the start (full circle).
const FULL_CIRCLE_EPSILON_MM: f64 = 1.0e-6;
/// Largest accepted `G51` scale factor.
const MAX_SCALE_FACTOR: f64 = 1_000.0;

/// Which optional words the block's command consumed.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Used {
    /// X, Y, Z consumed.
    pub axes: [bool; 3],
    /// I consumed.
    pub i: bool,
    /// J consumed.
    pub j: bool,
    /// K consumed.
    pub k: bool,
    /// R consumed.
    pub r: bool,
    /// P consumed.
    pub p: bool,
    /// D consumed.
    pub d: bool,
    /// Q consumed.
    pub q: bool,
    /// L consumed.
    pub l: bool,
}

impl Used {
    fn axis(&mut self, axis: usize) {
        if let Some(slot) = self.axes.get_mut(axis) {
            *slot = true;
        }
    }
}

/// One prepared motion in program (work) or machine coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlannedMove {
    /// Straight move whose target is in program (work) coordinates.
    Line {
        /// Target in program coordinates, millimetres.
        to_work: [f64; 3],
        /// Motion style.
        kind: SegmentKind,
    },
    /// Straight move whose target is an absolute machine coordinate (`G53`,
    /// `G28`, `G30`).
    MachineLine {
        /// Target in machine coordinates, millimetres.
        to_machine: [f64; 3],
        /// Motion style.
        kind: SegmentKind,
    },
    /// Circular (or helical) move in program coordinates.
    Arc {
        /// Arc endpoint in program coordinates, millimetres.
        to_work: [f64; 3],
        /// Arc center in program coordinates, millimetres.
        center_work: [f64; 3],
        /// Plane containing the arc.
        plane: Plane,
        /// Clockwise travel in the plane's right-handed frame.
        clockwise: bool,
    },
}

/// The command class of a straight motion, before semantic tagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinearClass {
    /// `G0` and machine/home travel.
    Rapid,
    /// `G1`.
    Feed,
    /// `G38.x`.
    Probe,
}

/// One real motion that becomes a [`MotionRecord`](crate::MotionRecord).
///
/// Display moves and real motions differ for canned cycles: a canned block
/// emits several display segments but exactly one motion record per hole.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlannedRecord {
    /// Straight move whose target is in program coordinates.
    Line {
        /// Target in program coordinates, millimetres.
        to_work: [f64; 3],
        /// Command class.
        class: LinearClass,
    },
    /// Straight move whose target is an absolute machine coordinate.
    MachineLine {
        /// Target in machine coordinates, millimetres.
        to_machine: [f64; 3],
        /// Command class.
        class: LinearClass,
    },
    /// Circular or helical move in program coordinates.
    Arc {
        /// Arc endpoint in program coordinates, millimetres.
        to_work: [f64; 3],
        /// Arc center in program coordinates, millimetres.
        center_work: [f64; 3],
        /// Plane containing the arc.
        plane: Plane,
        /// Clockwise travel in the plane's right-handed frame.
        clockwise: bool,
        /// Center or radius programming.
        format: ArcFormat,
    },
    /// One canned-cycle hole; sub-moves are generated on demand.
    Canned {
        /// Hole position and depth, in program coordinates.
        hole: PlannedHole,
    },
}

/// One canned-cycle hole in program coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlannedHole {
    /// Hole XY in program coordinates, millimetres.
    pub xy_work: [f64; 2],
    /// R plane in program coordinates, millimetres.
    pub r_mm: f64,
    /// Bottom Z in program coordinates, millimetres.
    pub z_mm: f64,
    /// Cycle kind.
    pub cycle: CannedCycleKind,
    /// Peck increment in millimetres.
    pub peck_q_mm: Option<f64>,
    /// Dwell in seconds.
    pub dwell_s: Option<f64>,
    /// Retract mode in effect for the hole.
    pub retract: RetractMode,
    /// Position after the cycle block, in program coordinates.
    pub end_work: [f64; 3],
}

/// One non-motion event prepared by a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlannedAnnotation {
    /// `G4`, seconds after dialect conversion.
    Dwell {
        /// Dwell duration in seconds.
        seconds: f64,
    },
    /// `M6`.
    ToolChange {
        /// Selected tool number.
        tool: u32,
    },
    /// `M3`/`M4`/`M5`.
    Spindle {
        /// `true` while the spindle is on.
        on: bool,
        /// Programmed speed in rpm.
        rpm: f64,
        /// `true` for clockwise.
        clockwise: bool,
    },
    /// `M7`/`M8`/`M9`.
    Coolant {
        /// `true` while coolant is on.
        on: bool,
    },
}

/// A validated block ready to be applied.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// Resulting modal state.
    pub state: ParserState,
    /// Resulting work coordinate offsets.
    pub offsets: [Position; 9],
    /// Resulting stored `G28`/`G30` positions.
    pub homes: [Option<[f64; 3]>; 2],
    /// Moves in execution order.
    pub moves: Vec<PlannedMove>,
    /// Real motions, one per line/arc/hole.
    pub records: Vec<PlannedRecord>,
    /// Non-motion events.
    pub annotations: Vec<PlannedAnnotation>,
    /// Non-fatal warnings produced by the block.
    pub warnings: Vec<WarningKind>,
    /// Arc blocks in this plan.
    pub arc_moves: usize,
    /// Canned cycles run by this block (0 or 1).
    pub canned_cycles: usize,
    /// Tool changes in this block.
    pub tool_changes: usize,
    /// The block ended the program with `M2`/`M30`.
    pub program_end: bool,
    /// Feed in millimetres per minute if this block set one in `G94`.
    pub feed_observed: Option<f64>,
    /// Spindle speed if this block set one.
    pub spindle_observed: Option<f64>,
    /// Tool selected by a `T` word, widened to `u32`.
    pub selected_tool: Option<u32>,
    /// The block declared `G20` or `G21`.
    pub explicit_units: bool,
    /// The block declared `G90` or `G91`.
    pub explicit_distance: bool,
    /// The block selected a new canned cycle.
    pub canned_selected: bool,
}

/// Mutable planning scratch state.
struct Draft {
    state: ParserState,
    offsets: [Position; 9],
    homes: [Option<[f64; 3]>; 2],
    moves: Vec<PlannedMove>,
    records: Vec<PlannedRecord>,
    annotations: Vec<PlannedAnnotation>,
    used: Used,
    warnings: Vec<WarningKind>,
    arc_moves: usize,
    canned_cycles: usize,
    tool_changes: usize,
    program_end: bool,
    feed_observed: Option<f64>,
    spindle_observed: Option<f64>,
    selected_tool: Option<u32>,
    explicit_units: bool,
    explicit_distance: bool,
    canned_selected: bool,
}

impl Draft {
    fn frame(&self) -> Frame {
        Frame::new(&self.state, &self.offsets)
    }
}

impl From<Draft> for Plan {
    fn from(draft: Draft) -> Self {
        Self {
            state: draft.state,
            offsets: draft.offsets,
            homes: draft.homes,
            moves: draft.moves,
            records: draft.records,
            annotations: draft.annotations,
            warnings: draft.warnings,
            arc_moves: draft.arc_moves,
            canned_cycles: draft.canned_cycles,
            tool_changes: draft.tool_changes,
            program_end: draft.program_end,
            feed_observed: draft.feed_observed,
            spindle_observed: draft.spindle_observed,
            selected_tool: draft.selected_tool,
            explicit_units: draft.explicit_units,
            explicit_distance: draft.explicit_distance,
            canned_selected: draft.canned_selected,
        }
    }
}

/// Affine program-to-machine transform: uniform scale about a pivot, then a
/// rotation about the Z axis, then a translation by the work offsets.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    scaling: Option<Scaling>,
    rotation: Option<Rotation>,
    shift: [f64; 3],
}

impl Frame {
    /// Builds the frame from modal state and work offsets.
    pub(crate) fn new(state: &ParserState, offsets: &[Position; 9]) -> Self {
        let offset = offsets
            .get(state.wcs.index())
            .copied()
            .unwrap_or_else(Position::zero);
        let tlo = state.tool_length.offset_mm();
        Self {
            scaling: state.scaling,
            rotation: state.rotation,
            shift: [
                offset.get(Axis::X) + state.g92[0] + tlo[0],
                offset.get(Axis::Y) + state.g92[1] + tlo[1],
                offset.get(Axis::Z) + state.g92[2] + tlo[2],
            ],
        }
    }

    /// Builds a frame with an explicit translation.
    fn with_shift(state: &ParserState, shift: [f64; 3]) -> Self {
        Self {
            scaling: state.scaling,
            rotation: state.rotation,
            shift,
        }
    }

    /// Applies scale and rotation without the translation.
    pub(crate) fn apply(&self, point: [f64; 3]) -> [f64; 3] {
        let mut point = point;
        if let Some(scaling) = self.scaling {
            point = [
                scaling.pivot_mm[0] + (point[0] - scaling.pivot_mm[0]) * scaling.factor,
                scaling.pivot_mm[1] + (point[1] - scaling.pivot_mm[1]) * scaling.factor,
                scaling.pivot_mm[2] + (point[2] - scaling.pivot_mm[2]) * scaling.factor,
            ];
        }
        if let Some(rotation) = self.rotation {
            let dx = point[0] - rotation.pivot_mm[0];
            let dy = point[1] - rotation.pivot_mm[1];
            let sin = math::sin(rotation.angle_rad);
            let cos = math::cos(rotation.angle_rad);
            point[0] = rotation.pivot_mm[0] + dx * cos - dy * sin;
            point[1] = rotation.pivot_mm[1] + dx * sin + dy * cos;
        }
        point
    }

    /// Reverses scale and rotation.
    pub(crate) fn unapply(&self, point: [f64; 3]) -> [f64; 3] {
        let mut point = point;
        if let Some(rotation) = self.rotation {
            let dx = point[0] - rotation.pivot_mm[0];
            let dy = point[1] - rotation.pivot_mm[1];
            let sin = math::sin(rotation.angle_rad);
            let cos = math::cos(rotation.angle_rad);
            point[0] = rotation.pivot_mm[0] + dx * cos + dy * sin;
            point[1] = rotation.pivot_mm[1] - dx * sin + dy * cos;
        }
        if let Some(scaling) = self.scaling
            && scaling.factor > 0.0
        {
            point = [
                scaling.pivot_mm[0] + (point[0] - scaling.pivot_mm[0]) / scaling.factor,
                scaling.pivot_mm[1] + (point[1] - scaling.pivot_mm[1]) / scaling.factor,
                scaling.pivot_mm[2] + (point[2] - scaling.pivot_mm[2]) / scaling.factor,
            ];
        }
        point
    }

    /// Converts a program point into machine/display coordinates.
    pub(crate) fn to_machine(self, point: [f64; 3]) -> [f64; 3] {
        let point = self.apply(point);
        [
            point[0] + self.shift[0],
            point[1] + self.shift[1],
            point[2] + self.shift[2],
        ]
    }

    /// Converts a machine/display point into program coordinates.
    pub(crate) fn to_work(self, point: [f64; 3]) -> [f64; 3] {
        self.unapply([
            point[0] - self.shift[0],
            point[1] - self.shift[1],
            point[2] - self.shift[2],
        ])
    }
}

/// Plans one block against the current modal state.
pub(crate) struct Planner<'a> {
    /// Starting modal state.
    pub state: ParserState,
    /// Starting work offsets.
    pub offsets: [Position; 9],
    /// Starting stored home positions.
    pub homes: [Option<[f64; 3]>; 2],
    /// Current machine/display position, in millimetres.
    pub machine: [f64; 3],
    /// Parser configuration.
    pub config: &'a ParserConfig,
    /// Source line for diagnostics.
    pub line: usize,
}

impl Planner<'_> {
    /// Validates `block` and returns the state and moves it produces.
    ///
    /// # Errors
    ///
    /// Returns structured errors; the caller must discard the block.
    pub(crate) fn plan(&self, block: &Block) -> Result<Plan, Vec<ParseError>> {
        let mut draft = Draft {
            state: self.state,
            offsets: self.offsets,
            homes: self.homes,
            moves: Vec::new(),
            records: Vec::new(),
            annotations: Vec::new(),
            used: Used::default(),
            warnings: Vec::new(),
            arc_moves: 0,
            canned_cycles: 0,
            tool_changes: 0,
            program_end: false,
            feed_observed: None,
            spindle_observed: None,
            selected_tool: None,
            explicit_units: false,
            explicit_distance: false,
            canned_selected: false,
        };

        let motion_g = block
            .g
            .iter()
            .find(|word| word.code.group() == crate::block::GGroup::Motion)
            .copied();
        let non_modal = block
            .g
            .iter()
            .find(|word| word.code.is_non_modal_action())
            .copied();
        let parameterized_transforms: Vec<GWord> = block
            .g
            .iter()
            .filter(|word| matches!(word.code, GCode::ToolLengthDynamic | GCode::ScalingSet))
            .copied()
            .collect();
        let parameterized_transform = parameterized_transforms.first().copied();
        let g53 = block.g.iter().any(|word| word.code == GCode::MachineCoords);

        let mut frame_changed = false;
        let mut feed_mode_changed = false;
        draft.explicit_units = block
            .g
            .iter()
            .any(|word| matches!(word.code, GCode::UnitsInch | GCode::UnitsMm));
        draft.explicit_distance = block
            .g
            .iter()
            .any(|word| matches!(word.code, GCode::Absolute | GCode::Incremental));
        for word in &block.g {
            match word.code {
                GCode::PlaneXy => draft.state.plane = Plane::Xy,
                GCode::PlaneZx => draft.state.plane = Plane::Zx,
                GCode::PlaneYz => draft.state.plane = Plane::Yz,
                GCode::UnitsInch => draft.state.units = nc_core::Units::Inch,
                GCode::UnitsMm => draft.state.units = nc_core::Units::Mm,
                GCode::Absolute => draft.state.distance = DistanceMode::Absolute,
                GCode::Incremental => draft.state.distance = DistanceMode::Incremental,
                GCode::FeedInverseTime => {
                    if draft.state.feed_mode != FeedMode::InverseTime {
                        draft.state.feed_mode = FeedMode::InverseTime;
                        feed_mode_changed = true;
                    }
                }
                GCode::FeedPerMinute => {
                    if draft.state.feed_mode != FeedMode::UnitsPerMinute {
                        draft.state.feed_mode = FeedMode::UnitsPerMinute;
                        feed_mode_changed = true;
                    }
                }
                GCode::RetractInitial => draft.state.retract = RetractMode::Initial,
                GCode::RetractR => draft.state.retract = RetractMode::RPlane,
                GCode::PathExact => draft.state.path_control = crate::state::PathControl::ExactPath,
                GCode::PathContinuous => {
                    draft.state.path_control = crate::state::PathControl::Continuous;
                }
                GCode::CutterCancel => draft.state.cutter_comp = CutterComp::Off,
                GCode::CutterLeft => draft.state.cutter_comp = CutterComp::Left,
                GCode::CutterRight => draft.state.cutter_comp = CutterComp::Right,
                GCode::Wcs(wcs) => {
                    if draft.state.wcs != wcs {
                        draft.state.wcs = wcs;
                        frame_changed = true;
                    }
                }
                GCode::ToolLengthCancel => {
                    if !matches!(draft.state.tool_length, ToolLength::Cancel) {
                        draft.state.tool_length = ToolLength::Cancel;
                        frame_changed = true;
                    }
                }
                GCode::ScalingCancel => {
                    frame_changed |= draft.state.scaling.take().is_some();
                }
                _ => {}
            }
        }
        if frame_changed {
            draft.state.position = draft.frame().to_work(self.machine);
        }
        // A feed programmed in the other mode is not meaningful after a mode
        // change; an F word in this block is applied below.
        if feed_mode_changed && block.f.is_none() {
            draft.state.feed = None;
        }

        // A parameterized transform changes the coordinate frame, so it cannot
        // share a block with a motion or another exclusive action.
        if let Some(word) = parameterized_transforms.get(1) {
            return Err(self.error(
                word.column,
                ParseErrorKind::ConflictingCommands,
                "only one coordinate-transform command may appear per block",
            ));
        }
        if let Some(word) = parameterized_transform
            && (motion_g.is_some() || non_modal.is_some() || g53)
        {
            return Err(self.error(
                word.column,
                ParseErrorKind::ConflictingCommands,
                "coordinate-transform commands (G43.1, G51) must be in their own block",
            ));
        }
        if let Some(word) = non_modal
            && (motion_g.is_some() || g53)
        {
            return Err(self.error(
                word.column,
                ParseErrorKind::ConflictingCommands,
                format!(
                    "G{} cannot share a block with a motion command",
                    trim_number(word.value)
                ),
            ));
        }

        self.apply_feed_speed_tool(&mut draft, block)?;

        if block
            .g
            .iter()
            .any(|word| matches!(word.code, GCode::CutterLeft | GCode::CutterRight))
        {
            if let Some(d_word) = block.d {
                draft.used.d = true;
                if as_index(d_word.value, u32::MAX).is_none() {
                    return Err(self.error(
                        d_word.column,
                        ParseErrorKind::InvalidParameter,
                        "the D cutter-compensation offset must be a non-negative integer",
                    ));
                }
            }
            draft.warnings.push(WarningKind::CutterCompNotApplied);
        }

        if let Some(word) = non_modal {
            self.plan_non_modal(&mut draft, block, word)?;
        } else if let Some(word) = parameterized_transform {
            self.plan_transform(&mut draft, block, word)?;
            draft.state.position = draft.frame().to_work(self.machine);
        } else {
            self.plan_motion(&mut draft, block, motion_g, g53)?;
        }

        for word in &block.m {
            match word.code {
                MCode::SpindleClockwise => {
                    draft.state.spindle = SpindleMode::Clockwise;
                    draft.annotations.push(PlannedAnnotation::Spindle {
                        on: true,
                        rpm: draft.state.spindle_rpm,
                        clockwise: true,
                    });
                }
                MCode::SpindleCounterClockwise => {
                    draft.state.spindle = SpindleMode::CounterClockwise;
                    draft.annotations.push(PlannedAnnotation::Spindle {
                        on: true,
                        rpm: draft.state.spindle_rpm,
                        clockwise: false,
                    });
                }
                MCode::SpindleStop => {
                    draft.state.spindle = SpindleMode::Off;
                    draft.annotations.push(PlannedAnnotation::Spindle {
                        on: false,
                        rpm: draft.state.spindle_rpm,
                        clockwise: false,
                    });
                }
                MCode::MistOn => {
                    draft.state.coolant = if draft.state.coolant.flood() {
                        CoolantMode::MistAndFlood
                    } else {
                        CoolantMode::Mist
                    };
                    draft
                        .annotations
                        .push(PlannedAnnotation::Coolant { on: true });
                }
                MCode::FloodOn => {
                    draft.state.coolant = if draft.state.coolant.mist() {
                        CoolantMode::MistAndFlood
                    } else {
                        CoolantMode::Flood
                    };
                    draft
                        .annotations
                        .push(PlannedAnnotation::Coolant { on: true });
                }
                MCode::CoolantOff => {
                    draft.state.coolant = CoolantMode::Off;
                    draft
                        .annotations
                        .push(PlannedAnnotation::Coolant { on: false });
                }
                MCode::ToolChange => {
                    draft.tool_changes += 1;
                    draft.annotations.push(PlannedAnnotation::ToolChange {
                        tool: draft.state.tool,
                    });
                }
                MCode::ProgramEnd | MCode::ProgramEndRewind => draft.program_end = true,
                MCode::ProgramStop | MCode::OptionalStop => {}
            }
        }

        let unused = self.unused_words(block, &draft.used);
        if !unused.is_empty() {
            return Err(unused);
        }

        Ok(draft.into())
    }

    /// Applies `F`, `S`, and `T` words.
    fn apply_feed_speed_tool(
        &self,
        draft: &mut Draft,
        block: &Block,
    ) -> Result<(), Vec<ParseError>> {
        // grblHAL clears the inverse-time feed before and after every block;
        // a G93 motion without a fresh F word is therefore an error.
        if draft.state.feed_mode == FeedMode::InverseTime {
            draft.state.feed = None;
        }
        if let Some(word) = block.f {
            if word.value < 0.0 {
                return Err(self.error(
                    word.column,
                    ParseErrorKind::NegativeValue,
                    "negative feed rate",
                ));
            }
            let feed = match draft.state.feed_mode {
                FeedMode::UnitsPerMinute => draft.state.units.to_mm(word.value),
                FeedMode::InverseTime => word.value,
            };
            if !feed.is_finite() || feed > 1.0e7 {
                return Err(self.error(
                    word.column,
                    ParseErrorKind::ValueOutOfRange,
                    "feed rate is not a finite value below 10,000,000",
                ));
            }
            draft.state.feed = Some(feed);
            if draft.state.feed_mode == FeedMode::UnitsPerMinute {
                draft.feed_observed = Some(feed);
            }
        }
        if let Some(word) = block.s {
            if word.value < 0.0 {
                return Err(self.error(
                    word.column,
                    ParseErrorKind::NegativeValue,
                    "negative spindle speed",
                ));
            }
            if word.value > 1_000_000.0 {
                return Err(self.error(
                    word.column,
                    ParseErrorKind::ValueOutOfRange,
                    "spindle speed exceeds 1,000,000 rpm",
                ));
            }
            draft.state.spindle_rpm = word.value;
            draft.spindle_observed = Some(word.value);
        }
        if let Some(word) = block.t {
            let Some(tool) = as_index(word.value, u32::MAX) else {
                return Err(self.error(
                    word.column,
                    ParseErrorKind::ToolNumberOutOfRange,
                    format!(
                        "tool number T{} must be an integer between 0 and {}",
                        word.value,
                        u32::MAX
                    ),
                ));
            };
            draft.state.tool = tool;
            draft.selected_tool = Some(tool);
        }
        Ok(())
    }

    /// Plans a non-modal action (`G4`, `G10`, `G28`, `G30`, `G92`, `G92.1`).
    fn plan_non_modal(
        &self,
        draft: &mut Draft,
        block: &Block,
        word: GWord,
    ) -> Result<(), Vec<ParseError>> {
        match word.code {
            GCode::Dwell => {
                let Some(p) = block.p else {
                    return Err(self.error(
                        word.column,
                        ParseErrorKind::MissingParameter,
                        "G4 requires a P (dwell seconds) word",
                    ));
                };
                draft.used.p = true;
                if p.value < 0.0 {
                    return Err(self.error(
                        p.column,
                        ParseErrorKind::NegativeValue,
                        "G4 dwell cannot be negative",
                    ));
                }
                let seconds = if self.config.dialect.dwell_p_is_milliseconds() {
                    p.value / 1000.0
                } else {
                    p.value
                };
                draft.annotations.push(PlannedAnnotation::Dwell { seconds });
                Ok(())
            }
            GCode::SetCoordinateSystem => self.plan_g10(draft, block, word.column),
            GCode::Home => self.plan_home(draft, block, false),
            GCode::SecondHome => self.plan_home(draft, block, true),
            GCode::SetHome | GCode::SetSecondHome => {
                let index = usize::from(word.code == GCode::SetSecondHome);
                draft.homes[index] = Some(self.machine);
                Ok(())
            }
            GCode::SetOffsets => self.plan_g92(draft, block, word.column),
            GCode::ClearOffsets => {
                draft.state.g92 = [0.0; 3];
                draft.state.suspended_g92 = [0.0; 3];
                draft.state.position = draft.frame().to_work(self.machine);
                Ok(())
            }
            GCode::SuspendOffsets => {
                draft.state.suspended_g92 = draft.state.g92;
                draft.state.g92 = [0.0; 3];
                draft.state.position = draft.frame().to_work(self.machine);
                Ok(())
            }
            GCode::RestoreOffsets => {
                draft.state.g92 = draft.state.suspended_g92;
                draft.state.position = draft.frame().to_work(self.machine);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Plans `G10 L2`/`L20` work-offset changes.
    fn plan_g10(
        &self,
        draft: &mut Draft,
        block: &Block,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        let Some(l_word) = block.l else {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "G10 requires an L word (L2 or L20)",
            ));
        };
        draft.used.l = true;
        let Some(operation) = as_index(l_word.value, 100) else {
            return Err(self.error(
                l_word.column,
                ParseErrorKind::InvalidParameter,
                format!("G10 L{} is not a valid operation", l_word.value),
            ));
        };
        let Some(p_word) = block.p else {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "G10 requires a P word selecting the work coordinate system",
            ));
        };
        draft.used.p = true;
        let Some(index) = as_index(p_word.value, 9) else {
            return Err(self.error(
                p_word.column,
                ParseErrorKind::InvalidParameter,
                format!(
                    "G10 P{} must be 0 (current) through 9 (G59.3)",
                    p_word.value
                ),
            ));
        };
        let target = if index == 0 {
            draft.state.wcs
        } else {
            let Some(wcs) = Wcs::from_index(index as usize - 1) else {
                return Err(self.error(
                    p_word.column,
                    ParseErrorKind::InvalidParameter,
                    format!("G10 P{index} is not a work coordinate system"),
                ));
            };
            wcs
        };
        if block.r.is_some() {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "G10 L2/L20 R (per-WCS rotation) is not supported yet",
            ));
        }
        if !block.has_axis_words() {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "G10 requires at least one axis word",
            ));
        }

        match operation {
            2 => {
                let mut offset = draft.offsets[target.index()];
                for axis in 0..3 {
                    if let Some(axis_word) = axis_word(block, axis) {
                        draft.used.axis(axis);
                        // grblHAL sets L2 offsets absolutely, even under G91.
                        let millimetres = draft.state.units.to_mm(axis_word.value);
                        offset.set(axis_from_index(axis), millimetres);
                    }
                }
                self.check_coordinate(
                    [
                        offset.get(Axis::X),
                        offset.get(Axis::Y),
                        offset.get(Axis::Z),
                    ],
                    column,
                )?;
                draft.offsets[target.index()] = offset;
            }
            20 => {
                let old = draft.offsets[target.index()];
                let tool_length = draft.state.tool_length.offset_mm();
                let frame = Frame::with_shift(
                    &draft.state,
                    [
                        old.get(Axis::X) + draft.state.g92[0] + tool_length[0],
                        old.get(Axis::Y) + draft.state.g92[1] + tool_length[1],
                        old.get(Axis::Z) + draft.state.g92[2] + tool_length[2],
                    ],
                );
                let mut given = frame.to_work(self.machine);
                for (axis, value) in given.iter_mut().enumerate() {
                    if let Some(axis_word) = axis_word(block, axis) {
                        draft.used.axis(axis);
                        *value = draft.state.units.to_mm(axis_word.value);
                    }
                }
                self.check_coordinate(given, column)?;
                let transformed = frame.apply(given);
                let mut offset = Position::zero();
                for axis in 0..3 {
                    offset.set(
                        axis_from_index(axis),
                        self.machine[axis]
                            - draft.state.g92[axis]
                            - tool_length[axis]
                            - transformed[axis],
                    );
                }
                draft.offsets[target.index()] = offset;
            }
            _ => {
                return Err(self.error(
                    l_word.column,
                    ParseErrorKind::UnsupportedCode,
                    format!("G10 L{operation} is not supported; use L2 or L20"),
                ));
            }
        }

        draft.state.position = draft.frame().to_work(self.machine);
        Ok(())
    }

    /// Plans `G28`/`G30` and their stored-position variants.
    fn plan_home(
        &self,
        draft: &mut Draft,
        block: &Block,
        second: bool,
    ) -> Result<(), Vec<ParseError>> {
        let fallback = first_column(block);
        let mut current_machine = self.machine;
        if block.has_axis_words() {
            let target = self.resolve_work_target(draft, block);
            self.check_coordinate(target, fallback)?;
            current_machine = draft.frame().to_machine(target);
            draft.moves.push(PlannedMove::Line {
                to_work: target,
                kind: SegmentKind::Rapid,
            });
            draft.records.push(PlannedRecord::Line {
                to_work: target,
                class: LinearClass::Rapid,
            });
        }
        let stored = if second {
            draft.homes[1]
        } else {
            draft.homes[0]
        };
        let home = match stored {
            Some(home) => home,
            None => {
                draft.warnings.push(WarningKind::AssumedHomePosition);
                [0.0; 3]
            }
        };
        self.check_coordinate(home, fallback)?;
        if distance(current_machine, home) > EPSILON_MM {
            draft.moves.push(PlannedMove::MachineLine {
                to_machine: home,
                kind: SegmentKind::Rapid,
            });
            draft.records.push(PlannedRecord::MachineLine {
                to_machine: home,
                class: LinearClass::Rapid,
            });
        }
        draft.state.position = draft.frame().to_work(home);
        Ok(())
    }

    /// Plans a `G92` offset change.
    fn plan_g92(
        &self,
        draft: &mut Draft,
        block: &Block,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        if !block.has_axis_words() {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "G92 requires at least one axis word",
            ));
        }
        let mut given = draft.state.position;
        for (axis, value) in given.iter_mut().enumerate() {
            if let Some(word) = axis_word(block, axis) {
                draft.used.axis(axis);
                *value = draft.state.units.to_mm(word.value);
            }
        }
        self.check_coordinate(given, column)?;
        let frame = draft.frame();
        let tool_length = draft.state.tool_length.offset_mm();
        let offset = draft.offsets[draft.state.wcs.index()];
        let transformed = frame.apply(given);
        let mut g92 = [0.0; 3];
        for (axis, value) in g92.iter_mut().enumerate() {
            *value = self.machine[axis]
                - offset.get(axis_from_index(axis))
                - tool_length[axis]
                - transformed[axis];
        }
        draft.state.g92 = g92;
        draft.state.position = draft.frame().to_work(self.machine);
        Ok(())
    }

    /// Plans a parameterized coordinate transform (`G43.1`, `G51`).
    fn plan_transform(
        &self,
        draft: &mut Draft,
        block: &Block,
        word: GWord,
    ) -> Result<(), Vec<ParseError>> {
        match word.code {
            GCode::ToolLengthDynamic => {
                if !block.has_axis_words() {
                    return Err(self.error(
                        word.column,
                        ParseErrorKind::MissingParameter,
                        "G43.1 requires at least one axis word for the offset",
                    ));
                }
                let mut offset_mm = [0.0; 3];
                for (axis, value) in offset_mm.iter_mut().enumerate() {
                    if let Some(axis_word) = axis_word(block, axis) {
                        draft.used.axis(axis);
                        *value = draft.state.units.to_mm(axis_word.value);
                    }
                }
                self.check_coordinate(offset_mm, word.column)?;
                draft.state.tool_length = ToolLength::Dynamic { offset_mm };
            }
            GCode::ScalingSet => {
                let Some(p_word) = block.p else {
                    return Err(self.error(
                        word.column,
                        ParseErrorKind::MissingParameter,
                        "G51 requires a P scale factor",
                    ));
                };
                draft.used.p = true;
                if !p_word.value.is_finite()
                    || p_word.value <= 0.0
                    || p_word.value > MAX_SCALE_FACTOR
                {
                    return Err(self.error(
                        p_word.column,
                        ParseErrorKind::ValueOutOfRange,
                        format!(
                            "G51 scale factor must be greater than 0 and at most {MAX_SCALE_FACTOR}"
                        ),
                    ));
                }
                // Unspecified pivot axes default to the current position,
                // matching grblHAL's Fanuc-style scaling.
                let mut pivot_mm = draft.state.position;
                for (axis, value) in pivot_mm.iter_mut().enumerate() {
                    if let Some(axis_word) = axis_word(block, axis) {
                        draft.used.axis(axis);
                        *value = draft.state.units.to_mm(axis_word.value);
                    }
                }
                self.check_coordinate(pivot_mm, word.column)?;
                draft.state.scaling = if p_word.value == 1.0 {
                    None
                } else {
                    Some(Scaling {
                        factor: p_word.value,
                        pivot_mm,
                    })
                };
            }
            _ => {}
        }
        Ok(())
    }

    /// Plans a motion block: lines, arcs, probes, canned cycles, and `G53`.
    fn plan_motion(
        &self,
        draft: &mut Draft,
        block: &Block,
        motion_g: Option<GWord>,
        g53: bool,
    ) -> Result<(), Vec<ParseError>> {
        let fallback = first_column(block);
        let mut motion = draft.state.motion;
        let mut selected_cycle = false;

        if let Some(word) = motion_g {
            match word.code {
                GCode::Rapid => {
                    motion = MotionMode::Rapid;
                    draft.state.canned = None;
                }
                GCode::Linear => {
                    motion = MotionMode::Linear;
                    draft.state.canned = None;
                }
                GCode::ArcCw => {
                    motion = MotionMode::ArcCw;
                    draft.state.canned = None;
                }
                GCode::ArcCcw => {
                    motion = MotionMode::ArcCcw;
                    draft.state.canned = None;
                }
                GCode::Probe(kind) => {
                    motion = MotionMode::Probe(kind);
                    draft.state.canned = None;
                }
                GCode::CancelCanned => {
                    motion = MotionMode::None;
                    draft.state.canned = None;
                }
                GCode::Canned(kind) => {
                    motion = MotionMode::Canned(kind);
                    self.select_canned_cycle(draft, block, kind, word.column)?;
                    selected_cycle = true;
                    draft.canned_selected = true;
                }
                _ => {}
            }
        }
        draft.state.motion = motion;

        if matches!(motion, MotionMode::Canned(_)) && !selected_cycle {
            self.update_canned_cycle(draft, block)?;
        }

        if g53 && !matches!(motion, MotionMode::Rapid | MotionMode::Linear) {
            return Err(self.error(
                fallback,
                ParseErrorKind::UnsupportedCode,
                "G53 supports G0 and G1 only",
            ));
        }

        match motion {
            MotionMode::None => {
                if block.has_axis_words() || block.has_center_words() {
                    return Err(self.error(
                        fallback,
                        ParseErrorKind::NoMotionMode,
                        "axis or arc words require a motion mode (G80 cancels it)",
                    ));
                }
            }
            MotionMode::Canned(kind) => {
                if selected_cycle || block.has_cycle_words() {
                    self.plan_canned(draft, block, kind, fallback)?;
                    draft.canned_cycles += 1;
                }
            }
            MotionMode::ArcCw | MotionMode::ArcCcw => {
                if block.has_axis_words() || block.has_center_words() || block.r.is_some() {
                    self.plan_arc(draft, block, matches!(motion, MotionMode::ArcCw), fallback)?;
                    draft.arc_moves += 1;
                }
            }
            MotionMode::Rapid | MotionMode::Linear | MotionMode::Probe(_) => {
                if g53 {
                    self.plan_machine_line(draft, block, motion, fallback)?;
                } else if block.has_axis_words() {
                    self.plan_work_line(draft, block, motion, fallback)?;
                } else if motion.is_probe() && motion_g.is_some() {
                    // A bare `G38.2` selects a probe move and needs an axis.
                    // Without a motion word this block only changes modal
                    // state (for example `G91`), so the modal probe must not
                    // execute or fail here.
                    return Err(self.error(
                        fallback,
                        ParseErrorKind::MissingParameter,
                        "a probing move requires at least one axis word",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validates and stores the parameters of a newly selected canned cycle.
    fn select_canned_cycle(
        &self,
        draft: &mut Draft,
        block: &Block,
        kind: CannedCycleKind,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        if draft.state.plane != Plane::Xy {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "canned cycles require the XY plane (G17)",
            ));
        }
        let initial_z = draft.state.position[2];
        let Some(r_word) = block.r else {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                format!(
                    "{} requires an R (clearance) word on its first block",
                    kind.g_code()
                ),
            ));
        };
        let Some(z_word) = block.z else {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                format!(
                    "{} requires a Z (depth) word on its first block",
                    kind.g_code()
                ),
            ));
        };
        draft.used.r = true;
        draft.used.axes[2] = true;
        let r_mm = match draft.state.distance {
            DistanceMode::Absolute => draft.state.units.to_mm(r_word.value),
            DistanceMode::Incremental => initial_z + draft.state.units.to_mm(r_word.value),
        };
        let z_mm = match draft.state.distance {
            DistanceMode::Absolute => draft.state.units.to_mm(z_word.value),
            DistanceMode::Incremental => r_mm + draft.state.units.to_mm(z_word.value),
        };
        let mut q_mm = None;
        if let Some(q_word) = block.q {
            draft.used.q = true;
            let q = draft.state.units.to_mm(q_word.value);
            if q <= 0.0 {
                return Err(self.error(
                    q_word.column,
                    ParseErrorKind::InvalidParameter,
                    "the Q peck increment must be greater than zero",
                ));
            }
            q_mm = Some(q);
        }
        if kind.needs_peck() && q_mm.is_none() {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                format!("{} requires a Q peck increment", kind.g_code()),
            ));
        }
        let mut p_seconds = None;
        if let Some(p_word) = block.p {
            draft.used.p = true;
            if p_word.value < 0.0 {
                return Err(self.error(
                    p_word.column,
                    ParseErrorKind::NegativeValue,
                    "the P dwell cannot be negative",
                ));
            }
            p_seconds = Some(p_word.value);
        } else if kind.has_dwell() {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                format!("{} requires a P dwell on its first block", kind.g_code()),
            ));
        }
        draft.state.canned = Some(CannedCycleParams {
            z_mm,
            r_mm,
            q_mm,
            p_seconds,
            initial_z_mm: initial_z,
        });
        Ok(())
    }

    /// Updates an active canned cycle from a repeat block.
    fn update_canned_cycle(&self, draft: &mut Draft, block: &Block) -> Result<(), Vec<ParseError>> {
        let Some(mut params) = draft.state.canned else {
            return Ok(());
        };
        if let Some(r_word) = block.r {
            draft.used.r = true;
            params.r_mm = match draft.state.distance {
                DistanceMode::Absolute => draft.state.units.to_mm(r_word.value),
                DistanceMode::Incremental => {
                    draft.state.position[2] + draft.state.units.to_mm(r_word.value)
                }
            };
        }
        if let Some(z_word) = block.z {
            draft.used.axes[2] = true;
            params.z_mm = match draft.state.distance {
                DistanceMode::Absolute => draft.state.units.to_mm(z_word.value),
                DistanceMode::Incremental => params.r_mm + draft.state.units.to_mm(z_word.value),
            };
        }
        if let Some(q_word) = block.q {
            draft.used.q = true;
            let q = draft.state.units.to_mm(q_word.value);
            if q <= 0.0 {
                return Err(self.error(
                    q_word.column,
                    ParseErrorKind::InvalidParameter,
                    "the Q peck increment must be greater than zero",
                ));
            }
            params.q_mm = Some(q);
        }
        if let Some(p_word) = block.p {
            draft.used.p = true;
            if p_word.value < 0.0 {
                return Err(self.error(
                    p_word.column,
                    ParseErrorKind::NegativeValue,
                    "the P dwell cannot be negative",
                ));
            }
            params.p_seconds = Some(p_word.value);
        }
        draft.state.canned = Some(params);
        Ok(())
    }

    /// Plans a straight move in program coordinates.
    fn plan_work_line(
        &self,
        draft: &mut Draft,
        block: &Block,
        motion: MotionMode,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        let kind = match motion {
            MotionMode::Rapid => SegmentKind::Rapid,
            MotionMode::Linear => SegmentKind::Feed,
            MotionMode::Probe(_) => SegmentKind::Probe,
            _ => return Ok(()),
        };
        let class = match motion {
            MotionMode::Rapid => LinearClass::Rapid,
            MotionMode::Probe(_) => LinearClass::Probe,
            _ => LinearClass::Feed,
        };
        if kind != SegmentKind::Rapid {
            self.require_feed(draft, column)?;
        }
        let target = self.resolve_work_target(draft, block);
        self.check_coordinate(target, column)?;
        draft.moves.push(PlannedMove::Line {
            to_work: target,
            kind,
        });
        draft.records.push(PlannedRecord::Line {
            to_work: target,
            class,
        });
        draft.state.position = target;
        Ok(())
    }

    /// Plans a `G53` move in absolute machine coordinates.
    ///
    /// `G53` axis words are absolute machine coordinates regardless of the
    /// active `G90`/`G91` distance mode, matching grblHAL.
    fn plan_machine_line(
        &self,
        draft: &mut Draft,
        block: &Block,
        motion: MotionMode,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        if draft.state.scaling.is_some() || draft.state.rotation.is_some() {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "G53 cannot be combined with active G51 scaling or rotation",
            ));
        }
        if !block.has_axis_words() {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "G53 requires at least one axis word",
            ));
        }
        let kind = match motion {
            MotionMode::Rapid => SegmentKind::Rapid,
            MotionMode::Linear => SegmentKind::Feed,
            _ => {
                return Err(self.error(
                    column,
                    ParseErrorKind::UnsupportedCode,
                    "G53 supports G0 and G1 only",
                ));
            }
        };
        let class = if motion == MotionMode::Rapid {
            LinearClass::Rapid
        } else {
            LinearClass::Feed
        };
        if kind != SegmentKind::Rapid {
            self.require_feed(draft, column)?;
        }
        let mut target = self.machine;
        for (axis, value) in target.iter_mut().enumerate() {
            if let Some(word) = axis_word(block, axis) {
                draft.used.axis(axis);
                *value = draft.state.units.to_mm(word.value);
            }
        }
        self.check_coordinate(target, column)?;
        draft.moves.push(PlannedMove::MachineLine {
            to_machine: target,
            kind,
        });
        draft.records.push(PlannedRecord::MachineLine {
            to_machine: target,
            class,
        });
        draft.state.position = draft.frame().to_work(target);
        Ok(())
    }

    /// Plans a `G2`/`G3` arc from I/J/K or R.
    fn plan_arc(
        &self,
        draft: &mut Draft,
        block: &Block,
        clockwise: bool,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        if draft.state.rotation.is_some() && draft.state.plane != Plane::Xy {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "rotation with a G18/G19 arc cannot be represented as a planar arc",
            ));
        }
        self.require_feed(draft, column)?;

        let (u, v) = draft.state.plane.axes();
        let normal = draft.state.plane.normal_axis();
        let start = draft.state.position;
        let mut end = start;
        for (axis, value) in end.iter_mut().enumerate() {
            if let Some(word) = axis_word(block, axis) {
                draft.used.axis(axis);
                let millimetres = draft.state.units.to_mm(word.value);
                *value = match draft.state.distance {
                    DistanceMode::Absolute => millimetres,
                    DistanceMode::Incremental => *value + millimetres,
                };
            }
        }

        let center_present = block.has_center_words();
        if center_present && block.r.is_some() {
            return Err(self.error(
                column,
                ParseErrorKind::InvalidParameter,
                "an arc needs either I/J/K or R, not both",
            ));
        }
        if !center_present && block.r.is_none() {
            return Err(self.error(
                column,
                ParseErrorKind::InvalidArc,
                "an arc requires I/J/K center offsets or an R radius",
            ));
        }

        let mut center = start;
        if let Some(r_word) = block.r {
            draft.used.r = true;
            let radius = draft.state.units.to_mm(r_word.value);
            if !radius.is_finite() || math::abs(radius) < MIN_ARC_RADIUS_MM {
                return Err(self.error(
                    r_word.column,
                    ParseErrorKind::InvalidArc,
                    "arc radius must not be zero",
                ));
            }
            let start_uv = [start[u], start[v]];
            let end_uv = [end[u], end[v]];
            if uv_distance(start_uv, end_uv) <= FULL_CIRCLE_EPSILON_MM {
                return Err(self.error(
                    r_word.column,
                    ParseErrorKind::InvalidArc,
                    "a full-circle arc requires I/J/K center offsets, not R",
                ));
            }
            let Some(center_uv) = arc_center_from_radius(start_uv, end_uv, radius, clockwise)
            else {
                return Err(self.error(
                    r_word.column,
                    ParseErrorKind::InvalidArc,
                    "the R radius is too small for the arc chord",
                ));
            };
            center[u] = center_uv[0];
            center[v] = center_uv[1];
            center[normal] = start[normal];
        } else {
            for axis in 0..3 {
                if let Some(word) = axis_word_from_center(block, axis) {
                    if axis == normal {
                        continue;
                    }
                    match axis {
                        0 => draft.used.i = true,
                        1 => draft.used.j = true,
                        _ => draft.used.k = true,
                    }
                    center[axis] = start[axis] + draft.state.units.to_mm(word.value);
                }
            }
            let radius = uv_distance([start[u], start[v]], [center[u], center[v]]);
            if radius < MIN_ARC_RADIUS_MM {
                return Err(self.error(
                    column,
                    ParseErrorKind::InvalidArc,
                    "arc has zero radius; check the I/J/K center offsets",
                ));
            }
            let end_radius = uv_distance([end[u], end[v]], [center[u], center[v]]);
            let tolerance = self.config.arc_radius_tolerance_mm.max(radius * 0.001);
            if math::abs(end_radius - radius) > tolerance {
                return Err(self.error(
                    column,
                    ParseErrorKind::InvalidArc,
                    format!(
                        "arc endpoint radius {end_radius:.4} mm does not match start radius {radius:.4} mm (tolerance {tolerance:.4} mm)"
                    ),
                ));
            }
        }

        self.check_coordinate(end, column)?;
        self.check_coordinate(center, column)?;
        let format = if block.r.is_some() {
            ArcFormat::Radius
        } else {
            ArcFormat::Center
        };
        draft.moves.push(PlannedMove::Arc {
            to_work: end,
            center_work: center,
            plane: draft.state.plane,
            clockwise,
        });
        draft.records.push(PlannedRecord::Arc {
            to_work: end,
            center_work: center,
            plane: draft.state.plane,
            clockwise,
            format,
        });
        draft.state.position = end;
        Ok(())
    }

    /// Plans the moves of an active canned cycle.
    fn plan_canned(
        &self,
        draft: &mut Draft,
        block: &Block,
        kind: CannedCycleKind,
        column: usize,
    ) -> Result<(), Vec<ParseError>> {
        let Some(params) = draft.state.canned else {
            return Err(self.error(
                column,
                ParseErrorKind::MissingParameter,
                "no canned cycle is active",
            ));
        };
        if draft.state.plane != Plane::Xy {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "canned cycles require the XY plane (G17)",
            ));
        }
        if draft.state.feed_mode == FeedMode::InverseTime {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "canned cycles are not supported with G93 inverse-time feed",
            ));
        }
        if block.l.is_some() {
            return Err(self.error(
                column,
                ParseErrorKind::UnsupportedCode,
                "canned-cycle L repeat counts are not supported yet",
            ));
        }
        self.require_feed(draft, column)?;

        let start = draft.state.position;
        let mut target_xy = [start[0], start[1]];
        for (axis, value) in target_xy.iter_mut().enumerate() {
            if let Some(word) = axis_word(block, axis) {
                draft.used.axis(axis);
                let millimetres = draft.state.units.to_mm(word.value);
                *value = match draft.state.distance {
                    DistanceMode::Absolute => millimetres,
                    DistanceMode::Incremental => *value + millimetres,
                };
            }
        }

        let (x, y) = (target_xy[0], target_xy[1]);
        let r = params.r_mm;
        let z = params.z_mm;
        let retract = match draft.state.retract {
            crate::state::RetractMode::Initial => params.initial_z_mm,
            crate::state::RetractMode::RPlane => r,
        };
        self.check_coordinate([x, y, z], column)?;
        if r < z {
            return Err(self.error(
                column,
                ParseErrorKind::InvalidParameter,
                "the R plane is below the canned-cycle depth",
            ));
        }

        let mut current = start;
        if current[2] < r {
            let (start_x, start_y) = (current[0], current[1]);
            self.push_line(
                draft,
                &mut current,
                [start_x, start_y, r],
                SegmentKind::Rapid,
            );
        }
        if math::abs(current[0] - x) > EPSILON_MM || math::abs(current[1] - y) > EPSILON_MM {
            let current_z = current[2];
            self.push_line(draft, &mut current, [x, y, current_z], SegmentKind::Rapid);
        }
        if current[2] > r {
            self.push_line(draft, &mut current, [x, y, r], SegmentKind::Rapid);
        }

        let has_material = z < r;
        match kind {
            CannedCycleKind::Drill | CannedCycleKind::DrillDwell => {
                if has_material {
                    self.push_line(draft, &mut current, [x, y, z], SegmentKind::Feed);
                } else {
                    draft.warnings.push(WarningKind::EmptyCannedCycle);
                }
                self.push_line(draft, &mut current, [x, y, retract], SegmentKind::Rapid);
            }
            CannedCycleKind::ChipBreak | CannedCycleKind::PeckDrill => {
                if has_material {
                    let Some(q) = params.q_mm else {
                        return Err(self.error(
                            column,
                            ParseErrorKind::MissingParameter,
                            "a peck cycle requires a Q peck increment",
                        ));
                    };
                    if q <= 0.0 {
                        return Err(self.error(
                            column,
                            ParseErrorKind::InvalidParameter,
                            "the Q peck increment must be greater than zero",
                        ));
                    }
                    let total = r - z;
                    let pecks = math::ceil(total / q);
                    if !pecks.is_finite() || pecks > MAX_CANNED_PECKS as f64 {
                        return Err(self.error(
                            column,
                            ParseErrorKind::ValueOutOfRange,
                            format!("peck increment produces more than {MAX_CANNED_PECKS} pecks"),
                        ));
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let pecks = pecks.max(1.0) as u64;
                    let mut previous = r;
                    for index in 1..=pecks {
                        #[allow(clippy::cast_precision_loss)]
                        let depth = (r - (index as f64) * q).max(z);
                        if index > 1 {
                            let clearance = (previous + self.config.peck_clearance_mm).min(r);
                            if kind == CannedCycleKind::PeckDrill {
                                self.push_line(draft, &mut current, [x, y, r], SegmentKind::Rapid);
                            }
                            if clearance > depth {
                                self.push_line(
                                    draft,
                                    &mut current,
                                    [x, y, clearance],
                                    SegmentKind::Rapid,
                                );
                            }
                        }
                        self.push_line(draft, &mut current, [x, y, depth], SegmentKind::Feed);
                        previous = depth;
                    }
                } else {
                    draft.warnings.push(WarningKind::EmptyCannedCycle);
                }
                self.push_line(draft, &mut current, [x, y, retract], SegmentKind::Rapid);
            }
            CannedCycleKind::BoreFeedOut
            | CannedCycleKind::BoreStop
            | CannedCycleKind::BoreDwell => {
                if has_material {
                    self.push_line(draft, &mut current, [x, y, z], SegmentKind::Feed);
                } else {
                    draft.warnings.push(WarningKind::EmptyCannedCycle);
                }
                if kind == CannedCycleKind::BoreStop {
                    draft.state.spindle = SpindleMode::Off;
                }
                self.push_line(draft, &mut current, [x, y, r], SegmentKind::Feed);
                if math::abs(retract - r) > EPSILON_MM {
                    self.push_line(draft, &mut current, [x, y, retract], SegmentKind::Rapid);
                }
            }
        }

        draft.state.position = [x, y, retract];
        draft.records.push(PlannedRecord::Canned {
            hole: PlannedHole {
                xy_work: [x, y],
                r_mm: r,
                z_mm: z,
                cycle: kind,
                peck_q_mm: if kind.needs_peck() { params.q_mm } else { None },
                dwell_s: if kind.has_dwell() {
                    params.p_seconds
                } else {
                    None
                },
                retract: draft.state.retract,
                end_work: [x, y, retract],
            },
        });
        Ok(())
    }

    /// Appends a straight move, skipping zero-length geometry.
    fn push_line(
        &self,
        draft: &mut Draft,
        current: &mut [f64; 3],
        to: [f64; 3],
        kind: SegmentKind,
    ) {
        if distance(*current, to) > EPSILON_MM {
            draft.moves.push(PlannedMove::Line { to_work: to, kind });
            *current = to;
        }
    }

    /// Builds the target of a program-coordinate move.
    fn resolve_work_target(&self, draft: &mut Draft, block: &Block) -> [f64; 3] {
        let mut target = draft.state.position;
        for (axis, value) in target.iter_mut().enumerate() {
            if let Some(word) = axis_word(block, axis) {
                draft.used.axis(axis);
                let millimetres = draft.state.units.to_mm(word.value);
                *value = match draft.state.distance {
                    DistanceMode::Absolute => millimetres,
                    DistanceMode::Incremental => *value + millimetres,
                };
            }
        }
        target
    }

    /// Requires a usable feed rate before a feed, arc, probe, or cycle move.
    fn require_feed(&self, draft: &Draft, column: usize) -> Result<(), Vec<ParseError>> {
        match draft.state.feed {
            Some(feed) if feed > 0.0 => Ok(()),
            _ => Err(self.error(
                column,
                ParseErrorKind::FeedRateUndefined,
                "feed rate is not set; add an F word before this move",
            )),
        }
    }

    /// Rejects non-finite or absurdly distant coordinates.
    fn check_coordinate(&self, point: [f64; 3], column: usize) -> Result<(), Vec<ParseError>> {
        for value in point {
            if !value.is_finite() || math::abs(value) > self.config.max_coordinate_mm {
                return Err(self.error(
                    column,
                    ParseErrorKind::ValueOutOfRange,
                    format!(
                        "coordinate {value} is outside +/-{} mm",
                        self.config.max_coordinate_mm
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Reports words that the block's command did not consume.
    fn unused_words(&self, block: &Block, used: &Used) -> Vec<ParseError> {
        let mut errors = Vec::new();
        for (axis, letter) in [(0usize, 'X'), (1, 'Y'), (2, 'Z')] {
            if let Some(word) = axis_word(block, axis)
                && !used.axes.get(axis).copied().unwrap_or(false)
            {
                errors.push(self.unused(letter, word.column));
            }
        }
        for (present, consumed, letter, word) in [
            (block.i.is_some(), used.i, 'I', block.i),
            (block.j.is_some(), used.j, 'J', block.j),
            (block.k.is_some(), used.k, 'K', block.k),
            (block.r.is_some(), used.r, 'R', block.r),
            (block.p.is_some(), used.p, 'P', block.p),
            (block.d.is_some(), used.d, 'D', block.d),
            (block.q.is_some(), used.q, 'Q', block.q),
            (block.l.is_some(), used.l, 'L', block.l),
        ] {
            if present
                && !consumed
                && let Some(word) = word
            {
                errors.push(self.unused(letter, word.column));
            }
        }
        errors
    }

    /// Builds one unused-word error.
    fn unused(&self, letter: char, column: usize) -> ParseError {
        ParseError::new(
            self.line,
            column,
            ParseErrorKind::UnusedWord,
            format!("'{letter}' is not used by this block's command"),
        )
    }

    /// Builds one error.
    fn error(
        &self,
        column: usize,
        kind: ParseErrorKind,
        message: impl Into<String>,
    ) -> Vec<ParseError> {
        vec![ParseError::new(self.line, column, kind, message)]
    }
}

/// Returns the X, Y, or Z word of a block.
fn axis_word(block: &Block, axis: usize) -> Option<Value> {
    match axis {
        0 => block.x,
        1 => block.y,
        _ => block.z,
    }
}

/// Returns the I, J, or K word of a block.
fn axis_word_from_center(block: &Block, axis: usize) -> Option<Value> {
    match axis {
        0 => block.i,
        1 => block.j,
        _ => block.k,
    }
}

/// Converts an axis index into the domain enum without panicking.
fn axis_from_index(index: usize) -> Axis {
    match index {
        0 => Axis::X,
        1 => Axis::Y,
        _ => Axis::Z,
    }
}

/// Parses a non-negative integer word.
fn as_index(value: f64, maximum: u32) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(maximum) {
        return None;
    }
    let rounded = math::round(value);
    if math::abs(value - rounded) > 1.0e-9 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(rounded as u32)
}

/// Formats a code value without a trailing `.0`.
fn trim_number(value: f64) -> String {
    format!("{value}")
}

/// Column of the first word, used when a command-level error has no better
/// word to point at.
fn first_column(block: &Block) -> usize {
    block
        .g
        .first()
        .map(|word| word.column)
        .or_else(|| block.m.first().map(|word| word.column))
        .or_else(|| block.x.map(|word| word.column))
        .or_else(|| block.y.map(|word| word.column))
        .or_else(|| block.z.map(|word| word.column))
        .unwrap_or(1)
}

/// Euclidean distance between two 3D points.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    math::sqrt(dx * dx + dy * dy + dz * dz)
}

/// Distance between two 2D points.
fn uv_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    math::sqrt(dx * dx + dy * dy)
}

/// Solves the center of an arc from its endpoints and signed radius.
///
/// Positive `radius` selects the minor (at most 180 degree) arc; negative
/// selects the major arc. Returns `None` for a zero-length chord, a zero
/// radius, or a radius smaller than half the chord.
fn arc_center_from_radius(
    start: [f64; 2],
    end: [f64; 2],
    radius: f64,
    clockwise: bool,
) -> Option<[f64; 2]> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let chord = math::sqrt(dx * dx + dy * dy);
    let radius_abs = math::abs(radius);
    if !chord.is_finite()
        || chord <= 0.0
        || !radius_abs.is_finite()
        || radius_abs < MIN_ARC_RADIUS_MM
        || chord > 2.0 * radius_abs + EPSILON_MM
    {
        return None;
    }
    let half_chord = chord * 0.5;
    let height_squared = radius_abs * radius_abs - half_chord * half_chord;
    let height = math::sqrt(height_squared.max(0.0));
    let midpoint = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
    let inverse_chord = 1.0 / chord;
    let left = [-dy * inverse_chord, dx * inverse_chord];
    let sign = if (!clockwise) == (radius > 0.0) {
        1.0
    } else {
        -1.0
    };
    Some([
        midpoint[0] + left[0] * height * sign,
        midpoint[1] + left[1] * height * sign,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1.0e-6 && (a[1] - b[1]).abs() < 1.0e-6
    }

    #[test]
    fn radius_form_centers_match_direction_and_major_flag() {
        // Counter-clockwise minor arc through the origin center.
        let center = arc_center_from_radius([1.0, 0.0], [0.0, 1.0], 1.0, false).expect("center");
        assert!(close(center, [0.0, 0.0]));

        // Counter-clockwise major arc through the far center.
        let center = arc_center_from_radius([1.0, 0.0], [0.0, 1.0], -1.0, false).expect("center");
        assert!(close(center, [1.0, 1.0]));

        // Clockwise minor arc through the far center.
        let center = arc_center_from_radius([1.0, 0.0], [0.0, 1.0], 1.0, true).expect("center");
        assert!(close(center, [1.0, 1.0]));

        // Clockwise major arc through the origin center.
        let center = arc_center_from_radius([1.0, 0.0], [0.0, 1.0], -1.0, true).expect("center");
        assert!(close(center, [0.0, 0.0]));
    }

    #[test]
    fn radius_form_rejects_impossible_geometry() {
        assert!(arc_center_from_radius([0.0, 0.0], [0.0, 0.0], 1.0, false).is_none());
        assert!(arc_center_from_radius([0.0, 0.0], [10.0, 0.0], 4.0, false).is_none());
        assert!(arc_center_from_radius([0.0, 0.0], [2.0, 0.0], 0.0, false).is_none());
    }

    #[test]
    fn frame_round_trips_with_scale_and_rotation() {
        let state = ParserState {
            rotation: Some(crate::state::Rotation {
                angle_rad: 0.5,
                pivot_mm: [1.0, 2.0],
            }),
            scaling: Some(Scaling {
                factor: 2.0,
                pivot_mm: [3.0, 4.0, 5.0],
            }),
            ..ParserState::default()
        };
        let offsets = [Position::xyz(10.0, 20.0, 30.0); 9];
        let frame = Frame::new(&state, &offsets);
        let point = [7.5, -3.25, 12.0];
        let machine = frame.to_machine(point);
        let back = frame.to_work(machine);
        assert!((back[0] - point[0]).abs() < 1.0e-9);
        assert!((back[1] - point[1]).abs() < 1.0e-9);
        assert!((back[2] - point[2]).abs() < 1.0e-9);
    }
}
