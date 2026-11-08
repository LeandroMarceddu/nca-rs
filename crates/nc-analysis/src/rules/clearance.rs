//! `clearance.*` rules beyond the vertical-slice rapid-at-depth check.
//!
//! All three rules read the stored motion stream, not the expanded canned-cycle
//! sub-moves: a peck retract is already modeled as a `Retract` and must not be
//! mistaken for a rapid that crosses the stock.

use std::collections::BTreeMap;

use nc_gcode::{MotionRecord, MoveKind};
use nc_stock::{InferenceBasis, StockModel, StockSource};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Stable rule id for rapids that cross the stock footprint below its top.
const RAPID_THROUGH_STOCK_ID: &str = "clearance.rapid_through_stock";

/// Stable rule id for retracts that end below the stock top.
const RETRACT_BELOW_STOCK_ID: &str = "clearance.retract_below_stock";

/// Stable rule id for a program that ends with the tool in material.
const TOOL_END_IN_MATERIAL_ID: &str = "clearance.tool_end_in_material";

/// Every rule in this module, in catalog order.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(RapidThroughStock),
        Box::new(RetractBelowStock),
        Box::new(ToolEndInMaterial),
    ]
}

/// Checks XY rapids that cross the stock footprint while below its top.
struct RapidThroughStock;

impl Rule for RapidThroughStock {
    fn id(&self) -> RuleId {
        RuleId::new(RAPID_THROUGH_STOCK_ID)
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

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        let stock = &model.stock;
        let mut findings = Vec::new();

        for motion in &model.program.motions {
            if motion.kind != MoveKind::Rapid {
                continue;
            }
            if !motion
                .start_mm
                .iter()
                .chain(motion.end_mm.iter())
                .all(|value| value.is_finite())
            {
                continue;
            }
            // A pure Z rapid is a plunge or retract, not a sideways move
            // through material.
            if motion.start_mm[0] == motion.end_mm[0] && motion.start_mm[1] == motion.end_mm[1] {
                continue;
            }
            if !crosses_footprint_below_top(motion, stock) {
                continue;
            }
            findings.push(rapid_through_stock_finding(stock, motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A rapid that crosses the stock below its top drags the cutter sideways \
         through material with no feed control; the machine cannot react before \
         the tool snaps or the part is torn."
    }
}

/// Checks retracts whose end Z sits below the stock top.
struct RetractBelowStock;

impl Rule for RetractBelowStock {
    fn id(&self) -> RuleId {
        RuleId::new(RETRACT_BELOW_STOCK_ID)
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

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        let stock = &model.stock;
        let top_z_mm = stock.top_z_mm;
        let mut findings = Vec::new();

        for motion in &model.program.motions {
            if motion.kind != MoveKind::Retract {
                continue;
            }
            let end_z_mm = motion.end_mm[2];
            if !end_z_mm.is_finite() || end_z_mm >= top_z_mm {
                continue;
            }
            findings.push(retract_below_stock_finding(stock, motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A retract that stops below the stock top leaves the cutter buried in \
         material; the next move can drag it sideways and break the flutes."
    }
}

/// Checks the program's final tool pose against the stock top.
struct ToolEndInMaterial;

impl Rule for ToolEndInMaterial {
    fn id(&self) -> RuleId {
        RuleId::new(TOOL_END_IN_MATERIAL_ID)
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

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        let Some(last) = model.program.motions.last() else {
            return RuleOutcome::Findings(Vec::new());
        };
        let end_z_mm = last.end_mm[2];
        if !end_z_mm.is_finite() || end_z_mm >= model.stock.top_z_mm {
            return RuleOutcome::Findings(Vec::new());
        }
        RuleOutcome::Findings(vec![tool_end_in_material_finding(&model.stock, last)])
    }

    fn why_it_matters(&self) -> &'static str {
        "Ending the program with the tool inside the stock leaves the cutter in \
         contact with the part, so unclamping or lifting the tool can break it \
         or gouge the finished surface."
    }
}

/// Returns `true` when the segment has a point strictly inside the stock
/// footprint and strictly below the stock top.
///
/// The intersection uses open intervals, so a segment that only runs along a
/// stock edge or touches a corner without crossing the interior does not count.
fn crosses_footprint_below_top(motion: &MotionRecord, stock: &StockModel) -> bool {
    let [x0, y0, z0] = motion.start_mm;
    let [x1, y1, z1] = motion.end_mm;

    let Some((x_lo, x_hi)) = open_axis_interval(x0, x1, stock.min_mm[0], stock.max_mm[0]) else {
        return false;
    };
    let Some((y_lo, y_hi)) = open_axis_interval(y0, y1, stock.min_mm[1], stock.max_mm[1]) else {
        return false;
    };
    let Some((z_lo, z_hi)) = below_top_interval(z0, z1, stock.top_z_mm) else {
        return false;
    };

    let lo = x_lo.max(y_lo).max(z_lo).max(0.0);
    let hi = x_hi.min(y_hi).min(z_hi).min(1.0);
    lo < hi
}

/// Open parameter interval where a linear axis value is strictly between the
/// two bounds.
///
/// A segment that stays exactly on a bound has no such interval.
fn open_axis_interval(start: f64, end: f64, min: f64, max: f64) -> Option<(f64, f64)> {
    if start == end {
        return (start > min && start < max).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let first = (min - start) / (end - start);
    let second = (max - start) / (end - start);
    Some((first.min(second), first.max(second)))
}

/// Open parameter interval where a linear Z value is strictly below `top_z`.
fn below_top_interval(start_z: f64, end_z: f64, top_z: f64) -> Option<(f64, f64)> {
    if start_z == end_z {
        return (start_z < top_z).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let crossing = (top_z - start_z) / (end_z - start_z);
    if end_z > start_z {
        Some((f64::NEG_INFINITY, crossing))
    } else {
        Some((crossing, f64::INFINITY))
    }
}

/// Builds one finding for a rapid that crosses the stock below its top.
fn rapid_through_stock_finding(stock: &StockModel, motion: &MotionRecord) -> Diagnostic {
    let mut evidence = stock_evidence(stock);
    let deepest_z_mm = motion.start_mm[2].min(motion.end_mm[2]);
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3(motion.start_mm),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    evidence.insert(
        "depth_mm".to_string(),
        EvidenceValue::Float(stock.top_z_mm - deepest_z_mm),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );

    Diagnostic {
        rule: RuleId::new(RAPID_THROUGH_STOCK_ID),
        severity: Severity::Warning,
        basis: Basis::Model,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "rapid crosses the stock footprint at Z {deepest_z_mm:.3} mm, below the stock top {:.3} mm",
            stock.top_z_mm
        ),
        evidence,
    }
}

/// Builds one finding for a retract that ends below the stock top.
fn retract_below_stock_finding(stock: &StockModel, motion: &MotionRecord) -> Diagnostic {
    let mut evidence = stock_evidence(stock);
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    evidence.insert(
        "depth_mm".to_string(),
        EvidenceValue::Float(stock.top_z_mm - motion.end_mm[2]),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );

    Diagnostic {
        rule: RuleId::new(RETRACT_BELOW_STOCK_ID),
        severity: Severity::Warning,
        basis: Basis::Model,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "retract ends at Z {:.3} mm, below the stock top {:.3} mm",
            motion.end_mm[2], stock.top_z_mm
        ),
        evidence,
    }
}

/// Builds one finding for a program that ends with the tool in material.
fn tool_end_in_material_finding(stock: &StockModel, motion: &MotionRecord) -> Diagnostic {
    let mut evidence = stock_evidence(stock);
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    evidence.insert(
        "depth_mm".to_string(),
        EvidenceValue::Float(stock.top_z_mm - motion.end_mm[2]),
    );
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );

    Diagnostic {
        rule: RuleId::new(TOOL_END_IN_MATERIAL_ID),
        severity: Severity::Warning,
        basis: Basis::Model,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "program ends with the tool at Z {:.3} mm, below the stock top {:.3} mm",
            motion.end_mm[2], stock.top_z_mm
        ),
        evidence,
    }
}

/// Stock provenance evidence shared by every rule in this module.
fn stock_evidence(stock: &StockModel) -> BTreeMap<String, EvidenceValue> {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "stock_source".to_string(),
        EvidenceValue::Text(stock_source_label(stock.source).to_string()),
    );
    if let Some(basis) = stock.inference {
        evidence.insert(
            "inference_basis".to_string(),
            EvidenceValue::Text(inference_label(basis).to_string()),
        );
    }
    evidence
}

/// Returns a skip outcome when the stock box is non-finite or degenerate.
fn stock_skip(stock: &StockModel) -> Option<RuleOutcome> {
    let finite = stock
        .min_mm
        .iter()
        .chain(stock.max_mm.iter())
        .chain(std::iter::once(&stock.top_z_mm))
        .all(|value| value.is_finite());
    let ordered = (0..3).all(|axis| stock.min_mm[axis] < stock.max_mm[axis]);
    if finite && ordered {
        return None;
    }
    Some(RuleOutcome::Skipped {
        missing_input: "stock".to_string(),
        reason: "stock extents are degenerate".to_string(),
    })
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
