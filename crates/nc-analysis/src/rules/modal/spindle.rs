//! `modal.spindle_stopped`, `modal.spindle_speed_zero`, and
//! `modal.spindle_direction`.
//!
//! The spindle state of a motion comes from the motion record, which the parser
//! fills from the effective modal state. Commanded `M3`/`M4`/`M5` events come
//! from the annotation stream, where the programmed speed and direction are
//! preserved even when no motion follows.

use std::collections::BTreeMap;

use nc_core::ToolField;
use nc_gcode::{AnnotationKind, MotionRecord, Program};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, Severity};

use super::{diagnostic, insert_motion_evidence};

/// Stable id of the stopped-spindle rule.
const STOPPED_ID: &str = "modal.spindle_stopped";

/// Stable id of the zero-speed rule.
const SPEED_ZERO_ID: &str = "modal.spindle_speed_zero";

/// Stable id of the direction rule.
const DIRECTION_ID: &str = "modal.spindle_direction";

/// Missing-input key when the spindle is started outside the program.
const MANUAL_KEY: &str = "machine.spindle_manual";

/// Returns the rules in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(SpindleStopped),
        Box::new(SpindleSpeedZero),
        Box::new(SpindleDirection),
    ]
}

/// Checks every cutting motion for a spindle that was never commanded on.
struct SpindleStopped;

impl Rule for SpindleStopped {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(STOPPED_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Error
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        // A manually started spindle is not driven by the program, so the
        // programmed M3/M5 state says nothing about whether it is turning.
        if let Some(machine) = model.machine
            && machine.spindle_manual
        {
            return RuleOutcome::Skipped {
                missing_input: MANUAL_KEY.to_string(),
                reason:
                    "the spindle is started manually; programmed spindle state is not authoritative"
                        .to_string(),
            };
        }

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            if !motion.kind.is_cutting() {
                continue;
            }
            // `spindle_cw` is `Some` exactly while M3 or M4 is active.
            if motion.spindle_cw.is_some() {
                continue;
            }
            findings.push(finding(motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A cutting move with the spindle stopped drags the flutes through the \
         material without a cutting edge speed and snaps the cutter. Confirm the \
         spindle is running before the first plunge."
    }
}

/// Checks that every `M3`/`M4` commands a positive speed.
struct SpindleSpeedZero;

impl Rule for SpindleSpeedZero {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(SPEED_ZERO_ID)
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
        if model.machine.is_some_and(|machine| machine.spindle_manual) {
            return RuleOutcome::Skipped {
                missing_input: MANUAL_KEY.to_string(),
                reason: "the spindle is started manually; a programmed S0 is not authoritative"
                    .to_string(),
            };
        }
        let mut findings = Vec::new();
        for annotation in &model.program.annotations {
            if let AnnotationKind::Spindle {
                on: true,
                rpm,
                clockwise,
            } = &annotation.kind
            {
                if rpm.is_finite() && *rpm > 0.0 {
                    continue;
                }
                findings.push(speed_finding(annotation.line, *rpm, *clockwise));
            }
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "M3 or M4 without a positive S commands the spindle on at zero speed. \
         The controller accepts the mode, and the first cutting move then rubs \
         or breaks the tool instead of cutting."
    }
}

/// Checks that `M4` is not used with a right-hand cutter.
struct SpindleDirection;

impl Rule for SpindleDirection {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(DIRECTION_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_tools(&[ToolField::RightHand])
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        for annotation in &model.program.annotations {
            if let AnnotationKind::Spindle {
                on: true,
                clockwise: false,
                rpm,
            } = &annotation.kind
            {
                let tool = tool_for_annotation(model.program, annotation.next_motion_index);
                let Some(entry) = model.tools.get(tool) else {
                    continue;
                };
                if entry.right_hand != Some(true) {
                    continue;
                }
                let mut evidence = BTreeMap::new();
                evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
                evidence.insert("clockwise".to_string(), EvidenceValue::Bool(false));
                if rpm.is_finite() {
                    evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(*rpm));
                }
                if let Some(source) = model.tools.source_of(tool, ToolField::RightHand) {
                    evidence.insert(
                        "tool_source".to_string(),
                        EvidenceValue::Text(source.label()),
                    );
                }
                findings.push(diagnostic(
                    DIRECTION_ID,
                    Severity::Info,
                    Basis::Exact,
                    annotation.line,
                    format!("M4 runs right-hand tool T{tool} counter-clockwise"),
                    evidence,
                ));
            }
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A right-hand cutter is designed to turn clockwise. Running it in \
         reverse dulls the edge, pulls the part upward, and can unscrew a \
         chuck-mounted tool."
    }
}

/// Resolves the tool active at an annotation.
///
/// The next motion carries the state the block will use; when the annotation is
/// after the last motion, the most recent motion's tool is still active.
fn tool_for_annotation(program: &Program, next_motion_index: usize) -> u32 {
    if let Some(motion) = program.motions.get(next_motion_index) {
        return motion.tool;
    }
    next_motion_index
        .checked_sub(1)
        .and_then(|index| program.motions.get(index))
        .map(|motion| motion.tool)
        .unwrap_or(0)
}

/// Builds one finding for a stopped-spindle cutting motion.
fn finding(motion: &MotionRecord) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_motion_evidence(&mut evidence, motion);
    if let Some(rpm) = motion.spindle_rpm
        && rpm.is_finite()
    {
        evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(rpm));
    }
    diagnostic(
        STOPPED_ID,
        Severity::Error,
        Basis::Exact,
        motion.line,
        "cutting motion runs without the spindle commanded on",
        evidence,
    )
}

/// Builds one finding for a zero-speed spindle command.
fn speed_finding(line: usize, rpm: f64, clockwise: bool) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert("clockwise".to_string(), EvidenceValue::Bool(clockwise));
    if rpm.is_finite() {
        evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(rpm));
    }
    let message = if rpm.is_finite() {
        format!("M3/M4 commands the spindle on at S{rpm:.0} rpm")
    } else {
        "M3/M4 commands the spindle on with a non-finite speed".to_string()
    };
    diagnostic(
        SPEED_ZERO_ID,
        Severity::Warning,
        Basis::Exact,
        line,
        message,
        evidence,
    )
}
