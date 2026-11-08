//! `parse.dialect_assumed`: the file was parsed with the default `Generic`
//! dialect because no dialect was configured.

use std::collections::BTreeMap;

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, Severity};

/// Stable rule id.
const RULE_ID: &str = "parse.dialect_assumed";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(DialectAssumed)]
}

/// Emits one informational finding when `Generic` was assumed.
struct DialectAssumed;

impl Rule for DialectAssumed {
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
        if !model.dialect_assumed {
            return RuleOutcome::Findings(Vec::new());
        }
        RuleOutcome::Findings(vec![Diagnostic {
            rule: crate::types::RuleId::new(RULE_ID),
            severity: Severity::Info,
            basis: Basis::Exact,
            line: 1,
            column: 0,
            line_end: 1,
            message: "no dialect was configured; Generic semantics were assumed".to_string(),
            evidence: BTreeMap::new(),
        }])
    }

    fn why_it_matters(&self) -> &'static str {
        "The dialect decides which codes are understood. Assuming Generic can \
         reject a post-processor's supported command or accept one with a \
         different meaning; selecting the real post removes the guesswork."
    }
}
