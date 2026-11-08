//! `modal.*` rules that inspect the program as a whole: machine coordinates,
//! probing, `G92`, work-coordinate changes, program end, end state, and
//! block-delete lines.

use std::collections::BTreeMap;

use nc_core::Wcs;
use nc_gcode::{AnnotationKind, MotionRecord, MoveKind, Program, ProgramLine, SpindleMode};
use nc_stock::{InferenceBasis, StockSource};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, Severity};

use super::{WORD_EPSILON, diagnostic, insert_motion_evidence, line_has_word, word_matches};

/// Stable id of the machine-coordinate rule.
const MACHINE_ID: &str = "modal.machine_coords";

/// Stable id of the probing rule.
const PROBE_ID: &str = "modal.probe_in_job";

/// Stable id of the unrestored-`G92` rule.
const G92_ID: &str = "modal.g92_unrestored";

/// Stable id of the work-coordinate-change rule.
const WCS_ID: &str = "modal.wcs_change";

/// Stable id of the program-end rule.
const END_ID: &str = "modal.no_program_end";

/// Stable id of the end-state rule.
const STATE_ID: &str = "modal.end_state";

/// Stable id of the block-delete rule.
const BLOCK_DELETE_ID: &str = "modal.block_delete_unsafe";

/// Machine-coordinate G codes, including the `G28.1`/`G30.1` set-home forms.
const MACHINE_CODES: [f64; 5] = [28.0, 28.1, 30.0, 30.1, 53.0];

/// `G92` family codes: set, cancel, suspend, and restore.
const G92_CODES: [f64; 4] = [92.0, 92.1, 92.2, 92.3];

/// Spindle M codes (`M3`/`M4`/`M5`).
const SPINDLE_CODES: [f64; 3] = [3.0, 4.0, 5.0];

/// Returns the rules in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(MachineCoords),
        Box::new(ProbeInJob),
        Box::new(G92Unrestored),
        Box::new(WcsChange),
        Box::new(NoProgramEnd),
        Box::new(EndState),
        Box::new(BlockDeleteUnsafe),
    ]
}

/// Reports any `G53`/`G28`/`G30` machine-coordinate command.
struct MachineCoords;

impl Rule for MachineCoords {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(MACHINE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        for line in &model.program.lines {
            let Some(word) = line.words.iter().find(|word| {
                word.letter == 'G'
                    && MACHINE_CODES
                        .iter()
                        .any(|code| word_matches(word.value, *code))
            }) else {
                continue;
            };
            let mut evidence = BTreeMap::new();
            evidence.insert("raw".to_string(), EvidenceValue::Text(line.source.clone()));
            evidence.insert(
                "reason".to_string(),
                EvidenceValue::Text(format!("G{}", code_label(word.value))),
            );
            findings.push(diagnostic(
                MACHINE_ID,
                Severity::Warning,
                Basis::Exact,
                line.line,
                "program uses a machine-coordinate command (G53/G28/G30)",
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "G53/G28/G30 move in machine coordinates, ignoring the work offset. In \
         a job set on a fixture this drives to the machine reference instead of \
         the part, which can crash the tool or the fixture."
    }
}

/// Reports probing moves inside an ordinary milling job.
struct ProbeInJob;

impl Rule for ProbeInJob {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(PROBE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        for motion in &model.program.motions {
            if motion.kind != MoveKind::Probe {
                continue;
            }
            findings.push(probe_finding(motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A probe move in a cutting job expects a probe and a skip signal. With \
         a cutter installed it drives the tool into the part until the machine \
         faults, and the measured position is never used."
    }
}

/// Reports an unrestored `G92` coordinate offset at program end.
struct G92Unrestored;

impl Rule for G92Unrestored {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(G92_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if model.program.state.g92 == [0.0; 3] {
            return RuleOutcome::Findings(Vec::new());
        }
        let line = last_line_with_codes(model.program, 'G', &G92_CODES)
            .or_else(|| model.program.motions.last().map(|motion| motion.line))
            .unwrap_or(1);
        let mut evidence = BTreeMap::new();
        evidence.insert(
            "reason".to_string(),
            EvidenceValue::Text("G92 offset active at end".to_string()),
        );
        if let Some(source) = line_source(model.program, line) {
            evidence.insert("raw".to_string(), EvidenceValue::Text(source));
        }
        RuleOutcome::Findings(vec![diagnostic(
            G92_ID,
            Severity::Warning,
            Basis::Exact,
            line,
            "a G92 coordinate offset is still active at program end",
            evidence,
        )])
    }

    fn why_it_matters(&self) -> &'static str {
        "A G92 offset shifts every programmed coordinate until it is cancelled. \
         Left active, it silently offsets the next program or the next run and \
         cuts the part in the wrong place."
    }
}

/// Reports a work-coordinate change after cutting has started.
struct WcsChange;

impl Rule for WcsChange {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(WCS_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let Some(first_cut) = model
            .program
            .motions
            .iter()
            .position(|motion| motion.kind.is_cutting())
        else {
            return RuleOutcome::Findings(Vec::new());
        };

        let mut findings = Vec::new();
        let first = &model.program.motions[first_cut];
        let mut current = model.program.modal(first.modal).wcs;
        let mut previous_line = first.line;
        for motion in model.program.motions.iter().skip(first_cut + 1) {
            let wcs = model.program.modal(motion.modal).wcs;
            if wcs != current {
                let line = wcs_change_line(model.program, previous_line, motion.line, wcs)
                    .unwrap_or(motion.line);
                findings.push(wcs_finding(line, current, wcs));
                current = wcs;
            }
            previous_line = motion.line;
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Changing the work coordinate system mid-cut re-references every later \
         coordinate. The tool then moves to a position measured from a different \
         datum, which usually cuts through the part or the fixture."
    }
}

/// Reports a program without an `M2`/`M30` end block.
struct NoProgramEnd;

impl Rule for NoProgramEnd {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(END_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let has_end = model
            .program
            .lines
            .iter()
            .any(|line| line_has_word(line, 'M', 2.0) || line_has_word(line, 'M', 30.0));
        if has_end {
            return RuleOutcome::Findings(Vec::new());
        }
        let Some(last) = model.program.lines.last() else {
            return RuleOutcome::Findings(Vec::new());
        };
        let mut evidence = BTreeMap::new();
        evidence.insert("raw".to_string(), EvidenceValue::Text(last.source.clone()));
        RuleOutcome::Findings(vec![diagnostic(
            END_ID,
            Severity::Warning,
            Basis::Exact,
            last.line,
            "program has no M2/M30 end block",
            evidence,
        )])
    }

    fn why_it_matters(&self) -> &'static str {
        "M2/M30 stops the spindle, coolant, and motion and resets the controller. \
         Without it the machine can keep the last modal state and the spindle \
         running after the toolpath ends."
    }
}

/// Reports a spindle/coolant left on, or a tool left below safe Z, at the end.
struct EndState;

impl Rule for EndState {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(STATE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_stock(StockRequirement::InferredOk)
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        let last_motion_line = model
            .program
            .motions
            .last()
            .map(|motion| motion.line)
            .unwrap_or(1);

        if model.program.state.spindle != SpindleMode::Off {
            let line = last_spindle_on_line(model.program).unwrap_or(last_motion_line);
            findings.push(spindle_end_finding(model, line));
        }
        if model.program.state.coolant != nc_gcode::CoolantMode::Off {
            let line = last_coolant_on_line(model.program).unwrap_or(last_motion_line);
            findings.push(coolant_end_finding(model, line));
        }

        let safe_z_mm = model.stock.top_z_mm + model.tolerances.clearance_mm;
        if let Some(motion) = model.program.motions.last() {
            let end_z = motion.end_mm[2];
            if end_z.is_finite() && end_z < safe_z_mm {
                findings.push(tool_end_finding(model, motion, safe_z_mm));
            }
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A program that ends with the spindle or coolant on leaves the machine \
         running unattended, and a tool parked below the clearance plane can \
         crash on the next rapid or scratch the finished surface."
    }
}

/// Reports a block-delete line that bypasses a spindle, tool, or retract move.
struct BlockDeleteUnsafe;

impl Rule for BlockDeleteUnsafe {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(BLOCK_DELETE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        for line in &model.program.lines {
            if !line.block_delete {
                continue;
            }
            let spindle_or_tool = line.words.iter().any(|word| {
                word.letter == 'T'
                    || (word.letter == 'M'
                        && (word_matches(word.value, 6.0)
                            || SPINDLE_CODES
                                .iter()
                                .any(|code| word_matches(word.value, *code))))
            });
            let retract = model.program.motions.iter().any(|motion| {
                motion.line == line.line
                    && (motion.kind == MoveKind::Retract
                        || motion.end_mm[2] > motion.start_mm[2] + WORD_EPSILON)
            });
            if !spindle_or_tool && !retract {
                continue;
            }
            findings.push(block_delete_finding(line, spindle_or_tool, retract));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A block-delete switch decides at run time whether a `/` line executes. \
         If an operator disables it, a skipped tool change or retract leaves the \
         wrong tool in the spindle or the tool down in the material."
    }
}

/// Formats the numeric part of a G code without trailing zeros.
fn code_label(value: f64) -> String {
    if word_matches(value, nc_core::math::round(value)) {
        format!("{:.0}", nc_core::math::round(value))
    } else {
        format!("{value:.1}")
    }
}

/// Returns the last line that has a `letter` word matching `codes`.
fn last_line_with_codes(program: &Program, letter: char, codes: &[f64]) -> Option<usize> {
    program
        .lines
        .iter()
        .rev()
        .find(|line| {
            line.words.iter().any(|word| {
                word.letter == letter && codes.iter().any(|code| word_matches(word.value, *code))
            })
        })
        .map(|line| line.line)
}

/// Finds the source line in `(after, at]` that selects `wcs`.
///
/// The modal stream names the new state and the motion that uses it; this
/// locates the `G54`-`G59.3` word for the report, falling back to the motion
/// line for a hand-built program without source words.
fn wcs_change_line(program: &Program, after: usize, at: usize, wcs: Wcs) -> Option<usize> {
    program
        .lines
        .iter()
        .rev()
        .filter(|line| line.line > after && line.line <= at)
        .find(|line| {
            line.words
                .iter()
                .any(|word| word.letter == 'G' && wcs_from_value(word.value) == Some(wcs))
        })
        .map(|line| line.line)
}

/// Maps a `G` word value to a work coordinate system.
fn wcs_from_value(value: f64) -> Option<Wcs> {
    if word_matches(value, 54.0) {
        Some(Wcs::G54)
    } else if word_matches(value, 55.0) {
        Some(Wcs::G55)
    } else if word_matches(value, 56.0) {
        Some(Wcs::G56)
    } else if word_matches(value, 57.0) {
        Some(Wcs::G57)
    } else if word_matches(value, 58.0) {
        Some(Wcs::G58)
    } else if word_matches(value, 59.0) {
        Some(Wcs::G59)
    } else if word_matches(value, 59.1) {
        Some(Wcs::G59_1)
    } else if word_matches(value, 59.2) {
        Some(Wcs::G59_2)
    } else if word_matches(value, 59.3) {
        Some(Wcs::G59_3)
    } else {
        None
    }
}

/// Returns the raw source of `line`, when it exists.
fn line_source(program: &Program, line: usize) -> Option<String> {
    program
        .lines
        .iter()
        .find(|candidate| candidate.line == line)
        .map(|candidate| candidate.source.clone())
}

/// Returns the line of the last `M3`/`M4` annotation.
fn last_spindle_on_line(program: &Program) -> Option<usize> {
    program
        .annotations
        .iter()
        .rev()
        .find_map(|annotation| match &annotation.kind {
            AnnotationKind::Spindle { on: true, .. } => Some(annotation.line),
            _ => None,
        })
}

/// Returns the line of the last coolant-on annotation.
fn last_coolant_on_line(program: &Program) -> Option<usize> {
    program
        .annotations
        .iter()
        .rev()
        .find_map(|annotation| match &annotation.kind {
            AnnotationKind::Coolant { on: true } => Some(annotation.line),
            _ => None,
        })
}

/// Builds the spindle-left-on end-state finding.
fn spindle_end_finding(model: &AnalysisModel<'_>, line: usize) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_stock_evidence(&mut evidence, model);
    evidence.insert(
        "clockwise".to_string(),
        EvidenceValue::Bool(model.program.state.spindle == SpindleMode::Clockwise),
    );
    let rpm = model.program.state.spindle_rpm;
    if rpm.is_finite() {
        evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(rpm));
    }
    diagnostic(
        STATE_ID,
        Severity::Warning,
        Basis::Model,
        line,
        "program ends with the spindle still on",
        evidence,
    )
}

/// Builds the coolant-left-on end-state finding.
fn coolant_end_finding(model: &AnalysisModel<'_>, line: usize) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_stock_evidence(&mut evidence, model);
    diagnostic(
        STATE_ID,
        Severity::Warning,
        Basis::Model,
        line,
        "program ends with coolant still on",
        evidence,
    )
}

/// Builds the tool-below-safe-Z end-state finding.
fn tool_end_finding(
    model: &AnalysisModel<'_>,
    motion: &MotionRecord,
    safe_z_mm: f64,
) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_stock_evidence(&mut evidence, model);
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    let depth_mm = safe_z_mm - motion.end_mm[2];
    if depth_mm.is_finite() {
        evidence.insert("depth_mm".to_string(), EvidenceValue::Float(depth_mm));
    }
    diagnostic(
        STATE_ID,
        Severity::Warning,
        Basis::Model,
        motion.line,
        format!(
            "program ends with the tool at Z {:.3} mm, below safe Z {:.3} mm",
            motion.end_mm[2], safe_z_mm
        ),
        evidence,
    )
}

/// Adds stock provenance to end-state findings.
fn insert_stock_evidence(
    evidence: &mut BTreeMap<String, EvidenceValue>,
    model: &AnalysisModel<'_>,
) {
    evidence.insert(
        "stock_source".to_string(),
        EvidenceValue::Text(stock_source_label(model.stock.source).to_string()),
    );
    if let Some(basis) = model.stock.inference {
        evidence.insert(
            "inference_basis".to_string(),
            EvidenceValue::Text(inference_label(basis).to_string()),
        );
    }
}

/// Builds a work-coordinate-change finding.
fn wcs_finding(line: usize, previous: Wcs, current: Wcs) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "wcs".to_string(),
        EvidenceValue::Text(current.as_str().to_string()),
    );
    evidence.insert(
        "reason".to_string(),
        EvidenceValue::Text(format!(
            "changed from {} to {}",
            previous.as_str(),
            current.as_str()
        )),
    );
    diagnostic(
        WCS_ID,
        Severity::Warning,
        Basis::Exact,
        line,
        format!(
            "work coordinate system changed from {} to {} after cutting started",
            previous.as_str(),
            current.as_str()
        ),
        evidence,
    )
}

/// Builds a block-delete finding.
fn block_delete_finding(line: &ProgramLine, spindle_or_tool: bool, retract: bool) -> Diagnostic {
    let message = match (spindle_or_tool, retract) {
        (true, true) => {
            "block-delete line contains a spindle or tool-change command and an upward move"
        }
        (true, false) => "block-delete line contains a spindle or tool-change command",
        (false, true) => "block-delete line contains a retract or Z-up motion",
        (false, false) => "block-delete line is unsafe",
    };
    let mut evidence = BTreeMap::new();
    evidence.insert("raw".to_string(), EvidenceValue::Text(line.source.clone()));
    diagnostic(
        BLOCK_DELETE_ID,
        Severity::Warning,
        Basis::Exact,
        line.line,
        message,
        evidence,
    )
}

/// Builds a probing-in-a-job finding.
fn probe_finding(motion: &MotionRecord) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_motion_evidence(&mut evidence, motion);
    diagnostic(
        PROBE_ID,
        Severity::Warning,
        Basis::Exact,
        motion.line,
        "probing move (G38.x) inside a normal milling job",
        evidence,
    )
}

/// Stable text label for stock provenance.
fn stock_source_label(source: StockSource) -> &'static str {
    match source {
        StockSource::Inferred => "inferred",
        StockSource::Explicit => "explicit",
    }
}

/// Stable text label for the inference basis.
fn inference_label(basis: InferenceBasis) -> &'static str {
    match basis {
        InferenceBasis::ZeroAtTop => "zero_at_top",
        InferenceBasis::AllCutsAboveZero => "all_cuts_above_zero",
        InferenceBasis::NoCuttingMoves => "no_cutting_moves",
    }
}
