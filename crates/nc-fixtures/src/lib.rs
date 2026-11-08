//! Fixture loading, analysis runs, and golden-report comparison.
//!
//! A fixture is a directory containing `source.nc`, `config.toml`, and
//! optionally a reviewed `expected.json`. Loading never modifies the folder;
//! report comparison names the first differing JSON pointer so a golden review
//! does not require reading a whole-file diff.
//!
//! ```no_run
//! use nc_fixtures::run_fixture;
//!
//! let report = run_fixture(std::path::Path::new("fixtures/synthetic/missing-feed"))?;
//! assert_eq!(report.input.file_name, "source.nc");
//! # Ok::<(), nc_fixtures::FixtureError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nc_analysis::config_file::{ConfigError, load_config};
use nc_analysis::{AnalysisRequest, Report, RuleRegistry, analyze, rules};
use nc_core::ToolTable;
use nc_gcode::{ParserConfig, parse_program_lossy};
use serde_json::Value;

/// Base file names every fixture directory must contain.
pub const REQUIRED_FILES: [&str; 3] = ["source.nc", "config.toml", "NOTES.md"];

/// The optional reviewed golden report file name.
pub const EXPECTED_FILE: &str = "expected.json";

/// The synthetic fixtures that must each carry a reviewed `expected.json`.
///
/// The manifest is explicit so a missing folder or golden cannot quietly
/// reduce the regression corpus. Add a name only together with its reviewed
/// report.
pub const SYNTHETIC_FIXTURES: [&str; 13] = [
    "arc-smaller-than-tool",
    "bare-file",
    "clean-pocket",
    "cutter-comp",
    "drill-cycles",
    "duplicate-holes",
    "helical-bore",
    "inch-program",
    "manual-spindle",
    "missing-feed",
    "overlapping-holes",
    "rapid-through-stock",
    "unsupported-code",
];

/// Real fixtures deliberately exempt from the reviewed-golden requirement.
///
/// `box54-lid-50cm` is a large model-heavy program; it is locked by
/// `tests/real_file.rs` for determinism, caps, and performance instead.
/// `makera-latchbase` is the first real Makera/Smoothieware fixture; it is
/// locked for a clean parse and tool resolution until its model findings are
/// reviewed.
pub const REAL_FIXTURES_WITHOUT_GOLDEN: [&str; 2] = ["box54-lid-50cm", "makera-latchbase"];

/// A loaded fixture. Source and config are kept as text so tests can hash and
/// rerun the exact bytes that were reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Fixture directory.
    pub dir: PathBuf,
    /// Exact contents of `source.nc`.
    pub source: String,
    /// Exact contents of `config.toml`.
    pub config_text: String,
    /// Exact contents of `expected.json`, when present.
    pub expected_text: Option<String>,
}

impl Fixture {
    /// Loads `source.nc`, `config.toml`, the required `NOTES.md`, and the
    /// optional `expected.json`.
    pub fn load(dir: &Path) -> Result<Self, FixtureError> {
        let source = read_text(&dir.join("source.nc"))?;
        let config_text = read_text(&dir.join("config.toml"))?;
        // The notes are required even though they are not part of the report:
        // they carry the review date, config overrides, and expected coverage.
        let _notes = read_text(&dir.join("NOTES.md"))?;
        let expected_path = dir.join(EXPECTED_FILE);
        let expected_text = if expected_path.is_file() {
            Some(read_text(&expected_path)?)
        } else {
            None
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            source,
            config_text,
            expected_text,
        })
    }

    /// Runs analysis with the shared config loader and the workspace rules.
    pub fn run(&self) -> Result<Report, FixtureError> {
        let loaded = load_config(&self.dir.join("config.toml"))?;
        let parser_config = ParserConfig::default().with_dialect(loaded.dialect);
        let lossy = parse_program_lossy(&self.source, &parser_config);

        let tools = ToolTable::resolve(&loaded.analysis.tools, &lossy.program.tool_hints);
        let mut config = loaded.analysis;
        config.tools = tools;

        let file_name = self
            .dir
            .join("source.nc")
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source.nc")
            .to_string();
        let request = AnalysisRequest::new(&file_name, &self.source, &config, loaded.dialect)
            .with_dialect_assumed(loaded.dialect_assumed);
        let registry = RuleRegistry::from_rules(rules::all());
        Ok(analyze(&lossy.program, &request, &registry))
    }
}

/// Loads and runs one fixture directory.
pub fn run_fixture(dir: &Path) -> Result<Report, FixtureError> {
    Fixture::load(dir)?.run()
}

/// The exact golden bytes: canonical pretty JSON plus one trailing newline.
#[must_use]
pub fn golden_text(report: &Report) -> String {
    format!("{}\n", report.to_json_pretty())
}

/// Loads a fixture error from filesystem and configuration failures.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// A fixture file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// File that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The fixture's configuration is invalid.
    #[error(transparent)]
    Config(Box<ConfigError>),
}

impl From<ConfigError> for FixtureError {
    fn from(error: ConfigError) -> Self {
        Self::Config(Box::new(error))
    }
}

fn read_text(path: &Path) -> Result<String, FixtureError> {
    fs::read_to_string(path).map_err(|source| FixtureError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// First difference between two canonical reports.
///
/// Equality is defined on the canonical pretty JSON bytes. When the documents
/// differ, `pointer` names the first differing value using RFC 6901 escaping,
/// and the other fields carry short surrounding values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureDiff {
    /// RFC 6901 JSON pointer; the root document is the empty string.
    pub pointer: String,
    /// Expected value, compact JSON, truncated for display.
    pub expected: Option<String>,
    /// Actual value, compact JSON, truncated for display.
    pub actual: Option<String>,
    /// Parent container or immediate surrounding object, truncated.
    pub context: Option<String>,
}

impl FixtureDiff {
    /// Pointer as shown to a person. The root pointer is displayed as `/`.
    #[must_use]
    pub fn display_pointer(&self) -> &str {
        if self.pointer.is_empty() {
            "/"
        } else {
            &self.pointer
        }
    }
}

impl fmt::Display for FixtureDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "first difference at {}", self.display_pointer())?;
        if let Some(expected) = &self.expected {
            write!(f, "\n  expected: {expected}")?;
        }
        if let Some(actual) = &self.actual {
            write!(f, "\n  actual:   {actual}")?;
        }
        if let Some(context) = &self.context {
            write!(f, "\n  context:  {context}")?;
        }
        Ok(())
    }
}

/// Compares two reports by canonical pretty JSON, returning the first
/// structural difference rather than a whole-file diff.
pub fn compare_reports(expected: &Report, actual: &Report) -> Result<(), FixtureDiff> {
    let expected_text = expected.to_json_pretty();
    let actual_text = actual.to_json_pretty();
    if expected_text == actual_text {
        return Ok(());
    }

    let expected_value: Value = match serde_json::from_str(&expected_text) {
        Ok(value) => value,
        Err(error) => {
            return Err(FixtureDiff {
                pointer: String::new(),
                expected: Some(truncate(&expected_text)),
                actual: None,
                context: Some(format!("expected report did not parse as JSON: {error}")),
            });
        }
    };
    let actual_value: Value = match serde_json::from_str(&actual_text) {
        Ok(value) => value,
        Err(error) => {
            return Err(FixtureDiff {
                pointer: String::new(),
                expected: None,
                actual: Some(truncate(&actual_text)),
                context: Some(format!("actual report did not parse as JSON: {error}")),
            });
        }
    };

    if let Some(difference) = first_value_difference(&expected_value, &actual_value, "", None) {
        return Err(difference);
    }

    // Values are equal but the canonical renderers disagree: this is a
    // formatting/rounding regression, not a data difference.
    Err(FixtureDiff {
        pointer: String::new(),
        expected: Some(truncate(&expected_text)),
        actual: Some(truncate(&actual_text)),
        context: Some(
            "documents hold the same values but their canonical bytes differ".to_string(),
        ),
    })
}

fn first_value_difference(
    expected: &Value,
    actual: &Value,
    path: &str,
    context: Option<&Value>,
) -> Option<FixtureDiff> {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            for (key, expected_value) in expected_map {
                let child = join_pointer(path, key);
                match actual_map.get(key) {
                    Some(actual_value) => {
                        if let Some(difference) = first_value_difference(
                            expected_value,
                            actual_value,
                            &child,
                            Some(expected),
                        ) {
                            return Some(difference);
                        }
                    }
                    None => {
                        return Some(difference(
                            child,
                            Some(expected_value),
                            None,
                            Some(expected),
                        ));
                    }
                }
            }
            for (key, actual_value) in actual_map {
                if !expected_map.contains_key(key) {
                    return Some(difference(
                        join_pointer(path, key),
                        None,
                        Some(actual_value),
                        Some(actual),
                    ));
                }
            }
            None
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            let common = expected_items.len().min(actual_items.len());
            for index in 0..common {
                let child = join_pointer(path, &index.to_string());
                if let Some(difference) = first_value_difference(
                    &expected_items[index],
                    &actual_items[index],
                    &child,
                    Some(expected),
                ) {
                    return Some(difference);
                }
            }
            if expected_items.len() != actual_items.len() {
                let index = common;
                return Some(difference(
                    join_pointer(path, &index.to_string()),
                    expected_items.get(index),
                    actual_items.get(index),
                    Some(expected),
                ));
            }
            None
        }
        _ => {
            if expected == actual {
                None
            } else {
                Some(difference(
                    path.to_string(),
                    Some(expected),
                    Some(actual),
                    context,
                ))
            }
        }
    }
}

fn difference(
    pointer: String,
    expected: Option<&Value>,
    actual: Option<&Value>,
    context: Option<&Value>,
) -> FixtureDiff {
    FixtureDiff {
        pointer,
        expected: expected.map(compact),
        actual: actual.map(compact),
        context: context.map(compact),
    }
}

fn compact(value: &Value) -> String {
    truncate(&serde_json::to_string(value).unwrap_or_else(|_| "<unrenderable>".to_string()))
}

fn truncate(text: &str) -> String {
    const LIMIT: usize = 400;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    let mut short: String = text.chars().take(LIMIT).collect();
    short.push_str("...");
    short
}

/// Joins an RFC 6901 pointer with one path segment.
fn join_pointer(path: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{path}/{escaped}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nca-fixtures-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn load_requires_notes_md() {
        let dir = scratch_dir("notes");
        std::fs::create_dir_all(&dir).expect("create fixture directory");
        std::fs::write(dir.join("source.nc"), "G21\nM30\n").expect("write source");
        std::fs::write(dir.join("config.toml"), "").expect("write config");

        let error = Fixture::load(&dir).expect_err("NOTES.md must be required");
        assert!(matches!(error, FixtureError::Read { .. }), "{error}");
        assert!(error.to_string().contains("NOTES.md"), "{error}");

        std::fs::write(dir.join("NOTES.md"), "# fixture\n").expect("write notes");
        let fixture = Fixture::load(&dir).expect("fixture with notes loads");
        assert!(fixture.expected_text.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn synthetic_manifest_lists_each_fixture_once() {
        let names: std::collections::BTreeSet<&str> = SYNTHETIC_FIXTURES.into_iter().collect();
        assert_eq!(names.len(), SYNTHETIC_FIXTURES.len());
    }
}
