//! Report types. Field order and collection order are the JSON contract.

use std::collections::BTreeMap;

use nc_core::Dialect;
use nc_stock::StockSource;
use serde::{Deserialize, Serialize};

use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Maximum findings retained for one rule after deduplication.
pub const MAX_FINDINGS_PER_RULE: usize = 100;

/// Maximum findings retained in one report across all rules.
pub const MAX_FINDINGS_TOTAL: usize = 1000;

/// One finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable rule id.
    pub rule: RuleId,
    /// Effective severity after configuration overrides.
    pub severity: Severity,
    /// Exact or model basis.
    pub basis: Basis,
    /// 1-based source line.
    pub line: usize,
    /// 0-based column, or `0` for the whole line.
    pub column: usize,
    /// Last source line the finding spans.
    pub line_end: usize,
    /// Plain-language explanation.
    pub message: String,
    /// Typed evidence values, sorted by key.
    pub evidence: BTreeMap<String, EvidenceValue>,
}

/// A rule that was not evaluated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedRule {
    /// Stable rule id.
    pub rule: RuleId,
    /// Stable missing-input key, e.g. `tool.flutes:T3` or `stock.explicit`.
    pub missing_input: String,
    /// Human reason naming the missing input.
    pub reason: String,
}

impl SkippedRule {
    /// Creates a skip record.
    #[must_use]
    pub fn new(rule: RuleId, missing_input: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            rule,
            missing_input: missing_input.into(),
            reason: reason.into(),
        }
    }
}

/// Evaluated and skipped rule counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    /// Rules that returned `Findings`, including zero findings.
    pub evaluated: usize,
    /// Rules in the registry, including disabled ones.
    pub total: usize,
    /// Inputs that prevented evaluation, sorted by `(rule id, missing_input)`.
    ///
    /// A rule normally contributes one record. A rule that needs a tool field
    /// missing from several used tools contributes one record per tool; use
    /// [`Coverage::skipped_rule_count`] for the rule-level count.
    pub skipped: Vec<SkippedRule>,
}

impl Coverage {
    /// Number of skip records, which may exceed the number of skipped rules
    /// when one rule reports several missing tool inputs.
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    /// Number of distinct rules represented in [`Coverage::skipped`].
    #[must_use]
    pub fn skipped_rule_count(&self) -> usize {
        let mut ids: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for skip in &self.skipped {
            ids.insert(skip.rule.as_str());
        }
        ids.len()
    }
}

/// One known tool field with its provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InventoryField {
    /// The value the analysis used.
    pub value: EvidenceValue,
    /// `"config"` or `"comment:L<line>"`.
    pub source: String,
}

/// What the report knew about one tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInventoryEntry {
    /// Tool number.
    pub tool: u32,
    /// First source line that selected or used the tool.
    pub first_line: usize,
    /// Number of motions attributed to the tool.
    pub motion_count: usize,
    /// Known fields in name order.
    pub fields: BTreeMap<String, InventoryField>,
}

/// Input identity. Never contains an absolute path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputInfo {
    /// File name only.
    pub file_name: String,
    /// SHA-256 of the input bytes, lowercase hex.
    pub sha256: String,
    /// Number of source lines.
    pub line_count: usize,
    /// Effective dialect.
    pub dialect: Dialect,
    /// First line after which geometry is no longer trusted.
    pub geometry_uncertain_after: Option<usize>,
}

/// The versioned, deterministic analysis report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Schema identifier, `"nca.report/1"`.
    pub schema: String,
    /// Ruleset version.
    pub ruleset_version: String,
    /// Input identity.
    pub input: InputInfo,
    /// SHA-256 over the canonical serialized config.
    pub config_digest: String,
    /// Inferred or explicit stock.
    pub stock_source: StockSource,
    /// Tools used by the program, sorted by number.
    pub tool_inventory: Vec<ToolInventoryEntry>,
    /// Findings, sorted deterministically.
    pub diagnostics: Vec<Diagnostic>,
    /// Findings dropped by per-rule or total caps.
    pub truncated: BTreeMap<String, usize>,
    /// Evaluated and skipped counts.
    pub coverage: Coverage,
}

impl Report {
    /// Compact deterministic JSON without a trailing newline.
    ///
    /// This is the canonical report serialization: floats are quantized by
    /// evidence-key convention and written with a fixed decimal count.
    #[must_use]
    pub fn to_json(&self) -> String {
        crate::json::render(self, false)
    }

    /// Two-space pretty JSON without a trailing newline.
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        crate::json::render(self, true)
    }

    /// Error-severity findings.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    /// Warning-or-error findings.
    #[must_use]
    pub fn warning_or_error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity != Severity::Info)
            .count()
    }

    /// Returns `true` when any diagnostic is at or above `threshold`.
    #[must_use]
    pub fn fails_at(&self, threshold: Severity) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity <= threshold)
    }
}

/// Sorts diagnostics by `(severity_rank, line, column, rule_id)`.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        (a.severity.rank(), a.line, a.column, a.rule.as_str()).cmp(&(
            b.severity.rank(),
            b.line,
            b.column,
            b.rule.as_str(),
        ))
    });
}

/// Sorts skips by `(rule id, missing input)` and keeps one record per key.
///
/// The first record for a key wins, so callers should append the most
/// specific reason before duplicates.
pub fn sort_skips(skips: &mut Vec<SkippedRule>) {
    skips.sort_by(|a, b| {
        (a.rule.as_str(), a.missing_input.as_str())
            .cmp(&(b.rule.as_str(), b.missing_input.as_str()))
    });
    skips.dedup_by(|a, b| a.rule == b.rule && a.missing_input == b.missing_input);
}

/// Sorts and deduplicates diagnostics.
pub fn normalize_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    sort_diagnostics(diagnostics);
    diagnostics.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(rule: &'static str, severity: Severity, line: usize) -> Diagnostic {
        Diagnostic {
            rule: RuleId::new(rule),
            severity,
            basis: Basis::Exact,
            line,
            column: 0,
            line_end: line,
            message: String::new(),
            evidence: BTreeMap::new(),
        }
    }

    #[test]
    fn diagnostics_sort_errors_first_then_line() {
        let mut diagnostics = vec![
            diagnostic("b", Severity::Info, 1),
            diagnostic("a", Severity::Warning, 9),
            diagnostic("c", Severity::Error, 5),
            diagnostic("a", Severity::Warning, 2),
        ];
        sort_diagnostics(&mut diagnostics);
        let order = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.severity, diagnostic.line))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                (Severity::Error, 5),
                (Severity::Warning, 2),
                (Severity::Warning, 9),
                (Severity::Info, 1),
            ]
        );
    }

    #[test]
    fn identical_diagnostics_are_deduplicated() {
        let mut diagnostics = vec![
            diagnostic("a", Severity::Warning, 3),
            diagnostic("a", Severity::Warning, 3),
        ];
        normalize_diagnostics(&mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn skips_are_sorted_and_deduplicated() {
        let mut skips = vec![
            SkippedRule::new(RuleId::new("b"), "tool.flutes:T1", "x"),
            SkippedRule::new(RuleId::new("a"), "stock.explicit", "y"),
            SkippedRule::new(RuleId::new("b"), "tool.flutes:T1", "x"),
        ];
        sort_skips(&mut skips);
        assert_eq!(skips.len(), 2);
        assert_eq!(skips[0].rule.as_str(), "a");
        assert_eq!(skips[1].rule.as_str(), "b");
    }
}
