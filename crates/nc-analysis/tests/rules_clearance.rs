//! `clearance.*` rule tests: through-stock rapids, low retracts, final pose,
//! inferred-stock evidence, coverage, and byte determinism.
//!
//! Each rule has a positive and a near-miss fixture, plus the exact-boundary
//! cases documented in `docs/RULES.md`.

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, EvidenceValue, Report, Rule, RuleRegistry, Severity,
    StockConfig, StockRequirement, analyze, rules,
};
use nc_core::{Dialect, MachineConfig};
use nc_gcode::{
    CannedCycleKind, CannedRecord, MotionRecord, MoveKind, Program, ProgramBuilder, RetractMode,
};

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

/// Stock block from -20..60 XY and -10..0 Z, so the top is exactly Z0.
fn explicit_stock_config() -> AnalysisConfig {
    AnalysisConfig {
        stock: Some(StockConfig {
            origin_mm: [-20.0, -20.0, -10.0],
            size_mm: [80.0, 80.0, 10.0],
        }),
        ..AnalysisConfig::default()
    }
}

/// Machine travel -10..30 XY and -10..10 Z.
fn machine_config() -> MachineConfig {
    MachineConfig::new([-10.0, -10.0, -10.0], [30.0, 30.0, 10.0]).expect("valid machine")
}

fn registry(rule_list: Vec<Box<dyn Rule>>) -> RuleRegistry {
    RuleRegistry::from_rules(rule_list)
}

#[test]
fn rapid_through_stock_fires_when_crossing_below_the_top() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [-30.0, 20.0, -1.0],
            [70.0, 20.0, -1.0],
            None,
            7,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [70.0, 20.0, -1.0],
            [70.0, 20.0, 5.0],
            None,
            7,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.total, 3);
    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "clearance.rapid_through_stock");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 2);
    assert_eq!(finding.line_end, 2);
    assert!(finding.message.contains("crosses the stock footprint"));
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&EvidenceValue::Text("explicit".to_string()))
    );
    assert!(!finding.evidence.contains_key("inference_basis"));
    assert_eq!(
        finding.evidence.get("start_mm"),
        Some(&EvidenceValue::Point3([-30.0, 20.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&EvidenceValue::Point3([70.0, 20.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(1.0))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(7)));
}

#[test]
fn rapid_through_stock_ignores_boundary_only_travel() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [-20.0, -10.0, -1.0],
            [-20.0, 10.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [-30.0, 0.0, -1.0],
            [-20.0, 0.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            4,
            MoveKind::Rapid,
            [-30.0, -30.0, -1.0],
            [-20.0, -20.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            5,
            MoveKind::Rapid,
            [-20.0, -20.0, -1.0],
            [-20.0, -20.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(5),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(
        report.diagnostics.is_empty(),
        "a segment that only touches the footprint does not cross it"
    );
}

#[test]
fn rapid_through_stock_allows_crossing_exactly_at_the_stock_top() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [-30.0, 20.0, 0.0],
            [70.0, 20.0, 0.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [70.0, 20.0, 0.0],
            [70.0, 20.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.diagnostics.is_empty(), "at the top is not below it");
}

#[test]
fn rapid_through_stock_ignores_pure_z_and_retract_moves() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 5.0],
            [0.0, 0.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Retract,
            [-30.0, 20.0, -1.0],
            [70.0, 20.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(
        report.diagnostics.is_empty(),
        "pure Z rapids and retracts are out of scope for this rule"
    );
}

#[test]
fn retract_below_stock_fires_when_the_end_is_below_the_top() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Retract,
            [5.0, 5.0, -3.0],
            [5.0, 5.0, -1.0],
            None,
            4,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [5.0, 5.0, -1.0],
            [5.0, 5.0, 5.0],
            None,
            4,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "clearance.retract_below_stock");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 2);
    assert!(finding.message.contains("below the stock top"));
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&EvidenceValue::Text("explicit".to_string()))
    );
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&EvidenceValue::Point3([5.0, 5.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(1.0))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(4)));
}

#[test]
fn retract_exactly_at_the_stock_top_is_allowed() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Retract,
            [5.0, 5.0, -1.0],
            [5.0, 5.0, 0.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [5.0, 5.0, 0.0],
            [5.0, 5.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn retract_below_stock_ignores_canned_cycle_sub_retracts() {
    let mut peck = MotionRecord::linear(2, 0, MoveKind::Peck, [0.0, 0.0, 2.0], [0.0, 0.0, -5.0]);
    peck.canned = Some(Box::new(CannedRecord {
        cycle: CannedCycleKind::PeckDrill,
        hole_index: 0,
        r_plane_mm: -0.5,
        bottom_z_mm: -5.0,
        peck_q_mm: Some(2.0),
        dwell_s: None,
        retract_mode: RetractMode::RPlane,
    }));
    let program = ProgramBuilder::new()
        .record(peck)
        .record(motion(
            3,
            MoveKind::Rapid,
            [0.0, 0.0, -5.0],
            [0.0, 0.0, 5.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(
        report.diagnostics.is_empty(),
        "peck retracts come from expanded motions and must not create findings"
    );
}

#[test]
fn tool_end_in_material_fires_on_the_last_motion() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 5.0],
            [10.0, 0.0, -2.0],
            Some(100.0),
            3,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [10.0, 0.0, -2.0],
            [20.0, 0.0, -1.0],
            Some(100.0),
            3,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "clearance.tool_end_in_material");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 3);
    assert!(finding.message.contains("program ends with the tool"));
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&EvidenceValue::Text("explicit".to_string()))
    );
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&EvidenceValue::Point3([20.0, 0.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(1.0))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(3)));
}

#[test]
fn tool_end_exactly_at_the_stock_top_is_allowed() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 5.0],
            [10.0, 0.0, 0.0],
            Some(100.0),
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &explicit_stock_config(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn clearance_evidence_reports_inferred_stock_and_basis() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [10.0, 0.0, -1.0],
            Some(100.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [10.0, 0.0, -1.0],
            [20.0, 0.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            4,
            MoveKind::Retract,
            [20.0, 0.0, -1.0],
            [20.0, 0.0, -0.5],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(4),
        &AnalysisConfig::default(),
        &registry(rules::clearance::rules()),
    );

    assert_eq!(report.coverage.total, 3);
    assert_eq!(report.coverage.evaluated, 3);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );
    assert_eq!(report.diagnostics.len(), 3);

    let ids: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|finding| finding.rule.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "clearance.rapid_through_stock",
            "clearance.retract_below_stock",
            "clearance.tool_end_in_material"
        ]
    );
    for finding in &report.diagnostics {
        assert_eq!(
            finding.evidence.get("stock_source"),
            Some(&EvidenceValue::Text("inferred".to_string()))
        );
        assert_eq!(
            finding.evidence.get("inference_basis"),
            Some(&EvidenceValue::Text("zero_at_top".to_string()))
        );
    }
    assert_eq!(
        report.diagnostics[0].evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(1.0))
    );
    assert_eq!(
        report.diagnostics[1].evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(0.5))
    );
}

#[test]
fn clearance_rule_metadata_is_stable() {
    let rules = rules::clearance::rules();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert_eq!(
        ids,
        vec![
            "clearance.rapid_through_stock",
            "clearance.retract_below_stock",
            "clearance.tool_end_in_material"
        ]
    );

    for rule in &rules {
        assert_eq!(rule.default_severity(), Severity::Warning);
        assert_eq!(rule.basis(), Basis::Model);
        assert_eq!(rule.requirements().stock, StockRequirement::InferredOk);
        assert!(!rule.requirements().machine);
        assert!(!rule.requirements().material);
        assert!(rule.requirements().tool_fields.is_empty());
        assert!(!rule.why_it_matters().is_empty());
    }
}

#[test]
fn multi_rule_report_is_byte_deterministic() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [70.0, 20.0, -1.0],
            Some(100.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [70.0, 20.0, -1.0],
            [-30.0, 20.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            4,
            MoveKind::Retract,
            [-30.0, 20.0, -1.0],
            [-30.0, 20.0, -0.5],
            None,
            1,
        ))
        .record(motion(
            5,
            MoveKind::Rapid,
            [-30.0, 20.0, -0.5],
            [-30.0, 20.0, -11.0],
            None,
            1,
        ))
        .build();
    let config = AnalysisConfig {
        machine: Some(machine_config()),
        ..explicit_stock_config()
    };
    let source = source_lines(5);
    let mut rule_list = rules::travel::rules();
    rule_list.extend(rules::clearance::rules());
    let registry = registry(rule_list);

    let first = run(&program, &source, &config, &registry);
    let second = run(&program, &source, &config, &registry);

    assert_eq!(first.coverage.total, 5);
    assert_eq!(first.coverage.evaluated, 5);
    assert_eq!(first.diagnostics.len(), 5);
    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
}
