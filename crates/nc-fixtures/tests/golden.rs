//! Golden-report tests over every reviewed fixture directory.

use std::fs;
use std::path::{Path, PathBuf};

use nc_analysis::Report;
use nc_fixtures::{Fixture, FixtureDiff, compare_reports, golden_text, run_fixture};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .to_path_buf()
}

fn fixture_dirs() -> Vec<PathBuf> {
    let root = workspace_root().join("fixtures");
    let mut dirs = Vec::new();
    for category in ["synthetic", "real"] {
        let Ok(entries) = fs::read_dir(root.join(category)) else {
            continue;
        };
        let mut category_dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        category_dirs.sort();
        dirs.extend(category_dirs);
    }
    dirs
}

#[test]
fn every_reviewed_fixture_matches_its_golden() {
    let mut reviewed = 0usize;
    let mut failures = Vec::new();

    for dir in fixture_dirs() {
        let fixture = Fixture::load(&dir)
            .unwrap_or_else(|error| panic!("fixture {} failed to load: {error}", dir.display()));
        let report = fixture
            .run()
            .unwrap_or_else(|error| panic!("fixture {} failed to run: {error}", dir.display()));

        // Determinism: the same input twice is byte-identical.
        let second = fixture
            .run()
            .unwrap_or_else(|error| panic!("fixture {} failed on rerun: {error}", dir.display()));
        assert_eq!(
            report.to_json(),
            second.to_json(),
            "fixture {} is not deterministic",
            dir.display()
        );

        let actual = golden_text(&report);
        if let Some(root) = workspace_root().to_str()
            && actual.contains(root)
        {
            failures.push(format!(
                "fixture {} contains the absolute workspace path",
                dir.display()
            ));
        }

        let Some(expected_text) = fixture.expected_text.as_deref() else {
            continue;
        };
        reviewed += 1;

        if !expected_text.ends_with('\n') || expected_text.ends_with("\r\n") {
            failures.push(format!(
                "fixture {} expected.json must end with exactly one LF",
                dir.display()
            ));
        }

        if expected_text != actual {
            let reason = describe_mismatch(expected_text, &report);
            failures.push(format!("fixture {}: {reason}", dir.display()));
        }
    }

    if reviewed == 0 {
        eprintln!("no fixture has expected.json yet; golden comparison is skipped");
        return;
    }
    assert!(
        failures.is_empty(),
        "{} fixture golden failure(s):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Turns a byte mismatch into the first JSON pointer difference.
fn describe_mismatch(expected_text: &str, actual: &Report) -> String {
    let expected: Report = match serde_json::from_str(expected_text) {
        Ok(report) => report,
        Err(error) => {
            return format!("expected.json is not a report: {error}");
        }
    };
    match compare_reports(&expected, actual) {
        Ok(()) => "canonical bytes differ only in whitespace/formatting".to_string(),
        Err(diff) => format_diff(&diff),
    }
}

fn format_diff(diff: &FixtureDiff) -> String {
    format!("{diff}")
}

#[test]
fn compare_reports_names_the_first_difference() {
    let dir = workspace_root().join("fixtures/synthetic/clean-pocket");
    let report = run_fixture(&dir).expect("fixture");
    assert!(golden_text(&report).ends_with('\n'));

    let mut changed = report.clone();
    changed.input.line_count += 1;
    let diff = compare_reports(&report, &changed).expect_err("must differ");
    assert_eq!(diff.pointer, "/input/line_count");
    assert!(diff.expected.is_some());
    assert!(diff.actual.is_some());
    assert!(diff.context.is_some());
    assert!(diff.to_string().contains("/input/line_count"));

    let mut diagnostic_changed = report.clone();
    if let Some(first) = diagnostic_changed.diagnostics.first_mut() {
        first.message.push_str(" changed");
    } else {
        // clean-pocket currently has no findings; add one for the pointer test.
        diagnostic_changed
            .diagnostics
            .push(nc_analysis::Diagnostic {
                rule: nc_analysis::RuleId::new("modal.feed_missing"),
                severity: nc_analysis::Severity::Error,
                basis: nc_analysis::Basis::Exact,
                line: 1,
                column: 0,
                line_end: 1,
                message: "synthetic".to_string(),
                evidence: std::collections::BTreeMap::new(),
            });
    }
    let diff = compare_reports(&report, &diagnostic_changed).expect_err("must differ");
    assert_eq!(diff.pointer, "/diagnostics/0");
}

/// Explicit draft generator. Never runs as part of the normal test suite.
///
/// Run it with:
///
/// ```text
/// NCA_WRITE_GOLDENS=1 cargo test -p nc-fixtures write_draft_goldens -- --ignored
/// ```
///
/// It only writes `expected.json`; review the diff before committing.
#[test]
#[ignore = "writes draft expected.json files; run explicitly with NCA_WRITE_GOLDENS=1"]
fn write_draft_goldens() {
    assert_eq!(
        std::env::var("NCA_WRITE_GOLDENS").as_deref(),
        Ok("1"),
        "refusing to write goldens without NCA_WRITE_GOLDENS=1"
    );
    let root = workspace_root().join("fixtures/synthetic");
    let mut entries: Vec<PathBuf> = fs::read_dir(&root)
        .expect("fixtures/synthetic")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for dir in entries {
        let report = run_fixture(&dir)
            .unwrap_or_else(|error| panic!("fixture {} failed: {error}", dir.display()));
        let target = dir.join("expected.json");
        fs::write(&target, golden_text(&report))
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", target.display()));
        println!("wrote {}", target.display());
    }
}
