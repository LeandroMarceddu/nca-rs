//! `modal.*` rules: units, distance, feed, spindle, tool, WCS, end state.
//!
//! Each rule is a zero-sized struct in a submodule. [`rules`] returns the
//! module's rules in catalog order; the engine evaluates them against the
//! shared [`AnalysisModel`](crate::rule::AnalysisModel) and normalizes ids,
//! severity, and source-line bounds.
//!
//! No rule invents an input. A rule that cannot answer from the program text
//! returns [`RuleOutcome::Skipped`](crate::rule::RuleOutcome::Skipped) with a
//! stable missing-input key instead of guessing.

pub mod feed;
pub mod program;
pub mod spindle;
pub mod tool;
pub mod units;

use std::collections::BTreeMap;

use nc_gcode::ProgramLine;

use crate::report::Diagnostic;
use crate::rule::Rule;
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Tolerance for comparing word values such as `G1` or `M30`.
pub(crate) const WORD_EPSILON: f64 = 1.0e-9;

/// Every rule in this module, in catalog order.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(units::rules());
    rules.extend(feed::rules());
    rules.extend(spindle::rules());
    rules.extend(tool::rules());
    rules.extend(program::rules());
    rules
}

/// Builds a finding that spans one source line.
pub(crate) fn diagnostic(
    rule: &'static str,
    severity: Severity,
    basis: Basis,
    line: usize,
    message: impl Into<String>,
    evidence: BTreeMap<String, EvidenceValue>,
) -> Diagnostic {
    Diagnostic {
        rule: RuleId::new(rule),
        severity,
        basis,
        line,
        column: 0,
        line_end: line,
        message: message.into(),
        evidence,
    }
}

/// Returns `true` when `value` is the numeric code `expected`.
pub(crate) fn word_matches(value: f64, expected: f64) -> bool {
    value.is_finite() && nc_core::math::abs(value - expected) <= WORD_EPSILON
}

/// Returns `true` when the line has a `letter` word with numeric `expected`.
pub(crate) fn line_has_word(line: &ProgramLine, letter: char, expected: f64) -> bool {
    line.words
        .iter()
        .any(|word| word.letter == letter && word_matches(word.value, expected))
}

/// Converts an integral word value to a tool number.
///
/// Returns `None` for a negative, non-finite, or fractional value; the parser
/// already rejects those, but rules also run on hand-built and deserialized
/// programs.
pub(crate) fn tool_number(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }
    let rounded = nc_core::math::round(value);
    if nc_core::math::abs(rounded - value) > WORD_EPSILON {
        return None;
    }
    Some(rounded as u32)
}

/// Inserts the motion kind and tool number evidence shared by motion rules.
pub(crate) fn insert_motion_evidence(
    evidence: &mut BTreeMap<String, EvidenceValue>,
    motion: &nc_gcode::MotionRecord,
) {
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
}
