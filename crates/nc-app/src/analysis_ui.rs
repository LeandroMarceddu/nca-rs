//! Pure presentation models for the analysis milestone.
//!
//! Everything in this module is display logic over an [`nc_analysis::Report`]
//! and a parsed [`nc_gcode::Program`]. It never runs analysis, touches a
//! window, or writes G-code, so the filters, coverage grouping, gutters, and
//! program-line focus bounds are unit-tested without a display.
//!
//! Coverage honesty is a product rule: this module produces plain evaluated and
//! skipped counts and never a percentage, progress bar, or safety verdict.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use nc_analysis::{Basis, Coverage, Diagnostic, Report, Severity};
use nc_gcode::Program;
use nc_viewer::Bounds3;

/// Longest diagnostic line span expanded into gutter/overlay lines.
///
/// Reports produced by the engine clamp line ranges to the source, but this
/// bound keeps a hand-built or corrupted report from expanding a huge loop.
pub const MAX_DIAGNOSTIC_LINE_SPAN: usize = 1_000;

/// One severity filter selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeverityFilter {
    /// Every finding.
    #[default]
    All,
    /// Errors only.
    Errors,
    /// Warnings only.
    Warnings,
    /// Info only.
    Info,
}

impl SeverityFilter {
    /// Every selection in display order.
    pub const ALL: [Self; 4] = [Self::All, Self::Errors, Self::Warnings, Self::Info];

    /// Short uppercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Errors => "ERRORS",
            Self::Warnings => "WARNINGS",
            Self::Info => "INFO",
        }
    }

    /// Whether a finding of `severity` passes this filter.
    #[must_use]
    pub const fn accepts(self, severity: Severity) -> bool {
        match self {
            Self::All => true,
            Self::Errors => matches!(severity, Severity::Error),
            Self::Warnings => matches!(severity, Severity::Warning),
            Self::Info => matches!(severity, Severity::Info),
        }
    }
}

/// Diagnostic filter state: severity, rule/category, and free search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticFilter {
    /// Selected severity.
    pub severity: SeverityFilter,
    /// Rule id or category text. Empty means no rule filter.
    pub rule: String,
    /// Case-insensitive text over message, rule id, and line.
    pub search: String,
}

impl DiagnosticFilter {
    /// Fills the filter from raw input strings.
    #[must_use]
    pub fn new(severity: SeverityFilter, rule: &str, search: &str) -> Self {
        Self {
            severity,
            rule: rule.trim().to_string(),
            search: search.trim().to_string(),
        }
    }

    /// Whether one finding passes every filter.
    #[must_use]
    pub fn matches(&self, diagnostic: &Diagnostic) -> bool {
        if !self.severity.accepts(diagnostic.severity) {
            return false;
        }
        let rule = diagnostic.rule.as_str();
        let rule_filter = self.rule.to_ascii_lowercase();
        if !rule_filter.is_empty() && !matches_rule(rule, &rule_filter) {
            return false;
        }
        let search = self.search.to_ascii_lowercase();
        if search.is_empty() {
            return true;
        }
        if rule.to_ascii_lowercase().contains(&search) {
            return true;
        }
        if diagnostic.message.to_ascii_lowercase().contains(&search) {
            return true;
        }
        diagnostic.line.to_string().contains(&search)
            || format!("L{}", diagnostic.line)
                .to_ascii_lowercase()
                .contains(&search)
    }

    /// Report indices of every matching finding, in report order.
    #[must_use]
    pub fn indices(&self, report: &Report) -> Vec<usize> {
        report
            .diagnostics
            .iter()
            .enumerate()
            .filter_map(|(index, diagnostic)| self.matches(diagnostic).then_some(index))
            .collect()
    }
}

/// Matches a rule id against a lowercase rule/category filter.
fn matches_rule(rule: &str, filter: &str) -> bool {
    let lower = rule.to_ascii_lowercase();
    if lower.contains(filter) {
        return true;
    }
    matches!(rule_category(rule), Some(category) if category.to_ascii_lowercase().starts_with(filter))
}

/// The part of a rule id before the first dot.
#[must_use]
pub fn rule_category(rule: &str) -> Option<&str> {
    rule.split_once('.').map(|(category, _)| category)
}

/// Finding counts by severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeverityCounts {
    /// Error findings.
    pub errors: usize,
    /// Warning findings.
    pub warnings: usize,
    /// Info findings.
    pub info: usize,
}

impl SeverityCounts {
    /// Counts every finding in a report.
    #[must_use]
    pub fn from_report(report: &Report) -> Self {
        let mut counts = Self::default();
        for diagnostic in &report.diagnostics {
            match diagnostic.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => counts.info += 1,
            }
        }
        counts
    }

    /// Total finding count.
    #[must_use]
    pub const fn total(self) -> usize {
        self.errors + self.warnings + self.info
    }

    /// Compact top-bar text, for example `2E 5W 7I`.
    #[must_use]
    pub fn compact(self) -> String {
        format!("{}E {}W {}I", self.errors, self.warnings, self.info)
    }
}

/// Worst severity for every source line touched by a finding.
///
/// A finding that spans lines marks every line in its bounded span so the
/// gutter and the warning overlay agree.
#[must_use]
pub fn line_severities(report: &Report) -> BTreeMap<usize, Severity> {
    let mut lines = BTreeMap::new();
    for diagnostic in &report.diagnostics {
        let end = diagnostic
            .line_end
            .max(diagnostic.line)
            .min(diagnostic.line.saturating_add(MAX_DIAGNOSTIC_LINE_SPAN));
        for line in diagnostic.line..=end {
            let slot = lines.entry(line).or_insert(diagnostic.severity);
            if diagnostic.severity.rank() < slot.rank() {
                *slot = diagnostic.severity;
            }
        }
    }
    lines
}

/// Sorted source-line sets for the warning overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayLines {
    /// Lines that carry at least one error finding.
    pub errors: Vec<usize>,
    /// Lines that carry at least one warning and no error.
    pub warnings: Vec<usize>,
}

impl OverlayLines {
    /// Whether the overlay has anything to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }
}

/// Builds deterministic warning/error line sets from a report.
///
/// Info findings never enter the overlay. A line with both an error and a
/// warning appears once, as an error.
#[must_use]
pub fn overlay_lines(report: &Report) -> OverlayLines {
    let mut errors = BTreeSet::new();
    let mut warnings = BTreeSet::new();
    for diagnostic in &report.diagnostics {
        let end = diagnostic
            .line_end
            .max(diagnostic.line)
            .min(diagnostic.line.saturating_add(MAX_DIAGNOSTIC_LINE_SPAN));
        for line in diagnostic.line..=end {
            match diagnostic.severity {
                Severity::Error => {
                    errors.insert(line);
                }
                Severity::Warning => {
                    warnings.insert(line);
                }
                Severity::Info => {}
            }
        }
    }
    let warnings = warnings.difference(&errors).copied().collect();
    OverlayLines {
        errors: errors.into_iter().collect(),
        warnings,
    }
}

/// A coverage group's action target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageTarget {
    /// A tool field for one tool number.
    Tool(u32),
    /// Machine limits or a machine option.
    Machine,
    /// Material preset.
    Material,
    /// Stock, explicit or inferred.
    Stock,
    /// The rule is disabled in the analysis configuration.
    RuleDisabled,
    /// No editor exists in v0.1.
    Other,
}

impl CoverageTarget {
    /// Parses a stable missing-input key.
    #[must_use]
    pub fn parse(missing_input: &str) -> Self {
        if missing_input == "rule.disabled" {
            return Self::RuleDisabled;
        }
        if missing_input == "material" {
            return Self::Material;
        }
        if missing_input == "machine" || missing_input.starts_with("machine.") {
            return Self::Machine;
        }
        if missing_input == "stock" || missing_input.starts_with("stock.") {
            return Self::Stock;
        }
        if let Some((_, tail)) = missing_input.split_once(":T")
            && missing_input.starts_with("tool.")
            && let Ok(number) = tail.parse::<u32>()
        {
            return Self::Tool(number);
        }
        Self::Other
    }

    /// Label for the group's set-value action, when it has one.
    #[must_use]
    pub const fn action_label(self) -> Option<&'static str> {
        match self {
            Self::Tool(_) => Some("SET TOOL"),
            Self::Machine => Some("SET MACHINE"),
            Self::Material => Some("SET MATERIAL"),
            Self::Stock => Some("SET STOCK"),
            Self::RuleDisabled | Self::Other => None,
        }
    }

    /// Plain explanation shown under the missing-input key.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Tool(number) => {
                if number > 0 {
                    "Set this tool's diameter, shape, or flute count."
                } else {
                    "Set the missing tool field."
                }
            }
            Self::Machine => "Machine travel and limits are not set in v0.1.",
            Self::Material => "No material preset is set in v0.1.",
            Self::Stock => "Stock is inferred; the setup sheet cannot set it yet.",
            Self::RuleDisabled => "Disabled in the analysis config, so it never ran.",
            Self::Other => "No editor exists for this input in v0.1.",
        }
    }
}

/// One skipped rule inside a coverage group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageEntry {
    /// Stable rule id.
    pub rule: String,
    /// Human reason from the report.
    pub reason: String,
}

/// All skipped rules that share one missing-input key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageGroup {
    /// Stable missing-input key, for example `tool.diameter:T1`.
    pub missing_input: String,
    /// Where the set-value action should go.
    pub target: CoverageTarget,
    /// Distinct rules and reasons, sorted by rule id then reason.
    pub entries: Vec<CoverageEntry>,
}

/// Groups `Coverage::skipped` by missing input in key order.
///
/// One rule can appear once per missing tool field, so grouping is by the
/// record key rather than by rule. Every group's entries are sorted and
/// deduplicated, which makes the panel stable across runs.
#[must_use]
pub fn coverage_groups(coverage: &Coverage) -> Vec<CoverageGroup> {
    let mut grouped: BTreeMap<String, BTreeMap<(String, String), ()>> = BTreeMap::new();
    for skip in &coverage.skipped {
        grouped
            .entry(skip.missing_input.clone())
            .or_default()
            .insert((skip.rule.as_str().to_string(), skip.reason.clone()), ());
    }
    grouped
        .into_iter()
        .map(|(missing_input, entries)| CoverageGroup {
            target: CoverageTarget::parse(&missing_input),
            missing_input,
            entries: entries
                .into_keys()
                .map(|(rule, reason)| CoverageEntry { rule, reason })
                .collect(),
        })
        .collect()
}

/// Plain `N evaluated · M skipped` text. Never a percentage.
#[must_use]
pub fn coverage_summary(coverage: &Coverage) -> String {
    format!(
        "{} evaluated \u{00b7} {} skipped",
        coverage.evaluated,
        coverage.skipped_count()
    )
}

/// Distinct-rule count text, using `Coverage::skipped_rule_count`.
#[must_use]
pub fn coverage_rule_summary(coverage: &Coverage) -> String {
    let rules = coverage.skipped_rule_count();
    if rules == 1 {
        "1 rule skipped".to_string()
    } else {
        format!("{rules} rules skipped")
    }
}

/// The visible coverage caveat. It never claims safety.
pub const COVERAGE_NOTE: &str = "Coverage is not a safety verdict. A skipped rule did not run; a program with \
     no findings may still be unsafe.";

/// Text shown when no analysis has run.
pub const COVERAGE_EMPTY_NOTE: &str =
    "Press ANALYZE to compare the rule registry against this program.";

/// Every coverage text the UI can show, for honesty tests.
#[cfg(test)]
#[must_use]
pub fn coverage_texts(coverage: &Coverage) -> Vec<String> {
    let mut texts = vec![
        coverage_summary(coverage),
        coverage_rule_summary(coverage),
        COVERAGE_NOTE.to_string(),
        COVERAGE_EMPTY_NOTE.to_string(),
    ];
    for group in coverage_groups(coverage) {
        texts.push(group.missing_input);
        texts.push(group.target.description().to_string());
        if let Some(label) = group.target.action_label() {
            texts.push(label.to_string());
        }
        for entry in group.entries {
            texts.push(entry.rule);
            texts.push(entry.reason);
        }
    }
    texts
}

/// Direct segment ranges for the lines in `start..=end`, sorted by line.
#[must_use]
pub fn segment_ranges(program: &Program, start: usize, end: usize) -> Vec<Range<usize>> {
    let end = end.max(start);
    let mut ranges = Vec::new();
    let mut index = program.lines.partition_point(|line| line.line < start);
    while let Some(line) = program.lines.get(index) {
        if line.line > end {
            break;
        }
        if !line.segments.is_empty() {
            ranges.push(line.segments.clone());
        }
        index += 1;
    }
    ranges
}

/// Segment ranges for a diagnostic line, falling back to the nearest geometry.
///
/// Modal-only and comment lines have no segments. Looking forward, then
/// backward, keeps click-to-zoom useful for a missing-feed finding on a comment
/// or a spindle finding on an `M` line.
#[must_use]
pub fn segment_ranges_with_fallback(
    program: &Program,
    start: usize,
    end: usize,
) -> Vec<Range<usize>> {
    let direct = segment_ranges(program, start, end);
    if !direct.is_empty() {
        return direct;
    }
    let start = start.max(1);
    if program.lines.last().is_some_and(|line| start > line.line) {
        return Vec::new();
    }
    if let Some(line) = program
        .lines
        .iter()
        .find(|line| line.line >= start && !line.segments.is_empty() && line.line > end)
    {
        return vec![line.segments.clone()];
    }
    if let Some(line) = program
        .lines
        .iter()
        .rev()
        .find(|line| line.line < start && !line.segments.is_empty())
    {
        return vec![line.segments.clone()];
    }
    Vec::new()
}

/// World bounds around every segment generated by a diagnostic's line span.
#[must_use]
pub fn focus_bounds(program: &Program, start: usize, end: usize) -> Option<Bounds3> {
    let ranges = segment_ranges_with_fallback(program, start, end);
    let segments = program.toolpath.segments();
    let mut bounds = Bounds3::empty();
    for range in ranges {
        let start = range.start.min(segments.len());
        let end = range.end.min(segments.len());
        for segment in &segments[start..end] {
            bounds.union(&segment.bounds());
        }
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// Which analysis panel is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisPanel {
    /// Findings list and selected-finding details.
    Diagnostics,
    /// Evaluated/skipped counts and grouped skip reasons.
    Coverage,
}

impl AnalysisPanel {
    /// Short uppercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Diagnostics => "DIAGNOSTICS",
            Self::Coverage => "COVERAGE",
        }
    }

    /// Parses a review flag value.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "diagnostics" => Some(Self::Diagnostics),
            "coverage" => Some(Self::Coverage),
            _ => None,
        }
    }
}

/// Generation identity for one analysis run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalysisGeneration {
    /// Monotonic run counter.
    pub run: u64,
    /// Loaded-program generation this run analyzes.
    pub program: u64,
}

/// Presentation state of the last analysis run.
#[derive(Clone, Debug, Default)]
pub enum AnalysisState {
    /// No run has been requested.
    #[default]
    Idle,
    /// A worker is running the request.
    Running(AnalysisGeneration),
    /// The current request completed.
    Ready {
        /// Generation of the accepted report.
        generation: AnalysisGeneration,
        /// The deterministic report.
        report: Arc<Report>,
    },
    /// The current request failed.
    Failed {
        /// Generation that failed.
        generation: AnalysisGeneration,
        /// Plain-language error.
        message: String,
    },
    /// The current request was canceled by the operator.
    Canceled,
}

impl AnalysisState {
    /// Starts a run.
    pub fn begin(&mut self, generation: AnalysisGeneration) {
        *self = Self::Running(generation);
    }

    /// Accepts a completion only when it belongs to the current run.
    ///
    /// Returns `false` and leaves the state untouched for a stale generation,
    /// so an older file can never replace a newer one.
    pub fn complete(
        &mut self,
        generation: AnalysisGeneration,
        result: Result<Report, String>,
    ) -> bool {
        let current = match self {
            Self::Running(current) => *current,
            _ => return false,
        };
        if current != generation {
            return false;
        }
        *self = match result {
            Ok(report) => Self::Ready {
                generation,
                report: Arc::new(report),
            },
            Err(message) => Self::Failed {
                generation,
                message,
            },
        };
        true
    }

    /// The accepted report, when the state is ready.
    #[must_use]
    pub fn report(&self) -> Option<&Arc<Report>> {
        match self {
            Self::Ready { report, .. } => Some(report),
            _ => None,
        }
    }

    /// Whether a worker is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    /// Short top-bar state text.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "NOT ANALYZED",
            Self::Running(_) => "ANALYZING",
            Self::Ready { .. } => "READY",
            Self::Failed { .. } => "ANALYSIS FAILED",
            Self::Canceled => "ANALYSIS CANCELED",
        }
    }

    /// Failure text, when the current request failed.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed { message, .. } => Some(message),
            _ => None,
        }
    }
}

/// Review-only launch options used by screenshot captures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewOptions {
    /// Ensure analysis runs after the startup load (it normally does anyway).
    pub analyze: bool,
    /// Panel to open for review.
    pub panel: Option<AnalysisPanel>,
    /// Select the first finding of this rule id once analysis finishes.
    pub select_rule: Option<String>,
}

/// One rule's explanatory text, resolved from the live registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleNote {
    /// Stable rule id.
    pub rule: String,
    /// Default severity from the registry.
    pub severity: Severity,
    /// Exact or model basis.
    pub basis: Basis,
    /// Plain-language explanation.
    pub why_it_matters: String,
}

/// Rule descriptions keyed by stable id.
#[derive(Clone, Debug, Default)]
pub struct RuleCatalog {
    notes: BTreeMap<String, RuleNote>,
}

impl RuleCatalog {
    /// Builds the catalog from `nc_analysis::rules::all()`.
    #[must_use]
    pub fn load() -> Self {
        let mut notes = BTreeMap::new();
        for rule in nc_analysis::rules::all() {
            let id = rule.id();
            notes.insert(
                id.as_str().to_string(),
                RuleNote {
                    rule: id.as_str().to_string(),
                    severity: rule.default_severity(),
                    basis: rule.basis(),
                    why_it_matters: rule.why_it_matters().to_string(),
                },
            );
        }
        Self { notes }
    }

    /// Note for a rule id.
    #[must_use]
    pub fn note(&self, rule: &str) -> Option<&RuleNote> {
        self.notes.get(rule)
    }

    /// Number of registered rules described.
    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nc_analysis::{
        Coverage, Diagnostic, EvidenceValue, InputInfo, Report, RuleId, SkippedRule,
    };
    use nc_core::Dialect;
    use nc_gcode::parse_program;
    use nc_stock::StockSource;

    use super::*;

    fn diagnostic(
        rule: &'static str,
        severity: Severity,
        line: usize,
        message: &str,
    ) -> Diagnostic {
        Diagnostic {
            rule: RuleId::new(rule),
            severity,
            basis: Basis::Exact,
            line,
            column: 0,
            line_end: line,
            message: message.to_string(),
            evidence: BTreeMap::<String, EvidenceValue>::new(),
        }
    }

    fn report(diagnostics: Vec<Diagnostic>, skipped: Vec<SkippedRule>) -> Report {
        Report {
            schema: "nca.report/1".to_string(),
            ruleset_version: "1.0.0".to_string(),
            input: InputInfo {
                file_name: "test.nc".to_string(),
                sha256: "0".repeat(64),
                line_count: 20,
                dialect: Dialect::Generic,
                geometry_uncertain_after: None,
            },
            config_digest: "digest".to_string(),
            stock_source: StockSource::Inferred,
            tool_inventory: Vec::new(),
            diagnostics,
            truncated: BTreeMap::new(),
            coverage: Coverage {
                evaluated: 2,
                total: 6,
                skipped,
            },
        }
    }

    fn skip(rule: &'static str, missing: &str, reason: &str) -> SkippedRule {
        SkippedRule::new(RuleId::new(rule), missing, reason)
    }

    #[test]
    fn diagnostics_filter_by_severity_rule_and_search() {
        let report = report(
            vec![
                diagnostic(
                    "clearance.rapid_at_depth",
                    Severity::Warning,
                    12,
                    "rapid at depth crosses the stock",
                ),
                diagnostic(
                    "geometry.duplicate_hole",
                    Severity::Error,
                    20,
                    "duplicate hole",
                ),
                diagnostic(
                    "travel.machine_envelope",
                    Severity::Warning,
                    30,
                    "outside travel",
                ),
                diagnostic(
                    "parse.dialect_assumed",
                    Severity::Info,
                    1,
                    "generic dialect",
                ),
            ],
            Vec::new(),
        );

        let all = DiagnosticFilter::default().indices(&report);
        assert_eq!(all, vec![0, 1, 2, 3]);

        let errors = DiagnosticFilter::new(SeverityFilter::Errors, "", "").indices(&report);
        assert_eq!(errors, vec![1]);

        let clearance =
            DiagnosticFilter::new(SeverityFilter::All, "clearance", "").indices(&report);
        assert_eq!(clearance, vec![0]);

        let by_category = DiagnosticFilter::new(SeverityFilter::All, "travel", "").indices(&report);
        assert_eq!(by_category, vec![2]);

        let by_search = DiagnosticFilter::new(SeverityFilter::All, "", "STOCK").indices(&report);
        assert_eq!(by_search, vec![0]);

        let by_line = DiagnosticFilter::new(SeverityFilter::All, "", "L30").indices(&report);
        assert_eq!(by_line, vec![2]);

        let combined =
            DiagnosticFilter::new(SeverityFilter::Warnings, "travel", "outside").indices(&report);
        assert_eq!(combined, vec![2]);
    }

    #[test]
    fn severity_counts_are_stable_and_total() {
        let report = report(
            vec![
                diagnostic("a.one", Severity::Info, 1, "i"),
                diagnostic("a.two", Severity::Error, 2, "e"),
                diagnostic("a.three", Severity::Warning, 3, "w"),
                diagnostic("a.four", Severity::Error, 4, "e2"),
            ],
            Vec::new(),
        );
        let counts = SeverityCounts::from_report(&report);
        assert_eq!(
            counts,
            SeverityCounts {
                errors: 2,
                warnings: 1,
                info: 1,
            }
        );
        assert_eq!(counts.total(), 4);
        assert_eq!(counts.compact(), "2E 1W 1I");
        assert_eq!(SeverityCounts::from_report(&report), counts);
    }

    #[test]
    fn coverage_groups_are_keyed_and_ordered() {
        let report = report(
            Vec::new(),
            vec![
                skip(
                    "process.chipload",
                    "tool.flutes:T1",
                    "tool T1 has no flute count",
                ),
                skip(
                    "geometry.arc_smaller_than_tool",
                    "tool.diameter:T1",
                    "tool T1 has no diameter",
                ),
                skip(
                    "process.chipload",
                    "material",
                    "a material preset is required",
                ),
                skip(
                    "travel.machine_envelope",
                    "machine",
                    "machine limits are required",
                ),
                skip(
                    "travel.stock_bounds",
                    "stock.explicit",
                    "explicit stock is required, but the stock was inferred",
                ),
                skip(
                    "geometry.overlapping_holes",
                    "tool.diameter:T1",
                    "tool T1 has no diameter",
                ),
                skip(
                    "process.mrr",
                    "rule.disabled",
                    "rule is disabled in configuration",
                ),
            ],
        );
        let groups = coverage_groups(&report.coverage);
        let keys: Vec<&str> = groups
            .iter()
            .map(|group| group.missing_input.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "machine",
                "material",
                "rule.disabled",
                "stock.explicit",
                "tool.diameter:T1",
                "tool.flutes:T1",
            ]
        );
        let diameter = &groups[4];
        assert_eq!(diameter.target, CoverageTarget::Tool(1));
        let rules: Vec<&str> = diameter
            .entries
            .iter()
            .map(|entry| entry.rule.as_str())
            .collect();
        assert_eq!(
            rules,
            vec![
                "geometry.arc_smaller_than_tool",
                "geometry.overlapping_holes"
            ]
        );
        assert_eq!(groups[2].target, CoverageTarget::RuleDisabled);
        assert!(groups[2].target.action_label().is_none());
        assert_eq!(groups[3].target, CoverageTarget::Stock);
        assert_eq!(groups[0].target, CoverageTarget::Machine);
        assert_eq!(groups[1].target, CoverageTarget::Material);

        // Repeating the grouping produces the same records.
        assert_eq!(coverage_groups(&report.coverage), groups);
    }

    #[test]
    fn coverage_texts_never_claim_a_percentage_or_safety() {
        let report = report(
            Vec::new(),
            vec![
                skip(
                    "process.chipload",
                    "material",
                    "a material preset is required",
                ),
                skip(
                    "travel.machine_envelope",
                    "machine",
                    "machine limits are required",
                ),
            ],
        );
        let summary = coverage_summary(&report.coverage);
        assert_eq!(summary, "2 evaluated \u{00b7} 2 skipped");
        assert_eq!(coverage_rule_summary(&report.coverage), "2 rules skipped");
        for text in coverage_texts(&report.coverage) {
            let lower = text.to_ascii_lowercase();
            assert!(
                !text.contains('%'),
                "coverage text has a percentage: {text}"
            );
            assert!(
                !text.contains("SAFE"),
                "coverage text implies safety: {text}"
            );
            assert!(
                !lower.contains("pass"),
                "coverage text implies a pass: {text}"
            );
            assert!(
                !lower.contains("green"),
                "coverage text implies a green state: {text}"
            );
        }
    }

    #[test]
    fn diagnostic_line_maps_to_segments_and_focus_bounds() {
        let source = "G21 G90\nG0 Z5\nG1 X10 F100\nG1 Y10\n";
        let program = parse_program(source).expect("program");
        let line = program
            .lines
            .iter()
            .find(|line| line.line == 3)
            .expect("line 3");
        let expected = line.segments.clone();
        assert!(!expected.is_empty());

        let ranges = segment_ranges(&program, 3, 3);
        assert_eq!(ranges, vec![expected.clone()]);
        let bounds = focus_bounds(&program, 3, 3).expect("bounds");
        let segment = &program.toolpath.segments()[expected.start];
        assert!(bounds.contains(segment.start));
        assert!(bounds.contains(segment.end));

        // A comment line with no geometry falls back to the next move.
        let fallback = segment_ranges_with_fallback(&program, 1, 1);
        assert!(!fallback.is_empty());
        assert!(focus_bounds(&program, 1, 1).is_some());
        assert!(focus_bounds(&program, 99, 99).is_none());
    }

    #[test]
    fn stale_analysis_generation_is_ignored() {
        let mut state = AnalysisState::default();
        let stale = AnalysisGeneration { run: 1, program: 7 };
        let current = AnalysisGeneration { run: 2, program: 8 };
        state.begin(current);
        let report = report(
            vec![diagnostic("a.one", Severity::Info, 1, "i")],
            Vec::new(),
        );
        assert!(!state.complete(stale, Ok(report.clone())));
        assert!(state.report().is_none());
        assert!(state.is_running());
        assert!(state.complete(current, Ok(report.clone())));
        assert!(!state.is_running());
        let accepted = state.report().expect("accepted report");
        assert_eq!(accepted.input.file_name, "test.nc");
        assert!(matches!(
            state,
            AnalysisState::Ready { generation, .. } if generation == current
        ));
        assert!(!state.complete(current, Ok(report)));
    }

    #[test]
    fn warning_overlay_lines_are_deterministic_and_exclude_info() {
        let mut warning = diagnostic("clearance.rapid_at_depth", Severity::Warning, 5, "w");
        warning.line_end = 6;
        let report = report(
            vec![
                warning,
                diagnostic("geometry.duplicate_hole", Severity::Error, 8, "e"),
                diagnostic("parse.dialect_assumed", Severity::Info, 9, "i"),
                diagnostic("travel.stock_bounds", Severity::Warning, 5, "w2"),
            ],
            Vec::new(),
        );
        let first = overlay_lines(&report);
        assert_eq!(first.warnings, vec![5, 6]);
        assert_eq!(first.errors, vec![8]);
        assert_eq!(first.errors.len() + first.warnings.len(), 3);
        assert!(!first.is_empty());
        assert!(!first.warnings.contains(&9));
        assert_eq!(overlay_lines(&report), first);

        let severities = line_severities(&report);
        assert_eq!(severities.get(&5), Some(&Severity::Warning));
        assert_eq!(severities.get(&8), Some(&Severity::Error));
        assert_eq!(severities.get(&9), Some(&Severity::Info));
    }

    #[test]
    fn rule_catalog_resolves_every_registry_rule() {
        let catalog = RuleCatalog::load();
        assert!(catalog.len() > 0);
        assert_eq!(catalog.len(), nc_analysis::rules::all().len());
        let note = catalog
            .note("clearance.rapid_through_stock")
            .expect("known rule");
        assert_eq!(note.rule, "clearance.rapid_through_stock");
        assert!(!note.why_it_matters.trim().is_empty());
        assert!(catalog.note("not.a.rule").is_none());
    }
}
