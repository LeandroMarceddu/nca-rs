//! Golden-report tests over every reviewed fixture directory.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nc_analysis::Report;
use nc_fixtures::{
    EXPECTED_FILE, Fixture, FixtureDiff, REAL_FIXTURES_WITHOUT_GOLDEN, REQUIRED_FILES,
    SYNTHETIC_FIXTURES, compare_reports, golden_text, run_fixture,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .to_path_buf()
}

fn category_dirs(category: &str) -> Vec<PathBuf> {
    let root = workspace_root().join("fixtures").join(category);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

fn fixture_dirs() -> Vec<PathBuf> {
    let mut dirs = category_dirs("synthetic");
    dirs.extend(category_dirs("real"));
    dirs
}

fn fixture_name(dir: &Path) -> Option<&str> {
    dir.file_name().and_then(|name| name.to_str())
}

fn fixture_category(dir: &Path) -> Option<&str> {
    dir.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
}

/// Every synthetic fixture needs a reviewed golden. Real fixtures need one
/// too, except the names listed in [`REAL_FIXTURES_WITHOUT_GOLDEN`].
fn requires_golden(dir: &Path) -> bool {
    match fixture_category(dir) {
        Some("synthetic") => true,
        Some("real") => {
            fixture_name(dir).is_some_and(|name| !REAL_FIXTURES_WITHOUT_GOLDEN.contains(&name))
        }
        _ => false,
    }
}

#[test]
fn every_reviewed_fixture_matches_its_golden() {
    let mut failures = Vec::new();

    for dir in fixture_dirs() {
        let fixture = match Fixture::load(&dir) {
            Ok(fixture) => fixture,
            Err(error) => {
                failures.push(format!("fixture {} failed to load: {error}", dir.display()));
                continue;
            }
        };
        let report = match fixture.run() {
            Ok(report) => report,
            Err(error) => {
                failures.push(format!("fixture {} failed to run: {error}", dir.display()));
                continue;
            }
        };

        // Determinism: the same input twice is byte-identical.
        let second = match fixture.run() {
            Ok(report) => report,
            Err(error) => {
                failures.push(format!(
                    "fixture {} failed on rerun: {error}",
                    dir.display()
                ));
                continue;
            }
        };
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
            if requires_golden(&dir) {
                failures.push(format!(
                    "fixture {} is missing the required {EXPECTED_FILE}",
                    dir.display()
                ));
            }
            continue;
        };

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

    assert!(
        failures.is_empty(),
        "{} fixture golden failure(s):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn synthetic_fixture_manifest_is_exact() {
    let root = workspace_root().join("fixtures/synthetic");
    let entries = fs::read_dir(&root).expect("fixtures/synthetic must exist");
    let mut actual = BTreeSet::new();
    for entry in entries {
        let entry = entry.expect("synthetic directory entry");
        if entry.file_type().expect("directory entry type").is_dir() {
            actual.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }

    let expected: BTreeSet<String> = SYNTHETIC_FIXTURES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let missing: Vec<&String> = expected.difference(&actual).collect();
    let extra: Vec<&String> = actual.difference(&expected).collect();
    assert!(
        missing.is_empty(),
        "missing synthetic fixture directories: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "unexpected synthetic fixture directories: {extra:?}"
    );

    let mut missing_files = Vec::new();
    for name in SYNTHETIC_FIXTURES {
        let dir = root.join(name);
        for required in REQUIRED_FILES {
            if !dir.join(required).is_file() {
                missing_files.push(format!("{name}/{required}"));
            }
        }
        if !dir.join(EXPECTED_FILE).is_file() {
            missing_files.push(format!("{name}/{EXPECTED_FILE}"));
            continue;
        }
        match Fixture::load(&dir) {
            Ok(fixture) => {
                if fixture.expected_text.is_none() {
                    missing_files.push(format!("{name}/{EXPECTED_FILE} did not load"));
                }
            }
            Err(error) => missing_files.push(format!("{name} did not load: {error}")),
        }
    }
    assert!(
        missing_files.is_empty(),
        "required fixture files are missing:\n{}",
        missing_files.join("\n")
    );

    for name in REAL_FIXTURES_WITHOUT_GOLDEN {
        let dir = workspace_root().join("fixtures/real").join(name);
        assert!(
            dir.is_dir(),
            "exempt real fixture {name} directory is missing"
        );
    }
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

/// Explicit review helper. Never runs as part of the normal test suite.
///
/// Run it with:
///
/// ```text
/// NCA_WRITE_GOLDENS=1 cargo test -p nc-fixtures write_goldens_for_review -- --ignored
/// ```
///
/// It only writes `expected.json`; inspect the diff and review every report
/// before committing. The normal suite never writes goldens.
#[test]
#[ignore = "writes expected.json files; run explicitly with NCA_WRITE_GOLDENS=1"]
fn write_goldens_for_review() {
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
        let target = dir.join(EXPECTED_FILE);
        fs::write(&target, golden_text(&report))
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", target.display()));
        println!("wrote {}", target.display());
    }
}
