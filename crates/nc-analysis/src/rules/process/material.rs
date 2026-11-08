//! Material-driven process rules: chipload and surface speed.
//!
//! Both compare the programmed cut against a configured [`MaterialPreset`]
//! range. A missing range skips the rule with `material`; a malformed range
//! endpoint is ignored, and a rule with neither endpoint is skipped rather than
//! silently passing.

use std::collections::BTreeMap;

use nc_core::ToolField;

use crate::config::RuleSettings;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

use super::{diagnostic, insert_tool_source, positive, skip, threshold};

/// Stable rule id for the chipload rule.
const CHIPLOAD_ID: &str = "process.chipload";

/// Stable rule id for the surface-speed rule.
const SURFACE_SPEED_ID: &str = "process.surface_speed";

/// Rules in catalog order.
#[must_use]
pub(crate) fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(Chipload), Box::new(SurfaceSpeed)]
}

/// Compares feed per tooth against the material's chipload range.
struct Chipload;

impl Rule for Chipload {
    fn id(&self) -> RuleId {
        RuleId::new(CHIPLOAD_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: &[ToolField::Flutes],
            material: true,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = super::invalid_tool_skip(model, ToolField::Flutes, "flute count") {
            return outcome;
        }
        let Some(material) = model.material else {
            return skip("material", "a material preset is required");
        };
        let min_mm = positive(material.chipload_min_mm);
        let max_mm = positive(material.chipload_max_mm);
        if min_mm.is_none() && max_mm.is_none() {
            return skip("material", "material preset has no chipload range");
        }

        let margin = threshold(cfg, "chipload_margin", 0.0);
        let lower_mm = min_mm.map(|value| (value - margin).max(0.0));
        let upper_mm = max_mm.map(|value| value + margin);
        let mut findings = Vec::new();
        let mut saw_feed = false;
        let mut saw_rpm = false;

        for motion in model.program.expanded_motions() {
            if !motion.kind.is_cutting() {
                continue;
            }
            let Some(feed_mm_min) = positive(motion.feed_mm_min) else {
                continue;
            };
            saw_feed = true;
            let Some(spindle_rpm) = positive(motion.spindle_rpm) else {
                continue;
            };
            saw_rpm = true;
            let Some(tool) = model.tools.get(motion.tool) else {
                continue;
            };
            let Some(flutes) = tool.flutes.filter(|flutes| *flutes > 0) else {
                continue;
            };

            let chipload_mm = feed_mm_min / (spindle_rpm * f64::from(flutes));
            let below = lower_mm.is_some_and(|lower| chipload_mm < lower);
            let above = upper_mm.is_some_and(|upper| chipload_mm > upper);
            if !below && !above {
                continue;
            }

            let mut evidence = BTreeMap::new();
            evidence.insert(
                "tool".to_string(),
                EvidenceValue::Int(i64::from(motion.tool)),
            );
            insert_tool_source(&mut evidence, model, motion.tool, &[ToolField::Flutes]);
            evidence.insert(
                "material".to_string(),
                EvidenceValue::Text(material.name.clone()),
            );
            evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(feed_mm_min));
            evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(spindle_rpm));
            evidence.insert("flutes".to_string(), EvidenceValue::Int(i64::from(flutes)));
            evidence.insert("chipload_mm".to_string(), EvidenceValue::Float(chipload_mm));
            evidence.insert("measured".to_string(), EvidenceValue::Float(chipload_mm));
            if let Some(lower) = lower_mm {
                evidence.insert("min_mm".to_string(), EvidenceValue::Float(lower));
            }
            if let Some(upper) = upper_mm {
                evidence.insert("max_mm".to_string(), EvidenceValue::Float(upper));
            }
            evidence.insert("margin_mm".to_string(), EvidenceValue::Float(margin));

            let message = match (lower_mm, upper_mm) {
                (Some(lower), Some(upper)) => format!(
                    "chipload {chipload_mm:.4} mm/tooth is outside the {} range \
                     {lower:.4}-{upper:.4} mm/tooth",
                    material.name
                ),
                (Some(lower), None) => format!(
                    "chipload {chipload_mm:.4} mm/tooth is below the {} minimum \
                     {lower:.4} mm/tooth",
                    material.name
                ),
                (None, Some(upper)) => format!(
                    "chipload {chipload_mm:.4} mm/tooth is above the {} maximum \
                     {upper:.4} mm/tooth",
                    material.name
                ),
                (None, None) => String::new(),
            };
            findings.push(diagnostic(
                CHIPLOAD_ID,
                Severity::Warning,
                motion.line,
                message,
                evidence,
            ));
        }

        if findings.is_empty() && !saw_feed {
            return skip("feed", "no cutting motion has a programmed feed");
        }
        if findings.is_empty() && !saw_rpm {
            return skip(
                "spindle_rpm",
                "no cutting motion has a programmed spindle speed",
            );
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Feed per tooth outside the material range either overloads the flutes \
         until they chip, or rubs instead of cutting and hardens the surface."
    }
}

/// Compares the programmed surface speed against the material's range.
struct SurfaceSpeed;

impl Rule for SurfaceSpeed {
    fn id(&self) -> RuleId {
        RuleId::new(SURFACE_SPEED_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: &[ToolField::Diameter],
            material: true,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = super::invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        let Some(material) = model.material else {
            return skip("material", "a material preset is required");
        };
        let min_mpm = positive(material.surface_speed_min_mpm);
        let max_mpm = positive(material.surface_speed_max_mpm);
        if min_mpm.is_none() && max_mpm.is_none() {
            return skip("material", "material preset has no surface-speed range");
        }

        let margin = threshold(cfg, "surface_speed_margin", 0.0);
        let lower_mpm = min_mpm.map(|value| (value - margin).max(0.0));
        let upper_mpm = max_mpm.map(|value| value + margin);
        let mut findings = Vec::new();
        let mut saw_rpm = false;

        for motion in model.program.expanded_motions() {
            if !motion.kind.is_cutting() {
                continue;
            }
            let Some(spindle_rpm) = positive(motion.spindle_rpm) else {
                continue;
            };
            saw_rpm = true;
            let Some(diameter_mm) = model
                .tools
                .get(motion.tool)
                .and_then(|tool| tool.diameter_mm)
                .filter(|diameter| diameter.is_finite() && *diameter > 0.0)
            else {
                continue;
            };

            let surface_speed_mpm = spindle_rpm * nc_core::math::PI * diameter_mm / 1000.0;
            let below = lower_mpm.is_some_and(|lower| surface_speed_mpm < lower);
            let above = upper_mpm.is_some_and(|upper| surface_speed_mpm > upper);
            if !below && !above {
                continue;
            }

            let mut evidence = BTreeMap::new();
            evidence.insert(
                "tool".to_string(),
                EvidenceValue::Int(i64::from(motion.tool)),
            );
            insert_tool_source(&mut evidence, model, motion.tool, &[ToolField::Diameter]);
            evidence.insert(
                "material".to_string(),
                EvidenceValue::Text(material.name.clone()),
            );
            evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(spindle_rpm));
            evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
            evidence.insert(
                "surface_speed_mpm".to_string(),
                EvidenceValue::Float(surface_speed_mpm),
            );
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(surface_speed_mpm),
            );
            if let Some(lower) = lower_mpm {
                evidence.insert("min_mpm".to_string(), EvidenceValue::Float(lower));
            }
            if let Some(upper) = upper_mpm {
                evidence.insert("max_mpm".to_string(), EvidenceValue::Float(upper));
            }
            evidence.insert("margin_mpm".to_string(), EvidenceValue::Float(margin));

            let message = match (lower_mpm, upper_mpm) {
                (Some(lower), Some(upper)) => format!(
                    "surface speed {surface_speed_mpm:.1} m/min is outside the {} \
                     range {lower:.1}-{upper:.1} m/min",
                    material.name
                ),
                (Some(lower), None) => format!(
                    "surface speed {surface_speed_mpm:.1} m/min is below the {} \
                     minimum {lower:.1} m/min",
                    material.name
                ),
                (None, Some(upper)) => format!(
                    "surface speed {surface_speed_mpm:.1} m/min is above the {} \
                     maximum {upper:.1} m/min",
                    material.name
                ),
                (None, None) => String::new(),
            };
            findings.push(diagnostic(
                SURFACE_SPEED_ID,
                Severity::Warning,
                motion.line,
                message,
                evidence,
            ));
        }

        if findings.is_empty() && !saw_rpm {
            return skip(
                "spindle_rpm",
                "no cutting motion has a programmed spindle speed",
            );
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A surface speed outside the material range burns the cutter at the top \
         of the range and smears or work-hardens the material at the bottom."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_ids_are_stable() {
        let ids: Vec<String> = rules()
            .iter()
            .map(|rule| rule.id().as_str().to_string())
            .collect();
        assert_eq!(ids, vec![CHIPLOAD_ID, SURFACE_SPEED_ID]);
    }
}
