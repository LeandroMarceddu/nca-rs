//! Human-readable report rendering for `nca check`.
//!
//! The JSON report is the stable interface; these renderings are a convenience
//! and deliberately carry the same counts, every diagnostic, and every
//! coverage skip.

use std::fmt::Write as _;

use nc_analysis::{Report, Severity, SkippedRule};

/// Everything a human summary needs beyond the canonical report.
pub struct Summary<'a> {
    /// Canonical analysis report.
    pub report: &'a Report,
    /// Number of motion records parsed from the source.
    pub record_count: usize,
    /// Number of blocks the lossy parser skipped.
    pub skipped_blocks: usize,
    /// `true` when no dialect was configured.
    pub dialect_assumed: bool,
}

/// Plain-text summary for terminals.
#[must_use]
pub fn text(summary: &Summary<'_>) -> String {
    let report = summary.report;
    let mut out = String::new();
    let _ = writeln!(out, "nca check: {}", report.input.file_name);
    let _ = writeln!(out, "schema: {}", report.schema);
    let _ = writeln!(out, "ruleset: {}", report.ruleset_version);
    let _ = writeln!(
        out,
        "lines: {} (records: {}, skipped blocks: {})",
        report.input.line_count, summary.record_count, summary.skipped_blocks
    );
    let _ = writeln!(
        out,
        "dialect: {}{}",
        report.input.dialect,
        if summary.dialect_assumed {
            " (assumed)"
        } else {
            ""
        }
    );
    if let Some(line) = report.input.geometry_uncertain_after {
        let _ = writeln!(out, "geometry uncertain after line: {line}");
    }
    let _ = writeln!(out, "stock: {}", stock_label(report));
    let _ = writeln!(out, "input sha256: {}", report.input.sha256);
    let _ = writeln!(out, "config sha256: {}", report.config_digest);
    let (errors, warnings, infos) = severity_counts(report);
    let _ = writeln!(
        out,
        "diagnostics: {} ({} error, {} warning, {} info)",
        report.diagnostics.len(),
        errors,
        warnings,
        infos
    );
    let _ = writeln!(
        out,
        "coverage: {}/{} evaluated, {} skipped",
        report.coverage.evaluated,
        report.coverage.total,
        report.coverage.skipped.len()
    );

    if report.diagnostics.is_empty() {
        let _ = writeln!(out, "\ndiagnostics:\n  none");
    } else {
        let _ = writeln!(out, "\ndiagnostics:");
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                out,
                "  [{}] {} line {} ({})",
                diagnostic.severity,
                diagnostic.rule,
                diagnostic.line,
                diagnostic.basis.label()
            );
            let _ = writeln!(out, "    {}", diagnostic.message);
            if !diagnostic.evidence.is_empty() {
                let evidence: Vec<String> = diagnostic
                    .evidence
                    .iter()
                    .map(|(key, value)| format!("{key}={}", value.display_text()))
                    .collect();
                let _ = writeln!(out, "    evidence: {}", evidence.join(" "));
            }
        }
    }

    if report.coverage.skipped.is_empty() {
        let _ = writeln!(out, "\ncoverage skips:\n  none");
    } else {
        let _ = writeln!(out, "\ncoverage skips:");
        for skip in &report.coverage.skipped {
            let _ = writeln!(out, "  {}", skip_line(skip));
        }
    }

    if !report.truncated.is_empty() {
        let _ = writeln!(out, "\ntruncated findings:");
        for (rule, count) in &report.truncated {
            let _ = writeln!(out, "  {rule}: {count}");
        }
    }

    out
}

/// Markdown summary suitable for a pull request or a CI artifact.
#[must_use]
pub fn markdown(summary: &Summary<'_>) -> String {
    let report = summary.report;
    let mut out = String::new();
    let _ = writeln!(out, "# nca check: {}", report.input.file_name);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(out, "| field | value |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| schema | `{}` |", report.schema);
    let _ = writeln!(out, "| ruleset | `{}` |", report.ruleset_version);
    let _ = writeln!(out, "| lines | {} |", report.input.line_count);
    let _ = writeln!(out, "| records | {} |", summary.record_count);
    let _ = writeln!(out, "| skipped blocks | {} |", summary.skipped_blocks);
    let _ = writeln!(
        out,
        "| dialect | `{}`{} |",
        report.input.dialect,
        if summary.dialect_assumed {
            " (assumed)"
        } else {
            ""
        }
    );
    if let Some(line) = report.input.geometry_uncertain_after {
        let _ = writeln!(out, "| geometry uncertain after | {line} |");
    }
    let _ = writeln!(out, "| stock | {} |", stock_label(report));
    let _ = writeln!(out, "| input sha256 | `{}` |", report.input.sha256);
    let _ = writeln!(out, "| config sha256 | `{}` |", report.config_digest);
    let (errors, warnings, infos) = severity_counts(report);
    let _ = writeln!(
        out,
        "| diagnostics | {} ({} error, {} warning, {} info) |",
        report.diagnostics.len(),
        errors,
        warnings,
        infos
    );
    let _ = writeln!(
        out,
        "| coverage | {}/{} evaluated, {} skipped |",
        report.coverage.evaluated,
        report.coverage.total,
        report.coverage.skipped.len()
    );

    let _ = writeln!(out);
    let _ = writeln!(out, "## Diagnostics");
    let _ = writeln!(out);
    if report.diagnostics.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                out,
                "- **{}** `{}` line {} ({}): {}",
                diagnostic.severity,
                diagnostic.rule,
                diagnostic.line,
                diagnostic.basis.label(),
                diagnostic.message
            );
            if !diagnostic.evidence.is_empty() {
                let evidence: Vec<String> = diagnostic
                    .evidence
                    .iter()
                    .map(|(key, value)| format!("`{key}`={}", value.display_text()))
                    .collect();
                let _ = writeln!(out, "  - evidence: {}", evidence.join(", "));
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Coverage");
    let _ = writeln!(out);
    if report.coverage.skipped.is_empty() {
        let _ = writeln!(out, "Every rule was evaluated.");
    } else {
        for skip in &report.coverage.skipped {
            let _ = writeln!(out, "- {}", skip_line(skip));
        }
    }

    if !report.truncated.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Truncated findings");
        let _ = writeln!(out);
        for (rule, count) in &report.truncated {
            let _ = writeln!(out, "- `{rule}`: {count} dropped");
        }
    }

    out
}

fn skip_line(skip: &SkippedRule) -> String {
    format!(
        "[{}] missing {}: {}",
        skip.rule, skip.missing_input, skip.reason
    )
}

fn stock_label(report: &Report) -> &'static str {
    match report.stock_source {
        nc_stock::StockSource::Inferred => "inferred",
        nc_stock::StockSource::Explicit => "explicit",
    }
}

fn severity_counts(report: &Report) -> (usize, usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    let mut infos = 0;
    for diagnostic in &report.diagnostics {
        match diagnostic.severity {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            Severity::Info => infos += 1,
        }
    }
    (errors, warnings, infos)
}
