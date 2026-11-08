//! `clearance.rapid_at_depth`: a rapid ends below the safe clearance plane.

use std::collections::BTreeMap;

use nc_gcode::{MotionRecord, MoveKind};
use nc_stock::{InferenceBasis, StockSource};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Stable rule id.
const RULE_ID: &str = "clearance.rapid_at_depth";

/// Threshold key used for the clearance above the stock top.
const CLEARANCE_KEY: &str = "clearance_mm";

/// Returns the rule in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(RapidAtDepth)]
}

/// Checks every rapid that ends below the safe Z plane.
struct RapidAtDepth;

impl Rule for RapidAtDepth {
    fn id(&self) -> RuleId {
        RuleId::new(RULE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_stock(StockRequirement::InferredOk)
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        let clearance_mm = sanitized_threshold(
            cfg.threshold(CLEARANCE_KEY, model.tolerances.clearance_mm),
            model.tolerances.clearance_mm,
        );
        let safe_z_mm = model.stock.top_z_mm + clearance_mm;
        let mut findings = Vec::new();

        for motion in model.program.expanded_motions() {
            if motion.kind != MoveKind::Rapid {
                continue;
            }
            let start_z = motion.start_mm[2];
            let end_z = motion.end_mm[2];
            if !start_z.is_finite() || !end_z.is_finite() {
                continue;
            }
            // A rapid that only travels above the safe plane is not at depth.
            if start_z > safe_z_mm && end_z > safe_z_mm {
                continue;
            }
            // Exactly at safe Z is allowed; only a strictly lower endpoint fires.
            if end_z >= safe_z_mm {
                continue;
            }

            findings.push(finding(
                model.stock.source,
                model.stock.inference,
                &motion,
                safe_z_mm,
                clearance_mm,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Rapids below the stock top can drag the cutter through material at rapid \
         rate; the machine has no chance to react before the tool or part is \
         damaged."
    }
}

/// Builds one finding for one rapid below the safe plane.
fn finding(
    source: StockSource,
    inference: Option<InferenceBasis>,
    motion: &MotionRecord,
    safe_z_mm: f64,
    clearance_mm: f64,
) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "stock_source".to_string(),
        EvidenceValue::Text(stock_source_label(source).to_string()),
    );
    if let Some(basis) = inference {
        evidence.insert(
            "inference_basis".to_string(),
            EvidenceValue::Text(inference_label(basis).to_string()),
        );
    }
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    evidence.insert(
        "depth_mm".to_string(),
        EvidenceValue::Float(safe_z_mm - motion.end_mm[2]),
    );
    evidence.insert("threshold".to_string(), EvidenceValue::Float(clearance_mm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );

    Diagnostic {
        rule: RuleId::new(RULE_ID),
        severity: Severity::Warning,
        basis: Basis::Model,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "rapid ends at Z {:.3} mm, below safe Z {:.3} mm",
            motion.end_mm[2], safe_z_mm
        ),
        evidence,
    }
}

/// Keeps a malformed threshold from silently disabling the rule.
fn sanitized_threshold(value: f64, default: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        default
    }
}

/// Stable text label for stock provenance.
fn stock_source_label(source: StockSource) -> &'static str {
    match source {
        StockSource::Inferred => "inferred",
        StockSource::Explicit => "explicit",
    }
}

/// Stable text label for the inference basis.
fn inference_label(basis: InferenceBasis) -> &'static str {
    match basis {
        InferenceBasis::ZeroAtTop => "zero_at_top",
        InferenceBasis::AllCutsAboveZero => "all_cuts_above_zero",
        InferenceBasis::NoCuttingMoves => "no_cutting_moves",
    }
}
