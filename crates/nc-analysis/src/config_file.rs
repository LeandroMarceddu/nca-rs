//! Strict TOML configuration loading shared by `nc-cli` and `nc-fixtures`.
//!
//! The schema is the one documented in `docs/INTERFACES.md`. Unknown rule ids,
//! unknown keys, and unknown enum values are hard errors (D015): a typo must
//! never silently change a rule's behavior. Every semantic error carries the
//! configuration file name, and TOML syntax/type errors also carry the line and
//! column when the parser reports a span.
//!
//! ```no_run
//! use nc_analysis::config_file::load_config;
//!
//! let loaded = load_config(std::path::Path::new("config.toml"))?;
//! println!("dialect: {} (assumed: {})", loaded.dialect, loaded.dialect_assumed);
//! # Ok::<(), nc_analysis::ConfigError>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nc_core::{
    Dialect, FieldSource, MachineConfig, MaterialPreset, Tool, ToolField, ToolShape, ToolTable,
};
use serde::Deserialize;

use crate::config::{AnalysisConfig, RuleSettings, StockConfig};
use crate::types::{RuleId, Severity};

/// Where a configuration error was found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigLocation {
    /// Configuration file, when the loader read one.
    pub file: Option<PathBuf>,
    /// 1-based line, when the TOML parser reported a span.
    pub line: Option<usize>,
    /// 1-based column, when the TOML parser reported a span.
    pub column: Option<usize>,
}

impl fmt::Display for ConfigLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.file, self.line, self.column) {
            (Some(file), Some(line), Some(column)) => {
                write!(f, "{}:{line}:{column}", file.display())
            }
            (Some(file), Some(line), None) => write!(f, "{}:{line}", file.display()),
            (Some(file), None, _) => write!(f, "{}", file.display()),
            (None, Some(line), Some(column)) => write!(f, "line {line}, column {column}"),
            (None, Some(line), None) => write!(f, "line {line}"),
            (None, None, _) => f.write_str("<config>"),
        }
    }
}

/// A strict configuration-loading failure.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("cannot read configuration {path}: {source}")]
    Read {
        /// File that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The text is not valid TOML or does not match the schema.
    #[error("{location}: {message}")]
    Toml {
        /// File and span, when known.
        location: ConfigLocation,
        /// TOML parser message.
        message: String,
    },
    /// `[parse] dialect` names an unsupported dialect.
    #[error(
        "{location}: unknown dialect {value:?}; expected generic, grbl, linuxcnc, mach3 or fanuc"
    )]
    UnknownDialect {
        /// File and span, when known.
        location: ConfigLocation,
        /// Offending value.
        value: String,
    },
    /// A tool `shape` is not a known shape name.
    #[error("{location}: unknown tool shape {value:?}")]
    UnknownToolShape {
        /// File and span, when known.
        location: ConfigLocation,
        /// Offending value.
        value: String,
    },
    /// `[material] preset` names no built-in preset.
    #[error("{location}: unknown material preset {value:?}; expected one of {expected}")]
    UnknownMaterialPreset {
        /// File and span, when known.
        location: ConfigLocation,
        /// Offending value.
        value: String,
        /// Comma-separated built-in names.
        expected: String,
    },
    /// A rule severity is not `error`, `warning`, or `info`.
    #[error(
        "{location}: unknown severity {value:?} for rule {rule:?}; expected error, warning or info"
    )]
    UnknownSeverity {
        /// File and span, when known.
        location: ConfigLocation,
        /// Rule whose severity was set.
        rule: String,
        /// Offending value.
        value: String,
    },
    /// A rule table contains a key that is neither `enabled`, `severity`, nor
    /// a numeric threshold.
    #[error(
        "{location}: rule {rule:?} has unknown key {key:?}; expected enabled, severity or a numeric threshold"
    )]
    UnknownRuleKey {
        /// File and span, when known.
        location: ConfigLocation,
        /// Rule id.
        rule: String,
        /// Offending key.
        key: String,
    },
    /// A `[rules."<id>"]` table names no registered rule.
    #[error("{location}: unknown rule id {rule:?}; expected a registered rule id")]
    UnknownRule {
        /// File and span, when known.
        location: ConfigLocation,
        /// Offending rule id.
        rule: String,
    },
    /// A rule threshold is not numeric.
    #[error("{location}: rule {rule:?} threshold {key:?} must be a number, got {kind}")]
    InvalidThreshold {
        /// File and span, when known.
        location: ConfigLocation,
        /// Rule id.
        rule: String,
        /// Threshold key.
        key: String,
        /// TOML value kind.
        kind: &'static str,
    },
    /// `enabled` or `severity` has the wrong TOML type.
    #[error("{location}: rule {rule:?} key {key:?} must be {expected}")]
    InvalidRuleValue {
        /// File and span, when known.
        location: ConfigLocation,
        /// Rule id.
        rule: String,
        /// Offending key.
        key: String,
        /// Expected TOML type.
        expected: &'static str,
    },
    /// The machine travel envelope is invalid.
    #[error("{location}: invalid machine configuration: {message}")]
    InvalidMachine {
        /// File and span, when known.
        location: ConfigLocation,
        /// Core validation message.
        message: String,
    },
    /// The explicit stock extent is invalid.
    #[error("{location}: invalid stock configuration: {message}")]
    InvalidStock {
        /// File and span, when known.
        location: ConfigLocation,
        /// Stock validation message.
        message: String,
    },
    /// The same tool number appears twice.
    #[error("{location}: duplicate tool number {number}")]
    DuplicateTool {
        /// File and span, when known.
        location: ConfigLocation,
        /// Repeated tool number.
        number: u32,
    },
}

impl ConfigError {
    /// Location attached to this error, when there is one.
    #[must_use]
    pub fn location(&self) -> Option<&ConfigLocation> {
        match self {
            Self::Read { .. } => None,
            Self::Toml { location, .. }
            | Self::UnknownDialect { location, .. }
            | Self::UnknownToolShape { location, .. }
            | Self::UnknownMaterialPreset { location, .. }
            | Self::UnknownSeverity { location, .. }
            | Self::UnknownRuleKey { location, .. }
            | Self::UnknownRule { location, .. }
            | Self::InvalidThreshold { location, .. }
            | Self::InvalidRuleValue { location, .. }
            | Self::InvalidMachine { location, .. }
            | Self::InvalidStock { location, .. }
            | Self::DuplicateTool { location, .. } => Some(location),
        }
    }

    /// Sets the file name on a location-bearing error.
    fn with_file(mut self, path: &Path) -> Self {
        if let Self::Toml { location, .. }
        | Self::UnknownDialect { location, .. }
        | Self::UnknownToolShape { location, .. }
        | Self::UnknownMaterialPreset { location, .. }
        | Self::UnknownSeverity { location, .. }
        | Self::UnknownRuleKey { location, .. }
        | Self::UnknownRule { location, .. }
        | Self::InvalidThreshold { location, .. }
        | Self::InvalidRuleValue { location, .. }
        | Self::InvalidMachine { location, .. }
        | Self::InvalidStock { location, .. }
        | Self::DuplicateTool { location, .. } = &mut self
            && location.file.is_none()
        {
            location.file = Some(path.to_path_buf());
        }
        self
    }
}

/// A configuration after resolution: analysis inputs plus parser selection.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    /// Everything the analysis engine reads.
    pub analysis: AnalysisConfig,
    /// Effective parser dialect.
    pub dialect: Dialect,
    /// `true` when `[parse] dialect` was absent and Generic was assumed.
    pub dialect_assumed: bool,
}

impl Default for LoadedConfig {
    fn default() -> Self {
        Self {
            analysis: AnalysisConfig::default(),
            dialect: Dialect::Generic,
            dialect_assumed: true,
        }
    }
}

/// Parses configuration text with the strict schema.
pub fn parse_config(text: &str) -> Result<LoadedConfig, ConfigError> {
    let raw: RawConfig = toml::from_str(text).map_err(|error| toml_error(text, error))?;
    let mut loaded = LoadedConfig::default();

    if let Some(parse) = raw.parse
        && let Some(dialect) = parse.dialect
    {
        loaded.dialect = dialect
            .parse::<Dialect>()
            .map_err(|_| ConfigError::UnknownDialect {
                location: ConfigLocation::default(),
                value: dialect,
            })?;
        loaded.dialect_assumed = false;
    }

    if let Some(machine) = raw.machine {
        let machine = MachineConfig {
            travel_min_mm: machine.travel_min_mm,
            travel_max_mm: machine.travel_max_mm,
            max_feed_mm_min: machine.max_feed_mm_min,
            max_spindle_rpm: machine.max_spindle_rpm,
            rapid_rate_mm_min: machine.rapid_rate_mm_min,
            spindle_manual: machine.spindle_manual,
        };
        machine
            .validate()
            .map_err(|error| ConfigError::InvalidMachine {
                location: ConfigLocation::default(),
                message: error.to_string(),
            })?;
        loaded.analysis.machine = Some(machine);
    }

    if let Some(stock) = raw.stock {
        let stock = StockConfig {
            origin_mm: stock.origin_mm,
            size_mm: stock.size_mm,
        };
        stock
            .to_stock()
            .map_err(|error| ConfigError::InvalidStock {
                location: ConfigLocation::default(),
                message: error.to_string(),
            })?;
        loaded.analysis.stock = Some(stock);
    }

    let mut tools = ToolTable::new();
    for raw_tool in raw.tool {
        let number = raw_tool.number;
        if tools.get(number).is_some() {
            return Err(ConfigError::DuplicateTool {
                location: ConfigLocation::default(),
                number,
            });
        }
        tools.insert(convert_tool(raw_tool)?);
    }
    loaded.analysis.tools = tools;

    if let Some(raw_material) = raw.material {
        loaded.analysis.material = Some(convert_material(raw_material)?);
    }

    let mut rules = BTreeMap::new();
    // D015: a rule table id must name a registered rule. A typo here would
    // otherwise create settings that no rule ever reads, silently leaving the
    // real rule at its defaults.
    let registered_rules: BTreeSet<String> = crate::rules::all()
        .into_iter()
        .map(|rule| rule.id().as_str().to_string())
        .collect();
    for (rule, entries) in raw.rules {
        if !registered_rules.contains(rule.as_str()) {
            return Err(ConfigError::UnknownRule {
                location: ConfigLocation::default(),
                rule,
            });
        }
        let mut settings = RuleSettings::default();
        for (key, value) in entries {
            match key.as_str() {
                "enabled" => {
                    settings.enabled =
                        value
                            .as_bool()
                            .ok_or_else(|| ConfigError::InvalidRuleValue {
                                location: ConfigLocation::default(),
                                rule: rule.clone(),
                                key: key.clone(),
                                expected: "a boolean",
                            })?;
                }
                "severity" => {
                    let text = value
                        .as_str()
                        .ok_or_else(|| ConfigError::InvalidRuleValue {
                            location: ConfigLocation::default(),
                            rule: rule.clone(),
                            key: key.clone(),
                            expected: "a string (error, warning or info)",
                        })?;
                    settings.severity = Some(parse_severity(&rule, text)?);
                }
                _ => {
                    // Threshold keys stay free-form (D032): `Rule` does not
                    // expose its documented default keys, so an unrecognized
                    // numeric key is stored rather than rejected.
                    let number = value
                        .as_float()
                        .or_else(|| value.as_integer().map(|integer| integer as f64))
                        .ok_or_else(|| ConfigError::InvalidThreshold {
                            location: ConfigLocation::default(),
                            rule: rule.clone(),
                            key: key.clone(),
                            kind: value_kind(&value),
                        })?;
                    settings.thresholds.insert(key, number);
                }
            }
        }
        rules.insert(RuleId::owned(rule), settings);
    }
    loaded.analysis.rules.0 = rules;

    Ok(loaded)
}

/// Reads and parses a configuration file.
pub fn load_config(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_config(&text).map_err(|error| error.with_file(path))
}

/// Raw TOML shape. `deny_unknown_fields` makes every typo a hard error.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    parse: Option<RawParse>,
    #[serde(default)]
    machine: Option<RawMachine>,
    #[serde(default)]
    stock: Option<RawStock>,
    #[serde(default, rename = "tool")]
    tool: Vec<RawTool>,
    #[serde(default)]
    material: Option<RawMaterial>,
    #[serde(default)]
    rules: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParse {
    #[serde(default)]
    dialect: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMachine {
    travel_min_mm: [f64; 3],
    travel_max_mm: [f64; 3],
    #[serde(default)]
    max_feed_mm_min: Option<f64>,
    #[serde(default)]
    max_spindle_rpm: Option<f64>,
    #[serde(default)]
    rapid_rate_mm_min: Option<f64>,
    #[serde(default)]
    spindle_manual: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStock {
    origin_mm: [f64; 3],
    size_mm: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTool {
    number: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    shape: Option<String>,
    #[serde(default)]
    diameter_mm: Option<f64>,
    #[serde(default)]
    flutes: Option<u32>,
    #[serde(default)]
    stickout_mm: Option<f64>,
    #[serde(default)]
    flute_length_mm: Option<f64>,
    #[serde(default)]
    tip_angle_deg: Option<f64>,
    #[serde(default)]
    corner_radius_mm: Option<f64>,
    #[serde(default)]
    shank_diameter_mm: Option<f64>,
    #[serde(default)]
    holder_diameter_mm: Option<f64>,
    #[serde(default)]
    right_hand: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMaterial {
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    chipload_min_mm: Option<f64>,
    #[serde(default)]
    chipload_max_mm: Option<f64>,
    #[serde(default)]
    surface_speed_min_mpm: Option<f64>,
    #[serde(default)]
    surface_speed_max_mpm: Option<f64>,
    #[serde(default)]
    max_mrr_mm3_min: Option<f64>,
    #[serde(default)]
    requires_coolant: Option<bool>,
    #[serde(default)]
    notes: Option<String>,
}

/// Converts one raw tool, marking every configured field as `FieldSource::Config`.
fn convert_tool(raw: RawTool) -> Result<Tool, ConfigError> {
    let mut tool = Tool::new(raw.number);
    if let Some(name) = raw.name {
        tool.name = Some(name);
        tool.sources.insert(ToolField::Name, FieldSource::Config);
    }
    if let Some(shape) = raw.shape {
        let shape = ToolShape::parse(&shape).ok_or_else(|| ConfigError::UnknownToolShape {
            location: ConfigLocation::default(),
            value: shape,
        })?;
        tool.shape = shape;
        if shape != ToolShape::Unknown {
            tool.sources.insert(ToolField::Shape, FieldSource::Config);
        }
    }
    if let Some(value) = raw.diameter_mm {
        tool.diameter_mm = Some(value);
        tool.sources
            .insert(ToolField::Diameter, FieldSource::Config);
    }
    if let Some(value) = raw.flutes {
        tool.flutes = Some(value);
        tool.sources.insert(ToolField::Flutes, FieldSource::Config);
    }
    if let Some(value) = raw.stickout_mm {
        tool.stickout_mm = Some(value);
        tool.sources
            .insert(ToolField::Stickout, FieldSource::Config);
    }
    if let Some(value) = raw.flute_length_mm {
        tool.flute_length_mm = Some(value);
        tool.sources
            .insert(ToolField::FluteLength, FieldSource::Config);
    }
    if let Some(value) = raw.tip_angle_deg {
        tool.tip_angle_deg = Some(value);
        tool.sources
            .insert(ToolField::TipAngle, FieldSource::Config);
    }
    if let Some(value) = raw.corner_radius_mm {
        tool.corner_radius_mm = Some(value);
        tool.sources
            .insert(ToolField::CornerRadius, FieldSource::Config);
    }
    if let Some(value) = raw.shank_diameter_mm {
        tool.shank_diameter_mm = Some(value);
        tool.sources
            .insert(ToolField::ShankDiameter, FieldSource::Config);
    }
    if let Some(value) = raw.holder_diameter_mm {
        tool.holder_diameter_mm = Some(value);
        tool.sources
            .insert(ToolField::HolderDiameter, FieldSource::Config);
    }
    if let Some(value) = raw.right_hand {
        tool.right_hand = Some(value);
        tool.sources
            .insert(ToolField::RightHand, FieldSource::Config);
    }
    Ok(tool)
}

/// Resolves a material preset and applies inline overrides on top.
fn convert_material(raw: RawMaterial) -> Result<MaterialPreset, ConfigError> {
    let mut preset = if let Some(name) = &raw.preset {
        MaterialPreset::builtin(name).ok_or_else(|| ConfigError::UnknownMaterialPreset {
            location: ConfigLocation::default(),
            value: name.clone(),
            expected: MaterialPreset::builtin_names().join(", "),
        })?
    } else {
        MaterialPreset::new(raw.name.clone().unwrap_or_else(|| "custom".to_string()))
    };

    if let Some(name) = raw.name {
        preset.name = name;
    }
    if let Some(value) = raw.chipload_min_mm {
        preset.chipload_min_mm = Some(value);
    }
    if let Some(value) = raw.chipload_max_mm {
        preset.chipload_max_mm = Some(value);
    }
    if let Some(value) = raw.surface_speed_min_mpm {
        preset.surface_speed_min_mpm = Some(value);
    }
    if let Some(value) = raw.surface_speed_max_mpm {
        preset.surface_speed_max_mpm = Some(value);
    }
    if let Some(value) = raw.max_mrr_mm3_min {
        preset.max_mrr_mm3_min = Some(value);
    }
    if let Some(value) = raw.requires_coolant {
        preset.requires_coolant = value;
    }
    if let Some(notes) = raw.notes {
        preset.notes = notes;
    }
    Ok(preset)
}

/// Parses a rule severity override.
fn parse_severity(rule: &str, text: &str) -> Result<Severity, ConfigError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "error" => Ok(Severity::Error),
        "warning" => Ok(Severity::Warning),
        "info" => Ok(Severity::Info),
        _ => Err(ConfigError::UnknownSeverity {
            location: ConfigLocation::default(),
            rule: rule.to_string(),
            value: text.to_string(),
        }),
    }
}

/// Builds a TOML parse error with a line/column span when available.
fn toml_error(text: &str, error: toml::de::Error) -> ConfigError {
    let (line, column) = error
        .span()
        .map(|span| line_column(text, span.start))
        .map_or((None, None), |(line, column)| (Some(line), Some(column)));
    ConfigError::Toml {
        location: ConfigLocation {
            file: None,
            line,
            column,
        },
        message: error.message().to_string(),
    }
}

/// Converts a byte offset into a 1-based line and column.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, character) in text.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// Human name for a TOML value type.
fn value_kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "an integer",
        toml::Value::Float(_) => "a float",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a datetime",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[parse]
dialect = "grbl"

[machine]
travel_min_mm = [0, 0, -120]
travel_max_mm = [400, 300, 0]
max_feed_mm_min = 3000
max_spindle_rpm = 24000
rapid_rate_mm_min = 3000
spindle_manual = false

[stock]
origin_mm = [0, 0, -20]
size_mm = [100, 80, 20]

[[tool]]
number = 1
name = "6 mm 2F flat"
shape = "flat"
diameter_mm = 6.0
flutes = 2
stickout_mm = 30
flute_length_mm = 18
right_hand = true

[[tool]]
number = 4
shape = "ball"

[material]
preset = "aluminum-6061"
chipload_max_mm = 0.08

[rules."clearance.rapid_at_depth"]
enabled = true
severity = "warning"
clearance_mm = 2.5
"#;

    #[test]
    fn full_config_is_loaded() {
        let loaded = parse_config(FULL).expect("full config");
        assert_eq!(loaded.dialect, Dialect::Grbl);
        assert!(!loaded.dialect_assumed);

        let machine = loaded.analysis.machine.expect("machine");
        assert_eq!(machine.travel_max_mm, [400.0, 300.0, 0.0]);
        assert_eq!(machine.max_feed_mm_min, Some(3000.0));

        let stock = loaded.analysis.stock.expect("stock");
        assert_eq!(stock.origin_mm, [0.0, 0.0, -20.0]);
        assert_eq!(stock.size_mm, [100.0, 80.0, 20.0]);

        assert_eq!(loaded.analysis.tools.len(), 2);
        let tool = loaded.analysis.tools.get(1).expect("tool 1");
        assert_eq!(tool.diameter_mm, Some(6.0));
        assert_eq!(tool.flutes, Some(2));
        assert_eq!(tool.right_hand, Some(true));

        let material = loaded.analysis.material.expect("material");
        assert_eq!(material.name, "aluminum-6061");
        assert_eq!(material.chipload_min_mm, Some(0.02));
        assert_eq!(material.chipload_max_mm, Some(0.08));

        let rule = RuleId::new("clearance.rapid_at_depth");
        let settings = loaded.analysis.rules.settings(&rule);
        assert!(settings.enabled);
        assert_eq!(settings.severity, Some(Severity::Warning));
        assert_eq!(settings.threshold("clearance_mm", 2.0), 2.5);
    }

    #[test]
    fn minimal_config_assumes_generic_dialect() {
        let loaded = parse_config("").expect("empty config");
        assert_eq!(loaded.dialect, Dialect::Generic);
        assert!(loaded.dialect_assumed);
        assert_eq!(loaded.analysis, AnalysisConfig::default());
    }

    #[test]
    fn explicit_dialect_is_not_assumed() {
        let loaded = parse_config("[parse]\ndialect = \"linux_cnc\"\n").expect("config");
        assert_eq!(loaded.dialect, Dialect::LinuxCnc);
        assert!(!loaded.dialect_assumed);
    }

    #[test]
    fn unknown_key_is_an_error_with_a_line() {
        let error = parse_config("[parse]\nbogus = 1\n").expect_err("unknown key");
        let location = error.location().expect("location");
        assert!(location.line.is_some(), "{error}");
        assert!(location.column.is_some(), "{error}");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn unknown_dialect_is_an_error() {
        let error = parse_config("[parse]\ndialect = \"siemens\"\n").expect_err("dialect");
        assert!(matches!(error, ConfigError::UnknownDialect { .. }));
    }

    #[test]
    fn unknown_material_preset_is_an_error() {
        let error = parse_config("[material]\npreset = \"unobtainium\"\n").expect_err("material");
        assert!(matches!(error, ConfigError::UnknownMaterialPreset { .. }));
    }

    #[test]
    fn unknown_material_key_is_an_error() {
        let error =
            parse_config("[material]\npreset = \"brass\"\nbogus = 1\n").expect_err("material");
        assert!(matches!(error, ConfigError::Toml { .. }));
    }

    #[test]
    fn inline_material_fields_extend_and_override_a_preset() {
        let loaded = parse_config(
            "[material]\npreset = \"brass\"\nchipload_max_mm = 0.09\nnotes = \"shop note\"\n",
        )
        .expect("material");
        let material = loaded.analysis.material.expect("material");
        assert_eq!(material.name, "brass");
        assert_eq!(material.chipload_min_mm, Some(0.02));
        assert_eq!(material.chipload_max_mm, Some(0.09));
        assert_eq!(material.surface_speed_max_mpm, Some(200.0));
        assert_eq!(material.notes, "shop note");
    }

    #[test]
    fn tool_fields_carry_config_provenance() {
        let loaded = parse_config(FULL).expect("config");
        let tool = loaded.analysis.tools.get(1).expect("tool");
        for field in [
            ToolField::Name,
            ToolField::Shape,
            ToolField::Diameter,
            ToolField::Flutes,
            ToolField::Stickout,
            ToolField::FluteLength,
            ToolField::RightHand,
        ] {
            assert_eq!(
                tool.sources.get(&field),
                Some(&FieldSource::Config),
                "{field}"
            );
        }
        assert!(!tool.sources.contains_key(&ToolField::TipAngle));
        assert!(!tool.sources.contains_key(&ToolField::CornerRadius));
    }

    #[test]
    fn unknown_tool_shape_is_an_error() {
        let error =
            parse_config("[[tool]]\nnumber = 1\nshape = \"teardrop\"\n").expect_err("shape");
        assert!(matches!(error, ConfigError::UnknownToolShape { .. }));
    }

    #[test]
    fn duplicate_tool_is_an_error() {
        let error =
            parse_config("[[tool]]\nnumber = 1\n[[tool]]\nnumber = 1\n").expect_err("duplicate");
        assert!(matches!(
            error,
            ConfigError::DuplicateTool { number: 1, .. }
        ));
    }

    #[test]
    fn rule_thresholds_flatten_and_enabled_defaults_true() {
        let loaded = parse_config(
            "[rules.\"process.plunge_feed\"]\nplunge_feed_ratio = 0.4\nfeed_margin = 12\n",
        )
        .expect("rules");
        let rule = RuleId::new("process.plunge_feed");
        let settings = loaded.analysis.rules.settings(&rule);
        assert!(settings.enabled);
        assert_eq!(settings.severity, None);
        assert_eq!(settings.threshold("plunge_feed_ratio", 0.0), 0.4);
        assert_eq!(settings.threshold("feed_margin", 0.0), 12.0);
    }

    #[test]
    fn unknown_rule_id_is_an_error() {
        let error = parse_config("[rules.\"modal.feed_mising\"]\nclearance_mm = 2.0\n")
            .expect_err("unknown rule id");
        assert!(matches!(
            error,
            ConfigError::UnknownRule { ref rule, .. } if rule == "modal.feed_mising"
        ));
        assert!(error.to_string().contains("unknown rule id"), "{error}");
        assert!(
            error.to_string().contains("modal.feed_mising"),
            "the error names the typo: {error}"
        );
    }

    #[test]
    fn non_numeric_rule_key_is_an_error() {
        let error =
            parse_config("[rules.\"modal.feed_missing\"]\nbogus = \"x\"\n").expect_err("rule key");
        assert!(matches!(error, ConfigError::InvalidThreshold { .. }));
        assert!(error.to_string().contains("must be a number"));
    }

    #[test]
    fn unknown_severity_is_an_error() {
        let error = parse_config("[rules.\"modal.feed_missing\"]\nseverity = \"fatal\"\n")
            .expect_err("severity");
        assert!(matches!(error, ConfigError::UnknownSeverity { .. }));
    }

    #[test]
    fn invalid_machine_travel_is_an_error() {
        let error =
            parse_config("[machine]\ntravel_min_mm = [0, 0, 0]\ntravel_max_mm = [-1, 10, 10]\n")
                .expect_err("machine");
        assert!(matches!(error, ConfigError::InvalidMachine { .. }));
    }

    #[test]
    fn invalid_stock_extent_is_an_error() {
        let error = parse_config("[stock]\norigin_mm = [0, 0, 0]\nsize_mm = [0, 10, 10]\n")
            .expect_err("stock");
        assert!(matches!(error, ConfigError::InvalidStock { .. }));
    }

    #[test]
    fn load_config_attaches_the_file_name() {
        let path = std::env::temp_dir().join(format!(
            "nca-config-test-{}-{}.toml",
            std::process::id(),
            line_column("", 0).0
        ));
        std::fs::write(&path, "[parse]\ndialect = \"nope\"\n").expect("write");
        let error = load_config(&path).expect_err("dialect");
        std::fs::remove_file(&path).ok();
        let location = error.location().expect("location");
        assert_eq!(location.file.as_deref(), Some(path.as_path()));
    }
}
