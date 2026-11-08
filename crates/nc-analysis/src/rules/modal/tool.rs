//! `modal.tool_unknown`, `modal.tool_zero`, and `modal.tool_change_mismatch`.

use std::collections::BTreeMap;

use nc_core::ToolTable;
use nc_gcode::{MotionRecord, Program};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, Severity};

use super::{diagnostic, insert_motion_evidence, tool_number, word_matches};

/// Stable id of the unknown-tool rule.
const UNKNOWN_ID: &str = "modal.tool_unknown";

/// Stable id of the zero-tool rule.
const ZERO_ID: &str = "modal.tool_zero";

/// Stable id of the tool-change pairing rule.
const CHANGE_ID: &str = "modal.tool_change_mismatch";

/// Returns the rules in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(ToolUnknown),
        Box::new(ToolZero),
        Box::new(ToolChangeMismatch),
    ]
}

/// Reports a tool used by a motion that the tool table and comment hints do not
/// describe.
struct ToolUnknown;

impl Rule for ToolUnknown {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(UNKNOWN_ID)
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
        // On a bare file every tool would be "unknown", which says nothing
        // about the program. The absence of all tool data is a skip, not a
        // finding.
        if model.tools.is_empty() && model.program.tool_hints.is_empty() {
            return RuleOutcome::Skipped {
                missing_input: "tool.data".to_string(),
                reason: "no tool data".to_string(),
            };
        }

        // The engine resolves hints into `model.tools` before the rules run; do
        // it again so a caller that passed a configuration-only table still
        // sees comment hints instead of false "unknown" findings.
        let resolved = ToolTable::resolve(model.tools, &model.program.tool_hints);
        let mut first_line: BTreeMap<u32, usize> = BTreeMap::new();
        for motion in &model.program.motions {
            first_line
                .entry(motion.tool)
                .and_modify(|line| *line = (*line).min(motion.line))
                .or_insert(motion.line);
        }

        let mut findings = Vec::new();
        for (tool, line) in first_line {
            // T0 means "no tool selected"; `modal.tool_zero` owns cutting with
            // it, and flagging every pre-change rapid would be noise.
            if tool == 0 || resolved.get(tool).is_some() {
                continue;
            }
            let mut evidence = BTreeMap::new();
            evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
            findings.push(diagnostic(
                UNKNOWN_ID,
                Severity::Warning,
                Basis::Exact,
                line,
                format!("tool T{tool} is used but has no tool-table entry or comment hint"),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A tool with no diameter, flute count, or shape cannot be checked \
         against the material or the feature. Every tool-dependent calculation \
         for that tool is silently skipped, so an unlisted tool hides problems."
    }
}

/// Reports cutting motions with no tool selected.
struct ToolZero;

impl Rule for ToolZero {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(ZERO_ID)
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
            if !motion.kind.is_cutting() || motion.tool != 0 {
                continue;
            }
            findings.push(zero_finding(motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "T0 means no tool is selected. Cutting with T0 uses whatever happens to \
         be in the spindle, with none of the feeds and speeds that tool needs."
    }
}

/// One pending `T` selection that no `M6` has consumed.
struct PendingTool {
    /// Source line of the `T` word.
    line: usize,
    /// Tool number, when the word value is a usable integer.
    tool: Option<u32>,
    /// Raw source line.
    source: String,
}

/// Pairs `T` selections with `M6` commands within a tool-change group.
struct ToolChangeMismatch;

impl Rule for ToolChangeMismatch {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(CHANGE_ID)
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
        RuleOutcome::Findings(scan_tool_changes(model.program))
    }

    fn why_it_matters(&self) -> &'static str {
        "A T without M6 selects a tool but never loads it, and an M6 without T \
         loads the previously selected tool. The program then cuts with a tool \
         different from the one its feeds and depths were written for."
    }
}

/// Scans the source lines for unpaired `T` and `M6` words.
fn scan_tool_changes(program: &Program) -> Vec<Diagnostic> {
    let mut findings = Vec::new();
    let mut pending: Option<PendingTool> = None;

    for line in &program.lines {
        let selection = line.words.iter().find(|word| word.letter == 'T');
        let change = line
            .words
            .iter()
            .any(|word| word.letter == 'M' && word_matches(word.value, 6.0));

        match (selection, change) {
            (Some(_word), true) => {
                // The word applies to this block's M6. An earlier pending
                // selection was superseded without ever being loaded.
                if let Some(previous) = pending.take() {
                    findings.push(selection_finding(previous));
                }
            }
            (Some(word), false) => {
                let current = PendingTool {
                    line: line.line,
                    tool: tool_number(word.value),
                    source: line.source.clone(),
                };
                if let Some(previous) = pending.replace(current) {
                    findings.push(selection_finding(previous));
                }
            }
            (None, true) => {
                if pending.take().is_none() {
                    findings.push(change_finding(line.line, &line.source));
                }
            }
            (None, false) => {}
        }
    }

    if let Some(previous) = pending {
        findings.push(selection_finding(previous));
    }
    findings
}

/// Builds a "T without M6" finding.
fn selection_finding(pending: PendingTool) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    if let Some(tool) = pending.tool {
        evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
    }
    evidence.insert("raw".to_string(), EvidenceValue::Text(pending.source));
    let message = match pending.tool {
        Some(tool) => format!("tool selection T{tool} is not followed by a tool change (M6)"),
        None => "tool selection is not followed by a tool change (M6)".to_string(),
    };
    diagnostic(
        CHANGE_ID,
        Severity::Warning,
        Basis::Exact,
        pending.line,
        message,
        evidence,
    )
}

/// Builds an "M6 without T" finding.
fn change_finding(line: usize, source: &str) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert("raw".to_string(), EvidenceValue::Text(source.to_string()));
    diagnostic(
        CHANGE_ID,
        Severity::Warning,
        Basis::Exact,
        line,
        "tool change (M6) has no selected tool (T)",
        evidence,
    )
}

/// Builds one finding for a cutting motion at T0.
fn zero_finding(motion: &MotionRecord) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_motion_evidence(&mut evidence, motion);
    diagnostic(
        ZERO_ID,
        Severity::Warning,
        Basis::Exact,
        motion.line,
        "cutting motion uses T0; no tool is selected",
        evidence,
    )
}
