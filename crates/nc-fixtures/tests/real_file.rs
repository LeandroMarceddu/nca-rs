//! Real-file regression for the 119k-line `box54` fixture.
//!
//! There is intentionally no `expected.json` here: the report is a large,
//! model-heavy corpus and its goldens will be blessed only after the model
//! rules are tuned. This test locks the properties that must not regress:
//! identity, determinism, caps, line ranges, coverage arithmetic and tool hints.

use std::path::{Path, PathBuf};

use nc_fixtures::{Fixture, run_fixture};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
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
