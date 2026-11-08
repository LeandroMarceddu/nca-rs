//! `travel.*` rules beyond the vertical-slice envelope check.
//!
//! `travel.z_below_table` checks motion endpoints against the configured
//! machine table Z. `travel.stock_bounds` checks
//! cutting endpoints against explicit stock. Both use the stored motion stream,
//! so each finding maps to one source line.

use std::collections::BTreeMap;

use nc_gcode::MotionRecord;
use nc_stock::{StockModel, StockSource};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Stable rule id for the table Z check.
const Z_BELOW_TABLE_ID: &str = "travel.z_below_table";

/// Stable rule id for the explicit-stock footprint check.
const STOCK_BOUNDS_ID: &str = "travel.stock_bounds";

/// Every rule in this module, in catalog order.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(ZBelowTable), Box::new(StockBounds)]
}

/// Checks every motion endpoint against the configured table Z.
struct ZBelowTable;

impl Rule for ZBelowTable {
    fn id(&self) -> RuleId {
        RuleId::new(Z_BELOW_TABLE_ID)
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
            return machine_skip("machine limits are required");
        };
        if machine.validate().is_err() {
            return machine_skip("configured machine travel limits are invalid");
        }
        let table_z_mm = machine.travel_min_mm[2];

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            let end_z_mm = motion.end_mm[2];
            if !end_z_mm.is_finite() {
                continue;
            }
            // Exactly at the table Z is allowed; only a strictly lower endpoint
            // fires.
            if end_z_mm >= table_z_mm {
                continue;
            }
            findings.push(z_below_table_finding(motion, end_z_mm, table_z_mm));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A Z target below the machine table commands the axis into its physical \
         stop or the table itself, which can stall the machine, lose position, \
         or break the cutter before the controller can react."
    }
}

/// Checks cutting endpoints against the explicit stock XY footprint.
struct StockBounds;

impl Rule for StockBounds {
    fn id(&self) -> RuleId {
        RuleId::new(STOCK_BOUNDS_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_stock(StockRequirement::Explicit)
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        let source = model.stock.source;
        let [min_x, min_y, _] = model.stock.min_mm;
        let [max_x, max_y, _] = model.stock.max_mm;

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            if !motion.kind.is_cutting() {
                continue;
            }
            let x = motion.end_mm[0];
            let y = motion.end_mm[1];
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            // Exactly on the stock edge is still inside the footprint.
            let inside = x >= min_x && x <= max_x && y >= min_y && y <= max_y;
            if inside {
                continue;
            }
            findings.push(stock_bounds_finding(source, motion));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A cutting move that ends outside the stock is cutting air where no \
         material is expected, usually from a wrong offset or a missing clamp \
         check; the pass can gouge the fixture or leave the part undersized."
    }
}

/// Builds one finding for an endpoint below the table.
fn z_below_table_finding(motion: &MotionRecord, value_mm: f64, min_mm: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert("axis".to_string(), EvidenceValue::Text("Z".to_string()));
    evidence.insert("value_mm".to_string(), EvidenceValue::Float(value_mm));
    evidence.insert("min_mm".to_string(), EvidenceValue::Float(min_mm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));

    Diagnostic {
        rule: RuleId::new(Z_BELOW_TABLE_ID),
        severity: Severity::Error,
        basis: Basis::Exact,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!("Z endpoint {value_mm:.3} mm is below the machine table Z {min_mm:.3} mm"),
        evidence,
    }
}

/// Builds one finding for a cutting endpoint outside the stock footprint.
fn stock_bounds_finding(source: StockSource, motion: &MotionRecord) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "stock_source".to_string(),
        EvidenceValue::Text(stock_source_label(source).to_string()),
    );
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3(motion.start_mm),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );

    Diagnostic {
        rule: RuleId::new(STOCK_BOUNDS_ID),
        severity: Severity::Warning,
        basis: Basis::Exact,
        line: motion.line,
        column: 0,
        line_end: motion.line,
        message: format!(
            "cutting endpoint [{:.3}, {:.3}] mm is outside the stock XY footprint",
            motion.end_mm[0], motion.end_mm[1]
        ),
        evidence,
    }
}

/// Returns a skip outcome when a machine is configured but unusable.
fn machine_skip(reason: &str) -> RuleOutcome {
    RuleOutcome::Skipped {
        missing_input: "machine".to_string(),
        reason: reason.to_string(),
    }
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
