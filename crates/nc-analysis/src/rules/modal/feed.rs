//! `modal.feed_nonpositive`: a cutting motion carries `F0`, a negative feed, or
//! a value that is not finite.
//!
//! A missing feed is [`modal.feed_missing`](super::super::feed_missing)'s
//! subject; this rule only judges a feed that is present.

use std::collections::BTreeMap;

use nc_gcode::MotionRecord;

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, Severity};

use super::{diagnostic, insert_motion_evidence};

/// Stable rule id.
const RULE_ID: &str = "modal.feed_nonpositive";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(FeedNonpositive)]
}

/// Checks every cutting motion that has a programmed feed.
struct FeedNonpositive;

impl Rule for FeedNonpositive {
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
        for motion in &model.program.motions {
            if !motion.kind.is_cutting() {
                continue;
            }
            let Some(feed_mm_min) = motion.feed_mm_min else {
                continue;
            };
            if feed_mm_min.is_finite() && feed_mm_min > 0.0 {
                continue;
            }
            findings.push(finding(motion, feed_mm_min));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A cutting move at zero feed dwells in the material, rubbing and \
         burning the cutter instead of cutting. A negative or invalid feed is \
         rejected or misread by the controller and can stop the program mid-cut."
    }
}

/// Builds one finding for a non-positive or non-finite feed.
fn finding(motion: &MotionRecord, feed_mm_min: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    insert_motion_evidence(&mut evidence, motion);
    let message = if feed_mm_min.is_finite() {
        evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(feed_mm_min));
        format!("cutting motion has non-positive feed F{feed_mm_min:.3} mm/min")
    } else {
        "cutting motion has a non-finite feed rate".to_string()
    };
    diagnostic(
        RULE_ID,
        Severity::Error,
        Basis::Exact,
        motion.line,
        message,
        evidence,
    )
}
