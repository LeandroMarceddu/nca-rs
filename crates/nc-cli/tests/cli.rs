//! End-to-end tests for the `nca` binary.
//!
//! These tests exercise the public command behavior: argument handling, exit
//! codes, canonical JSON, human formats, deterministic heightmap export, and
//! the absence of any GUI dependency edge.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .to_path_buf()
}

fn fixture(name: &str) -> PathBuf {
    workspace_root().join("fixtures/synthetic").join(name)
}

fn nca(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nca"))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("run nca")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn check_args(name: &str) -> Vec<String> {
    let dir = fixture(name);
    vec![
        "check".to_string(),
        dir.join("source.nc").display().to_string(),
        "--config".to_string(),
        dir.join("config.toml").display().to_string(),
    ]
}

fn run_check(name: &str, extra: &[&str]) -> Output {
    let mut args = check_args(name);
    args.extend(extra.iter().map(|arg| (*arg).to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    nca(&refs)
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nca-cli-{}-{name}", std::process::id()))
}

#[test]
fn json_is_canonical_and_reports_the_file_name_only() {
    let output = run_check("rapid-through-stock", &["--fail-on", "warning"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let stdout = text(&output);
    assert!(stdout.ends_with('\n'), "JSON must end with one newline");
    let value: Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(value["schema"], "nca.report/1");
    assert_eq!(value["input"]["file_name"], "source.nc");
    assert!(
        !stdout.contains(&workspace_root().display().to_string()),
        "report leaked an absolute path"
    );
    let diagnostics = value["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(diagnostics.len(), 2, "{stdout}");
    for diagnostic in diagnostics {
        assert_eq!(diagnostic["rule"], "clearance.rapid_at_depth");
        assert_eq!(diagnostic["severity"], "warning");
    }
}

#[test]
fn fail_on_error_ignores_warnings_and_fail_on_warning_catches_them() {
    let default = run_check("rapid-through-stock", &[]);
    assert_eq!(default.status.code(), Some(0), "{}", stderr(&default));
    let strict = run_check("rapid-through-stock", &["--fail-on", "warning"]);
    assert_eq!(strict.status.code(), Some(1), "{}", stderr(&strict));
    let clean = run_check("clean-pocket", &["--fail-on", "warning"]);
    assert_eq!(clean.status.code(), Some(0), "{}", stderr(&clean));
}

#[test]
fn human_formats_carry_counts_and_every_diagnostic() {
    let output = run_check("rapid-through-stock", &["--format", "text"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let stdout = text(&output);
    assert!(
        stdout.contains("diagnostics: 2 (0 error, 2 warning, 0 info)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("coverage: 2/3 evaluated, 1 skipped"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[warning] clearance.rapid_at_depth line 10"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[warning] clearance.rapid_at_depth line 11"),
        "{stdout}"
    );
    assert!(stdout.contains("evidence:"), "{stdout}");

    let output = run_check("rapid-through-stock", &["--format", "markdown"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let stdout = text(&output);
    assert!(stdout.contains("# nca check: source.nc"), "{stdout}");
    assert!(
        stdout.contains("| diagnostics | 2 (0 error, 2 warning, 0 info) |"),
        "{stdout}"
    );
    assert!(
        stdout.contains("**warning** `clearance.rapid_at_depth` line 10"),
        "{stdout}"
    );
    assert!(stdout.contains("## Coverage"), "{stdout}");
}

#[test]
fn output_writes_the_same_bytes_as_stdout() {
    let direct = run_check("missing-feed", &[]);
    assert_eq!(direct.status.code(), Some(1), "{}", stderr(&direct));
    let expected = text(&direct);
    assert!(
        expected.contains("\"rule\": \"parse.unsupported\""),
        "lossy blocks must surface as parse.unsupported findings:\n{expected}"
    );

    let path = temp_path("check.json");
    let output = run_check(
        "missing-feed",
        &["--output", path.to_str().expect("utf-8 path")],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(text(&output).is_empty(), "--output must not also print");
    let written = fs::read_to_string(&path).expect("read output");
    fs::remove_file(&path).ok();
    assert_eq!(written, expected);
    assert!(written.ends_with('\n'));
}

#[test]
fn missing_file_and_invalid_config_exit_two() {
    let missing = nca(&["check", "/definitely/not/here.nc"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("cannot read"));

    let config = temp_path("bad.toml");
    fs::write(&config, "[parse]\ndialect = \"siemens\"\n").expect("write config");
    let output = nca(&[
        "check",
        fixture("clean-pocket")
            .join("source.nc")
            .to_str()
            .expect("path"),
        "--config",
        config.to_str().expect("path"),
    ]);
    fs::remove_file(&config).ok();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("unknown dialect"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn help_version_and_rules_are_available() {
    let help = nca(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(text(&help).contains("usage:"));
    assert!(text(&help).contains("nca check"));

    let version = nca(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(text(&version).contains(env!("CARGO_PKG_VERSION")));
    assert!(text(&version).contains("nca.report/1"));

    let rules = nca(&["rules"]);
    assert_eq!(rules.status.code(), Some(0));
    let stdout = text(&rules);
    assert!(stdout.contains("rules: 3"), "{stdout}");
    for id in [
        "clearance.rapid_at_depth",
        "modal.feed_missing",
        "travel.machine_envelope",
    ] {
        assert!(stdout.contains(id), "missing {id} in:\n{stdout}");
    }
    assert!(stdout.contains("inferred ok"));
    assert!(stdout.contains("tool fields"));
}

#[test]
fn simulate_writes_a_deterministic_heightmap() {
    let first = temp_path("sim-a.png");
    let second = temp_path("sim-b.png");
    let args = |path: &Path| {
        let dir = fixture("clean-pocket");
        vec![
            "simulate".to_string(),
            dir.join("source.nc").display().to_string(),
            "--config".to_string(),
            dir.join("config.toml").display().to_string(),
            "--to-line".to_string(),
            "17".to_string(),
            "--out-heightmap".to_string(),
            path.display().to_string(),
        ]
    };

    for path in [&first, &second] {
        let owned = args(path);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let output = nca(&refs);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let stdout = text(&output);
        assert!(stdout.contains("tick:"), "{stdout}");
        assert!(stdout.contains("time:"), "{stdout}");
        assert!(stdout.contains("tool:"), "{stdout}");
        assert!(stdout.contains("volume:"), "{stdout}");
        assert!(stdout.contains("volume assumed:"), "{stdout}");
        assert!(stdout.contains("assumed:"), "{stdout}");
        assert!(stdout.contains("max depth: 1.000 mm"), "{stdout}");
        assert!(stdout.contains("coarse:"), "{stdout}");
        assert!(stdout.contains("heightmap:"));
    }

    let bytes_a = fs::read(&first).expect("first png");
    let bytes_b = fs::read(&second).expect("second png");
    fs::remove_file(&first).ok();
    fs::remove_file(&second).ok();
    assert_eq!(bytes_a, bytes_b, "heightmap PNG must be deterministic");
    assert_eq!(&bytes_a[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn simulate_to_line_is_validated() {
    let dir = fixture("clean-pocket");
    let output = nca(&[
        "simulate",
        dir.join("source.nc").to_str().expect("path"),
        "--config",
        dir.join("config.toml").to_str().expect("path"),
        "--to-line",
        "9999",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--to-line"), "{}", stderr(&output));
}

#[test]
fn nc_cli_has_no_gpui_or_nc_app_dependency() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata JSON");

    let packages = metadata["packages"].as_array().expect("packages");
    let names: std::collections::BTreeMap<&str, &str> = packages
        .iter()
        .filter_map(|package| {
            Some((
                package["id"].as_str()?,
                package["name"].as_str().unwrap_or("<unnamed>"),
            ))
        })
        .collect();
    let cli_id = packages
        .iter()
        .find(|package| package["name"] == "nc-cli")
        .and_then(|package| package["id"].as_str())
        .expect("nc-cli package");
    let nodes = metadata["resolve"]["nodes"].as_array().expect("nodes");

    let mut pending = vec![cli_id.to_string()];
    let mut visited = BTreeSet::new();
    let mut offenders = Vec::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if names
            .get(id.as_str())
            .is_some_and(|name| *name == "nc-app" || name.starts_with("gpui"))
        {
            offenders.push(names[id.as_str()].to_string());
            continue;
        }
        let Some(node) = nodes.iter().find(|node| node["id"] == id.as_str()) else {
            continue;
        };
        for dependency in node["deps"].as_array().into_iter().flatten() {
            if let Some(package) = dependency["pkg"].as_str() {
                pending.push(package.to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "nc-cli dependency tree contains GUI packages: {offenders:?}"
    );
}
