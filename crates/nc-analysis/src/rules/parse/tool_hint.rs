//! `parse.tool_hint_ambiguous`: a tool comment carried a dimension whose unit
//! could not be established, so the hint was dropped.
//!
//! Guessing whether `6.35` meant millimetres or inches would silently change
//! every tool-dependent rule, so the parser drops the hint and records a
//! warning; this rule surfaces that warning as a finding.

use std::collections::BTreeMap;

use nc_gcode::WarningKind;

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, Severity};

/// Stable rule id.
const RULE_ID: &str = "parse.tool_hint_ambiguous";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(ToolHintAmbiguous)]
}

/// Reports parser warnings of kind [`WarningKind::AmbiguousToolHint`].
struct ToolHintAmbiguous;

impl Rule for ToolHintAmbiguous {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(RULE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let mut findings = Vec::new();
        for warning in &model.program.warnings {
            if warning.kind != WarningKind::AmbiguousToolHint {
                continue;
            }
            let mut evidence = BTreeMap::new();
            evidence.insert(
                "reason".to_string(),
                EvidenceValue::Text(warning.message.clone()),
            );
            findings.push(Diagnostic {
                rule: crate::types::RuleId::new(RULE_ID),
                severity: Severity::Info,
                basis: Basis::Exact,
                line: warning.line,
                column: 0,
                line_end: warning.line,
                message: "ambiguous tool-comment dimension; the hint was dropped".to_string(),
                evidence,
            });
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A tool dimension with no unit could be millimetres or inches. Using \
         the wrong one changes the tool diameter and every feed, speed, and \
         clearance check that depends on it, so the hint is reported instead."
    }
}
