//! The deterministic analysis engine.
//!
//! The caller parses the program and resolves tool hints into
//! [`AnalysisConfig::tools`] before calling [`analyze`]. The engine never
//! re-parses the source and never invents a missing input: a rule whose
//! [`Requirements`] are unmet is reported in [`Coverage::skipped`] instead of
//! being evaluated.

use std::collections::{BTreeMap, BTreeSet};

use nc_core::{Dialect, Tool, ToolField, ToolTable};
use nc_gcode::Program;
use nc_stock::{StockModel, StockSource};
use sha2::{Digest, Sha256};

use crate::config::{AnalysisConfig, RuleSettings};
use crate::report::{
    Coverage, Diagnostic, InputInfo, InventoryField, MAX_FINDINGS_PER_RULE, MAX_FINDINGS_TOTAL,
    Report, SkippedRule, ToolInventoryEntry, normalize_diagnostics, sort_skips,
};
use crate::rule::{AnalysisModel, Rule, RuleOutcome, StockRequirement};
use crate::types::EvidenceValue;
use crate::{REPORT_SCHEMA, RULESET_VERSION};

/// Everything about the run that is not the program or the configuration.
///
/// `file_name` may be a path supplied by a file dialog; only its final
/// component reaches the report. `source` is the exact text that produced
/// `program`, because the report hashes those bytes and counts their lines.
/// The caller must pass the dialect selected by the effective parser config.
#[derive(Debug, Clone)]
pub struct AnalysisRequest<'a> {
    /// Input file name or path. Only the file name is serialized.
    pub file_name: &'a str,
    /// Exact source text that was parsed.
    pub source: &'a str,
    /// Resolved configuration; tool hints are already merged into `tools`.
    pub config: &'a AnalysisConfig,
    /// Dialect selected by the effective parser configuration.
    pub dialect: Dialect,
    /// Ruleset version recorded in the report.
    pub ruleset_version: &'a str,
    /// `true` when the dialect was not configured and `Generic` was assumed.
    pub dialect_assumed: bool,
}

impl<'a> AnalysisRequest<'a> {
    /// Builds a request with the workspace ruleset version.
    #[must_use]
    pub fn new(
        file_name: &'a str,
        source: &'a str,
        config: &'a AnalysisConfig,
        dialect: Dialect,
    ) -> Self {
        Self {
            file_name,
            source,
            config,
            dialect,
            ruleset_version: RULESET_VERSION,
            dialect_assumed: false,
        }
    }

    /// Records whether `Generic` was assumed instead of configured.
    #[must_use]
    pub fn with_dialect_assumed(mut self, assumed: bool) -> Self {
        self.dialect_assumed = assumed;
        self
    }

    /// Overrides the ruleset version recorded in the report.
    #[must_use]
    pub fn with_ruleset_version(mut self, ruleset_version: &'a str) -> Self {
        self.ruleset_version = ruleset_version;
        self
    }
}

/// An ordered collection of rules.
///
/// Registry order is evaluation order. Report collections are sorted after
/// evaluation, so changing registry order never changes the report bytes.
#[derive(Default)]
pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry from already boxed rules.
    #[must_use]
    pub fn from_rules(rules: impl IntoIterator<Item = Box<dyn Rule>>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
        }
    }

    /// Appends one rule.
    pub fn register(&mut self, rule: Box<dyn Rule>) {
        self.rules.push(rule);
    }

    /// Number of rules, including disabled ones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Returns `true` when the registry has no rules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Iterates rules in evaluation order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Rule> + '_ {
        self.rules.iter().map(AsRef::as_ref)
    }
}

/// Runs every rule that has its required inputs and assembles a report.
///
/// The result is deterministic for the same program, configuration, source,
/// and ruleset version. Findings are normalized to the evaluating rule's id,
/// basis, and effective severity, then sorted, deduplicated, and capped.
///
/// An explicit stock configuration that fails validation is treated as an
/// inferred stock and logged. The report records the stock actually used, so a
/// rule that requires explicit stock is skipped with `stock.explicit`.
#[must_use]
pub fn analyze(program: &Program, request: &AnalysisRequest<'_>, rules: &RuleRegistry) -> Report {
    let config = request.config;
    let tolerances = config.tolerances.sanitized();
    let stock = resolve_stock(config, program);
    let stock_source = stock.source;
    let line_count = source_line_count(request.source);
    let tools_with_motions = tools_with_motions(program);
    let model = AnalysisModel::new(
        program,
        stock,
        &config.tools,
        config.machine.as_ref(),
        config.material.as_ref(),
        &tolerances,
    )
    .with_dialect_assumed(request.dialect_assumed);

    let mut diagnostics = Vec::new();
    let mut skipped = Vec::new();
    let mut evaluated = 0usize;

    for rule in rules.iter() {
        let id = rule.id();
        let settings = config.rules.settings(&id);
        if !settings.enabled {
            skipped.push(SkippedRule::new(
                id,
                "rule.disabled",
                "rule is disabled in configuration",
            ));
            continue;
        }

        let missing = missing_requirements(rule, &model, &tools_with_motions, stock_source);
        if !missing.is_empty() {
            skipped.extend(missing);
            continue;
        }

        match rule.evaluate(&model, &settings) {
            RuleOutcome::Findings(mut findings) => {
                evaluated += 1;
                normalize_findings(&mut findings, rule, &settings, line_count);
                diagnostics.append(&mut findings);
            }
            RuleOutcome::Skipped {
                missing_input,
                reason,
            } => skipped.push(SkippedRule::new(id, missing_input, reason)),
        }
    }

    normalize_diagnostics(&mut diagnostics);
    let truncated = apply_caps(&mut diagnostics);
    sort_skips(&mut skipped);

    Report {
        schema: REPORT_SCHEMA.to_string(),
        ruleset_version: request.ruleset_version.to_string(),
        input: InputInfo {
            file_name: file_name_only(request.file_name),
            sha256: sha256_hex(request.source.as_bytes()),
            line_count,
            dialect: request.dialect,
            geometry_uncertain_after: program.geometry_uncertain_after(),
        },
        config_digest: config_digest(config),
        stock_source,
        tool_inventory: tool_inventory(program, &config.tools),
        diagnostics,
        truncated,
        coverage: Coverage {
            evaluated,
            total: rules.len(),
            skipped,
        },
    }
}

/// Resolves configured stock, falling back to inference after a bad config.
fn resolve_stock(config: &AnalysisConfig, program: &Program) -> StockModel {
    match config.stock {
        Some(stock) => match stock.to_stock() {
            Ok(model) => model,
            Err(error) => {
                tracing::warn!(%error, "explicit stock is invalid; using inferred stock");
                StockModel::infer(program)
            }
        },
        None => StockModel::infer(program),
    }
}

/// Sorts findings and records how many each rule lost to the caps.
fn apply_caps(diagnostics: &mut Vec<Diagnostic>) -> BTreeMap<String, usize> {
    let mut truncated = BTreeMap::new();
    let mut per_rule: BTreeMap<String, usize> = BTreeMap::new();
    let mut kept = Vec::with_capacity(diagnostics.len().min(MAX_FINDINGS_TOTAL));

    for diagnostic in diagnostics.drain(..) {
        let rule = diagnostic.rule.as_str().to_string();
        let count = per_rule.entry(rule).or_insert(0);
        if *count >= MAX_FINDINGS_PER_RULE {
            increment(&mut truncated, diagnostic.rule.as_str());
            continue;
        }
        *count += 1;
        kept.push(diagnostic);
    }

    if kept.len() > MAX_FINDINGS_TOTAL {
        let dropped = kept.split_off(MAX_FINDINGS_TOTAL);
        for diagnostic in dropped {
            increment(&mut truncated, diagnostic.rule.as_str());
        }
    }

    *diagnostics = kept;
    truncated
}

fn increment(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_insert(0) += 1;
}

/// Applies the effective rule id, basis, severity, and source-line bounds.
fn normalize_findings(
    findings: &mut [Diagnostic],
    rule: &dyn Rule,
    settings: &RuleSettings,
    line_count: usize,
) {
    let rule_id = rule.id();
    let basis = rule.basis();
    let severity = settings.severity.unwrap_or_else(|| rule.default_severity());
    for diagnostic in findings {
        diagnostic.rule = rule_id.clone();
        diagnostic.basis = basis;
        diagnostic.severity = severity;
        if line_count == 0 {
            diagnostic.line = 1;
            diagnostic.line_end = 1;
        } else {
            diagnostic.line = diagnostic.line.clamp(1, line_count);
            diagnostic.line_end = diagnostic.line_end.clamp(diagnostic.line, line_count);
        }
    }
}

/// Returns one skip record per missing input.
///
/// Tool fields are checked for every tool that has at least one motion; a tool
/// configured but never used cannot make a rule skip. When several inputs are
/// missing, each one is recorded, sorted later by `(rule id, missing_input)`.
fn missing_requirements(
    rule: &dyn Rule,
    model: &AnalysisModel<'_>,
    tools_with_motions: &[u32],
    stock_source: StockSource,
) -> Vec<SkippedRule> {
    let requirements = rule.requirements();
    let id = rule.id();
    let mut missing = Vec::new();

    if requirements.stock == StockRequirement::Explicit && stock_source != StockSource::Explicit {
        missing.push(SkippedRule::new(
            id.clone(),
            "stock.explicit",
            "explicit stock is required, but the stock was inferred",
        ));
    }
    if requirements.machine && model.machine.is_none() {
        missing.push(SkippedRule::new(
            id.clone(),
            "machine",
            "machine limits are required",
        ));
    }
    if requirements.material && model.material.is_none() {
        missing.push(SkippedRule::new(
            id.clone(),
            "material",
            "a material preset is required",
        ));
    }

    let fields: BTreeSet<ToolField> = requirements.tool_fields.iter().copied().collect();
    for field in fields {
        for tool_number in tools_with_motions {
            if tool_has_field(model.tools.get(*tool_number), field) {
                continue;
            }
            missing.push(SkippedRule::new(
                id.clone(),
                format!("tool.{}:T{tool_number}", field.key()),
                format!("tool T{tool_number} has no {}", field_label(field)),
            ));
        }
    }

    missing
}

/// Returns `true` when the tool has a known value for `field`.
fn tool_has_field(tool: Option<&Tool>, field: ToolField) -> bool {
    let Some(tool) = tool else {
        return false;
    };
    match field {
        ToolField::Name => tool.name.is_some(),
        ToolField::Shape => tool.shape != nc_core::ToolShape::Unknown,
        ToolField::Diameter => tool.diameter_mm.is_some(),
        ToolField::Flutes => tool.flutes.is_some(),
        ToolField::Stickout => tool.stickout_mm.is_some(),
        ToolField::FluteLength => tool.flute_length_mm.is_some(),
        ToolField::TipAngle => tool.tip_angle_deg.is_some(),
        ToolField::CornerRadius => tool.corner_radius_mm.is_some(),
        ToolField::ShankDiameter => tool.shank_diameter_mm.is_some(),
        ToolField::HolderDiameter => tool.holder_diameter_mm.is_some(),
        ToolField::RightHand => tool.right_hand.is_some(),
    }
}

/// Human phrase used in a skip reason.
fn field_label(field: ToolField) -> &'static str {
    match field {
        ToolField::Name => "name",
        ToolField::Shape => "shape",
        ToolField::Diameter => "diameter",
        ToolField::Flutes => "flute count",
        ToolField::Stickout => "stickout",
        ToolField::FluteLength => "flute length",
        ToolField::TipAngle => "tip angle",
        ToolField::CornerRadius => "corner radius",
        ToolField::ShankDiameter => "shank diameter",
        ToolField::HolderDiameter => "holder diameter",
        ToolField::RightHand => "handedness",
    }
}

/// Tool numbers that produced at least one motion, sorted and deduplicated.
fn tools_with_motions(program: &Program) -> Vec<u32> {
    let mut tools = BTreeSet::new();
    for motion in &program.motions {
        tools.insert(motion.tool);
    }
    tools.into_iter().collect()
}

/// Builds the tool inventory from the resolved tool table and motion stream.
///
/// One record per tool with at least one motion, sorted by tool number. The
/// motion count counts stored records, one per canned-cycle hole.
fn tool_inventory(program: &Program, tools: &ToolTable) -> Vec<ToolInventoryEntry> {
    let mut usage: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for motion in &program.motions {
        let entry = usage.entry(motion.tool).or_insert((motion.line, 0));
        entry.0 = entry.0.min(motion.line);
        entry.1 += 1;
    }
    usage
        .into_iter()
        .map(|(tool, (first_line, motion_count))| ToolInventoryEntry {
            tool,
            first_line,
            motion_count,
            fields: inventory_fields(tools.get(tool)),
        })
        .collect()
}

/// Known fields of one resolved tool, with provenance labels.
fn inventory_fields(tool: Option<&Tool>) -> BTreeMap<String, InventoryField> {
    let Some(tool) = tool else {
        return BTreeMap::new();
    };
    let mut fields = BTreeMap::new();
    let mut put = |field: ToolField, key: &str, value: EvidenceValue| {
        let source = tool
            .sources
            .get(&field)
            .map(|source| source.label())
            .unwrap_or_else(|| "config".to_string());
        fields.insert(key.to_string(), InventoryField { value, source });
    };

    if let Some(name) = &tool.name {
        put(ToolField::Name, "name", EvidenceValue::Text(name.clone()));
    }
    if tool.shape != nc_core::ToolShape::Unknown {
        put(
            ToolField::Shape,
            "shape",
            EvidenceValue::Text(tool.shape.label().to_string()),
        );
    }
    if let Some(diameter_mm) = tool.diameter_mm {
        put(
            ToolField::Diameter,
            "diameter_mm",
            EvidenceValue::Float(diameter_mm),
        );
    }
    if let Some(flutes) = tool.flutes {
        put(
            ToolField::Flutes,
            "flutes",
            EvidenceValue::Int(i64::from(flutes)),
        );
    }
    if let Some(stickout_mm) = tool.stickout_mm {
        put(
            ToolField::Stickout,
            "stickout_mm",
            EvidenceValue::Float(stickout_mm),
        );
    }
    if let Some(flute_length_mm) = tool.flute_length_mm {
        put(
            ToolField::FluteLength,
            "flute_length_mm",
            EvidenceValue::Float(flute_length_mm),
        );
    }
    if let Some(tip_angle_deg) = tool.tip_angle_deg {
        put(
            ToolField::TipAngle,
            "tip_angle_deg",
            EvidenceValue::Float(tip_angle_deg),
        );
    }
    if let Some(corner_radius_mm) = tool.corner_radius_mm {
        put(
            ToolField::CornerRadius,
            "corner_radius_mm",
            EvidenceValue::Float(corner_radius_mm),
        );
    }
    if let Some(shank_diameter_mm) = tool.shank_diameter_mm {
        put(
            ToolField::ShankDiameter,
            "shank_diameter_mm",
            EvidenceValue::Float(shank_diameter_mm),
        );
    }
    if let Some(holder_diameter_mm) = tool.holder_diameter_mm {
        put(
            ToolField::HolderDiameter,
            "holder_diameter_mm",
            EvidenceValue::Float(holder_diameter_mm),
        );
    }
    if let Some(right_hand) = tool.right_hand {
        put(
            ToolField::RightHand,
            "right_hand",
            EvidenceValue::Bool(right_hand),
        );
    }
    fields
}

/// Returns only the final path component, handling both separator styles.
fn file_name_only(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    name.to_string()
}

/// Counts source lines the same way the parser's `SourceLines` does.
fn source_line_count(source: &str) -> usize {
    if source.is_empty() {
        return 0;
    }
    let bytes = source.as_bytes();
    let mut count = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' || byte == b'\r' {
            count += 1;
            if index + 1 < bytes.len()
                && matches!((byte, bytes[index + 1]), (b'\r', b'\n') | (b'\n', b'\r'))
            {
                index += 1;
            }
            start = index + 1;
        }
        index += 1;
    }
    if start < bytes.len() {
        count += 1;
    }
    count
}

/// SHA-256 over a canonical serialization of the configuration.
///
/// `serde_json` preserves the `BTreeMap` order used by the config. A
/// non-serializable config (for example a non-finite float) hashes a stable
/// marker instead of panicking; config loading rejects such values earlier.
fn config_digest(config: &AnalysisConfig) -> String {
    let bytes = match serde_json::to_vec(config) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, "analysis config is not serializable; using digest marker");
            b"{\"error\":\"config-not-serializable\"}".to_vec()
        }
    };
    sha256_hex(&bytes)
}

/// Lowercase SHA-256 hex.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let high = char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0');
        let low = char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0');
        hex.push(high);
        hex.push(low);
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_count_matches_parser_line_splitting() {
        assert_eq!(source_line_count(""), 0);
        assert_eq!(source_line_count("a"), 1);
        assert_eq!(source_line_count("a\n"), 1);
        assert_eq!(source_line_count("a\n\n"), 2);
        assert_eq!(source_line_count("a\r\nb"), 2);
        assert_eq!(source_line_count("a\rb\nc"), 3);
        assert_eq!(source_line_count("a\n\rb"), 2);
    }

    #[test]
    fn file_name_strips_unix_and_windows_directories() {
        assert_eq!(file_name_only("part.nc"), "part.nc");
        assert_eq!(file_name_only("/tmp/x/part.nc"), "part.nc");
        assert_eq!(file_name_only(r"C:\jobs\part.nc"), "part.nc");
        assert_eq!(file_name_only("C:/jobs/part.nc"), "part.nc");
        assert_eq!(file_name_only("/tmp/x/"), "x");
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn tool_field_labels_are_stable() {
        assert_eq!(field_label(ToolField::Flutes), "flute count");
        assert_eq!(field_label(ToolField::RightHand), "handedness");
    }
}
