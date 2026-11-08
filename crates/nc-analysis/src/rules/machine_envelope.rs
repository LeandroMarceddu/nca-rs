//! `travel.machine_envelope`: a motion extent leaves the configured travel box.

use std::collections::BTreeMap;

use nc_gcode::MotionRecord;

use super::extents::motion_extents;
use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Stable rule id.
const RULE_ID: &str = "travel.machine_envelope";

/// Axis labels in machine X, Y, Z order.
const AXES: [&str; 3] = ["X", "Y", "Z"];

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(MachineEnvelope)]
}

/// Checks every motion's exact swept extent against the configured travel box.
struct MachineEnvelope;

impl Rule for MachineEnvelope {
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
        Requirements {
            machine: true,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let Some(machine) = model.machine else {
            return RuleOutcome::Skipped {
                missing_input: "machine".to_string(),
                reason: "machine limits are required".to_string(),
            };
        };
        if machine.validate().is_err() {
            return RuleOutcome::Skipped {
                missing_input: "machine".to_string(),
                reason: "configured machine travel limits are invalid".to_string(),
            };
        }

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            let Some((extent_min, extent_max)) = motion_extents(motion) else {
                continue;
            };
            for axis in 0..AXES.len() {
                let min = machine.travel_min_mm[axis];
                let max = machine.travel_max_mm[axis];
                let extent_min = extent_min[axis];
                let extent_max = extent_max[axis];
                let below = extent_min < min;
                let above = extent_max > max;
                if !below && !above {
                    continue;
                }
                // One finding per violating axis. When the extent crosses both
                // limits, the lower bound is the reported value; the message
                // and the extent evidence name both bounds.
                let value = if below { extent_min } else { extent_max };
                findings.push(finding(
                    motion, axis, value, extent_min, extent_max, min, max,
                ));
            }
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A move outside the machine's travel commands an axis past its physical \
         limit, which can jam the machine or lose position before the controller \
         has a chance to stop it."
    }
}

/// Builds one finding for one axis violation.
fn finding(
    motion: &MotionRecord,
    axis: usize,
    value: f64,
    extent_min: f64,
    extent_max: f64,
    min: f64,
    max: f64,
) -> Diagnostic {
    let axis_label = AXES.get(axis).copied().unwrap_or("?");
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "axis".to_string(),
        EvidenceValue::Text(axis_label.to_string()),
    );
    evidence.insert("value_mm".to_string(), EvidenceValue::Float(value));
    evidence.insert("min_mm".to_string(), EvidenceValue::Float(min));
    evidence.insert("max_mm".to_string(), EvidenceValue::Float(max));
    evidence.insert(
        "extent_min_mm".to_string(),
        EvidenceValue::Float(extent_min),
    );
    evidence.insert(
        "extent_max_mm".to_string(),
        EvidenceValue::Float(extent_max),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));

    Diagnostic {
        rule: RuleId::new(RULE_ID),
        severity: Severity::Error,
        basis: Basis::Exact,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "{axis_label} extent [{extent_min:.3}, {extent_max:.3}] mm is outside machine travel \
             [{min:.3}, {max:.3}] mm"
        ),
        evidence,
    }
}
