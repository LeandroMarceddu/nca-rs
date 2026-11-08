//! Engagement- and feed-driven process rules.
//!
//! Axial and radial depth, removal rate, flute length, stickout, and plunge
//! feed. Depth and removal-rate rules read the shared engagement pass; the
//! stickout and plunge rules need only programmed data.

use std::collections::BTreeMap;

use nc_core::ToolField;
use nc_gcode::MoveKind;

use crate::config::RuleSettings;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

use super::{
    DIAMETER, diagnostic, engagement_data, first_tool_line, insert_tool_source, invalid_tool_skip,
    pass_diameter, positive, skip, stock_evidence, stock_skip, threshold, used_tools,
};

/// Stable rule id for the axial-depth rule.
const AXIAL_DEPTH_ID: &str = "process.axial_depth";
/// Stable rule id for the radial-engagement rule.
const RADIAL_ENGAGEMENT_ID: &str = "process.radial_engagement";
/// Stable rule id for the removal-rate rule.
const MRR_ID: &str = "process.mrr";
/// Stable rule id for the flute-length rule.
const FLUTE_LENGTH_ID: &str = "process.flute_length";
/// Stable rule id for the stickout-ratio rule.
const STICKOUT_RATIO_ID: &str = "process.stickout_ratio";
/// Stable rule id for the plunge-feed rule.
const PLUNGE_FEED_ID: &str = "process.plunge_feed";

/// Rules in catalog order.
#[must_use]
pub(crate) fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(AxialDepth),
        Box::new(RadialEngagement),
        Box::new(Mrr),
        Box::new(FluteLength),
        Box::new(StickoutRatio),
        Box::new(PlungeFeed),
    ]
}

/// Flags axial depth of cut above `axial_doc_ratio * diameter`.
struct AxialDepth;

impl Rule for AxialDepth {
    fn id(&self) -> RuleId {
        RuleId::new(AXIAL_DEPTH_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: DIAMETER,
            stock: StockRequirement::InferredOk,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        let ratio = threshold(cfg, "axial_doc_ratio", 1.0);
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };

        let mut findings = Vec::new();
        for pass in &data.passes {
            let Some((tool, diameter_mm)) = pass_diameter(model, pass) else {
                continue;
            };
            let threshold_mm = ratio * diameter_mm;
            if pass.axial_doc_mm <= threshold_mm {
                continue;
            }

            let mut evidence = stock_evidence(&model.stock);
            evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
            insert_tool_source(&mut evidence, model, tool, DIAMETER);
            evidence.insert(
                "axial_doc_mm".to_string(),
                EvidenceValue::Float(pass.axial_doc_mm),
            );
            evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
            evidence.insert("ratio".to_string(), EvidenceValue::Float(ratio));
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(pass.axial_doc_mm),
            );
            evidence.insert("threshold".to_string(), EvidenceValue::Float(threshold_mm));

            findings.push(diagnostic(
                AXIAL_DEPTH_ID,
                Severity::Warning,
                pass.line,
                format!(
                    "axial depth of cut {:.3} mm exceeds {:.3} x tool diameter ({:.3} mm)",
                    pass.axial_doc_mm, ratio, threshold_mm
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "An axial depth of cut beyond the cutter's rated bite overloads the \
         flutes, raises cutting force, and can snap the tool in a deep pocket."
    }
}

/// Flags radial engagement above `radial_engagement_ratio * diameter`.
struct RadialEngagement;

impl Rule for RadialEngagement {
    fn id(&self) -> RuleId {
        RuleId::new(RADIAL_ENGAGEMENT_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: DIAMETER,
            stock: StockRequirement::InferredOk,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        let ratio = threshold(cfg, "radial_engagement_ratio", 0.5);
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };

        let mut findings = Vec::new();
        for pass in &data.passes {
            // Radial engagement describes side cutting. A drill/plunge/bore is
            // fully engaged by definition; flagging it would be meaningless.
            if !matches!(
                pass.kind,
                nc_gcode::MoveKind::Feed
                    | nc_gcode::MoveKind::Ramp
                    | nc_gcode::MoveKind::ArcCw
                    | nc_gcode::MoveKind::ArcCcw
                    | nc_gcode::MoveKind::HelixCw
                    | nc_gcode::MoveKind::HelixCcw
            ) {
                continue;
            }
            let Some((tool, diameter_mm)) = pass_diameter(model, pass) else {
                continue;
            };
            let threshold_mm = ratio * diameter_mm;
            if pass.radial_doc_mm <= threshold_mm {
                continue;
            }

            let mut evidence = stock_evidence(&model.stock);
            evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
            insert_tool_source(&mut evidence, model, tool, DIAMETER);
            evidence.insert(
                "radial_doc_mm".to_string(),
                EvidenceValue::Float(pass.radial_doc_mm),
            );
            evidence.insert(
                "contact_angle_deg".to_string(),
                EvidenceValue::Float(nc_core::math::to_degrees(pass.contact_angle_rad)),
            );
            evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
            evidence.insert("ratio".to_string(), EvidenceValue::Float(ratio));
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(pass.radial_doc_mm),
            );
            evidence.insert("threshold".to_string(), EvidenceValue::Float(threshold_mm));

            findings.push(diagnostic(
                RADIAL_ENGAGEMENT_ID,
                Severity::Warning,
                pass.line,
                format!(
                    "estimated radial engagement {:.3} mm exceeds {:.3} x tool diameter \
                     ({:.3} mm)",
                    pass.radial_doc_mm, ratio, threshold_mm
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A radial engagement wider than the cutter is rated for raises cutting \
         force and heat, and can overload a small tool in a full-width slot."
    }
}

/// Flags material removal rate above the material maximum.
struct Mrr;

impl Rule for Mrr {
    fn id(&self) -> RuleId {
        RuleId::new(MRR_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: DIAMETER,
            material: true,
            stock: StockRequirement::InferredOk,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        let Some(material) = model.material else {
            return skip("material", "a material preset is required");
        };
        let Some(max_mrr_mm3_min) = positive(material.max_mrr_mm3_min) else {
            return skip("material", "material preset has no maximum removal rate");
        };
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };

        let margin = threshold(cfg, "mrr_margin", 0.0);
        let threshold_mm3_min = max_mrr_mm3_min + margin;
        let mut findings = Vec::new();
        for pass in &data.passes {
            let Some((tool, diameter_mm)) = pass_diameter(model, pass) else {
                continue;
            };
            if pass.mrr_mm3_min <= threshold_mm3_min {
                continue;
            }

            let mut evidence = stock_evidence(&model.stock);
            evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
            insert_tool_source(&mut evidence, model, tool, DIAMETER);
            evidence.insert(
                "material".to_string(),
                EvidenceValue::Text(material.name.clone()),
            );
            evidence.insert(
                "mrr_mm3_min".to_string(),
                EvidenceValue::Float(pass.mrr_mm3_min),
            );
            evidence.insert(
                "max_mrr_mm3_min".to_string(),
                EvidenceValue::Float(max_mrr_mm3_min),
            );
            evidence.insert(
                "axial_doc_mm".to_string(),
                EvidenceValue::Float(pass.axial_doc_mm),
            );
            evidence.insert(
                "radial_doc_mm".to_string(),
                EvidenceValue::Float(pass.radial_doc_mm),
            );
            evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
            if let Some(feed_mm_min) = model
                .program
                .motions
                .get(pass.motion_index)
                .and_then(|motion| motion.feed_mm_min)
            {
                evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(feed_mm_min));
            }
            evidence.insert("margin_mm3_min".to_string(), EvidenceValue::Float(margin));
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(pass.mrr_mm3_min),
            );
            evidence.insert(
                "threshold".to_string(),
                EvidenceValue::Float(threshold_mm3_min),
            );

            findings.push(diagnostic(
                MRR_ID,
                Severity::Warning,
                pass.line,
                format!(
                    "estimated removal rate {:.0} mm3/min exceeds the {} limit of \
                     {:.0} mm3/min",
                    pass.mrr_mm3_min, material.name, threshold_mm3_min
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Removal rate above the material limit outruns chip evacuation, heats \
         the cutter, and can weld chips to the flutes."
    }
}

/// Flags axial depth greater than the fluted length.
struct FluteLength;

impl Rule for FluteLength {
    fn id(&self) -> RuleId {
        RuleId::new(FLUTE_LENGTH_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: &[ToolField::FluteLength],
            stock: StockRequirement::InferredOk,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::FluteLength, "flute length") {
            return outcome;
        }
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };

        let mut findings = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for pass in &data.passes {
            let Some(tool) = super::pass_tool_number(model, pass) else {
                continue;
            };
            let Some(flute_length_mm) = model
                .tools
                .get(tool)
                .and_then(|tool| tool.flute_length_mm)
                .filter(|value| value.is_finite() && *value > 0.0)
            else {
                continue;
            };
            if pass.axial_doc_mm <= flute_length_mm {
                continue;
            }
            if !seen.insert(tool) {
                continue;
            }

            let mut evidence = stock_evidence(&model.stock);
            evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
            insert_tool_source(&mut evidence, model, tool, &[ToolField::FluteLength]);
            evidence.insert(
                "axial_doc_mm".to_string(),
                EvidenceValue::Float(pass.axial_doc_mm),
            );
            evidence.insert(
                "flute_length_mm".to_string(),
                EvidenceValue::Float(flute_length_mm),
            );
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(pass.axial_doc_mm),
            );
            evidence.insert(
                "threshold".to_string(),
                EvidenceValue::Float(flute_length_mm),
            );

            findings.push(diagnostic(
                FLUTE_LENGTH_ID,
                Severity::Warning,
                pass.line,
                format!(
                    "axial depth of cut {:.3} mm exceeds the flute length of {:.3} mm",
                    pass.axial_doc_mm, flute_length_mm
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Cutting deeper than the fluted length buries the smooth shank in the \
         cut, where it rubs, overheats, and cannot clear chips."
    }
}

/// Flags stickout-to-diameter ratio above the configured maximum.
struct StickoutRatio;

impl Rule for StickoutRatio {
    fn id(&self) -> RuleId {
        RuleId::new(STICKOUT_RATIO_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: super::STICKOUT_AND_DIAMETER,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Stickout, "stickout") {
            return outcome;
        }
        let limit = threshold(cfg, "stickout_ratio", 8.0);
        let mut findings = Vec::new();

        for tool_number in used_tools(model) {
            let Some(tool) = model.tools.get(tool_number) else {
                continue;
            };
            let Some(stickout_mm) = tool
                .stickout_mm
                .filter(|value| value.is_finite() && *value > 0.0)
            else {
                continue;
            };
            let Some(diameter_mm) = tool
                .diameter_mm
                .filter(|value| value.is_finite() && *value > 0.0)
            else {
                continue;
            };
            let ratio = stickout_mm / diameter_mm;
            if ratio <= limit {
                continue;
            }
            let Some(line) = first_tool_line(model, tool_number) else {
                continue;
            };

            let mut evidence = BTreeMap::new();
            evidence.insert(
                "tool".to_string(),
                EvidenceValue::Int(i64::from(tool_number)),
            );
            insert_tool_source(
                &mut evidence,
                model,
                tool_number,
                super::STICKOUT_AND_DIAMETER,
            );
            evidence.insert("stickout_mm".to_string(), EvidenceValue::Float(stickout_mm));
            evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
            evidence.insert("ratio".to_string(), EvidenceValue::Float(ratio));
            evidence.insert("measured".to_string(), EvidenceValue::Float(ratio));
            evidence.insert("threshold".to_string(), EvidenceValue::Float(limit));

            findings.push(diagnostic(
                STICKOUT_RATIO_ID,
                Severity::Warning,
                line,
                format!(
                    "stickout {:.1} mm is {:.1} times the {:.1} mm diameter, above \
                     the {:.1} limit",
                    stickout_mm, ratio, diameter_mm, limit
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A long stickout relative to the cutter diameter lets the tool deflect \
         and chatter, leaving a poor finish and risking a snapped shank."
    }
}

/// Flags a plunge feed above `plunge_feed_ratio` times the cutting feed.
struct PlungeFeed;

impl Rule for PlungeFeed {
    fn id(&self) -> RuleId {
        RuleId::new(PLUNGE_FEED_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        let ratio = threshold(cfg, "plunge_feed_ratio", 0.5);
        let mut reference_mm_min: Option<f64> = None;
        for motion in model.program.expanded_motions() {
            if !motion.kind.is_cutting() || motion.kind == MoveKind::Plunge {
                continue;
            }
            if let Some(feed) = positive(motion.feed_mm_min) {
                reference_mm_min = Some(reference_mm_min.map_or(feed, |best| best.max(feed)));
            }
        }
        let Some(reference_mm_min) = reference_mm_min else {
            return skip(
                "feed.reference",
                "no non-plunge cutting feed exists to use as a reference",
            );
        };
        let threshold_mm_min = ratio * reference_mm_min;
        let mut findings = Vec::new();

        for motion in model.program.expanded_motions() {
            if motion.kind != MoveKind::Plunge {
                continue;
            }
            let Some(feed_mm_min) = positive(motion.feed_mm_min) else {
                continue;
            };
            if feed_mm_min <= threshold_mm_min {
                continue;
            }

            let mut evidence = BTreeMap::new();
            evidence.insert(
                "tool".to_string(),
                EvidenceValue::Int(i64::from(motion.tool)),
            );
            evidence.insert(
                "kind".to_string(),
                EvidenceValue::Text(motion.kind.label().to_string()),
            );
            evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(feed_mm_min));
            evidence.insert(
                "reference_feed_mm_min".to_string(),
                EvidenceValue::Float(reference_mm_min),
            );
            evidence.insert("ratio".to_string(), EvidenceValue::Float(ratio));
            evidence.insert("measured".to_string(), EvidenceValue::Float(feed_mm_min));
            evidence.insert(
                "threshold".to_string(),
                EvidenceValue::Float(threshold_mm_min),
            );

            findings.push(diagnostic(
                PLUNGE_FEED_ID,
                Severity::Warning,
                motion.line,
                format!(
                    "plunge feed {:.1} mm/min exceeds {:.2} x the reference cutting \
                     feed of {:.1} mm/min",
                    feed_mm_min, ratio, reference_mm_min
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Plunging at a cutting feed overloads the flutes, which have almost no \
         room to clear chips when the cutter goes straight down."
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
        assert_eq!(
            ids,
            vec![
                AXIAL_DEPTH_ID,
                RADIAL_ENGAGEMENT_ID,
                MRR_ID,
                FLUTE_LENGTH_ID,
                STICKOUT_RATIO_ID,
                PLUNGE_FEED_ID,
            ]
        );
    }
}
