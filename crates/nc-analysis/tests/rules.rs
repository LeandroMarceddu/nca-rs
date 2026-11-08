//! Vertical-slice rule tests: positive, near-miss, skip, coverage, determinism.

use std::collections::BTreeMap;

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, Report, Rule, RuleConfig, RuleId, RuleRegistry,
    RuleSettings, Severity, StockConfig, StockRequirement, analyze, rules,
};
use nc_core::{Dialect, MachineConfig};
use nc_gcode::{FeedMode, MotionRecord, MoveKind, Program, ProgramBuilder};

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
fn feed_missing_fires_on_first_cutting_motion() {
    let program = ProgramBuilder::new()
        .motion(2, MoveKind::Rapid, [0.0, 0.0, 10.0], [5.0, 0.0, 10.0], None)
        .record(motion(
            3,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            None,
            4,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &AnalysisConfig::default(),
        &registry(rules::feed_missing::rules()),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.feed_missing");
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 3);
    assert_eq!(finding.line_end, 3);
    assert_eq!(finding.message, "first cutting motion has no feed rate");
    assert_eq!(
        finding.evidence.get("tool"),
        Some(&nc_analysis::EvidenceValue::Int(4))
    );
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&nc_analysis::EvidenceValue::Text("feed".to_string()))
    );
    assert_eq!(
        finding.evidence.get("start_mm"),
        Some(&nc_analysis::EvidenceValue::Point3([0.0, 0.0, 0.0]))
    );
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&nc_analysis::EvidenceValue::Point3([10.0, 0.0, 0.0]))
    );
    assert!(!finding.evidence.contains_key("feed_mm_min"));
}

#[test]
fn feed_missing_checks_only_the_first_cutting_motion() {
    let program = ProgramBuilder::new()
        .motion(2, MoveKind::Rapid, [0.0, 0.0, 10.0], [5.0, 0.0, 10.0], None)
        .motion(
            3,
            MoveKind::Retract,
            [5.0, 0.0, 10.0],
            [5.0, 0.0, 15.0],
            None,
        )
        .motion(4, MoveKind::Probe, [5.0, 0.0, 15.0], [6.0, 0.0, 15.0], None)
        .motion(
            5,
            MoveKind::Feed,
            [6.0, 0.0, 15.0],
            [10.0, 0.0, 15.0],
            Some(250.0),
        )
        .motion(
            6,
            MoveKind::Feed,
            [10.0, 0.0, 15.0],
            [20.0, 0.0, 15.0],
            None,
        )
        .build();
    let report = run(
        &program,
        &source_lines(6),
        &AnalysisConfig::default(),
        &registry(rules::feed_missing::rules()),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn feed_missing_leaves_nonpositive_feed_to_its_own_rule() {
    let program = ProgramBuilder::new()
        .motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 5.0],
            [0.0, 0.0, -1.0],
            Some(0.0),
        )
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &AnalysisConfig::default(),
        &registry(rules::feed_missing::rules()),
    );

    assert!(
        report.diagnostics.is_empty(),
        "modal.feed_nonpositive owns this case"
    );
    assert_eq!(report.coverage.evaluated, 1);
}

#[test]
fn feed_missing_skips_inverse_time_on_the_first_cut() {
    let mut program = ProgramBuilder::new()
        .motion(3, MoveKind::Feed, [0.0, 0.0, 0.0], [10.0, 0.0, 0.0], None)
        .build();
    program.modal_table[0].feed_mode = FeedMode::InverseTime;

    let report = run(
        &program,
        &source_lines(3),
        &AnalysisConfig::default(),
        &registry(rules::feed_missing::rules()),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert!(report.diagnostics.is_empty());
    let skip = &report.coverage.skipped[0];
    assert_eq!(skip.rule.as_str(), "modal.feed_missing");
    assert_eq!(skip.missing_input, "feed.inverse_time");
    assert!(skip.reason.contains("line 3"));
    assert!(skip.reason.contains("G93"));
}

#[test]
fn machine_envelope_flags_each_axis_outside_travel() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [40.0, 0.0, 0.0],
            Some(100.0),
            2,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [40.0, 0.0, 0.0],
            [20.0, -20.0, 0.0],
            None,
            2,
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
        &registry(rules::machine_envelope::rules()),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 2);

    let x_finding = &report.diagnostics[0];
    assert_eq!(x_finding.rule.as_str(), "travel.machine_envelope");
    assert_eq!(x_finding.severity, Severity::Error);
    assert_eq!(x_finding.basis, Basis::Exact);
    assert_eq!(x_finding.line, 2);
    assert_eq!(
        x_finding.evidence.get("axis"),
        Some(&nc_analysis::EvidenceValue::Text("X".to_string()))
    );
    assert_eq!(
        x_finding.evidence.get("value_mm"),
        Some(&nc_analysis::EvidenceValue::Float(40.0))
    );
    assert_eq!(
        x_finding.evidence.get("min_mm"),
        Some(&nc_analysis::EvidenceValue::Float(-10.0))
    );
    assert_eq!(
        x_finding.evidence.get("max_mm"),
        Some(&nc_analysis::EvidenceValue::Float(30.0))
    );
    assert_eq!(
        x_finding.evidence.get("tool"),
        Some(&nc_analysis::EvidenceValue::Int(2))
    );
    assert_eq!(
        x_finding.evidence.get("end_mm"),
        Some(&nc_analysis::EvidenceValue::Point3([40.0, 0.0, 0.0]))
    );

    let y_finding = &report.diagnostics[1];
    assert_eq!(y_finding.line, 3);
    assert_eq!(
        y_finding.evidence.get("axis"),
        Some(&nc_analysis::EvidenceValue::Text("Y".to_string()))
    );
    assert_eq!(
        y_finding.evidence.get("value_mm"),
        Some(&nc_analysis::EvidenceValue::Float(-20.0))
    );
}

#[test]
fn machine_envelope_allows_points_exactly_on_the_boundary() {
    let program = ProgramBuilder::new()
        .motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 0.0],
            [30.0, -10.0, 10.0],
            None,
        )
        .build();
    let config = AnalysisConfig {
        machine: Some(machine_config()),
        ..AnalysisConfig::default()
    };
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &registry(rules::machine_envelope::rules()),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn machine_envelope_skips_without_machine_limits() {
    let program = ProgramBuilder::new().build();
    let report = run(
        &program,
        "",
        &AnalysisConfig::default(),
        &registry(rules::machine_envelope::rules()),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "machine");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "machine limits are required"
    );
}

#[test]
fn machine_envelope_skips_invalid_machine_limits() {
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
        &registry(rules::machine_envelope::rules()),
    );

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "machine");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "configured machine travel limits are invalid"
    );
}

#[test]
fn rapid_at_depth_fires_below_safe_z() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 5.0],
            [10.0, 0.0, -1.0],
            None,
            3,
        ))
        .record(motion(
            3,
            MoveKind::Retract,
            [10.0, 0.0, -1.0],
            [10.0, 0.0, 5.0],
            None,
            3,
        ))
        .record(motion(
            4,
            MoveKind::Rapid,
            [10.0, 0.0, 5.0],
            [20.0, 0.0, 5.0],
            None,
            3,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(4),
        &explicit_stock_config(),
        &registry(rules::rapid_at_depth::rules()),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "clearance.rapid_at_depth");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 2);
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&nc_analysis::EvidenceValue::Text("explicit".to_string()))
    );
    assert!(!finding.evidence.contains_key("inference_basis"));
    assert_eq!(
        finding.evidence.get("end_mm"),
        Some(&nc_analysis::EvidenceValue::Point3([10.0, 0.0, -1.0]))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&nc_analysis::EvidenceValue::Float(3.0))
    );
    assert_eq!(
        finding.evidence.get("threshold"),
        Some(&nc_analysis::EvidenceValue::Float(2.0))
    );
    assert_eq!(
        finding.evidence.get("tool"),
        Some(&nc_analysis::EvidenceValue::Int(3))
    );
}

#[test]
fn rapid_at_depth_allows_safe_z_and_fires_one_tolerance_below() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 5.0],
            [10.0, 0.0, 2.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [10.0, 0.0, 2.0],
            [20.0, 0.0, 0.0],
            None,
            1,
        ))
        .record(motion(
            4,
            MoveKind::Rapid,
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 3.0],
            None,
            1,
        ))
        .record(motion(
            5,
            MoveKind::Retract,
            [30.0, 0.0, 3.0],
            [30.0, 0.0, 6.0],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(5),
        &explicit_stock_config(),
        &registry(rules::rapid_at_depth::rules()),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.line, 3);
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&nc_analysis::EvidenceValue::Float(2.0))
    );
}

#[test]
fn rapid_at_depth_honors_a_threshold_override() {
    let mut thresholds = BTreeMap::new();
    thresholds.insert("clearance_mm".to_string(), 5.0);
    let mut rule_config = RuleConfig::default();
    rule_config.set(
        RuleId::new("clearance.rapid_at_depth"),
        RuleSettings {
            thresholds,
            ..RuleSettings::default()
        },
    );
    let config = AnalysisConfig {
        rules: rule_config,
        ..explicit_stock_config()
    };
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 6.0],
            [10.0, 0.0, 4.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Rapid,
            [10.0, 0.0, 4.0],
            [20.0, 0.0, 5.5],
            None,
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &registry(rules::rapid_at_depth::rules()),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.line, 2);
    assert_eq!(
        finding.evidence.get("threshold"),
        Some(&nc_analysis::EvidenceValue::Float(5.0))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&nc_analysis::EvidenceValue::Float(1.0))
    );
}

#[test]
fn rapid_at_depth_reports_inferred_stock_and_coverage() {
    let program = ProgramBuilder::new()
        .motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 5.0],
            [10.0, 0.0, -1.0],
            Some(100.0),
        )
        .motion(
            3,
            MoveKind::Rapid,
            [10.0, 0.0, -1.0],
            [20.0, 0.0, -1.0],
            None,
        )
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &AnalysisConfig::default(),
        &registry(rules::rapid_at_depth::rules()),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(
        finding.evidence.get("stock_source"),
        Some(&nc_analysis::EvidenceValue::Text("inferred".to_string()))
    );
    assert_eq!(
        finding.evidence.get("inference_basis"),
        Some(&nc_analysis::EvidenceValue::Text("zero_at_top".to_string()))
    );
    assert_eq!(
        finding.evidence.get("depth_mm"),
        Some(&nc_analysis::EvidenceValue::Float(3.0))
    );
}

#[test]
fn catalog_metadata_and_order_are_stable() {
    let rules = rules::all();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert!(!ids.is_empty());
    let unique: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "rule ids must be unique");

    for required in [
        "modal.feed_missing",
        "travel.machine_envelope",
        "clearance.rapid_at_depth",
    ] {
        assert!(ids.iter().any(|id| id == required), "missing {required}");
    }

    let feed = rules
        .iter()
        .find(|rule| rule.id().as_str() == "modal.feed_missing")
        .expect("feed rule");
    assert_eq!(feed.default_severity(), Severity::Error);
    assert_eq!(feed.basis(), Basis::Exact);
    let feed_requirements = feed.requirements();
    assert!(feed_requirements.tool_fields.is_empty());
    assert!(!feed_requirements.machine);
    assert!(!feed_requirements.material);
    assert_eq!(feed_requirements.stock, StockRequirement::None);

    let envelope = rules
        .iter()
        .find(|rule| rule.id().as_str() == "travel.machine_envelope")
        .expect("envelope rule");
    assert_eq!(envelope.default_severity(), Severity::Error);
    assert!(envelope.requirements().machine);
    assert_eq!(envelope.requirements().stock, StockRequirement::None);

    let rapid = rules
        .iter()
        .find(|rule| rule.id().as_str() == "clearance.rapid_at_depth")
        .expect("rapid rule");
    assert_eq!(rapid.default_severity(), Severity::Warning);
    assert_eq!(rapid.basis(), Basis::Model);
    assert_eq!(rapid.requirements().stock, StockRequirement::InferredOk);
    assert!(!rapid.requirements().machine);

    for rule in &rules {
        assert!(!rule.id().as_str().is_empty());
        assert!(!rule.why_it_matters().is_empty(), "{}", rule.id());
    }
}

#[test]
fn registry_reports_coverage_and_stable_diagnostics() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 10.0],
            [10.0, 0.0, -1.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [10.0, 0.0, -1.0],
            [20.0, 0.0, -1.0],
            None,
            1,
        ))
        .build();
    let source = source_lines(3);
    let config = AnalysisConfig::default();
    let registry = registry(rules::all());

    let first = run(&program, &source, &config, &registry);
    let second = run(&program, &source, &config, &registry);

    assert_eq!(first.coverage.total, rules::all().len());
    assert_eq!(
        first.coverage.evaluated + first.coverage.skipped_rule_count(),
        first.coverage.total
    );
    assert!(first.coverage.evaluated > 0);
    assert!(first.coverage.skipped.iter().any(|skip| {
        skip.rule.as_str() == "travel.machine_envelope" && skip.missing_input == "machine"
    }));
    assert!(
        first
            .diagnostics
            .iter()
            .any(|finding| { finding.rule.as_str() == "modal.feed_missing" && finding.line == 3 })
    );

    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
}
