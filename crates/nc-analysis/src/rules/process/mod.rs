//! `process.*` rules driven by the shared material-engagement pass.
//!
//! Every rule in this module is [`Basis::Model`]: it reads a 2.5D z-map
//! engagement estimate or a configured material range, never a physical
//! simulation. Rules that need user-supplied stock are skipped when the stock
//! was inferred, because inference is fitted to the cuts.

mod cutting;
mod material;
mod stock;

use std::collections::{BTreeMap, BTreeSet};

use nc_core::ToolField;
use nc_stock::{InferenceBasis, StockModel, StockSource};

use crate::config::RuleSettings;
use crate::engagement::{EngagementError, engagement};
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, EngagementData, PassEngagement, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Every rule in this module, in the catalog order of `docs/RULES.md`.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(material::rules());
    rules.extend(cutting::rules());
    rules.extend(stock::rules());
    rules
}

/// Reads a threshold, falling back to the documented default.
///
/// A non-finite or negative configured value falls back rather than silently
/// disabling a rule.
pub(crate) fn threshold(cfg: &RuleSettings, key: &str, default: f64) -> f64 {
    match cfg.thresholds.get(key).copied() {
        Some(value) if value.is_finite() && value >= 0.0 => value,
        _ => default,
    }
}

/// Keeps a finite, non-negative optional range endpoint.
pub(crate) fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

/// Builds the stable skip outcome used by every rule in this module.
pub(crate) fn skip(missing_input: impl Into<String>, reason: impl Into<String>) -> RuleOutcome {
    RuleOutcome::Skipped {
        missing_input: missing_input.into(),
        reason: reason.into(),
    }
}

/// Stock provenance evidence shared by every stock-dependent rule.
pub(crate) fn stock_evidence(stock: &StockModel) -> BTreeMap<String, EvidenceValue> {
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
pub(crate) fn stock_skip(stock: &StockModel) -> Option<RuleOutcome> {
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
    Some(skip("stock", "stock extents are degenerate"))
}

/// Returns the shared engagement pass, or the skip that explains its absence.
pub(crate) fn engagement_data<'a>(
    model: &'a AnalysisModel<'_>,
) -> Result<&'a EngagementData, RuleOutcome> {
    match engagement(model) {
        Ok(data) => Ok(data),
        Err(EngagementError::Stock) => Err(skip("stock", "stock extents are degenerate")),
        Err(error) => Err(skip(
            "stock",
            format!("material-engagement grid could not be built: {error}"),
        )),
    }
}

/// Adds `tool_source` when one of `fields` came from a program comment.
pub(crate) fn insert_tool_source(
    evidence: &mut BTreeMap<String, EvidenceValue>,
    model: &AnalysisModel<'_>,
    tool: u32,
    fields: &[ToolField],
) {
    for field in fields {
        if let Some(source) = model.tools.source_of(tool, *field)
            && source.is_comment()
        {
            evidence.insert(
                "tool_source".to_string(),
                EvidenceValue::Text(source.label()),
            );
            return;
        }
    }
}

/// Tool numbers with at least one stored motion, sorted and deduplicated.
pub(crate) fn used_tools(model: &AnalysisModel<'_>) -> Vec<u32> {
    let mut tools = BTreeSet::new();
    for motion in &model.program.motions {
        tools.insert(motion.tool);
    }
    tools.into_iter().collect()
}

/// Returns a skip when a tool used by the program has no usable `field`.
///
/// The engine already checks field presence; this catches present but invalid
/// values such as a zero diameter or a negative flute length, so a rule never
/// silently evaluates against nonsense.
pub(crate) fn invalid_tool_skip(
    model: &AnalysisModel<'_>,
    field: ToolField,
    description: &str,
) -> Option<RuleOutcome> {
    for tool_number in used_tools(model) {
        let tool = model.tools.get(tool_number);
        let usable = match field {
            ToolField::Diameter => tool
                .and_then(|tool| tool.diameter_mm)
                .is_some_and(|value| value.is_finite() && value > 0.0),
            ToolField::Flutes => tool
                .and_then(|tool| tool.flutes)
                .is_some_and(|value| value > 0),
            ToolField::FluteLength => tool
                .and_then(|tool| tool.flute_length_mm)
                .is_some_and(|value| value.is_finite() && value > 0.0),
            ToolField::Stickout => tool
                .and_then(|tool| tool.stickout_mm)
                .is_some_and(|value| value.is_finite() && value > 0.0),
            _ => true,
        };
        if !usable {
            return Some(skip(
                format!("tool.{}:T{}", field.key(), tool_number),
                format!("tool T{tool_number} has no usable {description}"),
            ));
        }
    }
    None
}

/// Tool number and known diameter for one pass.
pub(crate) fn pass_diameter(
    model: &AnalysisModel<'_>,
    pass: &PassEngagement,
) -> Option<(u32, f64)> {
    let motion = model.program.motions.get(pass.motion_index)?;
    let diameter = model.tools.get(motion.tool)?.diameter_mm?;
    if diameter.is_finite() && diameter > 0.0 {
        Some((motion.tool, diameter))
    } else {
        None
    }
}

/// Tool number for one pass.
pub(crate) fn pass_tool_number(model: &AnalysisModel<'_>, pass: &PassEngagement) -> Option<u32> {
    model
        .program
        .motions
        .get(pass.motion_index)
        .map(|motion| motion.tool)
}

/// First cutting source line for a tool, or the first motion line when the tool
/// never cuts. `None` when the tool has no stored motion.
pub(crate) fn first_tool_line(model: &AnalysisModel<'_>, tool: u32) -> Option<usize> {
    let mut fallback = None;
    for motion in &model.program.motions {
        if motion.tool != tool {
            continue;
        }
        if motion.kind.is_cutting() {
            return Some(motion.line);
        }
        fallback.get_or_insert(motion.line);
    }
    fallback
}

/// Builds a model-basis diagnostic in this module's style.
pub(crate) fn diagnostic(
    rule: &'static str,
    severity: Severity,
    line: usize,
    message: String,
    evidence: BTreeMap<String, EvidenceValue>,
) -> Diagnostic {
    Diagnostic {
        rule: RuleId::new(rule),
        severity,
        basis: Basis::Model,
        line,
        column: 0,
        line_end: line,
        message,
        evidence,
    }
}

/// Stable label for stock provenance.
fn stock_source_label(source: StockSource) -> &'static str {
    match source {
        StockSource::Inferred => "inferred",
        StockSource::Explicit => "explicit",
    }
}

/// Stable label for the inference basis.
fn inference_label(basis: InferenceBasis) -> &'static str {
    match basis {
        InferenceBasis::ZeroAtTop => "zero_at_top",
        InferenceBasis::AllCutsAboveZero => "all_cuts_above_zero",
        InferenceBasis::NoCuttingMoves => "no_cutting_moves",
    }
}

/// Tool fields used by the requirements of several rules.
pub(crate) const DIAMETER: &[ToolField] = &[ToolField::Diameter];

/// Tool fields for the stickout-to-diameter rule.
pub(crate) const STICKOUT_AND_DIAMETER: &[ToolField] = &[ToolField::Stickout, ToolField::Diameter];
