//! Deterministic preflight analysis.
//!
//! The engine builds one [`AnalysisModel`] per run, checks every rule's
//! [`Requirements`] before evaluation, and reports rules that could not run as
//! [`SkippedRule`]s. A skipped rule is never silent: coverage names the missing
//! input and why it matters.
//!
//! ```
//! use nc_analysis::{AnalysisConfig, AnalysisRequest, RuleRegistry, analyze};
//! use nc_gcode::{Dialect, parse_program};
//!
//! let source = "G21 G90\n";
//! let program = parse_program(source).expect("valid program");
//! let config = AnalysisConfig::default();
//! let request = AnalysisRequest::new("part.nc", source, &config, Dialect::Generic);
//! let report = analyze(&program, &request, &RuleRegistry::new());
//!
//! assert_eq!(report.coverage.total, 0);
//! assert_eq!(report.input.file_name, "part.nc");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
pub mod config_file;
mod engine;
mod json;
mod report;
mod rule;
mod types;

pub mod rules;

pub use config::{AnalysisConfig, RuleConfig, RuleSettings, StockConfig, Tolerances};
pub use config_file::{ConfigError, ConfigLocation, LoadedConfig, load_config, parse_config};
pub use engine::{AnalysisRequest, RuleRegistry, analyze};
pub use report::{
    Coverage, Diagnostic, InputInfo, InventoryField, MAX_FINDINGS_PER_RULE, MAX_FINDINGS_TOTAL,
    Report, SkippedRule, ToolInventoryEntry, normalize_diagnostics, sort_diagnostics, sort_skips,
};
pub use rule::{
    AnalysisModel, EngagementData, PassEngagement, Requirements, Rule, RuleOutcome,
    StockRequirement,
};
pub use rules::all;
pub use types::{Basis, EvidenceValue, RuleId, Severity};

/// Report schema version.
pub const REPORT_SCHEMA: &str = "nca.report/1";

/// Ruleset version. Changing a rule's behavior requires bumping this.
pub const RULESET_VERSION: &str = "1.0.0";
