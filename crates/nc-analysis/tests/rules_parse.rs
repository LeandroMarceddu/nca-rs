//! `parse.*` rule tests: positive, near-miss, coverage, and determinism.

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, EvidenceValue, Report, Rule, RuleRegistry, Severity,
    analyze, rules,
};
use nc_core::Dialect;
use nc_gcode::{ParserConfig, Program, WarningKind, parse_program, parse_program_lossy};

/// Runs one set of rules against `program`.
fn run(
    program: &Program,
    source: &str,
    config: &AnalysisConfig,
    registry: &RuleRegistry,
) -> Report {
    let request = AnalysisRequest::new("part.nc", source, config, Dialect::Generic);
    analyze(program, &request, registry)
}

/// Registry with every rule in this module.
fn all_parse_rules() -> RuleRegistry {
    RuleRegistry::from_rules(rules::parse::rules())
}

/// Registry with only the named rule.
fn parse_only(id: &str) -> RuleRegistry {
    let selected: Vec<Box<dyn Rule>> = rules::parse::rules()
        .into_iter()
        .filter(|rule| rule.id().as_str() == id)
        .collect();
    RuleRegistry::from_rules(selected)
}

/// Findings for one rule id, in report order.
fn findings_for<'a>(report: &'a Report, id: &str) -> Vec<&'a nc_analysis::Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|finding| finding.rule.as_str() == id)
        .collect()
}

#[test]
fn unsupported_reports_each_skipped_block_with_raw_line_and_reason() {
    let source = "G21 G90\nG1 X2 Q3\nG1 X3 F100\nM30\n";
    let result = parse_program_lossy(source, &ParserConfig::default());
    assert_eq!(result.skipped_blocks.len(), 1);

    let report = run(
        &result.program,
        source,
        &AnalysisConfig::default(),
        &parse_only("parse.unsupported"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "parse.unsupported");
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(finding.line_end, 2);
    assert_eq!(
        finding.evidence.get("raw"),
        Some(&EvidenceValue::Text("G1 X2 Q3".to_string()))
    );
    assert_eq!(
        finding.evidence.get("reason"),
        Some(&EvidenceValue::Text("unused_word".to_string()))
    );
    assert!(finding.message.contains("unused_word"));
}

#[test]
fn unsupported_is_silent_when_every_block_parsed() {
    let source = "G21 G90\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert!(program.skipped_blocks().is_empty());

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &parse_only("parse.unsupported"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn dialect_assumed_fires_exactly_once_at_line_one() {
    let source = "G21 G90\nM30\n";
    let program = parse_program(source).expect("valid program");
    let config = AnalysisConfig::default();
    let request = AnalysisRequest::new("part.nc", source, &config, Dialect::Generic)
        .with_dialect_assumed(true);
    let report = analyze(&program, &request, &parse_only("parse.dialect_assumed"));

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "parse.dialect_assumed");
    assert_eq!(finding.severity, Severity::Info);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 1);
    assert_eq!(
        finding.message,
        "no dialect was configured; Generic semantics were assumed"
    );
}

#[test]
fn dialect_assumed_is_silent_when_a_dialect_was_configured() {
    let source = "G21 G90\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &parse_only("parse.dialect_assumed"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_hint_ambiguous_reports_the_warning_line() {
    let source = "(T1 DIA 6.35 flat)\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert!(
        program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::AmbiguousToolHint),
        "fixture must produce an ambiguous hint warning"
    );

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &parse_only("parse.tool_hint_ambiguous"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "parse.tool_hint_ambiguous");
    assert_eq!(finding.severity, Severity::Info);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 1);
    let reason = finding.evidence.get("reason").expect("reason evidence");
    let EvidenceValue::Text(text) = reason else {
        panic!("reason must be text");
    };
    assert!(text.contains("ambiguous tool-comment diameter"));
}

#[test]
fn tool_hint_ambiguous_is_silent_when_the_unit_is_known() {
    let source = "(T1 DIA 6.35mm flat)\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert!(
        !program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::AmbiguousToolHint)
    );

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &parse_only("parse.tool_hint_ambiguous"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn parse_module_coverage_counts_all_three_rules() {
    let source = "G21 G90\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &all_parse_rules(),
    );

    assert_eq!(report.coverage.total, 3);
    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.coverage.skipped.is_empty());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn parse_module_reports_are_byte_deterministic() {
    let source = "(T1 DIA 6.35 flat)\nG90\nG1 X2 Q3\nG1 X3 F100\nM30\n";
    let result = parse_program_lossy(source, &ParserConfig::default());
    let config = AnalysisConfig::default();
    let first = analyze(
        &result.program,
        &AnalysisRequest::new("part.nc", source, &config, Dialect::Generic)
            .with_dialect_assumed(true),
        &all_parse_rules(),
    );
    let second = analyze(
        &result.program,
        &AnalysisRequest::new("part.nc", source, &config, Dialect::Generic)
            .with_dialect_assumed(true),
        &all_parse_rules(),
    );

    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
    assert_eq!(findings_for(&first, "parse.unsupported").len(), 1);
    assert_eq!(findings_for(&first, "parse.tool_hint_ambiguous").len(), 1);
    assert_eq!(findings_for(&first, "parse.dialect_assumed").len(), 1);
}

#[test]
fn parse_catalog_metadata_is_stable() {
    let rules = rules::parse::rules();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert_eq!(
        ids,
        vec![
            "parse.unsupported",
            "parse.dialect_assumed",
            "parse.tool_hint_ambiguous"
        ]
    );

    let severities: Vec<Severity> = rules.iter().map(|rule| rule.default_severity()).collect();
    assert_eq!(
        severities,
        vec![Severity::Error, Severity::Info, Severity::Info]
    );
    for rule in &rules {
        assert_eq!(rule.basis(), Basis::Exact);
        let requirements = rule.requirements();
        assert!(requirements.tool_fields.is_empty());
        assert!(!requirements.machine);
        assert!(!requirements.material);
        assert_eq!(requirements.stock, nc_analysis::StockRequirement::None);
        assert!(!rule.why_it_matters().is_empty());
    }
}
