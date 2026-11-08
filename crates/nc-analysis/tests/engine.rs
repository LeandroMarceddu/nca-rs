//! Public engine tests: requirements, coverage, report assembly, determinism.

use std::collections::BTreeMap;

use nc_analysis::{
    AnalysisConfig, AnalysisModel, AnalysisRequest, Basis, Coverage, Diagnostic, EvidenceValue,
    Rule, RuleId, RuleOutcome, RuleRegistry, RuleSettings, Severity, StockConfig, StockRequirement,
    analyze,
};
use nc_core::{
    Dialect, FieldSource, MachineConfig, MaterialPreset, Tool, ToolField, ToolHint, ToolShape,
    ToolTable,
};
use nc_gcode::{MotionRecord, MoveKind, Program, ProgramBuilder};
use nc_stock::StockSource;
use sha2::{Digest, Sha256};

#[derive(Clone)]
enum TestOutcome {
    Findings(Vec<Diagnostic>),
    Skip(&'static str, &'static str),
}

struct TestRule {
    id: RuleId,
    default_severity: Severity,
    basis: Basis,
    requirements: nc_analysis::Requirements,
    outcome: TestOutcome,
}

impl TestRule {
    fn new(id: &str) -> Self {
        Self {
            id: RuleId::owned(id),
            default_severity: Severity::Warning,
            basis: Basis::Exact,
            requirements: nc_analysis::Requirements::NONE,
            outcome: TestOutcome::Findings(Vec::new()),
        }
    }

    fn severity(mut self, severity: Severity) -> Self {
        self.default_severity = severity;
        self
    }

    fn requirements(mut self, requirements: nc_analysis::Requirements) -> Self {
        self.requirements = requirements;
        self
    }

    fn findings(mut self, findings: Vec<Diagnostic>) -> Self {
        self.outcome = TestOutcome::Findings(findings);
        self
    }

    fn skip(mut self, missing_input: &'static str, reason: &'static str) -> Self {
        self.outcome = TestOutcome::Skip(missing_input, reason);
        self
    }
}

impl Rule for TestRule {
    fn id(&self) -> RuleId {
        self.id.clone()
    }

    fn default_severity(&self) -> Severity {
        self.default_severity
    }

    fn basis(&self) -> Basis {
        self.basis
    }

    fn requirements(&self) -> nc_analysis::Requirements {
        self.requirements
    }

    fn evaluate(&self, _model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        match &self.outcome {
            TestOutcome::Findings(findings) => RuleOutcome::Findings(findings.clone()),
            TestOutcome::Skip(missing_input, reason) => RuleOutcome::Skipped {
                missing_input: (*missing_input).to_string(),
                reason: (*reason).to_string(),
            },
        }
    }
}

fn boxed(rule: TestRule) -> Box<dyn Rule> {
    Box::new(rule)
}

fn finding(severity: Severity, line: usize) -> Diagnostic {
    Diagnostic {
        rule: RuleId::new("placeholder"),
        severity,
        basis: Basis::Model,
        line,
        column: 0,
        line_end: line,
        message: format!("finding at line {line}"),
        evidence: BTreeMap::new(),
    }
}

fn source_lines(count: usize) -> String {
    let mut source = String::new();
    for line in 1..=count {
        source.push_str(&format!("N{line}\n"));
    }
    source
}

fn program_with_tools(tools: &[u32]) -> Program {
    let mut builder = ProgramBuilder::new().source(1, "G21 G90");
    for (index, tool) in tools.iter().enumerate() {
        let line = index + 2;
        builder = builder.source(line, format!("G1 X{index} F100"));
        let mut motion = MotionRecord::linear(
            line,
            index,
            MoveKind::Feed,
            [index as f64, 0.0, -1.0],
            [index as f64 + 1.0, 0.0, -1.0],
        );
        motion.tool = *tool;
        motion.feed_mm_min = Some(100.0);
        builder = builder.record(motion);
    }
    builder.build()
}

fn run(
    config: &AnalysisConfig,
    program: &Program,
    source: &str,
    registry: &RuleRegistry,
) -> nc_analysis::Report {
    let request = AnalysisRequest::new("part.nc", source, config, Dialect::Generic);
    analyze(program, &request, registry)
}

fn tool_with_flutes(number: u32, flutes: u32) -> Tool {
    let mut tool = Tool::new(number);
    tool.flutes = Some(flutes);
    tool.sources.insert(ToolField::Flutes, FieldSource::Config);
    tool
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[test]
fn empty_registry_reports_zero_coverage() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let report = run(&config, &program, "", &RuleRegistry::new());

    assert_eq!(report.schema, "nca.report/1");
    assert_eq!(report.ruleset_version, "1.0.0");
    assert_eq!(report.input.file_name, "part.nc");
    assert_eq!(report.input.line_count, 0);
    assert_eq!(
        report.input.sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(report.input.dialect, Dialect::Generic);
    assert_eq!(report.stock_source, StockSource::Inferred);
    assert_eq!(report.config_digest.len(), 64);
    assert!(report.tool_inventory.is_empty());
    assert!(report.diagnostics.is_empty());
    assert!(report.truncated.is_empty());
    assert_eq!(
        report.coverage,
        Coverage {
            evaluated: 0,
            total: 0,
            skipped: Vec::new(),
        }
    );
    assert!(!report.to_json().ends_with('\n'));
    assert!(!report.to_json_pretty().ends_with('\n'));
}

#[test]
fn a_rule_without_requirements_returns_findings() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().source(1, "G21").build();
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.ok").findings(vec![finding(Severity::Info, 1)]),
    )]);
    let report = run(&config, &program, "G21\n", &registry);

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.coverage.total, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.rule.as_str(), "test.ok");
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.basis, Basis::Exact);
}

#[test]
fn zero_findings_still_counts_as_evaluated() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([boxed(TestRule::new("test.empty"))]);
    let report = run(&config, &program, "", &registry);

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.coverage.total, 1);
    assert!(report.coverage.skipped.is_empty());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn explicit_stock_requirement_skips_inferred_stock() {
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([boxed(TestRule::new("test.explicit").requirements(
        nc_analysis::Requirements::with_stock(StockRequirement::Explicit),
    ))]);

    let inferred = AnalysisConfig::default();
    let report = run(&inferred, &program, "", &registry);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].rule.as_str(), "test.explicit");
    assert_eq!(report.coverage.skipped[0].missing_input, "stock.explicit");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "explicit stock is required, but the stock was inferred"
    );

    let explicit = AnalysisConfig {
        stock: Some(StockConfig {
            origin_mm: [-10.0, -10.0, -10.0],
            size_mm: [20.0, 20.0, 10.0],
        }),
        ..AnalysisConfig::default()
    };
    let report = run(&explicit, &program, "", &registry);
    assert_eq!(report.stock_source, StockSource::Explicit);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
}

#[test]
fn inferred_and_no_stock_requirements_run_on_inferred_stock() {
    let program = ProgramBuilder::new().build();
    let registry =
        RuleRegistry::from_rules([
            boxed(
                TestRule::new("test.none").requirements(nc_analysis::Requirements::with_stock(
                    StockRequirement::None,
                )),
            ),
            boxed(TestRule::new("test.inferred").requirements(
                nc_analysis::Requirements::with_stock(StockRequirement::InferredOk),
            )),
        ]);
    let report = run(&AnalysisConfig::default(), &program, "", &registry);

    assert_eq!(report.stock_source, StockSource::Inferred);
    assert_eq!(report.coverage.evaluated, 2);
    assert!(report.coverage.skipped.is_empty());
}

#[test]
fn machine_and_material_requirements_use_exact_keys_and_reasons() {
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([
        boxed(
            TestRule::new("test.machine").requirements(nc_analysis::Requirements {
                machine: true,
                ..nc_analysis::Requirements::NONE
            }),
        ),
        boxed(
            TestRule::new("test.material").requirements(nc_analysis::Requirements {
                material: true,
                ..nc_analysis::Requirements::NONE
            }),
        ),
    ]);
    let report = run(&AnalysisConfig::default(), &program, "", &registry);

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 2);
    assert_eq!(report.coverage.skipped[0].rule.as_str(), "test.machine");
    assert_eq!(report.coverage.skipped[0].missing_input, "machine");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "machine limits are required"
    );
    assert_eq!(report.coverage.skipped[1].rule.as_str(), "test.material");
    assert_eq!(report.coverage.skipped[1].missing_input, "material");
    assert_eq!(
        report.coverage.skipped[1].reason,
        "a material preset is required"
    );

    let configured = AnalysisConfig {
        machine: Some(
            MachineConfig::new([0.0, 0.0, -100.0], [100.0, 100.0, 0.0]).expect("machine"),
        ),
        material: Some(MaterialPreset::new("aluminum-6061")),
        ..AnalysisConfig::default()
    };
    let report = run(&configured, &program, "", &registry);
    assert_eq!(report.coverage.evaluated, 2);
    assert!(report.coverage.skipped.is_empty());
}

#[test]
fn tool_field_skips_cover_every_tool_with_motions_only() {
    let mut tools = ToolTable::new();
    tools.insert(tool_with_flutes(1, 2));
    tools.insert(Tool::new(3));
    let config = AnalysisConfig {
        tools,
        ..AnalysisConfig::default()
    };
    let program = program_with_tools(&[1, 2]);
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.flutes")
            .requirements(nc_analysis::Requirements::with_tools(&[ToolField::Flutes])),
    )]);
    let report = run(&config, &program, &source_lines(3), &registry);

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "tool.flutes:T2");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "tool T2 has no flute count"
    );
    assert!(
        !report
            .coverage
            .skipped
            .iter()
            .any(|skip| skip.missing_input.contains("T3"))
    );
}

#[test]
fn multiple_tool_skips_are_sorted_and_deduplicated() {
    let config = AnalysisConfig::default();
    let program = program_with_tools(&[2, 1, 2]);
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.flutes")
            .requirements(nc_analysis::Requirements::with_tools(&[ToolField::Flutes])),
    )]);
    let report = run(&config, &program, &source_lines(5), &registry);

    let keys: Vec<&str> = report
        .coverage
        .skipped
        .iter()
        .map(|skip| skip.missing_input.as_str())
        .collect();
    assert_eq!(keys, vec!["tool.flutes:T1", "tool.flutes:T2"]);
    assert_eq!(report.coverage.skipped_rule_count(), 1);
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );
}

#[test]
fn underscore_field_keys_match_the_coverage_format() {
    let config = AnalysisConfig::default();
    let program = program_with_tools(&[1]);
    let registry =
        RuleRegistry::from_rules([boxed(TestRule::new("test.flute_length").requirements(
            nc_analysis::Requirements::with_tools(&[ToolField::FluteLength]),
        ))]);
    let report = run(&config, &program, &source_lines(2), &registry);

    assert_eq!(
        report.coverage.skipped[0].missing_input,
        "tool.flute_length:T1"
    );
    assert_eq!(
        report.coverage.skipped[0].reason,
        "tool T1 has no flute length"
    );
}

#[test]
fn disabled_rule_is_skipped_before_requirements() {
    let config = AnalysisConfig {
        rules: {
            let mut rules = nc_analysis::RuleConfig::default();
            rules.set(
                RuleId::new("test.disabled"),
                RuleSettings {
                    enabled: false,
                    ..RuleSettings::default()
                },
            );
            rules
        },
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([boxed(TestRule::new("test.disabled").requirements(
        nc_analysis::Requirements {
            machine: true,
            ..nc_analysis::Requirements::NONE
        },
    ))]);
    let report = run(&config, &program, "", &registry);

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "rule.disabled");
    assert_eq!(
        report.coverage.skipped[0].reason,
        "rule is disabled in configuration"
    );
}

#[test]
fn evaluate_skip_is_recorded_uniformly() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.evaluate_skip")
            .skip("geometry.cutter_comp", "cutter compensation is active"),
    )]);
    let report = run(&config, &program, "", &registry);

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(
        report.coverage.skipped[0].missing_input,
        "geometry.cutter_comp"
    );
    assert_eq!(
        report.coverage.skipped[0].reason,
        "cutter compensation is active"
    );
}

#[test]
fn coverage_counts_add_up_for_one_skip_per_rule() {
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([
        boxed(TestRule::new("test.ok")),
        boxed(
            TestRule::new("test.stock").requirements(nc_analysis::Requirements::with_stock(
                StockRequirement::Explicit,
            )),
        ),
        boxed(
            TestRule::new("test.machine").requirements(nc_analysis::Requirements {
                machine: true,
                ..nc_analysis::Requirements::NONE
            }),
        ),
    ]);
    let report = run(&AnalysisConfig::default(), &program, "", &registry);

    assert_eq!(report.coverage.total, 3);
    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.coverage.skipped.len(), 2);
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped.len(),
        report.coverage.total
    );
}

#[test]
fn diagnostics_sort_by_severity_then_line() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let source = source_lines(20);
    let registry = RuleRegistry::from_rules([
        boxed(
            TestRule::new("test.info")
                .severity(Severity::Info)
                .findings(vec![finding(Severity::Info, 1)]),
        ),
        boxed(
            TestRule::new("test.error")
                .severity(Severity::Error)
                .findings(vec![finding(Severity::Error, 10)]),
        ),
        boxed(
            TestRule::new("test.warning")
                .severity(Severity::Warning)
                .findings(vec![finding(Severity::Warning, 2)]),
        ),
    ]);
    let report = run(&config, &program, &source, &registry);

    let order: Vec<(Severity, usize)> = report
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.severity, diagnostic.line))
        .collect();
    assert_eq!(
        order,
        vec![
            (Severity::Error, 10),
            (Severity::Warning, 2),
            (Severity::Info, 1)
        ]
    );
}

#[test]
fn identical_diagnostics_are_deduplicated_before_caps() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let duplicate = finding(Severity::Warning, 3);
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.duplicate").findings(vec![duplicate.clone(), duplicate]),
    )]);
    let report = run(&config, &program, &source_lines(5), &registry);

    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.truncated.is_empty());
}

#[test]
fn per_rule_cap_is_one_hundred_and_reports_dropped_counts() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let findings: Vec<Diagnostic> = (1..=105)
        .map(|line| finding(Severity::Warning, line))
        .collect();
    let registry = RuleRegistry::from_rules([boxed(TestRule::new("cap.rule").findings(findings))]);
    let report = run(&config, &program, &source_lines(105), &registry);

    assert_eq!(report.diagnostics.len(), 100);
    assert_eq!(report.truncated.get("cap.rule"), Some(&5));
    assert_eq!(report.truncated.values().sum::<usize>(), 5);
    assert_eq!(report.diagnostics[0].line, 1);
    assert_eq!(report.diagnostics[99].line, 100);
}

#[test]
fn total_cap_is_one_thousand_and_reports_dropped_counts() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let source = source_lines(100);
    let mut registry = RuleRegistry::new();
    for index in 0..11 {
        let findings: Vec<Diagnostic> = (1..=100)
            .map(|line| finding(Severity::Warning, line))
            .collect();
        registry.register(boxed(
            TestRule::new(&format!("cap.rule.{index:02}")).findings(findings),
        ));
    }
    let report = run(&config, &program, &source, &registry);

    assert_eq!(report.diagnostics.len(), 1000);
    assert_eq!(report.truncated.values().sum::<usize>(), 100);
    assert_eq!(report.truncated.get("cap.rule.10"), Some(&10));
}

#[test]
fn diagnostic_lines_are_clamped_to_the_source() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let registry = RuleRegistry::from_rules([boxed(
        TestRule::new("test.line").findings(vec![finding(Severity::Warning, 999)]),
    )]);
    let report = run(&config, &program, &source_lines(5), &registry);

    assert_eq!(report.diagnostics[0].line, 5);
    assert_eq!(report.diagnostics[0].line_end, 5);
}

#[test]
fn report_json_is_byte_deterministic() {
    let config = AnalysisConfig::default();
    let program = program_with_tools(&[1]);
    let mut evidence = BTreeMap::new();
    evidence.insert("depth_mm".to_string(), EvidenceValue::Float(1.23456));
    let registry =
        RuleRegistry::from_rules([boxed(TestRule::new("test.deterministic").findings(vec![
            Diagnostic {
                evidence,
                ..finding(Severity::Warning, 2)
            },
        ]))]);
    let source = source_lines(2);

    let first = run(&config, &program, &source, &registry);
    let second = run(&config, &program, &source, &registry);
    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
}

#[test]
fn report_contains_no_absolute_path() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let request = AnalysisRequest::new("/tmp/x/part.nc", "", &config, Dialect::Generic);
    let report = analyze(&program, &request, &RuleRegistry::new());

    assert_eq!(report.input.file_name, "part.nc");
    let json = report.to_json();
    assert!(!json.contains("/tmp"));
    assert!(json.contains("\"file_name\":\"part.nc\""));
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(value["input"]["file_name"], "part.nc");
}

#[test]
fn tool_inventory_has_fields_and_comment_provenance() {
    let mut hint = ToolHint::new(1, 4);
    hint.diameter_mm = Some(3.175);
    hint.flutes = Some(2);
    let mut tools = ToolTable::resolve(&ToolTable::new(), &[hint]);
    let mut tool_two = Tool::new(2);
    tool_two.diameter_mm = Some(6.0);
    tool_two
        .sources
        .insert(ToolField::Diameter, FieldSource::Config);
    tool_two.shape = ToolShape::Flat;
    tools.insert(tool_two);

    let config = AnalysisConfig {
        tools,
        ..AnalysisConfig::default()
    };
    let mut builder = ProgramBuilder::new().source(1, "G21 G90");
    let mut first = MotionRecord::linear(2, 0, MoveKind::Feed, [0.0, 0.0, -1.0], [10.0, 0.0, -1.0]);
    first.tool = 1;
    let mut second =
        MotionRecord::linear(4, 1, MoveKind::Feed, [10.0, 0.0, -1.0], [20.0, 0.0, -1.0]);
    second.tool = 1;
    let mut third =
        MotionRecord::linear(6, 2, MoveKind::Feed, [20.0, 0.0, -1.0], [30.0, 0.0, -1.0]);
    third.tool = 2;
    builder = builder.record(first).record(second).record(third);
    let program = builder.build();
    let report = run(&config, &program, &source_lines(6), &RuleRegistry::new());

    assert_eq!(report.tool_inventory.len(), 2);
    let tool_one = &report.tool_inventory[0];
    assert_eq!(tool_one.tool, 1);
    assert_eq!(tool_one.first_line, 2);
    assert_eq!(tool_one.motion_count, 2);
    assert_eq!(
        tool_one.fields["diameter_mm"].value,
        EvidenceValue::Float(3.175)
    );
    assert_eq!(tool_one.fields["diameter_mm"].source, "comment:L4");
    assert_eq!(tool_one.fields["flutes"].value, EvidenceValue::Int(2));
    assert_eq!(tool_one.fields["flutes"].source, "comment:L4");

    let tool_two = &report.tool_inventory[1];
    assert_eq!(tool_two.tool, 2);
    assert_eq!(tool_two.first_line, 6);
    assert_eq!(tool_two.motion_count, 1);
    assert_eq!(
        tool_two.fields["diameter_mm"].value,
        EvidenceValue::Float(6.0)
    );
    assert_eq!(tool_two.fields["diameter_mm"].source, "config");
    assert_eq!(
        tool_two.fields["shape"].value,
        EvidenceValue::Text("flat".to_string())
    );

    let json = report.to_json();
    assert!(json.contains("\"diameter_mm\":{\"value\":3.175,\"source\":\"comment:L4\"}"));
    assert!(json.contains("\"diameter_mm\":{\"value\":6.000,\"source\":\"config\"}"));
}

#[test]
fn evidence_floats_are_quantized_by_key() {
    let mut evidence = BTreeMap::new();
    evidence.insert("depth_mm".to_string(), EvidenceValue::Float(1.23456));
    evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(800.04));
    evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(11999.6));
    evidence.insert("angle_deg".to_string(), EvidenceValue::Float(89.996));
    evidence.insert("width_percent".to_string(), EvidenceValue::Float(12.36));
    evidence.insert("other".to_string(), EvidenceValue::Float(2.3456));
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3([0.1234, 2.0, -0.0]),
    );
    evidence.insert("flag".to_string(), EvidenceValue::Bool(true));
    evidence.insert("tool".to_string(), EvidenceValue::Int(3));
    evidence.insert("note".to_string(), EvidenceValue::Text("hello".to_string()));

    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let registry =
        RuleRegistry::from_rules([boxed(TestRule::new("test.quantize").findings(vec![
            Diagnostic {
                evidence,
                ..finding(Severity::Warning, 1)
            },
        ]))]);
    let report = run(&config, &program, "N1\n", &registry);
    let json = report.to_json();

    assert!(json.contains("\"depth_mm\":1.235"));
    assert!(json.contains("\"feed_mm_min\":800.0"));
    assert!(json.contains("\"spindle_rpm\":12000,"));
    assert!(json.contains("\"angle_deg\":90.00"));
    assert!(json.contains("\"width_percent\":12.4"));
    assert!(json.contains("\"other\":2.346"));
    assert!(json.contains("\"start_mm\":[0.123,2.000,0.000]"));
    assert!(json.contains("\"flag\":true"));
    assert!(json.contains("\"tool\":3"));
    assert!(json.contains("\"note\":\"hello\""));

    let pretty = report.to_json_pretty();
    assert!(pretty.contains("\"depth_mm\": 1.235"));
    assert!(pretty.contains('\n'));
}

#[test]
fn sha256_and_line_count_describe_the_source_bytes() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let request = AnalysisRequest::new("part.nc", "abc", &config, Dialect::Generic);
    let report = analyze(&program, &request, &RuleRegistry::new());

    assert_eq!(
        report.input.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(report.input.line_count, 1);

    let request = AnalysisRequest::new("part.nc", "G21\nG90\n", &config, Dialect::Generic);
    let report = analyze(&program, &request, &RuleRegistry::new());
    assert_eq!(report.input.line_count, 2);
}

#[test]
fn config_digest_is_sha256_of_canonical_json() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let report = run(&config, &program, "", &RuleRegistry::new());
    let canonical = serde_json::to_vec(&config).expect("config serializes");
    assert_eq!(report.config_digest, sha256_hex(&canonical));
    assert_eq!(report.config_digest, report.config_digest.to_lowercase());

    let changed = AnalysisConfig {
        tolerances: nc_analysis::Tolerances {
            clearance_mm: 3.0,
            ..nc_analysis::Tolerances::default()
        },
        ..AnalysisConfig::default()
    };
    let changed_report = run(&changed, &program, "", &RuleRegistry::new());
    assert_ne!(report.config_digest, changed_report.config_digest);
}

#[test]
fn request_dialect_ruleset_version_and_geometry_uncertainty_are_reported() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().uncertain_after(7).build();
    let source = source_lines(7);
    let request = AnalysisRequest::new("part.nc", &source, &config, Dialect::LinuxCnc)
        .with_ruleset_version("2.3.4");
    let report = analyze(&program, &request, &RuleRegistry::new());

    assert_eq!(report.ruleset_version, "2.3.4");
    assert_eq!(report.input.dialect, Dialect::LinuxCnc);
    assert_eq!(report.input.geometry_uncertain_after, Some(7));
    let json = report.to_json();
    assert!(json.contains("\"dialect\":\"linux_cnc\""));
    assert!(json.contains("\"geometry_uncertain_after\":7"));
}

#[test]
fn report_deserializes_after_serialization() {
    let config = AnalysisConfig::default();
    let program = ProgramBuilder::new().build();
    let report = run(&config, &program, "N1\n", &RuleRegistry::new());
    let json = report.to_json();
    let parsed: nc_analysis::Report = serde_json::from_str(&json).expect("report json");
    assert_eq!(parsed, report);

    let pretty = report.to_json_pretty();
    assert!(pretty.starts_with("{\n  \"schema\":"));
    assert!(!pretty.ends_with('\n'));
    let parsed: nc_analysis::Report = serde_json::from_str(&pretty).expect("pretty report json");
    assert_eq!(parsed, report);
}
