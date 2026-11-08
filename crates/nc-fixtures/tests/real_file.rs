//! Real-file regressions for the reviewed real-world posts.
//!
//! There is intentionally no `expected.json` for these fixtures: the reports
//! are model-heavy corpora and their goldens will be blessed only after the
//! model rules are tuned. These tests lock the properties that must not
//! regress: identity, determinism, caps, line ranges, coverage arithmetic,
//! tool hints, and a clean parse.
//!
//! `box54-lid-50cm` is the 119k-line model corpus. `makera-latchbase` is the
//! first real Makera/Smoothieware post; it must parse with zero skipped blocks
//! and zero `parse.*` diagnostics.

use std::path::{Path, PathBuf};

use nc_analysis::EvidenceValue;
use nc_analysis::config_file::load_config;
use nc_core::Dialect;
use nc_fixtures::{Fixture, run_fixture};
use nc_gcode::{ParserConfig, parse_program_lossy};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn makera_latchbase_dir() -> PathBuf {
    workspace_root().join("fixtures/real/makera-latchbase")
}

#[test]
fn box54_report_is_deterministic_and_bounded() {
    let dir = workspace_root().join("fixtures/real/box54-lid-50cm");
    let fixture = Fixture::load(&dir).expect("load box54");
    let first = fixture.run().expect("first analysis");
    let second = run_fixture(&dir).expect("second analysis");

    assert_eq!(first.input.file_name, "source.nc");
    assert_eq!(first.input.line_count, 119_433);
    assert_eq!(first.input.geometry_uncertain_after, None);
    assert_eq!(first.stock_source, nc_stock::StockSource::Inferred);
    assert_eq!(first.tool_inventory.len(), 2);
    assert_eq!(first.tool_inventory[0].tool, 1);
    assert_eq!(first.tool_inventory[1].tool, 2);

    for tool in &first.tool_inventory {
        let diameter = tool.fields.get("diameter_mm").expect("hint diameter");
        assert!(diameter.source.starts_with("comment:L"));
    }

    assert_eq!(first.coverage.total, 45);
    assert_eq!(
        first.coverage.evaluated + first.coverage.skipped_rule_count(),
        first.coverage.total
    );
    assert!(first.coverage.evaluated > 0);
    assert!(!first.coverage.skipped.is_empty());

    assert!(first.diagnostics.len() <= 1_000);
    assert!(first.diagnostics.len() <= first.coverage.total * 100);
    assert!(
        first
            .truncated
            .get("clearance.rapid_at_depth")
            .is_some_and(|dropped| *dropped > 0)
    );
    assert!(
        first
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.line >= 1
                && diagnostic.line <= first.input.line_count
                && diagnostic.line_end >= diagnostic.line
                && diagnostic.line_end <= first.input.line_count)
    );

    let json = first.to_json();
    assert_eq!(json, second.to_json());
    let absolute = dir.display().to_string();
    assert!(!json.contains(&absolute));
    assert!(!json.contains("/home/"));
    assert!(!json.contains(&workspace_root().display().to_string()));
}

#[test]
fn box54_parse_and_analysis_fit_the_budget_in_debug() {
    use std::time::Instant;

    let dir = workspace_root().join("fixtures/real/box54-lid-50cm");
    let started = Instant::now();
    let report = run_fixture(&dir).expect("analysis");
    let elapsed = started.elapsed();
    println!(
        "box54: {} lines, {} diagnostics, {}/{} evaluated, {:?}",
        report.input.line_count,
        report.diagnostics.len(),
        report.coverage.evaluated,
        report.coverage.total,
        elapsed
    );
    // Debug-build ceiling; the release measurement lives in docs/PERFORMANCE.md.
    assert!(
        elapsed.as_secs_f64() < 30.0,
        "real-file analysis took {elapsed:?}"
    );
}

/// Asserts one `diameter_mm` field value and provenance.
fn assert_diameter(report: &nc_analysis::Report, tool: u32, diameter: f64, source: &str) {
    let entry = report
        .tool_inventory
        .iter()
        .find(|entry| entry.tool == tool)
        .unwrap_or_else(|| panic!("tool T{tool} is missing from the inventory"));
    let field = entry
        .fields
        .get("diameter_mm")
        .unwrap_or_else(|| panic!("tool T{tool} has no diameter_mm field"));
    match &field.value {
        EvidenceValue::Float(value) => assert!(
            (value - diameter).abs() < 1e-9,
            "T{tool} diameter is {value}, expected {diameter}"
        ),
        other => panic!("T{tool} diameter is not a float: {other:?}"),
    }
    assert_eq!(field.source, source, "T{tool} diameter provenance");
}

#[test]
fn makera_latchbase_parses_cleanly() {
    let dir = makera_latchbase_dir();
    let fixture = Fixture::load(&dir).expect("load makera-latchbase");
    let report = fixture.run().expect("analysis");

    assert_eq!(report.input.file_name, "source.nc");
    assert_eq!(report.input.line_count, 7_550);
    assert_eq!(
        report.input.sha256,
        "7aae6e520d549929ecc19fc869c82af192b648f4eb567921d0b4a68cf99ddb66"
    );
    assert_eq!(report.input.dialect, Dialect::Makera);
    assert_eq!(report.input.geometry_uncertain_after, None);

    let parse_rules: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule.as_str())
        .filter(|rule| rule.starts_with("parse."))
        .collect();
    assert!(
        parse_rules.is_empty(),
        "makera parse diagnostics: {parse_rules:?}"
    );

    let tools: Vec<u32> = report
        .tool_inventory
        .iter()
        .map(|entry| entry.tool)
        .collect();
    assert_eq!(tools, vec![1, 2, 5]);

    assert_diameter(&report, 1, 3.175, "comment:L7");
    assert_diameter(&report, 2, 3.175, "comment:L11");
    assert_diameter(&report, 5, 2.5, "comment:L6");

    // The recognized cutter shapes come from the same tool comments.
    let t1 = report
        .tool_inventory
        .iter()
        .find(|entry| entry.tool == 1)
        .expect("T1");
    let t1_shape = t1.fields.get("shape").expect("T1 shape hint");
    assert_eq!(t1_shape.value, EvidenceValue::Text("flat".to_string()));
    assert_eq!(t1_shape.source, "comment:L7");

    let t5 = report
        .tool_inventory
        .iter()
        .find(|entry| entry.tool == 5)
        .expect("T5");
    let t5_shape = t5.fields.get("shape").expect("T5 shape hint");
    assert_eq!(t5_shape.value, EvidenceValue::Text("drill".to_string()));
    assert_eq!(t5_shape.source, "comment:L6");

    assert_eq!(
        report.coverage.evaluated + report.coverage.skipped_rule_count(),
        report.coverage.total,
        "coverage arithmetic"
    );
    assert!(report.coverage.evaluated > 0);
    assert!(!report.coverage.skipped.is_empty());

    // The report must never leak the path the fixture was loaded from.
    let json = report.to_json();
    assert!(!json.contains(&dir.display().to_string()));
    assert!(!json.contains("/home/"));
    assert!(!json.contains(&workspace_root().display().to_string()));
}

#[test]
fn makera_latchbase_is_deterministic() {
    let dir = makera_latchbase_dir();
    let fixture = Fixture::load(&dir).expect("load makera-latchbase");
    let first = fixture.run().expect("first analysis");
    let second = run_fixture(&dir).expect("second analysis");

    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(first.to_json_pretty(), second.to_json_pretty());
}

#[test]
fn makera_latchbase_parses_without_skipped_blocks() {
    let dir = makera_latchbase_dir();
    let fixture = Fixture::load(&dir).expect("load makera-latchbase");
    let loaded = load_config(&dir.join("config.toml")).expect("load config");

    // `InputInfo` does not carry `dialect_assumed`; the report surfaces an
    // assumed dialect through the `parse.dialect_assumed` rule instead.
    assert_eq!(loaded.dialect, Dialect::Makera);
    assert!(!loaded.dialect_assumed);

    let parser_config = ParserConfig::default().with_dialect(loaded.dialect);
    let lossy = parse_program_lossy(&fixture.source, &parser_config);
    assert!(
        lossy.errors.is_empty(),
        "lossy parse errors: {:?}",
        lossy.errors.first()
    );
    assert!(
        lossy.skipped_blocks.is_empty(),
        "skipped blocks: {:?}",
        lossy.skipped_blocks.first()
    );
    assert!(lossy.program.skipped_blocks().is_empty());
    assert_eq!(lossy.program.geometry_uncertain_after(), None);

    let report = fixture.run().expect("analysis");
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule.as_str() == "parse.dialect_assumed")
    );
}
