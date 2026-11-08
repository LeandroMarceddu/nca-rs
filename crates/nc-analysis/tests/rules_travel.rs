//! `travel.*` rule tests: table Z, explicit stock bounds, coverage.
//!
//! Each rule has a positive and a near-miss fixture, plus the exact-boundary
//! cases documented in `docs/RULES.md`.

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, EvidenceValue, Report, Rule, RuleRegistry, Severity,
    StockConfig, StockRequirement, analyze, rules,
};
use nc_core::{Dialect, MachineConfig};
use nc_gcode::{MotionRecord, MoveKind, Program, ProgramBuilder};

/// Source text with `count` one-word lines so diagnostics can be clamped.
fn source_lines(count: usize) -> String {
    (1..=count).map(|line| format!("N{line}\n")).collect()
}

/// A motion record with an explicit tool and feed.
fn motion(
    line: usize,
    kind: MoveKind,
    start_mm: [f64; 3],
    end_mm: [f64; 3],
    feed_mm_min: Option<f64>,
    tool: u32,
) -> MotionRecord {
    let mut record = MotionRecord::linear(line, 0, kind, start_mm, end_mm);
    record.feed_mm_min = feed_mm_min;
    record.tool = tool;
    record
}

/// Runs `analyze` with the standard test file name and dialect.
fn run(
    program: &Program,
    source: &str,
    config: &AnalysisConfig,
    registry: &RuleRegistry,
) -> Report {
    let request = AnalysisRequest::new("part.nc", source, config, Dialect::Generic);
    analyze(program, &request, registry)
}

/// Stock block from -20..60 XY and -10..0 Z, so the top is exactly Z0, with a
/// valid machine envelope.
fn explicit_stock_config() -> AnalysisConfig {
    AnalysisConfig {
        stock: Some(StockConfig {
            origin_mm: [-20.0, -20.0, -10.0],
            size_mm: [80.0, 80.0, 10.0],
        }),
        machine: Some(machine_config()),
        ..AnalysisConfig::default()
    }
}

/// Machine travel -10..30 XY and -10..10 Z, so the table is exactly Z-10.
fn machine_config() -> MachineConfig {
    MachineConfig::new([-10.0, -10.0, -10.0], [30.0, 30.0, 10.0]).expect("valid machine")
}

fn registry(rule_list: Vec<Box<dyn Rule>>) -> RuleRegistry {
    RuleRegistry::from_rules(rule_list)
}

#[test]
fn z_below_table_fires_below_the_configured_table() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, -11.0],
            None,
            5,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [10.0, 0.0, -11.0],
            [10.0, 0.0, 5.0],
            None,
            5,
        ))
        .build();
    let config = AnalysisConfig {
        machine: Some(machine_config()),
        ..AnalysisConfig::default()
    };
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &registry(rules::travel::rules()),
    );

    assert_eq!(report.coverage.total, 2);
    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(
        report.coverage.skipped[0].rule.as_str(),
        "travel.stock_bounds"
    );
    assert_eq!(report.coverage.skipped[0].missing_input, "stock.explicit");
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "travel.z_below_table");
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(finding.line_end, 2);
    assert!(finding.message.contains("below the machine table"));
    assert_eq!(
        finding.evidence.get("axis"),
        Some(&EvidenceValue::Text("Z".to_string()))
    );
    assert_eq!(
        finding.evidence.get("value_mm"),
        Some(&EvidenceValue::Float(-11.0))
    );
    assert_eq!(
        finding.evidence.get("min_mm"),
        Some(&EvidenceValue::Float(-10.0))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(5)));
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&EvidenceValue::Point3([10.0, 0.0, -11.0]))
    );
}

#[test]
fn z_below_table_allows_exactly_the_table_z() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, -10.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Plunge,
            [10.0, 0.0, -10.0],
            [10.0, 0.0, -9.999],
            Some(100.0),
            1,
        ))
        .build();
    let config = AnalysisConfig {
        machine: Some(machine_config()),
        ..AnalysisConfig::default()
    };
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &registry(rules::travel::rules()),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn z_below_table_skips_without_machine_limits() {
    let program = ProgramBuilder::new()
        .motion(2, MoveKind::Rapid, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], None)
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &AnalysisConfig::default(),
        &registry(rules::travel::rules()),
    );

    let skip = report
        .coverage
        .skipped
        .iter()
        .find(|skip| skip.rule.as_str() == "travel.z_below_table")
        .expect("table rule reports a missing machine");
    assert_eq!(skip.missing_input, "machine");
    assert_eq!(skip.reason, "machine limits are required");
}

#[test]
fn z_below_table_skips_invalid_machine_limits() {
    let mut machine = machine_config();
    machine.travel_min_mm = [40.0, -10.0, -10.0];
    let config = AnalysisConfig {
        machine: Some(machine),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .motion(2, MoveKind::Rapid, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], None)
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &registry(rules::travel::rules()),
    );

    let skip = report
        .coverage
        .skipped
        .iter()
        .find(|skip| skip.rule.as_str() == "travel.z_below_table")
        .expect("table rule reports invalid machine limits");
    assert_eq!(skip.missing_input, "machine");
    assert_eq!(skip.reason, "configured machine travel limits are invalid");
}

#[test]
fn stock_bounds_fires_on_a_cutting_endpoint_outside_the_footprint() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [70.0, 0.0, -1.0],
            Some(100.0),
            2,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [70.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            Some(100.0),
            2,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::travel::rules()),
    );

    assert_eq!(report.coverage.evaluated, 2);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "travel.stock_bounds");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(finding.line_end, 2);
    assert!(finding.message.contains("outside the stock XY footprint"));
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&EvidenceValue::Text("explicit".to_string()))
    );
    assert_eq!(
        finding.evidence.get("start_mm"),
        Some(&EvidenceValue::Point3([0.0, 0.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&EvidenceValue::Point3([70.0, 0.0, -1.0]))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(2)));
}

#[test]
fn stock_bounds_allows_endpoints_exactly_on_the_stock_edge() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [60.0, -20.0, -1.0],
            Some(100.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [60.0, -20.0, -1.0],
            [-20.0, 60.0, -1.0],
            Some(100.0),
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::travel::rules()),
    );

    assert_eq!(report.coverage.evaluated, 2);
    assert!(
        report.diagnostics.is_empty(),
        "the stock footprint is inclusive"
    );
}

#[test]
fn stock_bounds_ignores_rapid_endpoints_outside_the_footprint() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 5.0],
            [70.0, 0.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &explicit_stock_config(),
        &registry(rules::travel::rules()),
    );

    assert_eq!(report.coverage.evaluated, 2);
    assert!(report.diagnostics.is_empty(), "only cutting moves count");
}

#[test]
fn travel_coverage_arithmetic_counts_each_rule() {
    let program = ProgramBuilder::new()
        .motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [10.0, 0.0, -1.0],
            Some(100.0),
        )
        .build();
    let source = source_lines(2);
    let registry = registry(rules::travel::rules());

    // Inferred stock: the table rule evaluates, the explicit-stock rule skips.
    let inferred = AnalysisConfig {
        machine: Some(machine_config()),
        ..AnalysisConfig::default()
    };
    let report = run(&program, &source, &inferred, &registry);
    assert_eq!(report.coverage.total, 2);
    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped_rule_count(), 1);
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );
    assert_eq!(report.coverage.skipped[0].missing_input, "stock.explicit");

    // Explicit stock and a machine: both rules evaluate with zero findings.
    let explicit = AnalysisConfig {
        machine: Some(machine_config()),
        ..explicit_stock_config()
    };
    let report = run(&program, &source, &explicit, &registry);
    assert_eq!(report.coverage.total, 2);
    assert_eq!(report.coverage.evaluated, 2);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );
    assert!(report.diagnostics.is_empty());
}

#[test]
fn travel_rule_metadata_is_stable() {
    let rules = rules::travel::rules();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert_eq!(ids, vec!["travel.z_below_table", "travel.stock_bounds"]);

    let table = &rules[0];
    assert_eq!(table.default_severity(), Severity::Error);
    assert_eq!(table.basis(), Basis::Exact);
    assert!(table.requirements().machine);
    assert_eq!(table.requirements().stock, StockRequirement::None);

    let stock = &rules[1];
    assert_eq!(stock.default_severity(), Severity::Warning);
    assert_eq!(stock.basis(), Basis::Exact);
    assert!(!stock.requirements().machine);
    assert_eq!(stock.requirements().stock, StockRequirement::Explicit);

    for rule in &rules {
        assert!(!rule.why_it_matters().is_empty());
    }
}
