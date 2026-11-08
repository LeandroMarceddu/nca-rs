//! `modal.feed_missing`: the first cutting motion has no usable feed rate.

use std::collections::BTreeMap;

use nc_gcode::{FeedMode, MotionRecord};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Stable rule id.
const RULE_ID: &str = "modal.feed_missing";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(FeedMissing)]
}

/// Checks only the first cutting motion.
struct FeedMissing;

impl Rule for FeedMissing {
    fn id(&self) -> RuleId {
        RuleId::new(RULE_ID)
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
        let Some(motion) = model
            .program
            .motions
            .iter()
            .find(|motion| motion.kind.is_cutting())
        else {
            return RuleOutcome::Findings(Vec::new());
        };

        match motion.feed_mm_min {
            None => {
                let modal = model.program.modal(motion.modal);
                if modal.feed_mode == FeedMode::InverseTime {
                    return RuleOutcome::Skipped {
                        missing_input: "feed.inverse_time".to_string(),
                        reason: format!(
                            "line {} uses inverse-time feed (G93); a feed in mm/min is undefined by design",
                            motion.line
                        ),
                    };
                }
                RuleOutcome::Findings(vec![finding(motion)])
            }
            // A present but non-positive feed belongs to `modal.feed_nonpositive`;
            // this rule answers only "was any feed set for the first cut?".
            Some(_) => RuleOutcome::Findings(Vec::new()),
        }
    }

    fn why_it_matters(&self) -> &'static str {
        "A cutting move with no feed rate runs at whatever value the controller \
         last held, which can be far above what the cutter and material allow. \
         Set F explicitly before the first cut."
    }
}

/// Builds the one finding for `motion`.
fn finding(motion: &MotionRecord) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3(motion.start_mm),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));

    Diagnostic {
        rule: RuleId::new(RULE_ID),
        severity: Severity::Error,
        basis: Basis::Exact,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: "first cutting motion has no feed rate".to_string(),
        evidence,
    }
}
