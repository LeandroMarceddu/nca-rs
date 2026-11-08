//! `modal.*` rule tests: positive, near-miss, skipped inputs, coverage, and
//! byte determinism.

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, EvidenceValue, Report, Rule, RuleRegistry, Severity,
    StockRequirement, analyze, rules,
};
use nc_core::{Dialect, FieldSource, MachineConfig, Tool, ToolField, ToolTable};
use nc_gcode::{MotionRecord, MoveKind, Program, ProgramBuilder, parse_program};

/// Source text with `count` one-word lines so diagnostics are not clamped.
fn source_lines(count: usize) -> String {
    (1..=count).map(|line| format!("N{line}\n")).collect()
}

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

/// Registry with only the named rule from the modal module.
fn modal_only(id: &str) -> RuleRegistry {
    let selected: Vec<Box<dyn Rule>> = rules::modal::rules()
        .into_iter()
        .filter(|rule| rule.id().as_str() == id)
        .collect();
    assert_eq!(selected.len(), 1, "rule {id} must exist exactly once");
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

/// A tool with a configured handedness.
fn handed_tool(number: u32, right_hand: bool) -> Tool {
    let mut tool = Tool::new(number);
    tool.right_hand = Some(right_hand);
    tool.sources
        .insert(ToolField::RightHand, FieldSource::Config);
    tool
}

/// A configuration with the given tools and nothing else.
fn config_with_tools(tools: ToolTable) -> AnalysisConfig {
    AnalysisConfig {
        tools,
        ..AnalysisConfig::default()
    }
}

#[test]
fn units_missing_reports_only_the_first_motion() {
    let source = "G0 X10\nG0 X20\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.units_missing"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.units_missing");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 1);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("rapid".to_string()))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(0)));
}

#[test]
fn units_missing_is_silent_after_g21() {
    let source = "G21\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.units_missing"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn distance_missing_reports_only_the_first_motion() {
    let source = "G0 X10\nG0 X20\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.distance_missing"),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.distance_missing");
    assert_eq!(finding.line, 1);
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
}

#[test]
fn distance_missing_is_silent_after_g90() {
    let source = "G90\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.distance_missing"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn feed_nonpositive_reports_each_cutting_motion_at_f0() {
    // The strict parser rejects a zero feed before a cut, so a hand-built
    // record stands in for a program that reached the analyzer another way
    // (lossy import, report reload, or a controller log).
    let program = ProgramBuilder::new()
        .source(1, "F0")
        .source(2, "G1 X10")
        .source(3, "G1 X20")
        .motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(0.0),
        )
        .motion(
            3,
            MoveKind::Feed,
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            Some(0.0),
        )
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &AnalysisConfig::default(),
        &modal_only("modal.feed_nonpositive"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    let findings = findings_for(&report, "modal.feed_nonpositive");
    assert_eq!(findings.len(), 2);
    let first = findings[0];
    assert_eq!(first.severity, Severity::Error);
    assert_eq!(first.basis, Basis::Exact);
    assert_eq!(first.line, 2);
    assert_eq!(
        first.evidence.get("feed_mm_min"),
        Some(&EvidenceValue::Float(0.0))
    );
    assert_eq!(
        first.evidence.get("kind"),
        Some(&EvidenceValue::Text("feed".to_string()))
    );
    assert_eq!(findings[1].line, 3);
}

#[test]
fn feed_nonpositive_is_silent_for_a_positive_feed() {
    let source = "G21 G90\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.feed_nonpositive"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn spindle_stopped_reports_a_cut_without_m3_or_m4() {
    let program = ProgramBuilder::new()
        .source(1, "G1 X10 F100")
        .motion(
            1,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(100.0),
        )
        .build();
    let report = run(
        &program,
        &source_lines(1),
        &AnalysisConfig::default(),
        &modal_only("modal.spindle_stopped"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.spindle_stopped");
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 1);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("feed".to_string()))
    );
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(0)));
}

#[test]
fn spindle_stopped_is_silent_when_m3_is_active() {
    let mut record = MotionRecord::linear(1, 0, MoveKind::Feed, [0.0, 0.0, 0.0], [10.0, 0.0, 0.0]);
    record.feed_mm_min = Some(100.0);
    record.spindle_rpm = Some(8000.0);
    record.spindle_cw = Some(true);
    let program = ProgramBuilder::new()
        .source(1, "S8000 M3 G1 X10 F100")
        .record(record)
        .build();
    let report = run(
        &program,
        &source_lines(1),
        &AnalysisConfig::default(),
        &modal_only("modal.spindle_stopped"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn spindle_stopped_skips_a_manual_spindle() {
    let mut machine = MachineConfig::new([0.0; 3], [100.0; 3]).expect("valid machine");
    machine.spindle_manual = true;
    let config = AnalysisConfig {
        machine: Some(machine),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .motion(
            1,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(100.0),
        )
        .build();
    let report = run(
        &program,
        &source_lines(1),
        &config,
        &modal_only("modal.spindle_stopped"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    let skip = &report.coverage.skipped[0];
    assert_eq!(skip.rule.as_str(), "modal.spindle_stopped");
    assert_eq!(skip.missing_input, "machine.spindle_manual");
    assert!(skip.reason.contains("manual"));
    assert!(report.diagnostics.is_empty());
}

#[test]
fn spindle_speed_zero_reports_m3_without_a_speed() {
    let source = "G21 G90\nM3\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.spindle_speed_zero"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.spindle_speed_zero");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(
        finding.evidence.get("clockwise"),
        Some(&EvidenceValue::Bool(true))
    );
    assert_eq!(
        finding.evidence.get("spindle_rpm"),
        Some(&EvidenceValue::Float(0.0))
    );
}

#[test]
fn spindle_speed_zero_is_silent_with_a_positive_speed() {
    let source = "G21 G90\nS8000 M3\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.spindle_speed_zero"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn spindle_direction_reports_m4_with_a_right_hand_tool() {
    let source = "G21 G90\nT1 M6\nS8000 M4\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let mut tools = ToolTable::new();
    tools.insert(handed_tool(1, true));
    let report = run(
        &program,
        source,
        &config_with_tools(tools),
        &modal_only("modal.spindle_direction"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.spindle_direction");
    assert_eq!(finding.severity, Severity::Info);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 3);
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(1)));
    assert_eq!(
        finding.evidence.get("clockwise"),
        Some(&EvidenceValue::Bool(false))
    );
    assert_eq!(
        finding.evidence.get("spindle_rpm"),
        Some(&EvidenceValue::Float(8000.0))
    );
    assert_eq!(
        finding.evidence.get("tool_source"),
        Some(&EvidenceValue::Text("config".to_string()))
    );
}

#[test]
fn spindle_direction_is_silent_for_a_left_hand_tool() {
    let source = "G21 G90\nT1 M6\nS8000 M4\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let mut tools = ToolTable::new();
    tools.insert(handed_tool(1, false));
    let report = run(
        &program,
        source,
        &config_with_tools(tools),
        &modal_only("modal.spindle_direction"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_unknown_reports_the_first_line_of_a_missing_tool() {
    let source = "G21 G90\nT1 M6\nT2 M6\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let mut tools = ToolTable::new();
    tools.insert(Tool::new(1));
    let report = run(
        &program,
        source,
        &config_with_tools(tools),
        &modal_only("modal.tool_unknown"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.tool_unknown");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 4);
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(2)));
}

#[test]
fn tool_unknown_is_silent_when_every_used_tool_is_known() {
    let source = "G21 G90\nT1 M6\nT2 M6\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let mut tools = ToolTable::new();
    tools.insert(Tool::new(1));
    tools.insert(Tool::new(2));
    let report = run(
        &program,
        source,
        &config_with_tools(tools),
        &modal_only("modal.tool_unknown"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_unknown_skips_when_there_is_no_tool_data() {
    let source = "G21 G90\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert!(program.tool_hints.is_empty());
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_unknown"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    let skip = &report.coverage.skipped[0];
    assert_eq!(skip.rule.as_str(), "modal.tool_unknown");
    assert_eq!(skip.missing_input, "tool.data");
    assert_eq!(skip.reason, "no tool data");
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_unknown_treats_comment_hints_as_tool_data() {
    let source = "(T1 DIA 6mm)\nG21 G90\nT1 M6\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert!(!program.tool_hints.is_empty(), "fixture must carry a hint");

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_unknown"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_zero_reports_a_cutting_motion_without_a_tool() {
    let source = "G21 G90\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_zero"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.tool_zero");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(0)));
}

#[test]
fn tool_zero_is_silent_when_a_tool_is_selected() {
    let source = "G21 G90\nT1 M6\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_zero"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn tool_change_mismatch_reports_a_selection_without_m6() {
    let source = "G21 G90\nT1\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_change_mismatch"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.tool_change_mismatch");
    assert_eq!(finding.line, 2);
    assert!(finding.message.contains("not followed"));
    assert_eq!(finding.evidence.get("tool"), Some(&EvidenceValue::Int(1)));
    assert_eq!(
        finding.evidence.get("raw"),
        Some(&EvidenceValue::Text("T1".to_string()))
    );
}

#[test]
fn tool_change_mismatch_reports_m6_without_a_selection() {
    let source = "G21 G90\nM6\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_change_mismatch"),
    );

    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.line, 2);
    assert!(finding.message.contains("no selected tool"));
}

#[test]
fn tool_change_mismatch_is_silent_for_a_paired_change() {
    let source = "G21 G90\nT1\nM6\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.tool_change_mismatch"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn machine_coords_reports_each_g53_g28_g30_line() {
    let source = "G21 G90\nG53 G0 X10\nG28\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.machine_coords"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 2);
    let first = &report.diagnostics[0];
    assert_eq!(first.rule.as_str(), "modal.machine_coords");
    assert_eq!(first.line, 2);
    assert_eq!(
        first.evidence.get("reason"),
        Some(&EvidenceValue::Text("G53".to_string()))
    );
    assert_eq!(
        first.evidence.get("raw"),
        Some(&EvidenceValue::Text("G53 G0 X10".to_string()))
    );
    assert_eq!(report.diagnostics[1].line, 3);
}

#[test]
fn machine_coords_skips_makera_clearance_g28_and_g30() {
    let source = "G21 G90\nG53 G0 X10\nG28\nG30\nM30\n";
    let program = parse_program(source).expect("valid program");
    let registry = modal_only("modal.machine_coords");
    let config = AnalysisConfig::default();
    let request = AnalysisRequest::new("part.nc", source, &config, Dialect::Makera);
    let report = analyze(&program, &request, &registry);

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].line, 2);
}

#[test]
fn machine_coords_is_silent_for_work_coordinates() {
    let source = "G21 G90\nG0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.machine_coords"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn probe_in_job_reports_a_probe_motion() {
    let source = "G21 G90\nG38.2 Z-5 F50\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.probe_in_job"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.probe_in_job");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("probe".to_string()))
    );
}

#[test]
fn probe_in_job_is_silent_for_a_normal_move() {
    let source = "G21 G90\nG1 Z-5 F50\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.probe_in_job"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn g92_unrestored_reports_the_last_g92_line() {
    let source = "G21 G90\nG92 X10\nG0 X20\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert_ne!(program.state.g92, [0.0; 3]);

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.g92_unrestored"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.g92_unrestored");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
}

#[test]
fn g92_unrestored_is_silent_after_g92_1_cancels() {
    let source = "G21 G90\nG92 X10\nG92.1\nG0 X20\nM30\n";
    let program = parse_program(source).expect("valid program");
    assert_eq!(program.state.g92, [0.0; 3]);

    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.g92_unrestored"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn wcs_change_reports_the_change_after_cutting_starts() {
    let source = "G21 G90\nG1 X10 F100\nG55\nG1 X20 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.wcs_change"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.wcs_change");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 3);
    assert_eq!(
        finding.evidence.get("wcs"),
        Some(&EvidenceValue::Text("G55".to_string()))
    );
    assert!(finding.message.contains("G54"));
    assert!(finding.message.contains("G55"));
}

#[test]
fn wcs_change_is_silent_when_the_wcs_is_set_before_cutting() {
    let source = "G21 G90\nG55\nG1 X10 F100\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.wcs_change"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn no_program_end_reports_the_last_line() {
    let source = "G21 G90\nG0 X10\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.no_program_end"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);
    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "modal.no_program_end");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
}

#[test]
fn no_program_end_is_silent_with_m2_or_m30() {
    for ending in ["M2", "M30"] {
        let source = format!("G21 G90\nG0 X10\n{ending}\n");
        let program = parse_program(&source).expect("valid program");
        let report = run(
            &program,
            &source,
            &AnalysisConfig::default(),
            &modal_only("modal.no_program_end"),
        );

        assert_eq!(report.coverage.evaluated, 1);
        assert!(
            report.diagnostics.is_empty(),
            "{ending} must satisfy the rule"
        );
    }
}

#[test]
fn end_state_reports_spindle_coolant_and_tool_depth() {
    let source = "G21 G90\nS8000 M3\nM8\nG1 Z-3 F100\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.end_state"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(
        report.coverage.skipped.is_empty(),
        "inferred stock is allowed for this rule"
    );
    let findings = findings_for(&report, "modal.end_state");
    assert_eq!(findings.len(), 3, "spindle, coolant, and tool depth");

    let spindle = findings
        .iter()
        .find(|finding| finding.message.contains("spindle"))
        .expect("spindle finding");
    assert_eq!(spindle.line, 2);
    assert_eq!(
        spindle.evidence.get("stock_source"),
        Some(&EvidenceValue::Text("inferred".to_string()))
    );
    assert_eq!(
        spindle.evidence.get("inference_basis"),
        Some(&EvidenceValue::Text("zero_at_top".to_string()))
    );
    assert_eq!(
        spindle.evidence.get("clockwise"),
        Some(&EvidenceValue::Bool(true))
    );
    assert_eq!(
        spindle.evidence.get("spindle_rpm"),
        Some(&EvidenceValue::Float(8000.0))
    );

    let coolant = findings
        .iter()
        .find(|finding| finding.message.contains("coolant"))
        .expect("coolant finding");
    assert_eq!(coolant.line, 3);

    let tool = findings
        .iter()
        .find(|finding| finding.message.contains("tool at Z"))
        .expect("tool depth finding");
    assert_eq!(tool.line, 4);
    assert_eq!(
        tool.evidence.get("depth_mm"),
        Some(&EvidenceValue::Float(5.0))
    );
    assert_eq!(tool.evidence.get("tool"), Some(&EvidenceValue::Int(0)));
    assert_eq!(tool.basis, Basis::Model);
}

#[test]
fn end_state_is_silent_after_stop_and_retract() {
    let source = "G21 G90\nS8000 M3\nM8\nG1 Z-3 F100\nM5\nM9\nG0 Z10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.end_state"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn block_delete_unsafe_reports_tool_and_retract_lines() {
    let source = "G21 G90\nT1\n/M6\n/G0 Z10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.block_delete_unsafe"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    let findings = findings_for(&report, "modal.block_delete_unsafe");
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].line, 3);
    assert_eq!(
        findings[0].evidence.get("raw"),
        Some(&EvidenceValue::Text("/M6".to_string()))
    );
    assert_eq!(findings[1].line, 4);
}

#[test]
fn block_delete_unsafe_is_silent_for_a_plain_travel_line() {
    let source = "G21 G90\n/G0 X10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let report = run(
        &program,
        source,
        &AnalysisConfig::default(),
        &modal_only("modal.block_delete_unsafe"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

/// Program and configuration shared by the coverage and determinism tests.
fn complete_program() -> (Program, String, AnalysisConfig) {
    let source = "G21 G90\nT1 M6\nS8000 M3\nG1 X10 F100\nM5\nG0 Z10\nM30\n";
    let program = parse_program(source).expect("valid program");
    let mut tools = ToolTable::new();
    tools.insert(handed_tool(1, true));
    (program, source.to_string(), config_with_tools(tools))
}

#[test]
fn modal_module_coverage_counts_every_rule() {
    let (program, source, config) = complete_program();
    let report = run(
        &program,
        &source,
        &config,
        &RuleRegistry::from_rules(rules::modal::rules()),
    );

    assert_eq!(report.coverage.total, 16);
    assert_eq!(report.coverage.evaluated, 16);
    assert!(report.coverage.skipped.is_empty());
    assert!(
        report.diagnostics.is_empty(),
        "the clean program must not produce findings: {:?}",
        report
            .diagnostics
            .iter()
            .map(|finding| finding.rule.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn modal_module_coverage_lists_both_missing_inputs() {
    let (program, source, _) = complete_program();
    let report = run(
        &program,
        &source,
        &AnalysisConfig::default(),
        &RuleRegistry::from_rules(rules::modal::rules()),
    );

    assert_eq!(report.coverage.total, 16);
    assert_eq!(report.coverage.evaluated, 14);
    assert_eq!(report.coverage.skipped.len(), 2);
    assert_eq!(report.coverage.skipped_rule_count(), 2);
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );

    let spindle = &report.coverage.skipped[0];
    assert_eq!(spindle.rule.as_str(), "modal.spindle_direction");
    assert_eq!(spindle.missing_input, "tool.right_hand:T1");
    let tool = &report.coverage.skipped[1];
    assert_eq!(tool.rule.as_str(), "modal.tool_unknown");
    assert_eq!(tool.missing_input, "tool.data");
    assert_eq!(tool.reason, "no tool data");
}

#[test]
fn modal_catalog_metadata_is_stable() {
    let rules = rules::modal::rules();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert_eq!(
        ids,
        vec![
            "modal.units_missing",
            "modal.distance_missing",
            "modal.feed_nonpositive",
            "modal.spindle_stopped",
            "modal.spindle_speed_zero",
            "modal.spindle_direction",
            "modal.tool_unknown",
            "modal.tool_zero",
            "modal.tool_change_mismatch",
            "modal.machine_coords",
            "modal.probe_in_job",
            "modal.g92_unrestored",
            "modal.wcs_change",
            "modal.no_program_end",
            "modal.end_state",
            "modal.block_delete_unsafe",
        ]
    );

    for rule in &rules {
        assert!(!rule.why_it_matters().is_empty(), "{}", rule.id());
        let requirements = rule.requirements();
        assert!(!requirements.machine, "{}", rule.id());
        assert!(!requirements.material, "{}", rule.id());
        match rule.id().as_str() {
            "modal.spindle_direction" => {
                assert_eq!(requirements.tool_fields, &[ToolField::RightHand]);
                assert_eq!(rule.basis(), Basis::Model);
                assert_eq!(rule.default_severity(), Severity::Info);
            }
            "modal.end_state" => {
                assert!(requirements.tool_fields.is_empty());
                assert_eq!(requirements.stock, StockRequirement::InferredOk);
                assert_eq!(rule.basis(), Basis::Model);
                assert_eq!(rule.default_severity(), Severity::Warning);
            }
            "modal.feed_nonpositive" | "modal.spindle_stopped" => {
                assert!(requirements.tool_fields.is_empty());
                assert_eq!(requirements.stock, StockRequirement::None);
                assert_eq!(rule.basis(), Basis::Exact);
                assert_eq!(rule.default_severity(), Severity::Error);
            }
            _ => {
                assert!(requirements.tool_fields.is_empty(), "{}", rule.id());
                assert_eq!(requirements.stock, StockRequirement::None);
                assert_eq!(rule.basis(), Basis::Exact);
            }
        }
    }
}

#[test]
fn modal_module_reports_are_byte_deterministic() {
    let source = "G21 G90\nT1\nS8000 M3\nG1 X10 F100\nG92 X0\nG55\nG1 X20 F100\nM8\n";
    let program = parse_program(source).expect("valid program");
    let config = AnalysisConfig::default();
    let registry = RuleRegistry::from_rules(rules::modal::rules());

    let first = run(&program, source, &config, &registry);
    let second = run(&program, source, &config, &registry);

    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
    for rule in [
        "modal.tool_change_mismatch",
        "modal.g92_unrestored",
        "modal.wcs_change",
        "modal.end_state",
    ] {
        assert!(
            !findings_for(&first, rule).is_empty(),
            "fixture must exercise {rule}"
        );
    }
}
