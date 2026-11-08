//! `geometry.*` rule tests: positive, near-miss, skip, coverage, determinism.
//!
//! Each test registers exactly one rule so a finding is attributable without
//! filtering. Catalog and coverage tests use the whole module.

use std::collections::BTreeMap;

use nc_analysis::{
    AnalysisConfig, AnalysisRequest, Basis, Diagnostic, EvidenceValue, Report, Rule, RuleRegistry,
    Severity, StockRequirement, Tolerances, analyze, rules,
};
use nc_core::{Dialect, FieldSource, MachineConfig, Tool, ToolField, ToolTable};
use nc_gcode::{
    ArcFormat, ArcRecord, CannedCycle, CannedRecord, CutterComp, ModalId, MotionRecord, MoveKind,
    Program, ProgramBuilder, RetractMode,
};

/// Source text with `count` one-word lines so diagnostics can be clamped.
fn source_lines(count: usize) -> String {
    (1..=count).map(|line| format!("N{line}\n")).collect()
}

/// A straight motion record with an explicit tool, feed, and spindle.
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

/// A drilling motion from `r_plane_mm` down to `bottom_z_mm`.
fn drill(line: usize, tool: u32, xy: [f64; 2], r_plane_mm: f64, bottom_z_mm: f64) -> MotionRecord {
    motion(
        line,
        MoveKind::Drill,
        [xy[0], xy[1], r_plane_mm],
        [xy[0], xy[1], bottom_z_mm],
        Some(100.0),
        tool,
    )
}

/// A canned-cycle hole at `xy` between `r_plane_mm` and `bottom_z_mm`.
fn canned(line: usize, tool: u32, xy: [f64; 2], r_plane_mm: f64, bottom_z_mm: f64) -> MotionRecord {
    // The parser leaves the machine at the R plane after the hole.
    let mut record = MotionRecord::linear(
        line,
        0,
        MoveKind::Drill,
        [xy[0], xy[1], r_plane_mm],
        [xy[0], xy[1], r_plane_mm],
    );
    record.tool = tool;
    record.canned = Some(Box::new(CannedRecord {
        cycle: CannedCycle::Drill,
        hole_index: 0,
        r_plane_mm,
        bottom_z_mm,
        peck_q_mm: None,
        dwell_s: None,
        retract_mode: RetractMode::Initial,
    }));
    record
}

/// An arc motion with the given start and end radius.
fn arc_motion(line: usize, tool: u32, radius_mm: f64, end_radius_mm: f64) -> MotionRecord {
    let center = [10.0, 10.0, 0.0];
    let mut record = MotionRecord::linear(
        line,
        0,
        MoveKind::ArcCw,
        [center[0] + radius_mm, center[1], 0.0],
        [center[0], center[1] + end_radius_mm, 0.0],
    );
    record.tool = tool;
    record.arc = Some(Box::new(ArcRecord {
        center_mm: center,
        radius_mm,
        end_radius_mm,
        start_angle_rad: 0.0,
        sweep_rad: nc_core::math::FRAC_PI_2,
        axial_delta_mm: 0.0,
        format: ArcFormat::Center,
    }));
    record
}

/// A tool table with one tool and an optional diameter.
fn tool_table(number: u32, diameter_mm: Option<f64>) -> ToolTable {
    let mut tool = Tool::new(number);
    tool.diameter_mm = diameter_mm;
    let mut table = ToolTable::new();
    table.insert(tool);
    table
}

fn config_with_diameter(number: u32, diameter_mm: f64) -> AnalysisConfig {
    AnalysisConfig {
        tools: tool_table(number, Some(diameter_mm)),
        ..AnalysisConfig::default()
    }
}

/// A valid machine box with no optional limits yet.
fn machine() -> MachineConfig {
    MachineConfig::new([-100.0, -100.0, -100.0], [100.0, 100.0, 100.0])
        .expect("valid machine travel")
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

/// A registry containing only the named geometry rule.
fn only(id: &str) -> RuleRegistry {
    let selected: Vec<Box<dyn Rule>> = rules::geometry::rules()
        .into_iter()
        .filter(|rule| rule.id().as_str() == id)
        .collect();
    assert_eq!(selected.len(), 1, "rule {id} must exist exactly once");
    RuleRegistry::from_rules(selected)
}

/// Reads one float evidence value.
fn float_evidence(finding: &Diagnostic, key: &str) -> f64 {
    match finding.evidence.get(key) {
        Some(EvidenceValue::Float(value)) => *value,
        other => panic!("evidence {key} is not a float: {other:?}"),
    }
}

/// Reads one integer evidence value.
fn int_evidence(finding: &Diagnostic, key: &str) -> i64 {
    match finding.evidence.get(key) {
        Some(EvidenceValue::Int(value)) => *value,
        other => panic!("evidence {key} is not an int: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// geometry.duplicate_hole
// ---------------------------------------------------------------------------

#[test]
fn duplicate_hole_fires_for_canned_records_with_overlapping_z() {
    let program = ProgramBuilder::new()
        .record(canned(2, 1, [10.0, 5.0], 2.0, -5.0))
        .record(canned(6, 1, [10.0, 5.0], 2.0, -12.0))
        // A third hole at the same XY but far below the others: no Z overlap.
        .record(canned(9, 1, [10.0, 5.0], -20.0, -30.0))
        .build();
    let report = run(
        &program,
        &source_lines(9),
        &AnalysisConfig::default(),
        &only("geometry.duplicate_hole"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.duplicate_hole");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 6);
    assert_eq!(finding.line_end, 6);
    assert_eq!(int_evidence(finding, "tool"), 1);
    assert_eq!(int_evidence(finding, "first_line"), 2);
    assert_eq!(int_evidence(finding, "second_line"), 6);
    assert_eq!(
        finding.evidence.get("first_mm"),
        Some(&EvidenceValue::Point3([10.0, 5.0, -5.0]))
    );
    assert_eq!(
        finding.evidence.get("second_mm"),
        Some(&EvidenceValue::Point3([10.0, 5.0, -12.0]))
    );
    assert_eq!(float_evidence(finding, "distance_mm"), 0.0);
    assert_eq!(float_evidence(finding, "tolerance_mm"), 0.05);
}

#[test]
fn duplicate_hole_requires_same_tool_and_overlapping_z() {
    let program = ProgramBuilder::new()
        .record(drill(2, 1, [10.0, 10.0], 2.0, -5.0))
        // Same XY and Z but another tool.
        .record(drill(3, 2, [10.0, 10.0], 2.0, -5.0))
        // Same tool and XY but a separate Z range.
        .record(drill(5, 1, [10.0, 10.0], -10.0, -20.0))
        .build();
    let report = run(
        &program,
        &source_lines(5),
        &AnalysisConfig::default(),
        &only("geometry.duplicate_hole"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(
        report.diagnostics.is_empty(),
        "different tools and non-overlapping Z are not duplicates"
    );
}

#[test]
fn duplicate_hole_xy_tolerance_is_inclusive_and_near_miss_is_quiet() {
    // A zero tolerance still accepts identical centres: the comparison is `<=`.
    let zero_tolerance = AnalysisConfig {
        tolerances: Tolerances {
            hole_xy_mm: 0.0,
            ..Tolerances::default()
        },
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(drill(2, 1, [0.0, 0.0], 2.0, -5.0))
        .record(drill(4, 1, [0.0, 0.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(4),
        &zero_tolerance,
        &only("geometry.duplicate_hole"),
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].line, 4);

    // Default tolerance 0.05: 0.04 fires, 0.06 does not.
    let program = ProgramBuilder::new()
        .record(drill(2, 1, [0.0, 0.0], 2.0, -5.0))
        .record(drill(3, 1, [0.04, 0.0], 2.0, -5.0))
        .record(drill(5, 1, [1.0, 0.0], 2.0, -5.0))
        .record(drill(6, 1, [1.06, 0.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(6),
        &AnalysisConfig::default(),
        &only("geometry.duplicate_hole"),
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].line, 3);
    assert!(float_evidence(&report.diagnostics[0], "distance_mm") < 0.05);
}

// ---------------------------------------------------------------------------
// geometry.overlapping_holes
// ---------------------------------------------------------------------------

#[test]
fn overlapping_holes_fire_below_the_tool_diameter() {
    let config = config_with_diameter(2, 6.0);
    let program = ProgramBuilder::new()
        .record(drill(2, 2, [0.0, 0.0], 2.0, -5.0))
        // 2 mm away with a 6 mm cutter: the holes overlap.
        .record(drill(5, 2, [2.0, 0.0], 2.0, -5.0))
        // 7 mm from the middle hole: clear of the 6 mm cutter.
        .record(drill(8, 2, [9.0, 0.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(8),
        &config,
        &only("geometry.overlapping_holes"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.overlapping_holes");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 5);
    assert_eq!(int_evidence(finding, "tool"), 2);
    assert_eq!(int_evidence(finding, "first_line"), 2);
    assert_eq!(int_evidence(finding, "second_line"), 5);
    assert_eq!(float_evidence(finding, "distance_mm"), 2.0);
    assert_eq!(float_evidence(finding, "diameter_mm"), 6.0);
    assert!(!finding.evidence.contains_key("tool_source"));
}

#[test]
fn overlapping_holes_reports_a_comment_derived_diameter() {
    let mut tool = Tool::new(2);
    tool.diameter_mm = Some(6.0);
    tool.sources
        .insert(ToolField::Diameter, FieldSource::Comment { line: 1 });
    let mut tools = ToolTable::new();
    tools.insert(tool);
    let config = AnalysisConfig {
        tools,
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(drill(2, 2, [0.0, 0.0], 2.0, -5.0))
        .record(drill(3, 2, [2.0, 0.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &only("geometry.overlapping_holes"),
    );

    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].evidence.get("tool_source"),
        Some(&EvidenceValue::Text("comment:L1".to_string()))
    );
}

#[test]
fn overlapping_holes_skips_without_a_tool_diameter() {
    let config = AnalysisConfig {
        tools: tool_table(2, None),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(drill(2, 2, [0.0, 0.0], 2.0, -5.0))
        .record(drill(3, 2, [2.0, 0.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &only("geometry.overlapping_holes"),
    );

    assert_eq!(report.coverage.total, 1);
    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "tool.diameter:T2");
    assert_eq!(report.coverage.skipped_rule_count(), 1);
    assert!(report.diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// geometry.tiny_segment
// ---------------------------------------------------------------------------

#[test]
fn tiny_segment_fires_below_the_default_minimum() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [0.005, 0.0, 0.0],
            Some(100.0),
            1,
        ))
        // 0.02 mm is above the 0.01 mm default.
        .record(motion(
            3,
            MoveKind::Feed,
            [1.0, 0.0, 0.0],
            [1.02, 0.0, 0.0],
            Some(100.0),
            1,
        ))
        // Canned cycles describe holes, not zero-length XY segments.
        .record(canned(5, 1, [10.0, 10.0], 2.0, -5.0))
        .build();
    let report = run(
        &program,
        &source_lines(5),
        &AnalysisConfig::default(),
        &only("geometry.tiny_segment"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.tiny_segment");
    assert_eq!(finding.severity, Severity::Info);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(float_evidence(finding, "length_mm"), 0.005);
    assert_eq!(float_evidence(finding, "threshold"), 0.01);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("feed".to_string()))
    );
    assert_eq!(int_evidence(finding, "tool"), 1);
}

#[test]
fn tiny_segment_allows_a_segment_exactly_at_the_threshold() {
    let config = AnalysisConfig {
        tolerances: Tolerances {
            min_segment_mm: 0.5,
            ..Tolerances::default()
        },
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            Some(100.0),
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &only("geometry.tiny_segment"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// geometry.arc_radius_mismatch
// ---------------------------------------------------------------------------

#[test]
fn arc_radius_mismatch_fires_beyond_the_tolerance() {
    let program = ProgramBuilder::new()
        .record(arc_motion(2, 1, 5.0, 5.1))
        // A 0.01 mm difference is inside the 0.05 mm default.
        .record(arc_motion(4, 1, 5.0, 5.01))
        .build();
    let report = run(
        &program,
        &source_lines(4),
        &AnalysisConfig::default(),
        &only("geometry.arc_radius_mismatch"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.arc_radius_mismatch");
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(float_evidence(finding, "radius_mm"), 5.0);
    assert_eq!(float_evidence(finding, "end_radius_mm"), 5.1);
    assert!((float_evidence(finding, "delta_mm") - 0.1).abs() < 1.0e-9);
    assert_eq!(float_evidence(finding, "threshold"), 0.05);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("arc cw".to_string()))
    );
}

#[test]
fn arc_radius_mismatch_allows_the_exact_tolerance() {
    // A 0.05 mm delta equals the default tolerance and must not fire.
    let program = ProgramBuilder::new()
        .record(arc_motion(2, 1, 5.0, 5.05))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &AnalysisConfig::default(),
        &only("geometry.arc_radius_mismatch"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// geometry.arc_smaller_than_tool
// ---------------------------------------------------------------------------

#[test]
fn arc_smaller_than_tool_fires_and_ignores_compensated_arcs() {
    let config = config_with_diameter(3, 6.0);
    let mut compensated = arc_motion(5, 3, 2.0, 2.0);
    compensated.modal = ModalId(1);
    let mut program = ProgramBuilder::new()
        // R2 is tighter than the 3 mm tool radius.
        .record(arc_motion(2, 3, 2.0, 2.0))
        // R4 clears the 3 mm tool radius.
        .record(arc_motion(3, 3, 4.0, 4.0))
        // R2 again, but cutter compensation is active: not evaluated.
        .record(compensated)
        .build();
    program.modal_table[1].cutter_comp = CutterComp::Left;

    let report = run(
        &program,
        &source_lines(5),
        &config,
        &only("geometry.arc_smaller_than_tool"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.arc_smaller_than_tool");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Model);
    assert_eq!(finding.line, 2);
    assert_eq!(float_evidence(finding, "radius_mm"), 2.0);
    assert_eq!(float_evidence(finding, "tool_diameter_mm"), 6.0);
    assert_eq!(float_evidence(finding, "shortfall_mm"), 1.0);
    assert_eq!(int_evidence(finding, "tool"), 3);
    assert!(!finding.evidence.contains_key("tool_source"));
}

#[test]
fn arc_smaller_than_tool_leaves_every_compensated_arc_alone() {
    let config = config_with_diameter(3, 6.0);
    let mut compensated = arc_motion(2, 3, 2.0, 2.0);
    compensated.modal = ModalId(1);
    let mut program = ProgramBuilder::new().record(compensated).build();
    program.modal_table[1].cutter_comp = CutterComp::Right;

    let report = run(
        &program,
        &source_lines(2),
        &config,
        &only("geometry.arc_smaller_than_tool"),
    );

    // Every arc was skipped, so the rule still evaluated with no findings.
    assert_eq!(report.coverage.evaluated, 1);
    assert!(report.coverage.skipped.is_empty());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn arc_smaller_than_tool_skips_without_a_tool_diameter() {
    let config = AnalysisConfig {
        tools: tool_table(3, None),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(arc_motion(2, 3, 2.0, 2.0))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &only("geometry.arc_smaller_than_tool"),
    );

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(report.coverage.skipped[0].missing_input, "tool.diameter:T3");
}

// ---------------------------------------------------------------------------
// geometry.feed_out_of_range
// ---------------------------------------------------------------------------

#[test]
fn feed_out_of_range_fires_above_the_maximum() {
    let mut machine = machine();
    machine.max_feed_mm_min = Some(1000.0);
    let config = AnalysisConfig {
        machine: Some(machine),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(1200.0),
            4,
        ))
        // Exactly the maximum is allowed.
        .record(motion(
            3,
            MoveKind::Feed,
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            Some(1000.0),
            4,
        ))
        // A rapid carries no cutting feed and is not checked.
        .record(motion(
            4,
            MoveKind::Rapid,
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            Some(5000.0),
            4,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(4),
        &config,
        &only("geometry.feed_out_of_range"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.feed_out_of_range");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(float_evidence(finding, "feed_mm_min"), 1200.0);
    assert_eq!(float_evidence(finding, "threshold"), 1000.0);
    assert_eq!(int_evidence(finding, "tool"), 4);
    assert_eq!(
        finding.evidence.get("kind"),
        Some(&EvidenceValue::Text("feed".to_string()))
    );
}

#[test]
fn feed_out_of_range_skips_when_the_machine_has_no_maximum() {
    let config = AnalysisConfig {
        machine: Some(machine()),
        ..AnalysisConfig::default()
    };
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(5000.0),
            1,
        ))
        .build();
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &only("geometry.feed_out_of_range"),
    );

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(
        report.coverage.skipped[0].missing_input,
        "machine.max_feed_mm_min"
    );
    assert!(report.diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// geometry.spindle_out_of_range
// ---------------------------------------------------------------------------

#[test]
fn spindle_out_of_range_fires_above_the_maximum() {
    let mut machine = machine();
    machine.max_spindle_rpm = Some(24_000.0);
    let config = AnalysisConfig {
        machine: Some(machine),
        ..AnalysisConfig::default()
    };
    let mut fast = motion(
        2,
        MoveKind::Feed,
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        Some(500.0),
        1,
    );
    fast.spindle_rpm = Some(26_000.0);
    let mut at_limit = motion(
        3,
        MoveKind::Rapid,
        [10.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        None,
        1,
    );
    at_limit.spindle_rpm = Some(24_000.0);
    let program = ProgramBuilder::new().record(fast).record(at_limit).build();
    let report = run(
        &program,
        &source_lines(3),
        &config,
        &only("geometry.spindle_out_of_range"),
    );

    assert_eq!(report.coverage.evaluated, 1);
    assert_eq!(report.diagnostics.len(), 1);

    let finding = &report.diagnostics[0];
    assert_eq!(finding.rule.as_str(), "geometry.spindle_out_of_range");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.basis, Basis::Exact);
    assert_eq!(finding.line, 2);
    assert_eq!(float_evidence(finding, "spindle_rpm"), 26_000.0);
    assert_eq!(float_evidence(finding, "threshold"), 24_000.0);
    assert_eq!(int_evidence(finding, "tool"), 1);
}

#[test]
fn spindle_out_of_range_skips_when_the_machine_has_no_maximum() {
    let config = AnalysisConfig {
        machine: Some(machine()),
        ..AnalysisConfig::default()
    };
    let mut program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            Some(500.0),
            1,
        ))
        .build();
    program.motions[0].spindle_rpm = Some(30_000.0);
    let report = run(
        &program,
        &source_lines(2),
        &config,
        &only("geometry.spindle_out_of_range"),
    );

    assert_eq!(report.coverage.evaluated, 0);
    assert_eq!(report.coverage.skipped.len(), 1);
    assert_eq!(
        report.coverage.skipped[0].missing_input,
        "machine.max_spindle_rpm"
    );
    assert!(report.diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// Catalog, coverage, determinism
// ---------------------------------------------------------------------------

fn rule_by_id<'a>(rules: &'a [Box<dyn Rule>], id: &str) -> &'a dyn Rule {
    rules
        .iter()
        .map(AsRef::as_ref)
        .find(|rule| rule.id().as_str() == id)
        .expect("rule is registered")
}

#[test]
fn geometry_catalog_metadata_matches_the_rule_table() {
    let rules = rules::geometry::rules();
    let ids: Vec<String> = rules.iter().map(|rule| rule.id().to_string()).collect();
    assert_eq!(
        ids,
        vec![
            "geometry.arc_radius_mismatch",
            "geometry.arc_smaller_than_tool",
            "geometry.duplicate_hole",
            "geometry.feed_out_of_range",
            "geometry.overlapping_holes",
            "geometry.spindle_out_of_range",
            "geometry.tiny_segment",
        ]
    );

    for rule in &rules {
        let expected = if matches!(
            rule.id().as_str(),
            "geometry.overlapping_holes" | "geometry.arc_smaller_than_tool"
        ) {
            Basis::Model
        } else {
            Basis::Exact
        };
        assert_eq!(rule.basis(), expected, "{}", rule.id());
        assert_eq!(
            rule.requirements().stock,
            StockRequirement::None,
            "{}",
            rule.id()
        );
        assert!(!rule.requirements().material, "{}", rule.id());
        assert!(!rule.why_it_matters().is_empty(), "{}", rule.id());
        assert!(rule.requirements().tool_fields.len() <= 1, "{}", rule.id());
    }

    let duplicate = rule_by_id(&rules, "geometry.duplicate_hole");
    assert_eq!(duplicate.default_severity(), Severity::Warning);
    assert!(duplicate.requirements().tool_fields.is_empty());
    assert!(!duplicate.requirements().machine);

    let overlapping = rule_by_id(&rules, "geometry.overlapping_holes");
    assert_eq!(overlapping.default_severity(), Severity::Warning);
    assert_eq!(
        overlapping.requirements().tool_fields,
        &[ToolField::Diameter]
    );
    assert!(!overlapping.requirements().machine);

    let tiny = rule_by_id(&rules, "geometry.tiny_segment");
    assert_eq!(tiny.default_severity(), Severity::Info);
    assert!(tiny.requirements().tool_fields.is_empty());
    assert!(!tiny.requirements().machine);

    let mismatch = rule_by_id(&rules, "geometry.arc_radius_mismatch");
    assert_eq!(mismatch.default_severity(), Severity::Error);
    assert!(mismatch.requirements().tool_fields.is_empty());
    assert!(!mismatch.requirements().machine);

    let smaller = rule_by_id(&rules, "geometry.arc_smaller_than_tool");
    assert_eq!(smaller.default_severity(), Severity::Warning);
    assert_eq!(smaller.requirements().tool_fields, &[ToolField::Diameter]);
    assert!(!smaller.requirements().machine);

    let feed = rule_by_id(&rules, "geometry.feed_out_of_range");
    assert_eq!(feed.default_severity(), Severity::Warning);
    assert!(feed.requirements().tool_fields.is_empty());
    assert!(feed.requirements().machine);

    let spindle = rule_by_id(&rules, "geometry.spindle_out_of_range");
    assert_eq!(spindle.default_severity(), Severity::Warning);
    assert!(spindle.requirements().tool_fields.is_empty());
    assert!(spindle.requirements().machine);
}

#[test]
fn geometry_coverage_arithmetic_and_report_bytes_are_deterministic() {
    // No machine and no tool diameter: three rules evaluate and four skip.
    let program = ProgramBuilder::new()
        .record(canned(2, 7, [1.0, 1.0], 2.0, -5.0))
        .record(canned(5, 7, [1.0, 1.0], 2.0, -12.0))
        .build();
    let source = source_lines(5);
    let config = AnalysisConfig::default();
    let registry = RuleRegistry::from_rules(rules::geometry::rules());

    let first = run(&program, &source, &config, &registry);
    let second = run(&program, &source, &config, &registry);

    assert_eq!(first.coverage.total, 7);
    assert_eq!(first.coverage.evaluated, 3);
    assert_eq!(first.coverage.skipped_rule_count(), 4);
    assert_eq!(first.coverage.skipped.len(), 4);
    assert_eq!(
        first.coverage.evaluated + first.coverage.skipped_rule_count(),
        first.coverage.total
    );

    let skipped: BTreeMap<&str, &str> = first
        .coverage
        .skipped
        .iter()
        .map(|skip| (skip.rule.as_str(), skip.missing_input.as_str()))
        .collect();
    assert_eq!(
        skipped.get("geometry.overlapping_holes").copied(),
        Some("tool.diameter:T7")
    );
    assert_eq!(
        skipped.get("geometry.arc_smaller_than_tool").copied(),
        Some("tool.diameter:T7")
    );
    assert_eq!(
        skipped.get("geometry.feed_out_of_range").copied(),
        Some("machine")
    );
    assert_eq!(
        skipped.get("geometry.spindle_out_of_range").copied(),
        Some("machine")
    );

    assert_eq!(first.diagnostics.len(), 1);
    assert_eq!(
        first.diagnostics[0].rule.as_str(),
        "geometry.duplicate_hole"
    );

    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
}
