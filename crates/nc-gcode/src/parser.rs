//! The stateful parser and its display-toolpath emitter.
//!
//! # Coordinates
//!
//! Program words are interpreted in the active [`Units`](nc_core::Units) and
//! converted to millimetres. The emitted [`Toolpath`](nc_viewer::Toolpath) is
//! in **machine/display millimetres**:
//!
//! 1. `G51` scaling and caller-supplied rotation are applied about their pivots.
//! 2. The active work offset (`G54`..`G59.3`), `G92`, and `G43.1` tool-length
//!    offset are added.
//!
//! With the default configuration (all offsets zero) the toolpath is identical
//! to the program coordinates. `G53` moves and `G28`/`G30` home moves target
//! machine coordinates directly.
//!
//! # Not applied to the display path
//!
//! - Stored `G28`/`G30` positions unless supplied in [`ParserConfig`].
//! - Cutter radius compensation offset geometry. `G41`/`G42` are recorded,
//!   but the toolpath stays on the programmed centerline.
//! - Acceleration, rapid rate, and `G93` inverse-time timing.
//! - Firmware-specific transforms such as skew compensation and backlash.
//! - `G4`/`G82`/`G86`/`G89` dwell time (no geometry).
//! - Flow control (`IF`/`WHILE`/`GOTO`/`M98`); parameters and `[...]`
//!   expressions are evaluated, but they never emit geometry by themselves.
//!
//! # Parameter state
//!
//! The parser owns a bounded [`Parameters`] table shared by every line.
//! Assignment values are a signed decimal, a parameter reference, or a
//! bracketed expression: `#1=5`, `#1=#2`, and `#1=[2+3*4]` are accepted, while
//! compound arithmetic without brackets (`#1=2+3`) is a
//! [`MalformedExpression`](ParseErrorKind::MalformedExpression). A trailing
//! `*` after an unbracketed value is the line checksum, and a top-level `(`
//! starts a comment, so assignment grouping uses `[...]`.
//!
//! Assignments are evaluated in source order into an overlay and committed
//! only after the whole block validates. A block that fails while lexing,
//! parsing, or planning therefore leaves the table untouched. Use
//! [`Parser::parameters`], [`Parser::parameters_mut`], or
//! [`Parser::set_parameters`] to inspect or seed the table.

use nc_core::{Position, ToolHint, ToolShape, Units, math};
use nc_viewer::{AnnotationKind, MoveKind, Point3, Toolpath};

use crate::block::Block;
use crate::config::ParserConfig;
use crate::error::{ParseError, ParseErrorKind, ParseWarning, WarningKind};
use crate::expression::{ParameterOverlay, Parameters};
use crate::lexer::{LexedLine, lex_line_with_overlay};
use crate::motion::{
    Annotation, ArcFormat, ArcRecord, CannedRecord, Comment, LossyProgram, ModalId, ModalState,
    MotionRecord, SkippedBlock,
};
use crate::planner::{
    Frame, Plan, PlannedAnnotation, PlannedMove, PlannedRecord, Planner, arc_kind, canned_kind,
};
use crate::program::{Program, ProgramLine, ProgramStats};
use crate::state::{CannedCycleKind, ParserState, Plane};

/// G-code parser with accumulating program, toolpath, and statistics.
///
/// Use [`Parser::parse_line`] for MDI-style incremental input, or
/// [`parse_program`] for a whole file. A failed block leaves the modal state
/// untouched, matching the controller's atomic block execution.
#[derive(Debug, Clone)]
pub struct Parser {
    config: ParserConfig,
    state: ParserState,
    parameters: Parameters,
    offsets: [Position; 9],
    homes: [Option<[f64; 3]>; 2],
    machine: [f64; 3],
    toolpath: Toolpath,
    stats: ProgramStats,
    lines: Vec<ProgramLine>,
    warnings: Vec<ParseWarning>,
    motions: Vec<MotionRecord>,
    annotations: Vec<Annotation>,
    modal_table: Vec<ModalState>,
    units_declared: bool,
    distance_declared: bool,
    geometry_uncertain_after_line: Option<usize>,
    suppress_geometry: bool,
    first_motion_line: Option<usize>,
    allow_missing_feed: bool,
    canned_run: Option<(CannedCycleKind, usize)>,
    next_line: usize,
    ended: bool,
    ended_warning_emitted: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// Creates a parser with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(ParserConfig::default())
    }

    /// Creates a parser with the given configuration.
    ///
    /// The parser starts at program position `(0, 0, 0)`, so the initial
    /// machine position is the active work offset.
    #[must_use]
    pub fn with_config(config: ParserConfig) -> Self {
        let config = config.normalized();
        let state = ParserState::default();
        let offsets = config.offsets;
        let homes = [config.home_position, config.second_home_position];
        let machine = Frame::new(&state, &offsets).to_machine(state.position);
        Self {
            config,
            state,
            parameters: Parameters::new(),
            offsets,
            homes,
            machine,
            toolpath: Toolpath::new(),
            stats: ProgramStats::default(),
            lines: Vec::new(),
            warnings: Vec::new(),
            motions: Vec::new(),
            annotations: Vec::new(),
            modal_table: Vec::new(),
            units_declared: false,
            distance_declared: false,
            geometry_uncertain_after_line: None,
            suppress_geometry: false,
            first_motion_line: None,
            allow_missing_feed: false,
            canned_run: None,
            next_line: 1,
            ended: false,
            ended_warning_emitted: false,
        }
    }

    /// Returns the configuration in effect.
    #[must_use]
    pub fn config(&self) -> &ParserConfig {
        &self.config
    }

    /// Returns the current modal state.
    #[must_use]
    pub fn state(&self) -> &ParserState {
        &self.state
    }

    /// Returns the current `#` parameter table.
    #[must_use]
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// Returns the `#` parameter table for direct mutation.
    pub fn parameters_mut(&mut self) -> &mut Parameters {
        &mut self.parameters
    }

    /// Replaces the `#` parameter table.
    pub fn set_parameters(&mut self, parameters: Parameters) {
        self.parameters = parameters;
    }

    /// Clears every `#` parameter.
    pub fn clear_parameters(&mut self) {
        self.parameters.clear();
    }

    /// Replaces the modal state, for example from a controller's `$G` report.
    ///
    /// The current machine position is kept; the program position is derived
    /// from it through the new state's offsets and transforms.
    pub fn set_state(&mut self, state: ParserState) {
        self.state = state;
        self.units_declared = true;
        self.distance_declared = true;
        self.machine = Frame::new(&self.state, &self.offsets).to_machine(self.state.position);
    }

    /// Returns the current work-offset table in `G54`..`G59.3` order.
    #[must_use]
    pub fn offsets(&self) -> &[Position; 9] {
        &self.offsets
    }

    /// Replaces the work-offset table and re-derives the program position.
    pub fn set_offsets(&mut self, offsets: [Position; 9]) {
        self.offsets = offsets;
        self.state.position = Frame::new(&self.state, &self.offsets).to_work(self.machine);
    }

    /// Returns the stored `G28`/`G30` positions in machine coordinates.
    #[must_use]
    pub fn homes(&self) -> &[Option<[f64; 3]>; 2] {
        &self.homes
    }

    /// Returns the current machine/display position in millimetres.
    #[must_use]
    pub fn machine_position(&self) -> [f64; 3] {
        self.machine
    }

    /// Moves the tracked machine position without emitting geometry.
    ///
    /// Intended for start-from-line and after a controller reconnect.
    pub fn set_machine_position(&mut self, machine: [f64; 3]) {
        self.machine = machine;
        self.state.position = Frame::new(&self.state, &self.offsets).to_work(machine);
    }

    /// Returns the toolpath generated so far.
    #[must_use]
    pub fn toolpath(&self) -> &Toolpath {
        &self.toolpath
    }

    /// Returns the warnings collected so far.
    #[must_use]
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }

    /// Parses one line, using the parser's next source line number.
    ///
    /// # Errors
    ///
    /// Returns every diagnostic for the line. The block is not applied.
    pub fn parse_line(&mut self, source: &str) -> Result<ProgramLine, Vec<ParseError>> {
        let line = self.next_line;
        self.next_line = self.next_line.saturating_add(1);
        self.parse_line_at(line, source)
    }

    /// Parses one line with an explicit 1-based source line number.
    ///
    /// The line number is used in diagnostics and stored in the returned
    /// [`ProgramLine`]. Parameter assignments are committed only when the
    /// whole block validates.
    ///
    /// # Errors
    ///
    /// Returns every diagnostic for the line. The block is not applied.
    pub fn parse_line_at(
        &mut self,
        line: usize,
        source: &str,
    ) -> Result<ProgramLine, Vec<ParseError>> {
        if line >= self.next_line {
            self.next_line = line.saturating_add(1);
        }
        let source = source.strip_suffix('\r').unwrap_or(source);
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        if source.len() > self.config.max_line_bytes {
            return Err(vec![ParseError::new(
                line,
                1,
                ParseErrorKind::LineTooLong,
                format!(
                    "line is {} bytes; the limit is {}",
                    source.len(),
                    self.config.max_line_bytes
                ),
            )]);
        }
        let mut overlay = ParameterOverlay::default();
        let lexed = lex_line_with_overlay(
            line,
            source,
            self.config.max_words,
            &self.parameters,
            &mut overlay,
        )?;
        let program_line = self.parse_lexed(line, source, lexed)?;
        // Commit staged assignments only after the whole block validated.
        overlay.commit(&mut self.parameters);
        Ok(program_line)
    }

    /// Applies an already-lexed line.
    ///
    /// This is an extension point for callers that preprocess or rewrite a
    /// [`LexedLine`] before parsing. Word values are expected to be already
    /// evaluated, so this method does not read or change the `#` parameter
    /// table; use [`Parser::parse_line`] or [`Parser::parse_line_at`] when a
    /// line contains assignments or unresolved expressions.
    ///
    /// # Errors
    ///
    /// Returns every diagnostic for the line. The block is not applied.
    pub fn parse_lexed(
        &mut self,
        line: usize,
        source: &str,
        lexed: LexedLine,
    ) -> Result<ProgramLine, Vec<ParseError>> {
        if line >= self.next_line {
            self.next_line = line.saturating_add(1);
        }
        if source.len() > self.config.max_line_bytes {
            return Err(vec![ParseError::new(
                line,
                1,
                ParseErrorKind::LineTooLong,
                format!(
                    "line is {} bytes; the limit is {}",
                    source.len(),
                    self.config.max_line_bytes
                ),
            )]);
        }
        if lexed.words.len() > self.config.max_words {
            return Err(vec![ParseError::new(
                line,
                1,
                ParseErrorKind::TooManyWords,
                format!(
                    "{} words; the limit is {}",
                    lexed.words.len(),
                    self.config.max_words
                ),
            )]);
        }

        let block = Block::parse(line, &lexed)?;
        let number = block.number;
        let planner = Planner {
            state: self.state,
            offsets: self.offsets,
            homes: self.homes,
            machine: self.machine,
            config: &self.config,
            allow_missing_feed: self.allow_missing_feed,
            line,
        };
        let plan = planner.plan(&block)?;

        let mut program_line = ProgramLine {
            line,
            number,
            source: source.to_string(),
            words: lexed.words,
            comments: lexed.comments,
            block_delete: lexed.block_delete,
            checksum: lexed.checksum,
            segments: 0..0,
        };

        if self.ended && !plan.moves.is_empty() && !self.ended_warning_emitted {
            self.ended_warning_emitted = true;
            self.warnings.push(ParseWarning::new(
                line,
                WarningKind::MotionAfterProgramEnd,
                warning_message(WarningKind::MotionAfterProgramEnd),
            ));
        }
        for kind in &plan.warnings {
            self.warnings
                .push(ParseWarning::new(line, *kind, warning_message(*kind)));
        }

        let start = self.toolpath.len();
        self.execute(line, plan);
        program_line.segments = start..self.toolpath.len();

        if program_line.has_words() {
            self.stats.blocks += 1;
        }
        self.stats.words += program_line.words.len();
        self.lines.push(program_line.clone());
        Ok(program_line)
    }

    /// Finishes the program and returns its lines, geometry, and statistics.
    #[must_use]
    pub fn finish(mut self) -> Program {
        self.stats.lines = self.lines.len();
        self.stats.segments = self.toolpath.len();
        let bounds = self.toolpath.bounds();
        let comments = self
            .lines
            .iter()
            .flat_map(|line| {
                line.comments.iter().map(|text| Comment {
                    line: line.line,
                    text: text.clone(),
                })
            })
            .collect::<Vec<_>>();
        let (tool_hints, hint_warnings) =
            extract_tool_hints(&self.lines, self.first_motion_line, &self.config);
        self.warnings.extend(hint_warnings);
        if self.modal_table.is_empty() {
            self.modal_table.push(ModalState::UNKNOWN);
        }
        Program {
            lines: self.lines,
            toolpath: self.toolpath,
            bounds,
            stats: self.stats,
            state: self.state,
            parameters: self.parameters,
            offsets: self.offsets,
            warnings: self.warnings,
            motions: self.motions,
            annotations: self.annotations,
            comments,
            tool_hints,
            skipped_blocks: Vec::new(),
            modal_table: self.modal_table,
            geometry_uncertain_after_line: self.geometry_uncertain_after_line,
        }
    }

    /// Applies a validated plan and emits its display geometry and records.
    fn execute(&mut self, line: usize, plan: Plan) {
        if self.first_motion_line.is_none() && (!plan.moves.is_empty() || !plan.records.is_empty())
        {
            self.first_motion_line = Some(line);
        }
        let Plan {
            state,
            offsets,
            homes,
            moves,
            records,
            annotations,
            warnings: _,
            arc_moves,
            canned_cycles,
            tool_changes,
            program_end,
            feed_observed,
            spindle_observed,
            selected_tool,
            explicit_units,
            explicit_distance,
            canned_selected,
        } = plan;
        let frame = Frame::new(&state, &offsets);
        let mut machine = self.machine;
        let emit = !self.suppress_geometry;
        self.units_declared |= explicit_units;
        self.distance_declared |= explicit_distance;

        if emit {
            for planned in &moves {
                match *planned {
                    PlannedMove::Line { to_work, kind } => {
                        let to_machine = frame.to_machine(to_work);
                        self.emit_line(machine, to_machine, kind, state.tool);
                        machine = to_machine;
                    }
                    PlannedMove::MachineLine { to_machine, kind } => {
                        self.emit_line(machine, to_machine, kind, state.tool);
                        machine = to_machine;
                    }
                    PlannedMove::Arc {
                        to_work,
                        center_work,
                        plane,
                        clockwise,
                    } => {
                        let end_machine = frame.to_machine(to_work);
                        let center_machine = frame.to_machine(center_work);
                        self.emit_arc(
                            machine,
                            end_machine,
                            center_machine,
                            plane,
                            clockwise,
                            state.tool,
                        );
                        machine = end_machine;
                    }
                }
            }

            if !records.is_empty() {
                let modal = self.intern_modal(ModalState {
                    units: self.units_declared.then_some(state.units),
                    distance: self.distance_declared.then_some(state.distance),
                    feed_mode: state.feed_mode,
                    wcs: state.wcs,
                    coolant: state.coolant,
                    cutter_comp: state.cutter_comp,
                    g92_active: state.g92 != [0.0; 3],
                    retract_mode: state.retract,
                });
                let feed = state.feed_mm_per_min();
                let spindle_rpm = (state.spindle_rpm > 0.0).then_some(state.spindle_rpm);
                let spindle_cw = match state.spindle {
                    crate::state::SpindleMode::Clockwise => Some(true),
                    crate::state::SpindleMode::CounterClockwise => Some(false),
                    crate::state::SpindleMode::Off => None,
                };
                let mut record_machine = self.machine;
                if canned_selected {
                    self.canned_run = None;
                }
                for planned in &records {
                    match *planned {
                        PlannedRecord::Line { to_work, kind } => {
                            let end = frame.to_machine(to_work);
                            if distance(record_machine, end) > 1.0e-9 {
                                self.push_motion(
                                    line,
                                    kind,
                                    record_machine,
                                    end,
                                    feed,
                                    spindle_rpm,
                                    spindle_cw,
                                    state.tool,
                                    state.plane,
                                    None,
                                    None,
                                    modal,
                                );
                            }
                            record_machine = end;
                            self.canned_run = None;
                        }
                        PlannedRecord::MachineLine { to_machine, kind } => {
                            if distance(record_machine, to_machine) > 1.0e-9 {
                                self.push_motion(
                                    line,
                                    kind,
                                    record_machine,
                                    to_machine,
                                    feed,
                                    spindle_rpm,
                                    spindle_cw,
                                    state.tool,
                                    state.plane,
                                    None,
                                    None,
                                    modal,
                                );
                            }
                            record_machine = to_machine;
                            self.canned_run = None;
                        }
                        PlannedRecord::Arc {
                            to_work,
                            center_work,
                            plane,
                            clockwise,
                            format,
                        } => {
                            let end = frame.to_machine(to_work);
                            let center = frame.to_machine(center_work);
                            let arc =
                                arc_record(record_machine, end, center, plane, clockwise, format);
                            let kind = arc_kind(clockwise, arc.axial_delta_mm);
                            self.push_motion(
                                line,
                                kind,
                                record_machine,
                                end,
                                feed,
                                spindle_rpm,
                                spindle_cw,
                                state.tool,
                                plane,
                                Some(Box::new(arc)),
                                None,
                                modal,
                            );
                            record_machine = end;
                            self.canned_run = None;
                        }
                        PlannedRecord::Canned { hole } => {
                            let end = frame.to_machine(hole.end_work);
                            let cycle = hole.cycle;
                            let hole_index = match self.canned_run {
                                Some((active, index)) if active == cycle => index.saturating_add(1),
                                _ => 0,
                            };
                            self.canned_run = Some((cycle, hole_index));
                            let r_plane_mm =
                                frame.to_machine([hole.xy_work[0], hole.xy_work[1], hole.r_mm])[2];
                            let bottom_z_mm =
                                frame.to_machine([hole.xy_work[0], hole.xy_work[1], hole.z_mm])[2];
                            let canned = CannedRecord {
                                cycle,
                                hole_index,
                                r_plane_mm,
                                bottom_z_mm,
                                peck_q_mm: hole.peck_q_mm,
                                dwell_s: hole.dwell_s,
                                retract_mode: hole.retract,
                            };
                            self.push_motion(
                                line,
                                canned_kind(cycle),
                                record_machine,
                                end,
                                feed,
                                spindle_rpm,
                                spindle_cw,
                                state.tool,
                                Plane::Xy,
                                None,
                                Some(Box::new(canned)),
                                modal,
                            );
                            record_machine = end;
                        }
                    }
                }
                machine = record_machine;
            }

            let next_motion_index = self.motions.len();
            for annotation in &annotations {
                self.annotations.push(Annotation {
                    line,
                    next_motion_index,
                    pos_mm: machine,
                    kind: annotation_kind(annotation),
                });
            }
        } else {
            machine = frame.to_machine(state.position);
        }

        self.machine = machine;
        self.state = state;
        self.offsets = offsets;
        self.homes = homes;
        self.stats.arc_moves += arc_moves;
        self.stats.canned_cycles += canned_cycles;
        self.stats.tool_changes += tool_changes;
        if let Some(feed) = feed_observed {
            self.stats.observe_feed(feed);
        }
        if let Some(rpm) = spindle_observed {
            self.stats.max_spindle_rpm = self.stats.max_spindle_rpm.max(rpm);
        }
        if let Some(tool) = selected_tool {
            self.stats.observe_tool(tool);
        }
        if program_end {
            self.ended = true;
        }
    }

    /// Pushes one motion record and assigns its program index.
    #[allow(clippy::too_many_arguments)]
    fn push_motion(
        &mut self,
        line: usize,
        kind: MoveKind,
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        feed_mm_min: Option<f64>,
        spindle_rpm: Option<f64>,
        spindle_cw: Option<bool>,
        tool: u32,
        plane: Plane,
        arc: Option<Box<ArcRecord>>,
        canned: Option<Box<CannedRecord>>,
        modal: ModalId,
    ) {
        self.motions.push(MotionRecord {
            line,
            index: self.motions.len(),
            kind,
            start_mm,
            end_mm,
            feed_mm_min,
            spindle_rpm,
            spindle_cw,
            tool,
            plane,
            arc,
            canned,
            modal,
        });
    }

    /// Interns a modal state, returning an existing id for an identical state.
    fn intern_modal(&mut self, state: ModalState) -> ModalId {
        if let Some(index) = self.modal_table.iter().position(|known| *known == state) {
            return ModalId(u32::try_from(index).unwrap_or(u32::MAX));
        }
        let id = ModalId(u32::try_from(self.modal_table.len()).unwrap_or(u32::MAX));
        self.modal_table.push(state);
        id
    }

    /// Stops new geometry at `line` and records the first untrusted line.
    fn mark_geometry_uncertain(&mut self, line: usize) {
        if self.geometry_uncertain_after_line.is_none() {
            self.geometry_uncertain_after_line = Some(line);
        }
        self.suppress_geometry = true;
    }

    /// Records a source line whose block failed in lossy mode.
    ///
    /// Words are deliberately not recovered: the block was not applied, so its
    /// `G20`/`G21`, `T`, or motion words must not influence modal state, tool
    /// hints, or line mapping. Comments are retained.
    fn record_unparsed_line(&mut self, line: usize, source: &str) {
        let comments = scan_comments(source);
        self.lines.push(ProgramLine {
            line,
            number: None,
            source: source.to_string(),
            words: Vec::new(),
            comments,
            block_delete: source.trim_start().starts_with('/'),
            checksum: None,
            segments: 0..0,
        });
    }

    /// Emits one straight segment and accounts for it.
    fn emit_line(
        &mut self,
        from_machine: [f64; 3],
        to_machine: [f64; 3],
        kind: MoveKind,
        tool: u32,
    ) {
        if distance(from_machine, to_machine) <= 1.0e-9 {
            return;
        }
        let from = point(from_machine);
        let to = point(to_machine);
        let length = f64::from(from.distance(to));
        self.toolpath
            .append_line(from, to, kind, 0, display_tool(tool));
        self.account_segment(kind, length, tool);
    }

    /// Emits an arc as chords, splitting at cardinal angles.
    ///
    /// The viewer's [`Arc`] helpers place tessellated points relative to the
    /// plane origin instead of the arc center, so this layer generates the
    /// chords itself. Including every cardinal angle as a vertex means the
    /// viewer's endpoint-based bounds still contain the exact circle extents.
    /// Helical arcs carry an interpolated normal coordinate and therefore need
    /// no planar slicing.
    fn emit_arc(
        &mut self,
        start: [f64; 3],
        end: [f64; 3],
        center: [f64; 3],
        plane: crate::state::Plane,
        clockwise: bool,
        tool: u32,
    ) {
        let (u, v) = plane.axes();
        let normal = plane.normal_axis();
        let kind = arc_kind(clockwise, end[normal] - start[normal]);
        let center_uv = [center[u], center[v]];
        let start_uv = [start[u], start[v]];
        let end_uv = [end[u], end[v]];
        let radius = uv_distance(start_uv, center_uv);
        let start_angle = math::atan2(start_uv[1] - center_uv[1], start_uv[0] - center_uv[0]);
        let end_angle = math::atan2(end_uv[1] - center_uv[1], end_uv[0] - center_uv[0]);
        if !radius.is_finite() || radius <= 0.0 {
            tracing::warn!("arc has no usable radius; drawing a straight move");
            self.emit_line(start, end, MoveKind::Feed, tool);
            return;
        }
        let sweep = directed_sweep(start_angle, end_angle, clockwise);
        let total = math::abs(sweep);
        let tolerance = self.config.chord_tolerance_mm.max(1.0e-6);
        let ratio = (tolerance / radius).clamp(0.0, 1.0);
        let step = math::clamp(2.0 * math::acos(1.0 - ratio), 1.0e-9, math::PI / 3.0);

        // Split at the plane's cardinal angles so the extrema are vertices.
        let mut fractions = vec![0.0_f64, 1.0_f64];
        for cardinal in [0.0, math::FRAC_PI_2, math::PI, math::PI * 1.5] {
            let traveled = if clockwise {
                start_angle - cardinal
            } else {
                cardinal - start_angle
            }
            .rem_euclid(math::TAU);
            if traveled > 1.0e-9 && traveled < total - 1.0e-9 {
                fractions.push(traveled / total);
            }
        }
        fractions.sort_by(f64::total_cmp);
        fractions.dedup_by(|first, second| math::abs(*first - *second) <= 1.0e-12);

        let normal_start = start[normal];
        let normal_end = end[normal];
        let mut previous = start;
        let mut count = 0usize;
        for interval in fractions.windows(2) {
            let (t0, t1) = (interval[0], interval[1]);
            let sub_sweep = total * (t1 - t0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let chords = math::clamp(math::ceil(sub_sweep / step), 1.0, 65_536.0) as usize;
            for chord in 1..=chords {
                #[allow(clippy::cast_precision_loss)]
                let local = chord as f64 / chords as f64;
                let t = t0 + (t1 - t0) * local;
                let angle = start_angle + sweep * t;
                let uv = [
                    center_uv[0] + radius * math::cos(angle),
                    center_uv[1] + radius * math::sin(angle),
                ];
                let mut next = [0.0; 3];
                next[u] = uv[0];
                next[v] = uv[1];
                next[normal] = normal_start + (normal_end - normal_start) * t;
                if t >= 1.0 - 1.0e-12 {
                    next = end;
                }
                if distance(previous, next) <= 1.0e-12 {
                    continue;
                }
                self.toolpath.append_line(
                    point(previous),
                    point(next),
                    kind,
                    0,
                    display_tool(tool),
                );
                previous = next;
                count += 1;
            }
        }
        if distance(previous, end) > 1.0e-9 {
            self.toolpath
                .append_line(point(previous), point(end), kind, 0, display_tool(tool));
            count += 1;
        }
        let planar_length = radius * total;
        let length = math::hypot(planar_length, normal_end - normal_start);
        self.account_segments(kind, count, length, tool);
    }

    /// Accounts for one segment's kind and length.
    fn account_segment(&mut self, kind: MoveKind, length_mm: f64, tool: u32) {
        self.account_segments(kind, 1, length_mm, tool);
    }

    /// Accounts for `count` segments of `kind` with a total exact length.
    fn account_segments(&mut self, kind: MoveKind, count: usize, length_mm: f64, tool: u32) {
        if count == 0 {
            return;
        }
        match kind {
            MoveKind::Rapid | MoveKind::Retract => {
                self.stats.rapid_segments += count;
                self.stats.rapid_length_mm += length_mm;
            }
            MoveKind::Feed | MoveKind::Ramp => {
                self.stats.feed_segments += count;
                self.stats.cutting_length_mm += length_mm;
            }
            MoveKind::Plunge | MoveKind::Drill | MoveKind::Peck | MoveKind::Bore => {
                self.stats.plunge_segments += count;
                self.stats.cutting_length_mm += length_mm;
            }
            MoveKind::ArcCw | MoveKind::ArcCcw | MoveKind::HelixCw | MoveKind::HelixCcw => {
                self.stats.arc_segments += count;
                self.stats.cutting_length_mm += length_mm;
            }
            MoveKind::Probe => {
                self.stats.probe_segments += count;
            }
        }
        self.stats.path_length_mm += length_mm;
        self.stats.observe_tool(tool);
    }
}

/// Converts a `u32` tool number to the display toolpath's `u16` id.
fn display_tool(tool: u32) -> u16 {
    u16::try_from(tool).unwrap_or(u16::MAX)
}

/// Builds the exact arc record for one planned arc.
fn arc_record(
    start: [f64; 3],
    end: [f64; 3],
    center: [f64; 3],
    plane: Plane,
    clockwise: bool,
    format: ArcFormat,
) -> ArcRecord {
    let (u, v) = plane.axes();
    let normal = plane.normal_axis();
    let su = start[u] - center[u];
    let sv = start[v] - center[v];
    let eu = end[u] - center[u];
    let ev = end[v] - center[v];
    let radius = math::hypot(su, sv);
    let end_radius = math::hypot(eu, ev);
    let start_angle = math::atan2(sv, su);
    let end_angle = math::atan2(ev, eu);
    let mut delta = if clockwise {
        start_angle - end_angle
    } else {
        end_angle - start_angle
    };
    delta = delta.rem_euclid(math::TAU);
    if !delta.is_finite() || delta <= 1e-9 {
        delta = math::TAU;
    }
    ArcRecord {
        center_mm: center,
        radius_mm: radius,
        end_radius_mm: end_radius,
        start_angle_rad: start_angle,
        sweep_rad: if clockwise { -delta } else { delta },
        axial_delta_mm: end[normal] - start[normal],
        format,
    }
}

/// Converts a planned annotation into the viewer-facing annotation.
fn annotation_kind(annotation: &PlannedAnnotation) -> AnnotationKind {
    match *annotation {
        PlannedAnnotation::Dwell { seconds } => AnnotationKind::Dwell { seconds },
        PlannedAnnotation::ToolChange { tool } => AnnotationKind::ToolChange { tool },
        PlannedAnnotation::Spindle { on, rpm, clockwise } => {
            AnnotationKind::Spindle { on, rpm, clockwise }
        }
        PlannedAnnotation::Coolant { on } => AnnotationKind::Coolant { on },
    }
}

/// Parses a whole program with the default configuration.
///
/// # Errors
///
/// Returns every collected [`ParseError`], sorted by line and column, when any
/// line fails. A program with errors produces no [`Program`].
pub fn parse_program(text: &str) -> Result<Program, Vec<ParseError>> {
    parse_program_with(text, &ParserConfig::default())
}

/// Parses a whole program with explicit configuration.
///
/// # Errors
///
/// Returns every collected [`ParseError`], sorted by line and column, when any
/// line fails. Parsing stops after
/// [`ParserConfig::max_errors`] diagnostics.
pub fn parse_program_with(text: &str, config: &ParserConfig) -> Result<Program, Vec<ParseError>> {
    let config = config.clone().normalized();
    if text.len() > config.max_file_bytes {
        return Err(vec![ParseError::new(
            1,
            1,
            ParseErrorKind::FileTooLarge,
            format!(
                "file is {} bytes; the limit is {}",
                text.len(),
                config.max_file_bytes
            ),
        )]);
    }
    let mut parser = Parser::with_config(config);
    let max_errors = parser.config.max_errors;
    let mut errors = Vec::new();
    for (index, source) in SourceLines::new(text).enumerate() {
        let line = index.saturating_add(1);
        if let Err(mut line_errors) = parser.parse_line_at(line, source) {
            errors.append(&mut line_errors);
            if errors.len() >= max_errors {
                errors.push(ParseError::new(
                    line,
                    1,
                    ParseErrorKind::TooManyErrors,
                    format!("stopped after {max_errors} errors"),
                ));
                break;
            }
        }
    }
    errors.sort_by_key(|error| (error.line, error.column));
    if errors.is_empty() {
        Ok(parser.finish())
    } else {
        Err(errors)
    }
}

/// Parses a whole program without failing on individual blocks.
///
/// A failed block is recorded in [`LossyProgram::errors`] and
/// [`LossyProgram::skipped_blocks`], its modal state is left unchanged, and
/// parsing continues. When an unsupported state-changing construct appears
/// (`M98`/`M99`, macros, or flow control), no further geometry is emitted and
/// [`LossyProgram::trusted_geometry_until`] names the first untrusted line.
/// `G41`/`G42` do not fall in this class: they are recorded and geometry
/// continues on the programmed centerline.
#[must_use]
pub fn parse_program_lossy(text: &str, config: &ParserConfig) -> LossyProgram {
    let config = config.clone().normalized();
    let max_errors = config.max_errors;
    let mut errors = Vec::new();
    let mut skipped_blocks = Vec::new();

    if text.len() > config.max_file_bytes {
        errors.push(ParseError::new(
            1,
            1,
            ParseErrorKind::FileTooLarge,
            format!(
                "file is {} bytes; the limit is {}",
                text.len(),
                config.max_file_bytes
            ),
        ));
        skipped_blocks.push(SkippedBlock {
            line: 1,
            reason: ParseErrorKind::FileTooLarge.key().to_string(),
            source: String::new(),
        });
        let mut parser = Parser::with_config(config);
        parser.allow_missing_feed = true;
        parser.mark_geometry_uncertain(1);
        let mut program = parser.finish();
        program.skipped_blocks = skipped_blocks.clone();
        return LossyProgram {
            program,
            errors,
            skipped_blocks,
            trusted_geometry_until: Some(1),
        };
    }

    let mut parser = Parser::with_config(config);
    parser.allow_missing_feed = true;
    for (index, source) in SourceLines::new(text).enumerate() {
        let line = index.saturating_add(1);
        if let Err(mut line_errors) = parser.parse_line_at(line, source) {
            let stops_geometry = line_errors.iter().any(|error| {
                matches!(
                    error.kind,
                    ParseErrorKind::UnsupportedCode
                        | ParseErrorKind::UnsupportedWord
                        | ParseErrorKind::UnsupportedExpression
                )
            }) || has_macro_call(source);
            let reason = line_errors
                .first()
                .map(|error| error.kind.key())
                .unwrap_or("parse_error")
                .to_string();
            errors.append(&mut line_errors);
            skipped_blocks.push(SkippedBlock {
                line,
                reason,
                source: source.to_string(),
            });
            parser.record_unparsed_line(line, source);
            if stops_geometry {
                parser.mark_geometry_uncertain(line);
            }
            if errors.len() >= max_errors {
                errors.push(ParseError::new(
                    line,
                    1,
                    ParseErrorKind::TooManyErrors,
                    format!("stopped after {max_errors} errors"),
                ));
                parser.mark_geometry_uncertain(line);
                break;
            }
        }
    }
    let mut program = parser.finish();
    program.skipped_blocks = skipped_blocks.clone();
    errors.sort_by_key(|error| (error.line, error.column));
    let trusted_geometry_until = program.geometry_uncertain_after_line;
    LossyProgram {
        program,
        errors,
        skipped_blocks,
        trusted_geometry_until,
    }
}

/// Returns `true` when the line calls a G65 macro.
fn has_macro_call(source: &str) -> bool {
    let code = strip_comments(source).to_ascii_uppercase();
    code.split(|character: char| !character.is_ascii_alphanumeric() && character != '.')
        .any(|token| token == "G65")
}

/// Removes parenthesised and semicolon comments from a source line.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_paren = false;
    for character in source.chars() {
        match character {
            '(' => in_paren = true,
            ')' => in_paren = false,
            ';' if !in_paren => break,
            _ if !in_paren => out.push(character),
            _ => {}
        }
    }
    out
}

/// Extracts comment text from a line without failing on invalid words.
fn scan_comments(source: &str) -> Vec<String> {
    let mut comments = Vec::new();
    let mut current: Option<String> = None;
    let mut paren = false;
    for character in source.chars() {
        match character {
            '(' if current.is_none() => {
                current = Some(String::new());
                paren = true;
            }
            ';' if current.is_none() => {
                current = Some(String::new());
                paren = false;
            }
            ')' if paren => {
                if let Some(text) = current.take() {
                    comments.push(text);
                }
                paren = false;
            }
            _ => {
                if let Some(text) = &mut current {
                    text.push(character);
                }
            }
        }
    }
    if let Some(text) = current {
        comments.push(if paren { text } else { text.trim().to_string() });
    }
    comments
}

/// Human-readable text for a warning class.
fn warning_message(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::AssumedHomePosition => {
            "G28/G30 has no stored position; the machine origin was assumed"
        }
        WarningKind::MotionAfterProgramEnd => {
            "motion after M2/M30 will not execute on the controller"
        }
        WarningKind::EmptyCannedCycle => {
            "canned-cycle depth is above the R plane; no material is removed"
        }
        WarningKind::CutterCompNotApplied => {
            "G41/G42 recorded; the toolpath shows the programmed centerline, not an offset path"
        }
        WarningKind::AmbiguousToolHint => {
            "tool-comment dimension has no determinable unit; the hint was dropped"
        }
        WarningKind::MissingFeed => "cutting move has no feed rate; feed-based rules must skip it",
    }
}

/// Converts f64 machine coordinates into the viewer's f32 point.
fn point(value: [f64; 3]) -> Point3 {
    Point3::new(value[0] as f32, value[1] as f32, value[2] as f32)
}

/// Distance between two f64 points.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    math::sqrt(dx * dx + dy * dy + dz * dz)
}

/// Distance between two f64 points in a plane.
fn uv_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    math::sqrt(dx * dx + dy * dy)
}

/// Directed arc sweep from `start` to `end`, treating a zero sweep as a full
/// circle (the G-code full-circle convention).
fn directed_sweep(start: f64, end: f64, clockwise: bool) -> f64 {
    let mut delta = if clockwise { start - end } else { end - start };
    delta = delta.rem_euclid(math::TAU);
    if !delta.is_finite() {
        return 0.0;
    }
    if delta <= 1.0e-9 {
        delta = math::TAU;
    }
    if clockwise { -delta } else { delta }
}

/// Extracts conservative tool hints from retained comments.
fn extract_tool_hints(
    lines: &[ProgramLine],
    first_motion_line: Option<usize>,
    config: &ParserConfig,
) -> (Vec<ToolHint>, Vec<ParseWarning>) {
    let first_motion_line = first_motion_line.unwrap_or(usize::MAX);
    let tool_lines: Vec<(usize, u32)> = lines
        .iter()
        .filter_map(|line| {
            line.words
                .iter()
                .rev()
                .find(|word| word.letter == 'T')
                .and_then(|word| tool_number(word.value))
                .map(|tool| (line.line, tool))
        })
        .collect();
    let anchors: Vec<usize> = lines
        .iter()
        .filter(|line| {
            line.words.iter().any(|word| word.letter == 'T')
                || line
                    .words
                    .iter()
                    .any(|word| word.letter == 'M' && math::round(word.value) == 6.0)
        })
        .map(|line| line.line)
        .collect();
    // Header hints may use a unit declared later in the header (D017), but
    // never a unit declared after the first motion.
    let declarations = unit_declarations(lines);
    let header_units = declarations
        .iter()
        .rev()
        .find(|(declaration_line, _)| *declaration_line < first_motion_line)
        .map(|(_, units)| *units);

    // Contextual hints (a tool named in the comment, or a comment tied to a
    // tool by its line) are applied before unanchored header hints so a more
    // specific comment wins per field even when it appears later in the file.
    let mut anchored = Vec::new();
    let mut unanchored = Vec::new();
    let mut warnings = Vec::new();
    for line in lines {
        for text in &line.comments {
            let header = line.line < first_motion_line;
            let near_tool = within_window(&anchors, line.line, config.tool_hint_window);
            if !header && !near_tool {
                continue;
            }
            let (tool, specific) = match tool_in_comment(text) {
                Some(tool) => (tool, true),
                None => {
                    let Some((tool, same_line)) =
                        contextual_tool(&tool_lines, line.line, first_motion_line, header)
                    else {
                        continue;
                    };
                    (tool, !header || same_line)
                }
            };
            let declared = if header {
                header_units
            } else {
                declarations
                    .iter()
                    .rev()
                    .find(|(declaration_line, _)| *declaration_line <= line.line)
                    .map(|(_, units)| *units)
            };
            let fields = parse_hint_fields(text, declared);
            if fields.ambiguous {
                warnings.push(ParseWarning::new(
                    line.line,
                    WarningKind::AmbiguousToolHint,
                    format!(
                        "parse.tool_hint_ambiguous: ambiguous tool-comment diameter: {}",
                        text.trim()
                    ),
                ));
                // The whole hint is dropped: guessing which number was meant
                // would be worse than reporting it.
                continue;
            }
            if fields.diameter_mm.is_none() && fields.shape.is_none() && fields.flutes.is_none() {
                continue;
            }
            let mut hint = ToolHint::new(tool, line.line);
            // An unanchored header note describes an operation ("clean pocket"),
            // not a tool name, so only an explicitly associated comment names
            // the tool. This also keeps resolved tool inventories stable when
            // the same fields are already user-configured.
            hint.name = if specific { fields.name } else { None };
            hint.shape = fields.shape;
            hint.diameter_mm = fields.diameter_mm;
            hint.flutes = fields.flutes;
            if specific {
                anchored.push(hint);
            } else {
                unanchored.push(hint);
            }
        }
    }
    anchored.extend(unanchored);
    (anchored, warnings)
}

/// Chooses the tool for a comment that does not name one.
///
/// In the header region (`line < first_motion_line`) the first `T` word at or
/// after the comment wins, so an unanchored header note attaches to the first
/// tool selected later in the header. If no later header `T` exists, the
/// nearest preceding header `T` is used. The bool is `true` when the `T` is on
/// the comment's own line.
fn contextual_tool(
    tool_lines: &[(usize, u32)],
    line: usize,
    first_motion_line: usize,
    header: bool,
) -> Option<(u32, bool)> {
    if header
        && let Some((tool_line, tool)) = tool_lines
            .iter()
            .find(|(tool_line, _)| *tool_line >= line && *tool_line < first_motion_line)
    {
        return Some((*tool, *tool_line == line));
    }
    let index = tool_lines.partition_point(|(tool_line, _)| *tool_line <= line);
    index
        .checked_sub(1)
        .and_then(|index| tool_lines.get(index))
        .map(|(_, tool)| (*tool, false))
}

/// One comment line's recognized tool fields.
#[derive(Debug, Default)]
struct HintFields {
    name: Option<String>,
    shape: Option<ToolShape>,
    diameter_mm: Option<f64>,
    flutes: Option<u32>,
    ambiguous: bool,
}

/// The unit of one diameter token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiameterUnit {
    Millimetres,
    Inches,
}

/// A diameter token found in a comment.
#[derive(Debug, Clone, Copy)]
struct DiameterToken {
    value: f64,
    unit: Option<DiameterUnit>,
}

/// Recognizes a conservative generic tool-comment hint.
fn parse_hint_fields(text: &str, declared: Option<Units>) -> HintFields {
    let mut fields = HintFields::default();
    let trimmed = text.trim();
    fields.name = (!trimmed.is_empty()
        && trimmed.len() <= 120
        && trimmed
            .chars()
            .any(|character| character.is_ascii_alphabetic()))
    .then(|| trimmed.to_string());
    fields.shape = find_shape(text);
    fields.flutes = find_flutes(text);
    if let Some(token) = find_diameter(text) {
        let unit = token.unit.or_else(|| {
            declared.map(|units| match units {
                Units::Mm => DiameterUnit::Millimetres,
                Units::Inch => DiameterUnit::Inches,
            })
        });
        match unit {
            Some(DiameterUnit::Millimetres) if token.value > 0.0 && token.value <= 1000.0 => {
                fields.diameter_mm = Some(token.value);
            }
            Some(DiameterUnit::Inches) if token.value > 0.0 && token.value <= 40.0 => {
                fields.diameter_mm = Some(Units::Inch.to_mm(token.value));
            }
            Some(_) => {}
            None => fields.ambiguous = true,
        }
    }
    fields
}

/// Returns the units declared by `G20`/`G21` at or before `line`.
/// Collects every `G20`/`G21` declaration in source order.
fn unit_declarations(lines: &[ProgramLine]) -> Vec<(usize, Units)> {
    let mut declarations = Vec::new();
    for source in lines {
        let mut units = None;
        for word in &source.words {
            if word.letter != 'G' {
                continue;
            }
            if number_is(word.value, 20.0) {
                units = Some(Units::Inch);
            } else if number_is(word.value, 21.0) {
                units = Some(Units::Mm);
            }
        }
        if let Some(units) = units {
            declarations.push((source.line, units));
        }
    }
    declarations
}

/// Returns `true` when `line` is within `window` lines of an anchor.
fn within_window(anchors: &[usize], line: usize, window: usize) -> bool {
    let index = anchors.partition_point(|anchor| *anchor < line);
    if let Some(previous) = index.checked_sub(1).and_then(|index| anchors.get(index))
        && line.saturating_sub(*previous) <= window
    {
        return true;
    }
    anchors
        .get(index)
        .is_some_and(|next| next.saturating_sub(line) <= window)
}

/// Parses a `T` value as a `u32` tool number.
fn tool_number(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }
    let rounded = math::round(value);
    if math::abs(value - rounded) > 1.0e-9 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(rounded as u32)
}

/// Compares a code-number word without exposing float comparison details.
fn number_is(value: f64, expected: f64) -> bool {
    value.is_finite() && math::abs(value - expected) <= 1.0e-9
}

/// Finds a tool number written in a comment (`T3`, `tool 3`).
fn tool_in_comment(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if at_boundary && bytes[index] == b't' {
            let mut cursor = index + 1;
            if lower[index..].starts_with("tool") {
                cursor = index + 4;
            }
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
            if cursor > start
                && bytes
                    .get(cursor)
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric())
                && let Ok(number) = lower[start..cursor].parse::<u32>()
            {
                return Some(number);
            }
        }
        index += 1;
    }
    None
}

/// Finds a recognized cutter shape word.
fn find_shape(text: &str) -> Option<ToolShape> {
    let lower = text.to_ascii_lowercase();
    let lower = lower.as_str();
    if contains_word(lower, "ball") || contains_phrase(lower, "ball-nose") {
        return Some(ToolShape::Ball);
    }
    if contains_word(lower, "bull") || contains_phrase(lower, "bull-nose") {
        return Some(ToolShape::Bull);
    }
    if contains_word(lower, "drill") || contains_word(lower, "drilling") {
        return Some(ToolShape::Drill);
    }
    if contains_phrase(lower, "v-bit")
        || contains_phrase(lower, "v bit")
        || contains_word(lower, "vbit")
        || contains_word(lower, "engrave")
        || contains_word(lower, "engraving")
    {
        return Some(ToolShape::VBit);
    }
    if contains_word(lower, "flat")
        || contains_word(lower, "endmill")
        || contains_phrase(lower, "end mill")
        || contains_word(lower, "square")
    {
        return Some(ToolShape::Flat);
    }
    None
}

/// Finds a flute count (`2F`, `3FL`, `3 flute`).
fn find_flutes(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index > start {
            let mut cursor = index;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let suffix_start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
                cursor += 1;
            }
            let suffix = &lower[suffix_start..cursor];
            let supported = matches!(suffix, "f" | "fl" | "flute" | "flutes");
            let boundary = bytes
                .get(cursor)
                .is_none_or(|byte| !byte.is_ascii_alphanumeric());
            let plausible = bytes
                .get(start.wrapping_sub(1))
                .is_none_or(|byte| *byte != b'.' && !byte.is_ascii_alphabetic());
            if supported
                && boundary
                && plausible
                && let Ok(count) = lower[start..index].parse::<u32>()
                && (1..=64).contains(&count)
            {
                return Some(count);
            }
        }
    }
    None
}

/// Finds the first diameter token in a comment.
fn find_diameter(text: &str) -> Option<DiameterToken> {
    let lower = text.to_ascii_lowercase();
    if let Some(token) = diameter_after_marker(&lower, "dia") {
        return Some(token);
    }
    if let Some(token) = diameter_after_marker(&lower, "d=") {
        return Some(token);
    }
    if let Some(token) = diameter_after_symbol(&lower) {
        return Some(token);
    }
    diameter_with_unit(&lower)
}

/// Parses a number after an explicit `DIA` or `D=` marker.
fn diameter_after_marker(text: &str, marker: &str) -> Option<DiameterToken> {
    let mut search = 0;
    while let Some(offset) = text[search..].find(marker) {
        let at = search + offset;
        if at > 0 && text.as_bytes()[at - 1].is_ascii_alphanumeric() {
            search = at + marker.len();
            continue;
        }
        let mut cursor = at + marker.len();
        while text
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if text.as_bytes().get(cursor) == Some(&b'=') {
            cursor += 1;
            while text
                .as_bytes()
                .get(cursor)
                .is_some_and(u8::is_ascii_whitespace)
            {
                cursor += 1;
            }
        }
        if let Some((value, end)) = parse_decimal(text, cursor) {
            return Some(DiameterToken {
                value,
                unit: unit_after(text, end),
            });
        }
        search = at + marker.len();
    }
    None
}

/// Parses a number after a diameter symbol.
fn diameter_after_symbol(text: &str) -> Option<DiameterToken> {
    let mut cursor = 0;
    for character in text.chars() {
        if matches!(character, 'ø' | '⌀' | 'Ø') {
            let offset = cursor + character.len_utf8();
            let mut number_at = offset;
            while text
                .as_bytes()
                .get(number_at)
                .is_some_and(u8::is_ascii_whitespace)
            {
                number_at += 1;
            }
            if let Some((value, end)) = parse_decimal(text, number_at) {
                return Some(DiameterToken {
                    value,
                    unit: unit_after(text, end),
                });
            }
        }
        cursor += character.len_utf8();
    }
    None
}

/// Finds a bare number carrying an explicit length unit.
fn diameter_with_unit(text: &str) -> Option<DiameterToken> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        if let Some((value, end)) = parse_decimal(text, start) {
            if let Some(unit) = unit_after(text, end) {
                return Some(DiameterToken {
                    value,
                    unit: Some(unit),
                });
            }
            if bytes.get(end) == Some(&b'/')
                && let Some((denominator, fraction_end)) = parse_decimal(text, end + 1)
            {
                if denominator > 0.0
                    && let Some(unit) = unit_after(text, fraction_end)
                {
                    return Some(DiameterToken {
                        value: value / denominator,
                        unit: Some(unit),
                    });
                }
                index = fraction_end;
                continue;
            }
            index = end;
        } else {
            index += 1;
        }
    }
    None
}

/// Parses an unsigned decimal number starting at `start`.
fn parse_decimal(text: &str, start: usize) -> Option<(f64, usize)> {
    let bytes = text.as_bytes();
    let mut cursor = start;
    let mut seen_digit = false;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
        seen_digit = true;
    }
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
            seen_digit = true;
        }
    }
    if !seen_digit {
        return None;
    }
    let value = text[start..cursor].parse::<f64>().ok()?;
    value.is_finite().then_some((value, cursor))
}

/// Detects an explicit unit immediately after a number.
fn unit_after(text: &str, end: usize) -> Option<DiameterUnit> {
    let bytes = text.as_bytes();
    let mut cursor = end;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if bytes.get(cursor) == Some(&b'"') {
        return Some(DiameterUnit::Inches);
    }
    let rest = &text[cursor..];
    if rest.starts_with("mm") {
        return Some(DiameterUnit::Millimetres);
    }
    if rest.starts_with("in")
        && !rest[2..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Some(DiameterUnit::Inches);
    }
    None
}

/// Returns `true` when `word` appears as a standalone token.
fn contains_word(text: &str, word: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| token == word)
}

/// Returns `true` when the phrase appears in the text.
fn contains_phrase(text: &str, phrase: &str) -> bool {
    text.contains(phrase)
}
struct SourceLines<'a> {
    rest: &'a str,
    done: bool,
}

impl<'a> SourceLines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            rest: text,
            done: text.is_empty(),
        }
    }
}

impl<'a> Iterator for SourceLines<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.done {
            return None;
        }
        let bytes = self.rest.as_bytes();
        let separator = bytes
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r');
        match separator {
            Some(index) => {
                let line = &self.rest[..index];
                let mut next = index + 1;
                let paired = matches!(
                    (bytes[index], bytes.get(index + 1)),
                    (b'\r', Some(b'\n')) | (b'\n', Some(b'\r'))
                );
                if paired {
                    next += 1;
                }
                self.rest = &self.rest[next..];
                if self.rest.is_empty() {
                    self.done = true;
                }
                Some(line)
            }
            None => {
                self.done = true;
                Some(self.rest)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_lines_accept_all_terminators() {
        let lines: Vec<&str> = SourceLines::new("a\nb\r\nc\rd").collect();
        assert_eq!(lines, vec!["a", "b", "c", "d"]);
        assert!(SourceLines::new("").next().is_none());
        let one: Vec<&str> = SourceLines::new("only").collect();
        assert_eq!(one, vec!["only"]);
        let mixed: Vec<&str> = SourceLines::new("a\n\rb").collect();
        assert_eq!(mixed, vec!["a", "b"]);
    }

    #[test]
    fn point_conversion_is_straightforward() {
        assert_eq!(point([1.0, 2.0, -3.0]), Point3::new(1.0, 2.0, -3.0));
    }
}
