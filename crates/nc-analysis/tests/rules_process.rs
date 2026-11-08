//! `process.*` rule tests: the shared engagement pass, every M2 rule, skip
//! keys, coverage arithmetic, and byte determinism.
//!
//! Each rule has a positive case and a near-miss or exact-boundary case. All
//! data is real `AnalysisConfig` stock, tools, and material; no rule is called
//! with synthetic values that the engine would reject.

use std::collections::BTreeMap;

use nc_analysis::{
    AnalysisConfig, AnalysisModel, AnalysisRequest, Basis, Diagnostic, EvidenceValue, Report, Rule,
    RuleOutcome, RuleRegistry, RuleSettings, Severity, StockConfig, analyze, rules,
};
use nc_core::{Dialect, FieldSource, MaterialPreset, Tool, ToolField, ToolShape, ToolTable, math};
use nc_gcode::{MotionRecord, MoveKind, Program, ProgramBuilder};

/// Source text with `count` one-word lines so diagnostics can be clamped.
fn source_lines(count: usize) -> String {
    (1..=count).map(|line| format!("N{line}\n")).collect()
}

/// A cutting or travel record with feed, spindle, and tool.
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
    record.spindle_rpm = Some(12_000.0);
    record.spindle_cw = Some(true);
    record.tool = tool;
    record
}

/// A 6 mm two-flute flat endmill with 30 mm stickout and 18 mm flute length.
fn endmill(number: u32) -> Tool {
    let mut tool = Tool::new(number);
    tool.shape = ToolShape::Flat;
    tool.diameter_mm = Some(6.0);
    tool.flutes = Some(2);
    tool.stickout_mm = Some(30.0);
    tool.flute_length_mm = Some(18.0);
    tool
}

/// One tool in a resolved table.
fn table(tool: Tool) -> ToolTable {
    let mut table = ToolTable::new();
    table.insert(tool);
    table
}

/// Aluminum-like material with every range populated.
fn test_material() -> MaterialPreset {
    let mut material = MaterialPreset::new("test-aluminum");
    material.chipload_min_mm = Some(0.01);
    material.chipload_max_mm = Some(0.05);
    material.surface_speed_min_mpm = Some(100.0);
    material.surface_speed_max_mpm = Some(200.0);
    material.max_mrr_mm3_min = Some(10_000.0);
    material
}

/// Explicit stock from -10..30 XY below Z0, 20 mm thick.
fn stock_config() -> StockConfig {
    StockConfig {
        origin_mm: [-10.0, -10.0, -20.0],
        size_mm: [40.0, 40.0, 20.0],
    }
}

/// Explicit stock deep enough for a 30 mm cut.
fn deep_stock_config() -> StockConfig {
    StockConfig {
        origin_mm: [-10.0, -10.0, -40.0],
        size_mm: [40.0, 40.0, 40.0],
    }
}

/// Builds a config from parts.
fn build_config(
    stock: Option<StockConfig>,
    tools: ToolTable,
    material: Option<MaterialPreset>,
) -> AnalysisConfig {
    AnalysisConfig {
        stock,
        tools,
        material,
        ..AnalysisConfig::default()
    }
}

/// Standard explicit-stock config with the endmill and material.
fn ready_config() -> AnalysisConfig {
    build_config(
        Some(stock_config()),
        table(endmill(1)),
        Some(test_material()),
    )
}

/// Rule settings with numeric thresholds.
fn settings(thresholds: &[(&str, f64)]) -> RuleSettings {
    RuleSettings {
        thresholds: thresholds
            .iter()
            .map(|(key, value)| ((*key).to_string(), *value))
            .collect(),
        ..RuleSettings::default()
    }
}

/// Runs one named process rule through the engine.
fn run_rule(rule_id: &str, program: &Program, config: &AnalysisConfig) -> Report {
    let rule = rules::process::rules()
        .into_iter()
        .find(|rule| rule.id().as_str() == rule_id)
        .expect("process rule exists");
    let source = source_lines(40);
    let request = AnalysisRequest::new("part.nc", &source, config, Dialect::Generic);
    let registry = RuleRegistry::from_rules([rule]);
    analyze(program, &request, &registry)
}

/// Evaluates a rule directly, bypassing engine requirement checks.
fn evaluate(rule: &dyn Rule, program: &Program, config: &AnalysisConfig) -> RuleOutcome {
    let stock = match config.stock {
        Some(stock) => stock.to_stock().expect("valid stock"),
        None => nc_stock::StockModel::infer(program),
    };
    let model = AnalysisModel::new(
        program,
        stock,
        &config.tools,
        config.machine.as_ref(),
        config.material.as_ref(),
        &config.tolerances,
    );
    rule.evaluate(&model, &RuleSettings::default())
}

/// Extracts findings, failing loudly when the rule was skipped.
fn extract_findings(outcome: RuleOutcome) -> Vec<Diagnostic> {
    match outcome {
        RuleOutcome::Findings(findings) => findings,
        RuleOutcome::Skipped {
            missing_input,
            reason,
        } => panic!("unexpected skip {missing_input}: {reason}"),
    }
}

/// Returns a process rule by id.
fn rule(rule_id: &str) -> Box<dyn Rule> {
    rules::process::rules()
        .into_iter()
        .find(|rule| rule.id().as_str() == rule_id)
        .expect("process rule exists")
}

/// Returns `(missing_input, reason)` records for one rule in a report.
fn skipped_keys(report: &Report, rule_id: &str) -> Vec<String> {
    report
        .coverage
        .skipped
        .iter()
        .filter(|skip| skip.rule.as_str() == rule_id)
        .map(|skip| skip.missing_input.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Shared engagement pass
// ---------------------------------------------------------------------------

#[test]
fn shared_pass_is_lazy_and_deterministic() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let radial = rule("process.radial_engagement");

    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    assert!(model.engagement.get().is_none(), "pass must be lazy");
    let _ = radial.evaluate(&model, &RuleSettings::default());
    let data = model.engagement.get().expect("pass was built");
    let first_hash = data.final_heightmap.content_hash();
    assert_eq!(data.passes.len(), 1);

    // A second rule call reuses the same shared data.
    let _ = radial.evaluate(&model, &RuleSettings::default());
    assert_eq!(
        model
            .engagement
            .get()
            .expect("pass stays built")
            .final_heightmap
            .content_hash(),
        first_hash
    );

    // A fresh model reaches the same bytes.
    let model_again = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let _ = radial.evaluate(&model_again, &RuleSettings::default());
    assert_eq!(
        model_again
            .engagement
            .get()
            .expect("pass was built")
            .final_heightmap
            .content_hash(),
        first_hash
    );
}

#[test]
fn rapids_never_cut_the_engagement_map() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [-5.0, 0.0, 5.0],
            [5.0, 0.0, -3.0],
            None,
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let radial = rule("process.radial_engagement");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let outcome = radial.evaluate(&model, &RuleSettings::default());
    assert!(extract_findings(outcome).is_empty());
    let data = model.engagement.get().expect("pass was built");
    assert!(data.passes.is_empty(), "a rapid is not a pass");
    let top = f64::from(data.final_heightmap.top_z_mm() as f32);
    assert!(
        data.final_heightmap
            .cells()
            .iter()
            .all(|cell| f64::from(*cell) == top),
        "a rapid must not lower any cell"
    );
}

#[test]
fn oversized_grid_coarsens_instead_of_failing() {
    let stock = StockConfig {
        origin_mm: [-1500.0, -1500.0, -5.0],
        size_mm: [3000.0, 3000.0, 5.0],
    };
    let config = build_config(Some(stock), table(endmill(1)), None);
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-2.0, 0.0, -1.0],
            [2.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let axial = rule("process.axial_depth");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let _ = axial.evaluate(&model, &RuleSettings::default());
    let data = model.engagement.get().expect("pass was built");
    assert!(data.final_heightmap.cell_mm() > 0.75);
    assert!(data.coarse, "a cell above 2 mm must be labeled coarse");
    assert!(data.final_heightmap.len() <= 4_000_000);
}

// ---------------------------------------------------------------------------
// process.chipload
// ---------------------------------------------------------------------------

#[test]
fn chipload_flags_outside_the_material_range_at_exact_bounds() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        // 240 / (12000 x 2) = 0.010, the exact minimum: no finding.
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [-3.0, 0.0, -1.0],
            Some(240.0),
         1))
        // 1200 / (12000 x 2) = 0.050, the exact maximum: no finding.
        .record(motion(
            3,
            MoveKind::Feed,
            [-3.0, 0.0, -1.0],
            [-1.0, 0.0, -1.0],
            Some(1200.0),
         1))
        // 2400 / (12000 x 2) = 0.100, above the maximum.
        .record(motion(
            4,
            MoveKind::Feed,
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            Some(2400.0),
         1))
        // 100 / (12000 x 2) = 0.0042, below the minimum.
        .record(motion(
            5,
            MoveKind::Feed,
            [1.0, 0.0, -1.0],
            [3.0, 0.0, -1.0],
            Some(100.0),
         1))
        .build();
    let outcome = evaluate(rule("process.chipload").as_ref(), &program, &config);
    let findings = extract_findings(outcome);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].line, 4);
    assert_eq!(findings[1].line, 5);
    assert_eq!(
        findings[0].evidence.get("chipload_mm"),
        Some(&EvidenceValue::Float(0.1))
    );
    assert_eq!(
        findings[0].evidence.get("max_mm"),
        Some(&EvidenceValue::Float(0.05))
    );
    assert_eq!(findings[0].basis, Basis::Model);
    assert_eq!(findings[0].severity, Severity::Warning);
}

#[test]
fn chipload_margin_widens_the_range() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            // 1440 / (12000 x 2) = 0.060.
            Some(1440.0),
            1,
        ))
        .build();
    let chipload = rule("process.chipload");
    let strict = extract_findings(evaluate(chipload.as_ref(), &program, &config));
    assert_eq!(strict.len(), 1);

    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let relaxed = chipload.evaluate(&model, &settings(&[("chipload_margin", 0.01)]));
    assert!(extract_findings(relaxed).is_empty());
}

#[test]
fn chipload_skips_when_flutes_or_material_are_missing() {
    let mut tool = endmill(1);
    tool.flutes = None;
    let without_flutes = build_config(Some(stock_config()), table(tool), Some(test_material()));
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();

    let report = run_rule("process.chipload", &program, &without_flutes);
    assert_eq!(
        skipped_keys(&report, "process.chipload"),
        vec!["tool.flutes:T1"]
    );

    let without_material = build_config(Some(stock_config()), table(endmill(1)), None);
    let report = run_rule("process.chipload", &program, &without_material);
    assert_eq!(skipped_keys(&report, "process.chipload"), vec!["material"]);
}

#[test]
fn chipload_uses_comment_derived_flutes_with_tool_source() {
    let mut tool = endmill(1);
    tool.sources
        .insert(ToolField::Flutes, FieldSource::Comment { line: 2 });
    let config = build_config(Some(stock_config()), table(tool), Some(test_material()));
    let program = ProgramBuilder::new()
        .record(motion(
            3,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(2400.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.chipload").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("tool_source"),
        Some(&EvidenceValue::Text("comment:L2".to_string()))
    );
}

// ---------------------------------------------------------------------------
// process.surface_speed
// ---------------------------------------------------------------------------

#[test]
fn surface_speed_compares_against_the_material_range() {
    let mut tool = endmill(1);
    tool.diameter_mm = Some(10.0);
    let config = build_config(Some(stock_config()), table(tool), Some(test_material()));
    let mut inside = motion(
        2,
        MoveKind::Feed,
        [-5.0, 0.0, -1.0],
        [5.0, 0.0, -1.0],
        Some(800.0),
        1,
    );
    inside.spindle_rpm = Some(6000.0);
    let mut above = motion(
        3,
        MoveKind::Feed,
        [-5.0, 1.0, -1.0],
        [5.0, 1.0, -1.0],
        Some(800.0),
        1,
    );
    above.spindle_rpm = Some(7000.0);
    let program = ProgramBuilder::new().record(inside).record(above).build();
    let findings = extract_findings(evaluate(
        rule("process.surface_speed").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 3);
    assert_eq!(
        findings[0].evidence.get("max_mpm"),
        Some(&EvidenceValue::Float(200.0))
    );
    let measured = match findings[0].evidence.get("surface_speed_mpm") {
        Some(EvidenceValue::Float(value)) => *value,
        other => panic!("missing surface speed: {other:?}"),
    };
    assert!(measured > 200.0);
}

#[test]
fn surface_speed_at_the_exact_material_bound_does_not_fire() {
    let mut material = test_material();
    // Use the same formula as the rule so the comparison is an exact tie.
    material.surface_speed_max_mpm = Some(6000.0 * math::PI * 10.0 / 1000.0);
    let mut tool = endmill(1);
    tool.diameter_mm = Some(10.0);
    let config = build_config(Some(stock_config()), table(tool), Some(material));
    let mut record = motion(
        2,
        MoveKind::Feed,
        [-5.0, 0.0, -1.0],
        [5.0, 0.0, -1.0],
        Some(800.0),
        1,
    );
    record.spindle_rpm = Some(6000.0);
    let program = ProgramBuilder::new().record(record).build();
    let outcome = evaluate(rule("process.surface_speed").as_ref(), &program, &config);
    assert!(extract_findings(outcome).is_empty());
}

#[test]
fn surface_speed_skips_without_a_range_or_spindle() {
    let mut material = test_material();
    material.surface_speed_min_mpm = None;
    material.surface_speed_max_mpm = None;
    let config = build_config(Some(stock_config()), table(endmill(1)), Some(material));
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let report = run_rule("process.surface_speed", &program, &config);
    assert_eq!(
        skipped_keys(&report, "process.surface_speed"),
        vec!["material"]
    );

    let mut record = motion(
        2,
        MoveKind::Feed,
        [-5.0, 0.0, -1.0],
        [5.0, 0.0, -1.0],
        Some(800.0),
        1,
    );
    record.spindle_rpm = None;
    let program = ProgramBuilder::new().record(record).build();
    let report = run_rule(
        "process.surface_speed",
        &program,
        &build_config(
            Some(stock_config()),
            table(endmill(1)),
            Some(test_material()),
        ),
    );
    assert_eq!(
        skipped_keys(&report, "process.surface_speed"),
        vec!["spindle_rpm"]
    );
}

// ---------------------------------------------------------------------------
// process.axial_depth
// ---------------------------------------------------------------------------

#[test]
fn axial_depth_flags_a_cut_deeper_than_one_diameter() {
    let config = ready_config();
    let deep = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -7.0],
            Some(300.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.axial_depth").as_ref(),
        &deep,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("axial_doc_mm"),
        Some(&EvidenceValue::Float(7.0))
    );
    assert_eq!(
        findings[0].evidence.get("threshold"),
        Some(&EvidenceValue::Float(6.0))
    );
    assert_eq!(
        findings[0].evidence.get("stock_source"),
        Some(&EvidenceValue::Text("explicit".to_string()))
    );

    let near_miss = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -6.0],
            Some(300.0),
            1,
        ))
        .build();
    let outcome = evaluate(rule("process.axial_depth").as_ref(), &near_miss, &config);
    assert!(
        extract_findings(outcome).is_empty(),
        "exactly 1 x D is allowed"
    );
}

#[test]
fn axial_depth_ratio_is_configurable() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -3.5],
            Some(300.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let axial = rule("process.axial_depth");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let outcome = axial.evaluate(&model, &settings(&[("axial_doc_ratio", 0.5)]));
    assert_eq!(
        extract_findings(outcome).len(),
        1,
        "3.5 mm exceeds 0.5 x 6 mm"
    );

    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let outcome = axial.evaluate(&model, &settings(&[("axial_doc_ratio", 1.0)]));
    assert!(extract_findings(outcome).is_empty());
}

// ---------------------------------------------------------------------------
// process.radial_engagement
// ---------------------------------------------------------------------------

#[test]
fn radial_engagement_flags_a_full_width_slot() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let radial = rule("process.radial_engagement");

    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let findings = extract_findings(radial.evaluate(&model, &RuleSettings::default()));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("radial_doc_mm"),
        Some(&EvidenceValue::Float(6.0))
    );
    let contact_deg = match findings[0].evidence.get("contact_angle_deg") {
        Some(EvidenceValue::Float(value)) => *value,
        other => panic!("missing contact angle: {other:?}"),
    };
    assert!(math::abs(contact_deg - 360.0) < 0.01);

    // A 100% engagement limit allows the full slot exactly.
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let outcome = radial.evaluate(&model, &settings(&[("radial_engagement_ratio", 1.0)]));
    assert!(extract_findings(outcome).is_empty());
}

#[test]
fn radial_engagement_sees_a_half_immersed_pass() {
    // Roughen the lower half below the finishing Z, then cut along the
    // boundary at a shallower Z: only the virgin upper half is above the cut.
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, -5.0, -4.0],
            [5.0, -5.0, -4.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [-5.0, -4.25, -4.0],
            [5.0, -4.25, -4.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            4,
            MoveKind::Feed,
            [-5.0, -3.5, -4.0],
            [5.0, -3.5, -4.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            5,
            MoveKind::Feed,
            [-5.0, 0.0, -3.0],
            [5.0, 0.0, -3.0],
            Some(800.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let radial = rule("process.radial_engagement");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let findings =
        extract_findings(radial.evaluate(&model, &settings(&[("radial_engagement_ratio", 0.4)])));
    let boundary = findings
        .iter()
        .find(|finding| finding.line == 5)
        .expect("boundary pass is flagged");
    let radial_doc = match boundary.evidence.get("radial_doc_mm") {
        Some(EvidenceValue::Float(value)) => *value,
        other => panic!("missing radial doc: {other:?}"),
    };
    assert!(
        radial_doc > 2.0 && radial_doc < 5.0,
        "half immersion should be near one radius, got {radial_doc}"
    );
}

// ---------------------------------------------------------------------------
// process.mrr
// ---------------------------------------------------------------------------

#[test]
fn mrr_flags_removal_above_the_material_limit() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -3.0],
            Some(1000.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(rule("process.mrr").as_ref(), &program, &config));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("mrr_mm3_min"),
        Some(&EvidenceValue::Float(18_000.0))
    );
    assert_eq!(
        findings[0].evidence.get("max_mrr_mm3_min"),
        Some(&EvidenceValue::Float(10_000.0))
    );
}

#[test]
fn mrr_at_the_exact_limit_does_not_fire() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -3.0],
            Some(1000.0),
            1,
        ))
        .build();
    let config = ready_config();
    let first = extract_findings(evaluate(rule("process.mrr").as_ref(), &program, &config));
    let measured = match first[0].evidence.get("mrr_mm3_min") {
        Some(EvidenceValue::Float(value)) => *value,
        other => panic!("missing mrr: {other:?}"),
    };

    let mut material = test_material();
    material.max_mrr_mm3_min = Some(measured);
    let config = build_config(Some(stock_config()), table(endmill(1)), Some(material));
    let outcome = evaluate(rule("process.mrr").as_ref(), &program, &config);
    assert!(extract_findings(outcome).is_empty());
}

#[test]
fn mrr_skips_without_a_material_limit() {
    let mut material = test_material();
    material.max_mrr_mm3_min = None;
    let config = build_config(Some(stock_config()), table(endmill(1)), Some(material));
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let report = run_rule("process.mrr", &program, &config);
    assert_eq!(skipped_keys(&report, "process.mrr"), vec!["material"]);
}

// ---------------------------------------------------------------------------
// process.flute_length
// ---------------------------------------------------------------------------

#[test]
fn flute_length_flags_a_cut_deeper_than_the_flutes() {
    let config = build_config(Some(deep_stock_config()), table(endmill(1)), None);
    let deep = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -19.0],
            Some(300.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.flute_length").as_ref(),
        &deep,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("flute_length_mm"),
        Some(&EvidenceValue::Float(18.0))
    );

    let near_miss = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -18.0],
            Some(300.0),
            1,
        ))
        .build();
    let outcome = evaluate(rule("process.flute_length").as_ref(), &near_miss, &config);
    assert!(extract_findings(outcome).is_empty());
}

// ---------------------------------------------------------------------------
// process.stickout_ratio
// ---------------------------------------------------------------------------

#[test]
fn stickout_ratio_flags_a_long_tool_and_allows_the_boundary() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();

    let mut long = endmill(1);
    long.stickout_mm = Some(60.0);
    let config = build_config(None, table(long), None);
    let findings = extract_findings(evaluate(
        rule("process.stickout_ratio").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("ratio"),
        Some(&EvidenceValue::Float(10.0))
    );

    let mut boundary = endmill(1);
    boundary.stickout_mm = Some(48.0);
    let config = build_config(None, table(boundary), None);
    let outcome = evaluate(rule("process.stickout_ratio").as_ref(), &program, &config);
    assert!(extract_findings(outcome).is_empty(), "8.0 x D is allowed");
}

// ---------------------------------------------------------------------------
// process.plunge_feed
// ---------------------------------------------------------------------------

#[test]
fn plunge_feed_compares_with_the_highest_cutting_feed() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(1000.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Plunge,
            [5.0, 0.0, 1.0],
            [5.0, 0.0, -1.0],
            Some(600.0),
            1,
        ))
        .record(motion(
            4,
            MoveKind::Plunge,
            [6.0, 0.0, 1.0],
            [6.0, 0.0, -1.0],
            Some(500.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.plunge_feed").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 3);
    assert_eq!(
        findings[0].evidence.get("reference_feed_mm_min"),
        Some(&EvidenceValue::Float(1000.0))
    );
}

#[test]
fn plunge_feed_skips_without_a_reference_feed() {
    let config = ready_config();
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            Some(300.0),
            1,
        ))
        .build();
    let report = run_rule("process.plunge_feed", &program, &config);
    assert_eq!(
        skipped_keys(&report, "process.plunge_feed"),
        vec!["feed.reference"]
    );
}

// ---------------------------------------------------------------------------
// process.shank_collision
// ---------------------------------------------------------------------------

#[test]
fn shank_collision_flags_the_shank_and_the_holder() {
    let config = build_config(Some(deep_stock_config()), table(endmill(1)), None);
    let clear = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -5.0],
            Some(300.0),
            1,
        ))
        .build();
    let outcome = evaluate(rule("process.shank_collision").as_ref(), &clear, &config);
    assert!(extract_findings(outcome).is_empty());

    let mut tool = endmill(1);
    tool.holder_diameter_mm = Some(20.0);
    tool.stickout_mm = Some(25.0);
    let config = build_config(Some(deep_stock_config()), table(tool), None);
    let deep = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -30.0],
            Some(300.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.shank_collision").as_ref(),
        &deep,
        &config,
    ));
    assert_eq!(findings.len(), 2, "one shank and one holder finding");
    let parts: Vec<String> = findings
        .iter()
        .filter_map(|finding| match finding.evidence.get("part") {
            Some(EvidenceValue::Text(part)) => Some(part.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(parts, vec!["shank".to_string(), "holder".to_string()]);
}

#[test]
fn shank_collision_without_holder_data_checks_only_the_shank() {
    let mut tool = endmill(1);
    tool.flute_length_mm = Some(10.0);
    let config = build_config(Some(deep_stock_config()), table(tool), None);
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -12.0],
            Some(300.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.shank_collision").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("part"),
        Some(&EvidenceValue::Text("shank".to_string()))
    );
}

// ---------------------------------------------------------------------------
// process.thin_wall
// ---------------------------------------------------------------------------

#[test]
fn thin_wall_flags_a_one_cell_standing_ridge() {
    let stock = StockConfig {
        origin_mm: [-6.0, -6.0, -4.0],
        size_mm: [12.0, 12.0, 4.0],
    };
    let mut tool = endmill(1);
    tool.diameter_mm = Some(8.0);
    let config = build_config(Some(stock), table(tool), None);
    // With R = 4, the left pass reaches x = -0.125 and the right pass starts
    // at x = 0.5, leaving exactly the column centred at x = 0.375 standing.
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-4.125, -5.0, -2.0],
            [-4.125, 5.0, -2.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [4.5, -5.0, -2.0],
            [4.5, 5.0, -2.0],
            Some(800.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.thin_wall").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].evidence.get("wall_mm"),
        Some(&EvidenceValue::Float(0.75))
    );
    assert_eq!(
        findings[0].evidence.get("threshold"),
        Some(&EvidenceValue::Float(0.8))
    );
    assert_eq!(
        findings[0].evidence.get("axis"),
        Some(&EvidenceValue::Text("X".to_string()))
    );
}

#[test]
fn thin_wall_allows_a_two_cell_ridge() {
    let stock = StockConfig {
        origin_mm: [-6.0, -6.0, -4.0],
        size_mm: [12.0, 12.0, 4.0],
    };
    let mut tool = endmill(1);
    tool.diameter_mm = Some(8.0);
    let config = build_config(Some(stock), table(tool), None);
    // Both passes stop one cell earlier, leaving a 1.5 mm ridge.
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-4.5, -5.0, -2.0],
            [-4.5, 5.0, -2.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [4.5, -5.0, -2.0],
            [4.5, 5.0, -2.0],
            Some(800.0),
            1,
        ))
        .build();
    let outcome = evaluate(rule("process.thin_wall").as_ref(), &program, &config);
    assert!(extract_findings(outcome).is_empty());
}

#[test]
fn thin_wall_and_rest_material_require_explicit_stock() {
    let tool = endmill(1);
    let inferred = build_config(None, table(tool), None);
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-5.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(800.0),
            1,
        ))
        .build();
    let report = run_rule("process.thin_wall", &program, &inferred);
    assert_eq!(
        skipped_keys(&report, "process.thin_wall"),
        vec!["stock.explicit"]
    );
    let report = run_rule("process.rest_material", &program, &inferred);
    assert_eq!(
        skipped_keys(&report, "process.rest_material"),
        vec!["stock.explicit"]
    );
}

// ---------------------------------------------------------------------------
// process.rest_material
// ---------------------------------------------------------------------------

#[test]
fn rest_material_flags_an_enclosed_island() {
    let stock = StockConfig {
        origin_mm: [-10.0, -10.0, -10.0],
        size_mm: [20.0, 20.0, 10.0],
    };
    let mut tool = endmill(1);
    tool.diameter_mm = Some(2.0);
    let config = build_config(Some(stock), table(tool), None);
    // Four slots close a square loop; the interior is enclosed rest material.
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-4.0, -4.0, -3.0],
            [4.0, -4.0, -3.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [4.0, -4.0, -3.0],
            [4.0, 4.0, -3.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            4,
            MoveKind::Feed,
            [4.0, 4.0, -3.0],
            [-4.0, 4.0, -3.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            5,
            MoveKind::Feed,
            [-4.0, 4.0, -3.0],
            [-4.0, -4.0, -3.0],
            Some(800.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.rest_material").as_ref(),
        &program,
        &config,
    ));
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.severity, Severity::Info);
    assert_eq!(
        finding.evidence.get("highest_mm"),
        Some(&EvidenceValue::Float(0.0))
    );
    assert_eq!(
        finding.evidence.get("lowest_final_height_mm"),
        Some(&EvidenceValue::Float(-3.0))
    );
    match finding.evidence.get("area_mm2") {
        Some(EvidenceValue::Float(value)) => assert!(*value > 0.0),
        other => panic!("missing area: {other:?}"),
    }
}

#[test]
fn rest_material_threshold_above_the_remaining_height_is_quiet() {
    let stock = StockConfig {
        origin_mm: [-10.0, -10.0, -10.0],
        size_mm: [20.0, 20.0, 10.0],
    };
    let mut tool = endmill(1);
    tool.diameter_mm = Some(2.0);
    let config = build_config(Some(stock), table(tool), None);
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Feed,
            [-2.0, -5.0, -3.0],
            [-2.0, 5.0, -3.0],
            Some(800.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [2.0, -5.0, -3.0],
            [2.0, 5.0, -3.0],
            Some(800.0),
            1,
        ))
        .build();
    let stock = config.stock.expect("stock").to_stock().expect("stock");
    let rest = rule("process.rest_material");
    let model = AnalysisModel::new(
        &program,
        stock,
        &config.tools,
        None,
        config.material.as_ref(),
        &config.tolerances,
    );
    let outcome = rest.evaluate(&model, &settings(&[("rest_material_mm", 3.5)]));
    assert!(extract_findings(outcome).is_empty());
}

// ---------------------------------------------------------------------------
// Coverage and determinism
// ---------------------------------------------------------------------------

#[test]
fn full_process_registry_evaluates_and_reports_zero_skips() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 2.0],
            [0.0, 0.0, 1.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -2.0],
            Some(300.0),
            1,
        ))
        .record(motion(
            4,
            MoveKind::Feed,
            [0.0, 0.0, -2.0],
            [5.0, 0.0, -2.0],
            Some(1000.0),
            1,
        ))
        .record(motion(
            5,
            MoveKind::Plunge,
            [5.0, 0.0, -2.0],
            [5.0, 0.0, -3.0],
            Some(400.0),
            1,
        ))
        .build();
    let config = ready_config();
    let source = source_lines(5);
    let request = AnalysisRequest::new("part.nc", &source, &config, Dialect::Generic);
    let registry = RuleRegistry::from_rules(rules::process::rules());
    let report = analyze(&program, &request, &registry);

    assert_eq!(report.coverage.total, 11);
    assert_eq!(report.coverage.evaluated, 11);
    assert!(report.coverage.skipped.is_empty());
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total
    );
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped.len(),
        report.coverage.total
    );

    let second = analyze(&program, &request, &registry);
    assert_eq!(report.to_json(), second.to_json());
    assert_eq!(report.to_json(), report.to_json());
}

#[test]
fn process_skips_name_exact_missing_inputs() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            Some(300.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [5.0, 0.0, -1.0],
            Some(1000.0),
            1,
        ))
        .build();
    let config = AnalysisConfig {
        stock: Some(stock_config()),
        ..AnalysisConfig::default()
    };
    let source = source_lines(3);
    let request = AnalysisRequest::new("part.nc", &source, &config, Dialect::Generic);
    let registry = RuleRegistry::from_rules(rules::process::rules());
    let report = analyze(&program, &request, &registry);

    assert_eq!(report.coverage.total, 11);
    assert_eq!(
        report.coverage.evaluated, 2,
        "plunge_feed and rest_material run"
    );
    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        11
    );

    assert_eq!(
        skipped_keys(&report, "process.chipload"),
        vec!["material", "tool.flutes:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.surface_speed"),
        vec!["material", "tool.diameter:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.axial_depth"),
        vec!["tool.diameter:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.radial_engagement"),
        vec!["tool.diameter:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.mrr"),
        vec!["material", "tool.diameter:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.flute_length"),
        vec!["tool.flute_length:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.stickout_ratio"),
        vec!["tool.diameter:T1", "tool.stickout:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.shank_collision"),
        vec!["tool.flute_length:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.thin_wall"),
        vec!["tool.diameter:T1"]
    );
    assert_eq!(
        skipped_keys(&report, "process.rest_material"),
        Vec::<String>::new()
    );
}

#[test]
fn process_rules_report_stock_provenance_on_inferred_stock() {
    let mut tool = endmill(1);
    tool.diameter_mm = Some(4.0);
    let config = build_config(None, table(tool), None);
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -5.0],
            Some(300.0),
            1,
        ))
        .record(motion(
            3,
            MoveKind::Feed,
            [0.0, 0.0, -5.0],
            [5.0, 0.0, -5.0],
            Some(800.0),
            1,
        ))
        .build();
    let findings = extract_findings(evaluate(
        rule("process.axial_depth").as_ref(),
        &program,
        &config,
    ));
    assert!(!findings.is_empty());
    assert_eq!(
        findings[0].evidence.get("stock_source"),
        Some(&EvidenceValue::Text("inferred".to_string()))
    );
    assert!(findings[0].evidence.contains_key("inference_basis"));
}

#[test]
fn process_threshold_keys_all_default_when_absent() {
    // A clean program that every rule evaluates without findings, proving the
    // documented defaults are not zero and do not spuriously fire.
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Rapid,
            [0.0, 0.0, 2.0],
            [0.0, 0.0, 1.0],
            None,
            1,
        ))
        .record(motion(
            3,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            Some(300.0),
            1,
        ))
        .record(motion(
            4,
            MoveKind::Feed,
            [0.0, 0.0, -1.0],
            [3.0, 0.0, -1.0],
            Some(1000.0),
            1,
        ))
        .build();
    let mut tool = endmill(1);
    tool.diameter_mm = Some(10.0);
    let config = build_config(Some(stock_config()), table(tool), Some(test_material()));
    let source = source_lines(4);
    let request = AnalysisRequest::new("part.nc", &source, &config, Dialect::Generic);
    let registry = RuleRegistry::from_rules(rules::process::rules());
    let report = analyze(&program, &request, &registry);
    assert_eq!(report.coverage.evaluated, 11);
    let unexpected: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|finding| finding.rule.as_str())
        .filter(|rule| {
            !matches!(
                *rule,
                "process.surface_speed" | "process.radial_engagement" | "process.rest_material"
            )
        })
        .collect();
    assert!(unexpected.is_empty(), "unexpected findings: {unexpected:?}");
}

#[test]
fn process_rule_evidence_is_typed_and_serializes_twice_identically() {
    let program = ProgramBuilder::new()
        .record(motion(
            2,
            MoveKind::Plunge,
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -7.0],
            Some(300.0),
            1,
        ))
        .build();
    let config = ready_config();
    let report = run_rule("process.axial_depth", &program, &config);
    assert_eq!(report.diagnostics.len(), 1);
    let evidence: &BTreeMap<String, EvidenceValue> = &report.diagnostics[0].evidence;
    assert!(matches!(evidence.get("tool"), Some(EvidenceValue::Int(1))));
    assert!(matches!(
        evidence.get("threshold"),
        Some(EvidenceValue::Float(_))
    ));
    assert!(matches!(
        evidence.get("stock_source"),
        Some(EvidenceValue::Text(_))
    ));
    assert_eq!(report.to_json(), report.to_json());
}
