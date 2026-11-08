//! `parse.unsupported`: one finding per block the lossy parser skipped.
//!
//! A skipped block is text the parser did not understand, so the geometry after
//! it may not be what the machine will run. The raw line and the parser's stable
//! reason key travel in the evidence so a reader can inspect the source.

use std::collections::BTreeMap;

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, Severity};

/// Stable rule id.
const RULE_ID: &str = "parse.unsupported";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(Unsupported)]
}

/// Reports every skipped block in source order.
struct Unsupported;

impl Rule for Unsupported {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(RULE_ID)
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
        let mut findings = Vec::new();
        for block in &model.program.skipped_blocks {
            let mut evidence = BTreeMap::new();
            evidence.insert("raw".to_string(), EvidenceValue::Text(block.source.clone()));
            evidence.insert(
                "reason".to_string(),
                EvidenceValue::Text(block.reason.clone()),
            );
            findings.push(Diagnostic {
                rule: crate::types::RuleId::new(RULE_ID),
                severity: Severity::Error,
                basis: Basis::Exact,
                line: block.line,
                column: 0,
                line_end: block.line,
                message: format!("block was skipped by the parser: {}", block.reason),
                evidence,
            });
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A block the parser could not understand was not applied. Geometry after \
         it may not match what the controller runs, so the program must be \
         inspected before it is trusted."
    }
}
