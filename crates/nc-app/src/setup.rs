//! Setup-sheet model: configuration, stock, tools, material, machine, rules.
//!
//! Everything in this module is pure or local-file I/O. It never touches a
//! window, a controller, or G-code: the shell reads it, renders editable
//! fields, and asks it for an [`AnalysisConfig`], a simulation config, and an
//! exported report body.
//!
//! Tool inventory is keyed by the source file's SHA-256, never by its path, so
//! moving a file keeps its local tool overlay and two files with identical
//! bytes share one. The report body never embeds an absolute path.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use nc_analysis::config_file::LoadedConfig;
use nc_analysis::{AnalysisConfig, Report, RuleId, RuleSettings, Severity, StockConfig};
use nc_core::{
    Dialect, FieldSource, MachineConfig, MaterialPreset, Tool, ToolField, ToolShape, ToolTable,
};
use nc_gcode::Program;
use nc_sim::SimConfig;
use nc_stock::{InferenceBasis, StockModel};
use sha2::{Digest, Sha256};

use crate::state::{LoadedProgram, NcaConfig};

/// Default export file name in the current directory.
pub const DEFAULT_EXPORT_JSON: &str = "nca-report.json";
/// Default Markdown export file name in the current directory.
pub const DEFAULT_EXPORT_MARKDOWN: &str = "nca-report.md";

/// Advisory line shown in every Markdown export and the export sheet.
pub const ADVISORY_NOTE: &str = "Advisory only. This report is not a safety verdict and does not \
     certify the program. A program with no findings may still be unsafe.";

/// Severity or information level of a setup message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// A successful action or neutral information.
    Info,
    /// An action succeeded but needs review.
    Warning,
    /// The action failed; nothing was applied.
    Error,
}

/// One line of feedback shown in the setup or export sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMessage {
    /// Level of the message.
    pub kind: MessageKind,
    /// Plain-language text.
    pub text: String,
}

impl StatusMessage {
    /// Informational message.
    #[must_use]
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            kind: MessageKind::Info,
            text: text.into(),
        }
    }

    /// Warning message.
    #[must_use]
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            kind: MessageKind::Warning,
            text: text.into(),
        }
    }

    /// Error message.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            kind: MessageKind::Error,
            text: text.into(),
        }
    }

    /// Whether this message reports a failed action.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.kind, MessageKind::Error)
    }
}

/// Export file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportFormat {
    /// Canonical pretty JSON, exactly as the CLI writes it.
    #[default]
    Json,
    /// Human-readable Markdown summary.
    Markdown,
}

impl ExportFormat {
    /// Every format in selector order.
    pub const ALL: [Self; 2] = [Self::Json, Self::Markdown];

    /// Short uppercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Markdown => "MARKDOWN",
        }
    }

    /// Default file name for this format.
    #[must_use]
    pub const fn default_path(self) -> &'static str {
        match self {
            Self::Json => DEFAULT_EXPORT_JSON,
            Self::Markdown => DEFAULT_EXPORT_MARKDOWN,
        }
    }
}

/// A material range edited in the setup sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialField {
    /// Minimum chipload in mm per tooth.
    ChiploadMin,
    /// Maximum chipload in mm per tooth.
    ChiploadMax,
    /// Minimum surface speed in m/min.
    SurfaceSpeedMin,
    /// Maximum surface speed in m/min.
    SurfaceSpeedMax,
    /// Maximum material removal rate in mm3/min.
    MaxMrr,
}

/// A machine numeric field edited in the setup sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineField {
    /// Travel minimum for axis 0..=2.
    TravelMin(usize),
    /// Travel maximum for axis 0..=2.
    TravelMax(usize),
    /// Maximum programmed feed in mm/min.
    MaxFeed,
    /// Maximum spindle speed in rpm.
    MaxSpindle,
    /// Rapid rate in mm/min.
    Rapid,
}

/// A rule tolerance edited in the setup sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleranceField {
    /// Arc start/end radius mismatch, mm.
    ArcRadius,
    /// Tiny-segment threshold, mm.
    MinSegment,
    /// Hole centre distance, mm.
    HoleXy,
    /// Rapid clearance, mm.
    Clearance,
}

/// A simulation field edited in the setup sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimField {
    /// Heightmap cell size, mm.
    CellMm,
    /// Maximum heightmap cells.
    MaxCells,
    /// Timeline rapid rate, mm/min.
    RapidRate,
    /// Pause at a tool change, seconds.
    ToolChangePause,
    /// Assumed cutter diameter for tools without one, mm.
    AssumedDiameter,
}

/// One tool number used by the program, in order of first use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryTool {
    /// Tool number.
    pub number: u32,
    /// First source line that used the tool.
    pub first_line: usize,
    /// Number of stored motion records attributed to the tool.
    pub motion_count: usize,
    /// First comment-hint line that mentions the tool, when there is one.
    pub hint_line: Option<usize>,
}

/// Editable setup state and its last committed analysis inputs.
///
/// The shell owns input widgets, but every value that reaches analysis or the
/// simulator lives here. Mutations bump [`Self::revision`], which the shell uses
/// to re-run analysis and rebuild the simulation.
#[derive(Clone, Debug)]
pub struct SetupState {
    /// Stock, machine, material, tolerances, and rule overrides.
    ///
    /// Tool data is not stored here: it is the per-file table keyed by the
    /// source SHA-256 in [`NcaConfig`].
    pub analysis: AnalysisConfig,
    /// Selected parser and analysis dialect.
    pub dialect: Dialect,
    /// `true` while Generic is the default and no dialect was chosen.
    pub dialect_assumed: bool,
    /// Simulation settings.
    pub sim: SimConfig,
    /// File name of the last loaded TOML config, never a path.
    pub config_name: Option<String>,
    /// Draft path typed into the config field.
    pub config_path_text: String,
    /// Validation error for the explicit stock, when invalid.
    pub stock_error: Option<String>,
    /// Validation error for machine limits, when invalid.
    pub machine_error: Option<String>,
    /// Last action feedback.
    pub message: Option<StatusMessage>,
    /// Program-derived tool inventory.
    pub inventory: Vec<InventoryTool>,
    /// Tool number expanded in the setup sheet.
    pub selected_tool: Option<u32>,
    /// Rule id expanded for threshold editing.
    pub selected_rule: Option<String>,
    /// Export path draft. Relative by default; the success message resolves it.
    pub export_path: String,
    /// Export format.
    pub export_format: ExportFormat,
    /// Last export feedback.
    pub export_status: Option<StatusMessage>,
    /// Monotonic edit counter used by tests and the shell.
    pub revision: u64,
}

impl Default for SetupState {
    fn default() -> Self {
        Self {
            analysis: AnalysisConfig::default(),
            dialect: Dialect::Generic,
            dialect_assumed: true,
            sim: SimConfig::default(),
            config_name: None,
            config_path_text: String::new(),
            stock_error: None,
            machine_error: None,
            message: None,
            inventory: Vec::new(),
            selected_tool: None,
            selected_rule: None,
            export_path: DEFAULT_EXPORT_JSON.to_string(),
            export_format: ExportFormat::Json,
            export_status: None,
            revision: 0,
        }
    }
}

impl SetupState {
    /// Applies a loaded TOML configuration and returns its tool table.
    ///
    /// The caller stores the returned table under the current file's SHA-256;
    /// this model never keys anything by path.
    pub fn apply_loaded(&mut self, loaded: LoadedConfig) -> ToolTable {
        let mut analysis = loaded.analysis;
        let tools = std::mem::take(&mut analysis.tools);
        self.analysis = analysis;
        self.dialect = loaded.dialect;
        self.dialect_assumed = loaded.dialect_assumed;
        if let Some(rapid) = self
            .analysis
            .machine
            .as_ref()
            .and_then(|machine| machine.rapid_rate_mm_min)
        {
            self.sim.rapid_rate_mm_min = rapid;
        }
        self.selected_rule = None;
        self.refresh_validation();
        self.revision = self.revision.wrapping_add(1);
        tools
    }

    /// Rebuilds the program-derived tool inventory.
    pub fn sync_program(&mut self, loaded: &LoadedProgram) {
        self.inventory = loaded
            .program
            .as_ref()
            .map(|program| inventory_for(program))
            .unwrap_or_default();
        if let Some(selected) = self.selected_tool
            && !self.inventory.iter().any(|tool| tool.number == selected)
        {
            self.selected_tool = self.inventory.first().map(|tool| tool.number);
        }
        if self.selected_tool.is_none() {
            self.selected_tool = self.inventory.first().map(|tool| tool.number);
        }
    }

    /// Stock chip text. Never claims explicit when the extents are invalid.
    #[must_use]
    pub fn stock_label(&self) -> &'static str {
        match self.analysis.stock.as_ref() {
            Some(stock) if stock.to_stock().is_ok() => "STOCK: SET",
            Some(_) => "STOCK: INVALID",
            None => "STOCK: INFERRED",
        }
    }

    /// Plain-language stock source, including the inferred basis and top Z.
    #[must_use]
    pub fn stock_basis_text(&self, inferred: Option<&StockModel>) -> String {
        if let Some(stock) = self.analysis.stock.as_ref() {
            return match stock.to_stock() {
                Ok(model) => format!("explicit origin and size; top Z {:.2} mm", model.top_z_mm),
                Err(error) => {
                    if inferred.is_some() {
                        format!(
                            "explicit stock is invalid: {error}; analysis and simulation fall \
                             back to inferred stock"
                        )
                    } else {
                        format!("explicit stock is invalid: {error}")
                    }
                }
            };
        }
        let Some(stock) = inferred else {
            return "no parsed geometry, so there is no stock box".to_string();
        };
        let basis = match stock.inference {
            Some(InferenceBasis::ZeroAtTop) => {
                "fitted to cutting bounds; a cut below Z0 puts the stock top at Z0"
            }
            Some(InferenceBasis::AllCutsAboveZero) => {
                "all cuts are above Z0; the top sits 2 mm above the highest cut"
            }
            Some(InferenceBasis::NoCuttingMoves) => {
                "no cutting moves; the box uses the full path bounds"
            }
            None => "inferred from the toolpath",
        };
        format!("{basis}; top Z {:.2} mm", stock.top_z_mm)
    }

    /// The explicit stock model, when the configured extents are valid.
    #[must_use]
    pub fn explicit_stock(&self) -> Option<StockModel> {
        self.analysis
            .stock
            .as_ref()
            .and_then(|stock| stock.to_stock().ok())
    }

    /// The stock used by simulation and the chips: explicit when valid, else
    /// the inferred model supplied by the caller.
    #[must_use]
    pub fn effective_stock(&self, inferred: Option<StockModel>) -> Option<StockModel> {
        self.explicit_stock().or(inferred)
    }

    /// Supplies current stock origin and size for the editable fields.
    #[must_use]
    pub fn stock_values(&self, inferred: Option<&StockModel>) -> ([f64; 3], [f64; 3]) {
        if let Some(stock) = self.analysis.stock.as_ref() {
            return (stock.origin_mm, stock.size_mm);
        }
        if let Some(stock) = inferred {
            return (
                stock.min_mm,
                [
                    stock.max_mm[0] - stock.min_mm[0],
                    stock.max_mm[1] - stock.min_mm[1],
                    stock.max_mm[2] - stock.min_mm[2],
                ],
            );
        }
        ([0.0, 0.0, 0.0], [100.0, 80.0, 20.0])
    }

    /// Switches to explicit stock, seeded from the inferred box.
    pub fn set_stock_explicit(&mut self, inferred: Option<&StockModel>) {
        let (origin, size) = self.stock_values(inferred);
        self.analysis.stock = Some(StockConfig {
            origin_mm: origin,
            size_mm: size,
        });
        self.refresh_validation();
        self.bump();
    }

    /// Switches back to inferred stock.
    pub fn set_stock_inferred(&mut self) {
        self.analysis.stock = None;
        self.stock_error = None;
        self.bump();
    }

    /// Sets one explicit stock origin or size component.
    pub fn set_stock_axis(&mut self, origin: bool, axis: usize, value: f64) {
        if axis > 2 || !value.is_finite() {
            return;
        }
        let stock = self.analysis.stock.get_or_insert(StockConfig {
            origin_mm: [0.0, 0.0, 0.0],
            size_mm: [100.0, 80.0, 20.0],
        });
        if origin {
            stock.origin_mm[axis] = value;
        } else {
            stock.size_mm[axis] = value;
        }
        self.refresh_validation();
        self.bump();
    }

    /// Enables or disables machine limits, seeding a travel box when enabled.
    pub fn set_machine_enabled(&mut self, enabled: bool, inferred: Option<&StockModel>) {
        if !enabled {
            self.analysis.machine = None;
            self.machine_error = None;
            self.bump();
            return;
        }
        let (origin, size) = self.stock_values(inferred);
        let machine = MachineConfig {
            travel_min_mm: [origin[0] - 10.0, origin[1] - 10.0, origin[2] - 10.0],
            travel_max_mm: [
                origin[0] + size[0] + 10.0,
                origin[1] + size[1] + 10.0,
                origin[2] + size[2] + 10.0,
            ],
            max_feed_mm_min: None,
            max_spindle_rpm: None,
            rapid_rate_mm_min: None,
            spindle_manual: false,
        };
        self.analysis.machine = Some(machine);
        self.refresh_validation();
        self.bump();
    }

    /// Sets one machine numeric field. `None` clears an optional limit.
    pub fn set_machine_value(&mut self, field: MachineField, value: Option<f64>) {
        let Some(machine) = self.analysis.machine.as_mut() else {
            return;
        };
        match field {
            MachineField::TravelMin(axis) | MachineField::TravelMax(axis) => {
                if axis > 2 {
                    return;
                }
                let Some(value) = value.filter(|value| value.is_finite()) else {
                    return;
                };
                if matches!(field, MachineField::TravelMin(_)) {
                    machine.travel_min_mm[axis] = value;
                } else {
                    machine.travel_max_mm[axis] = value;
                }
            }
            MachineField::MaxFeed => machine.max_feed_mm_min = positive(value),
            MachineField::MaxSpindle => machine.max_spindle_rpm = positive(value),
            MachineField::Rapid => {
                let rapid = positive(value);
                machine.rapid_rate_mm_min = rapid;
                if let Some(rapid) = rapid {
                    self.sim.rapid_rate_mm_min = rapid;
                }
            }
        }
        self.refresh_validation();
        self.bump();
    }

    /// Sets manual-spindle mode.
    pub fn set_spindle_manual(&mut self, manual: bool) {
        if let Some(machine) = self.analysis.machine.as_mut() {
            machine.spindle_manual = manual;
            self.bump();
        }
    }

    /// Chooses a built-in material preset, or clears the material.
    pub fn set_material_preset(&mut self, name: Option<&str>) {
        self.analysis.material = name.map(|name| {
            MaterialPreset::builtin(name).unwrap_or_else(|| {
                let mut preset = MaterialPreset::new(name);
                preset.notes.clear();
                preset
            })
        });
        self.bump();
    }

    /// Sets one inline material range. Creates a custom preset when needed.
    pub fn set_material_value(&mut self, field: MaterialField, value: Option<f64>) {
        let value = positive(value);
        if value.is_none() && self.analysis.material.is_none() {
            return;
        }
        let preset = self
            .analysis
            .material
            .get_or_insert_with(|| MaterialPreset::new("custom"));
        match field {
            MaterialField::ChiploadMin => preset.chipload_min_mm = value,
            MaterialField::ChiploadMax => preset.chipload_max_mm = value,
            MaterialField::SurfaceSpeedMin => preset.surface_speed_min_mpm = value,
            MaterialField::SurfaceSpeedMax => preset.surface_speed_max_mpm = value,
            MaterialField::MaxMrr => preset.max_mrr_mm3_min = value,
        }
        self.bump();
    }

    /// Sets the material notes.
    pub fn set_material_notes(&mut self, notes: String) {
        if notes.trim().is_empty() && self.analysis.material.is_none() {
            return;
        }
        let preset = self
            .analysis
            .material
            .get_or_insert_with(|| MaterialPreset::new("custom"));
        preset.notes = notes;
        self.bump();
    }

    /// Sets `requires_coolant` on the current material.
    pub fn set_requires_coolant(&mut self, required: bool) {
        if let Some(preset) = self.analysis.material.as_mut() {
            preset.requires_coolant = required;
            self.bump();
        }
    }

    /// Sets one rule tolerance.
    pub fn set_tolerance(&mut self, field: ToleranceField, value: f64) {
        if !value.is_finite() || value < 0.0 {
            return;
        }
        match field {
            ToleranceField::ArcRadius => self.analysis.tolerances.arc_radius_mm = value,
            ToleranceField::MinSegment => self.analysis.tolerances.min_segment_mm = value,
            ToleranceField::HoleXy => self.analysis.tolerances.hole_xy_mm = value,
            ToleranceField::Clearance => self.analysis.tolerances.clearance_mm = value,
        }
        self.bump();
    }

    /// Sets one simulation field.
    pub fn set_sim_value(&mut self, field: SimField, value: Option<f64>) {
        match field {
            SimField::CellMm => {
                if let Some(value) = positive(value) {
                    self.sim.cell_mm = value;
                }
            }
            SimField::MaxCells => {
                if let Some(value) = positive(value) {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        self.sim.max_cells = (value as usize).max(1);
                    }
                }
            }
            SimField::RapidRate => {
                if let Some(value) = positive(value) {
                    self.sim.rapid_rate_mm_min = value;
                }
            }
            SimField::ToolChangePause => {
                if let Some(value) = value.filter(|value| value.is_finite() && *value >= 0.0) {
                    self.sim.tool_change_pause_s = value;
                }
            }
            SimField::AssumedDiameter => {
                self.sim.assumed_tool_diameter_mm = positive(value);
            }
        }
        self.sim = self.sim.sanitized();
        self.bump();
    }

    /// Toggles simulation crash preview.
    pub fn set_crash_preview(&mut self, enabled: bool) {
        self.sim.crash_preview = enabled;
        self.bump();
    }

    /// Selects a dialect. Any explicit choice clears the assumed flag.
    pub fn set_dialect(&mut self, dialect: Dialect) {
        self.dialect = dialect;
        self.dialect_assumed = false;
        self.bump();
    }

    /// Settings for one rule, always at least the defaults.
    #[must_use]
    pub fn rule_settings(&self, rule: &str) -> RuleSettings {
        self.analysis.rules.settings(&RuleId::owned(rule))
    }

    /// Enables or disables one rule.
    pub fn set_rule_enabled(&mut self, rule: &str, enabled: bool) {
        let mut settings = self.rule_settings(rule);
        settings.enabled = enabled;
        self.store_rule(rule, settings);
        self.bump();
    }

    /// Sets or clears one rule's severity override.
    pub fn set_rule_severity(&mut self, rule: &str, severity: Option<Severity>) {
        let mut settings = self.rule_settings(rule);
        settings.severity = severity;
        self.store_rule(rule, settings);
        self.bump();
    }

    /// Inserts or replaces one numeric rule threshold.
    pub fn set_rule_threshold(&mut self, rule: &str, key: &str, value: f64) {
        let key = key.trim();
        if key.is_empty() || !value.is_finite() {
            return;
        }
        let mut settings = self.rule_settings(rule);
        settings.thresholds.insert(key.to_string(), value);
        self.store_rule(rule, settings);
        self.bump();
    }

    /// Removes one numeric rule threshold.
    pub fn remove_rule_threshold(&mut self, rule: &str, key: &str) {
        let mut settings = self.rule_settings(rule);
        if settings.thresholds.remove(key).is_some() {
            self.store_rule(rule, settings);
            self.bump();
        }
    }

    /// Builds the config the analysis engine reads.
    ///
    /// Invalid stock or machine values are omitted, so the engine is never
    /// asked to fall back silently; the sheet keeps showing the validation
    /// error and the stock chip shows `STOCK: INVALID`.
    #[must_use]
    pub fn analysis_config(&self, tools: ToolTable) -> AnalysisConfig {
        let mut config = self.analysis.clone();
        config.stock = self.analysis.stock.filter(|stock| stock.to_stock().is_ok());
        if config
            .machine
            .as_ref()
            .is_some_and(|machine| machine.validate().is_err())
        {
            config.machine = None;
        }
        config.tools = tools;
        config.tolerances = config.tolerances.sanitized();
        config
    }

    /// Re-runs stock and machine validation after an edit.
    fn refresh_validation(&mut self) {
        self.stock_error = self
            .analysis
            .stock
            .as_ref()
            .and_then(|stock| stock.to_stock().err().map(|error| error.to_string()));
        self.machine_error = self
            .analysis
            .machine
            .as_ref()
            .and_then(|machine| machine.validate().err().map(|error| error.to_string()));
    }

    fn store_rule(&mut self, rule: &str, settings: RuleSettings) {
        let id = RuleId::owned(rule);
        if settings == RuleSettings::default() {
            self.analysis.rules.0.remove(&id);
        } else {
            self.analysis.rules.0.insert(id, settings);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// Builds the program inventory in first-use order.
#[must_use]
pub fn inventory_for(program: &Program) -> Vec<InventoryTool> {
    let mut usage: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for motion in &program.motions {
        if motion.tool == 0 {
            continue;
        }
        let entry = usage.entry(motion.tool).or_insert((motion.line, 0usize));
        entry.0 = entry.0.min(motion.line);
        entry.1 = entry.1.saturating_add(1);
    }
    let mut inventory: Vec<InventoryTool> = usage
        .into_iter()
        .map(|(number, (first_line, motion_count))| InventoryTool {
            number,
            first_line,
            motion_count,
            hint_line: program
                .tool_hints
                .iter()
                .find(|hint| hint.tool == number)
                .map(|hint| hint.line),
        })
        .collect();
    inventory.sort_by_key(|tool| (tool.first_line, tool.number));
    inventory
}

/// SHA-256 of the source bytes, lowercase hexadecimal.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Value text for an editable optional number.
#[must_use]
pub fn format_optional(value: Option<f64>) -> String {
    value.map_or_else(String::new, format_number)
}

/// Value text for a number, trimming a trailing `.0`.
#[must_use]
pub fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    if value.fract().abs() < 1.0e-9 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Parses an optional finite number. Empty is `None`, invalid is an error.
pub fn parse_optional_number(raw: &str, field: &str) -> Result<Option<f64>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        _ => Err(format!("{field} must be a number, or empty for unknown.")),
    }
}

/// Parses an optional positive number.
pub fn parse_optional_positive(raw: &str, field: &str) -> Result<Option<f64>, String> {
    match parse_optional_number(raw, field)? {
        Some(value) if value > 0.0 => Ok(Some(value)),
        Some(_) => Err(format!("{field} must be greater than zero, or empty.")),
        None => Ok(None),
    }
}

/// Parses a required non-negative number.
pub fn parse_non_negative(raw: &str, field: &str) -> Result<f64, String> {
    match parse_optional_number(raw, field)? {
        Some(value) if value >= 0.0 => Ok(value),
        Some(_) => Err(format!("{field} must not be negative.")),
        None => Err(format!("{field} is required.")),
    }
}

/// Parses an optional positive whole number.
pub fn parse_optional_count(raw: &str, field: &str) -> Result<Option<u32>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u32>() {
        Ok(value) if value > 0 => Ok(Some(value)),
        _ => Err(format!(
            "{field} must be a positive whole number, or empty."
        )),
    }
}

/// Writes the report body. No absolute path is ever included.
pub fn export_text(report: &Report, format: ExportFormat) -> String {
    match format {
        ExportFormat::Json => {
            let mut text = report.to_json_pretty();
            text.push('\n');
            text
        }
        ExportFormat::Markdown => markdown(report),
    }
}

/// Markdown summary with every diagnostic and every coverage skip.
#[must_use]
pub fn markdown(report: &Report) -> String {
    let mut out = String::new();
    let file_name = &report.input.file_name;
    let _ = writeln!(out, "# nca report: {file_name}");
    let _ = writeln!(out);
    let _ = writeln!(out, "> {ADVISORY_NOTE}");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(out, "| field | value |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| input | `{file_name}` |");
    let _ = writeln!(out, "| sha256 | `{}` |", report.input.sha256);
    let _ = writeln!(out, "| lines | {} |", report.input.line_count);
    let _ = writeln!(out, "| dialect | `{}` |", report.input.dialect);
    if let Some(line) = report.input.geometry_uncertain_after {
        let _ = writeln!(out, "| geometry uncertain after | {line} |");
    }
    let _ = writeln!(out, "| stock source | {} |", stock_label(report));
    let _ = writeln!(out, "| schema | `{}` |", report.schema);
    let _ = writeln!(out, "| ruleset | `{}` |", report.ruleset_version);
    let _ = writeln!(out, "| config sha256 | `{}` |", report.config_digest);
    let (errors, warnings, infos) = severity_counts(report);
    let _ = writeln!(
        out,
        "| diagnostics | {} ({} error, {} warning, {} info) |",
        report.diagnostics.len(),
        errors,
        warnings,
        infos
    );
    let _ = writeln!(
        out,
        "| coverage | {} evaluated, {} skipped rules (of {} registered) |",
        report.coverage.evaluated,
        report.coverage.skipped_rule_count(),
        report.coverage.total
    );

    let _ = writeln!(out);
    let _ = writeln!(out, "## Tool inventory");
    let _ = writeln!(out);
    if report.tool_inventory.is_empty() {
        let _ = writeln!(out, "No tool was used by a motion.");
    } else {
        let _ = writeln!(out, "| tool | first line | motions |");
        let _ = writeln!(out, "|---|---|---|");
        for entry in &report.tool_inventory {
            let _ = writeln!(
                out,
                "| T{} | {} | {} |",
                entry.tool, entry.first_line, entry.motion_count
            );
        }
        for entry in &report.tool_inventory {
            let _ = writeln!(out);
            let _ = writeln!(out, "### T{}", entry.tool);
            let _ = writeln!(out);
            if entry.fields.is_empty() {
                let _ = writeln!(out, "No field was known; rules that need one were skipped.");
            } else {
                for (name, field) in &entry.fields {
                    let _ = writeln!(
                        out,
                        "- `{name}` = {} (`{}`)",
                        field.value.display_text(),
                        field.source
                    );
                }
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Diagnostics");
    let _ = writeln!(out);
    if report.diagnostics.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                out,
                "- **{}** `{}` line {} ({}): {}",
                diagnostic.severity,
                diagnostic.rule,
                diagnostic.line,
                diagnostic.basis.label(),
                diagnostic.message
            );
            if !diagnostic.evidence.is_empty() {
                let evidence: Vec<String> = diagnostic
                    .evidence
                    .iter()
                    .map(|(key, value)| format!("`{key}`={}", value.display_text()))
                    .collect();
                let _ = writeln!(out, "  - evidence: {}", evidence.join(", "));
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Coverage");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{} evaluated, {} skipped rules ({} skip entries, {} registered).",
        report.coverage.evaluated,
        report.coverage.skipped_rule_count(),
        report.coverage.skipped_count(),
        report.coverage.total
    );
    if report.coverage.skipped.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Every registered rule was evaluated.");
    } else {
        let _ = writeln!(out);
        let _ = writeln!(out, "Skip entries:");
        let _ = writeln!(out);
        for skip in &report.coverage.skipped {
            let _ = writeln!(
                out,
                "- `{}` missing `{}`: {}",
                skip.rule, skip.missing_input, skip.reason
            );
        }
    }

    if !report.truncated.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Truncated findings");
        let _ = writeln!(out);
        for (rule, count) in &report.truncated {
            let _ = writeln!(out, "- `{rule}`: {count} dropped");
        }
    }

    out
}

/// Result of a successful export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportOutcome {
    /// Written path, resolved against the current directory.
    pub path: PathBuf,
    /// Number of bytes written.
    pub bytes: usize,
}

/// Writes the report body to `path_text`.
///
/// A relative path is resolved against the current directory; `~/` is expanded
/// with `HOME`. Missing parent directories are created. The report body never
/// contains the path.
pub fn write_export(
    report: &Report,
    path_text: &str,
    format: ExportFormat,
) -> Result<ExportOutcome, String> {
    let trimmed = path_text.trim();
    if trimmed.is_empty() {
        return Err("Enter an export path first.".to_string());
    }
    let requested = expand_home(trimmed);
    let path = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot resolve the current directory: {error}"))?
            .join(requested)
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let text = export_text(report, format);
    fs::write(&path, text.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(ExportOutcome {
        path,
        bytes: text.len(),
    })
}

/// Expands a leading `~` with `HOME`, without touching any other variable.
#[must_use]
pub fn expand_home(text: &str) -> PathBuf {
    if text == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(text)
}

/// Display name for a config or export path, never the full path.
#[must_use]
pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string())
}

/// Parses a placement text as a tool shape, or `Unknown` when unrecognized.
#[must_use]
pub fn parse_shape(raw: &str) -> ToolShape {
    ToolShape::parse(raw).unwrap_or(ToolShape::Unknown)
}

/// Parses a three-state handedness value.
#[must_use]
pub fn parse_handedness(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "right" | "right-hand" | "right_hand" | "yes" | "true" | "1" => Some(true),
        "left" | "left-hand" | "left_hand" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// Provenance label shown next to one tool field.
///
/// Matches the report vocabulary exactly: `config`, `comment:L12`, or `unset`.
#[must_use]
pub fn provenance_label(source: Option<FieldSource>) -> String {
    source.map_or_else(|| "unset".to_string(), FieldSource::label)
}

/// Handedness label for a tool field.
#[cfg(test)]
#[must_use]
pub fn handedness_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "right hand",
        Some(false) => "left hand",
        None => "unknown",
    }
}

/// Applies one tool field edit to a persisted overlay entry.
///
/// Empty numeric text clears the field. Every set field is labeled
/// [`FieldSource::Config`], so a later report says the value came from config.
pub fn apply_tool_field(tool: &mut Tool, field: ToolField, raw: &str) -> Result<(), String> {
    let before = tool.clone();
    let mut changed = true;
    match field {
        ToolField::Name => {
            let text = raw.trim();
            tool.name = (!text.is_empty()).then(|| text.to_string());
        }
        ToolField::Shape => {
            let shape = parse_shape(raw);
            tool.shape = shape;
            changed = shape != ToolShape::Unknown;
        }
        ToolField::Diameter => {
            let value = parse_optional_positive(raw, "Diameter")?;
            tool.diameter_mm = value;
            changed = value.is_some();
        }
        ToolField::Flutes => {
            let value = parse_optional_count(raw, "Flutes")?;
            tool.flutes = value;
            changed = value.is_some();
        }
        ToolField::Stickout => {
            let value = parse_optional_positive(raw, "Stickout")?;
            tool.stickout_mm = value;
            changed = value.is_some();
        }
        ToolField::FluteLength => {
            let value = parse_optional_positive(raw, "Flute length")?;
            tool.flute_length_mm = value;
            changed = value.is_some();
        }
        ToolField::TipAngle => {
            let value = parse_optional_positive(raw, "Tip angle")?;
            if value.is_some_and(|angle| angle > 180.0) {
                *tool = before;
                return Err("Tip angle must be at most 180 degrees.".to_string());
            }
            tool.tip_angle_deg = value;
            changed = value.is_some();
        }
        ToolField::CornerRadius => {
            let value = parse_optional_number(raw, "Corner radius")?;
            if value.is_some_and(|radius| radius < 0.0) {
                *tool = before;
                return Err("Corner radius must not be negative.".to_string());
            }
            tool.corner_radius_mm = value;
            changed = value.is_some();
        }
        ToolField::ShankDiameter => {
            let value = parse_optional_positive(raw, "Shank diameter")?;
            tool.shank_diameter_mm = value;
            changed = value.is_some();
        }
        ToolField::HolderDiameter => {
            let value = parse_optional_positive(raw, "Holder diameter")?;
            tool.holder_diameter_mm = value;
            changed = value.is_some();
        }
        ToolField::RightHand => {
            let value = parse_handedness(raw);
            tool.right_hand = value;
            changed = value.is_some();
        }
    }
    if changed {
        tool.sources.insert(field, FieldSource::Config);
    } else {
        tool.sources.remove(&field);
    }
    Ok(())
}

/// Returns the persisted overlay entry for one tool, or a blank entry.
#[must_use]
pub fn overlay_tool(config: &NcaConfig, hash: &str, number: u32) -> Tool {
    config
        .tools_for(hash)
        .get(number)
        .cloned()
        .unwrap_or_else(|| Tool::new(number))
}

/// Stores an edited tool overlay entry, removing fully blank entries.
pub fn store_overlay_tool(config: &mut NcaConfig, hash: &str, tool: Tool) {
    let mut table = config.tools_for(hash);
    if tool_is_blank(&tool) {
        table.remove(tool.number);
    } else {
        table.insert(tool);
    }
    config.set_tools_for(hash, table);
}

/// `true` when an entry carries no user value.
#[must_use]
pub fn tool_is_blank(tool: &Tool) -> bool {
    tool.name.is_none()
        && tool.shape == ToolShape::Unknown
        && tool.diameter_mm.is_none()
        && tool.flutes.is_none()
        && tool.stickout_mm.is_none()
        && tool.flute_length_mm.is_none()
        && tool.tip_angle_deg.is_none()
        && tool.corner_radius_mm.is_none()
        && tool.shank_diameter_mm.is_none()
        && tool.holder_diameter_mm.is_none()
        && tool.right_hand.is_none()
        && tool.sources.is_empty()
}

fn stock_label(report: &Report) -> &'static str {
    match report.stock_source {
        nc_stock::StockSource::Inferred => "inferred",
        nc_stock::StockSource::Explicit => "explicit",
    }
}

fn severity_counts(report: &Report) -> (usize, usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    let mut infos = 0;
    for diagnostic in &report.diagnostics {
        match diagnostic.severity {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            Severity::Info => infos += 1,
        }
    }
    (errors, warnings, infos)
}

fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_analysis::config_file::parse_config;
    use nc_analysis::{AnalysisRequest, Basis, Coverage, Diagnostic, EvidenceValue, InputInfo};
    use nc_analysis::{RuleRegistry, SkippedRule, analyze};
    use nc_gcode::parse_program;
    use nc_stock::StockSource;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "nca-setup-{tag}-{}-{nanos}-{id}.json",
            std::process::id()
        ))
    }

    const FULL_TOML: &str = r#"
[parse]
dialect = "grbl"

[machine]
travel_min_mm = [-10, -10, -120]
travel_max_mm = [410, 310, 10]
max_feed_mm_min = 3000
max_spindle_rpm = 24000
rapid_rate_mm_min = 5000
spindle_manual = true

[stock]
origin_mm = [0, 0, -20]
size_mm = [100, 80, 20]

[[tool]]
number = 1
name = "6 mm 2F flat"
shape = "flat"
diameter_mm = 6.0
flutes = 2

[material]
preset = "aluminum-6061"
chipload_max_mm = 0.08

[rules."clearance.rapid_at_depth"]
enabled = false
severity = "warning"
clearance_mm = 1.5
"#;

    #[test]
    fn toml_load_maps_every_setup_input() {
        let loaded = parse_config(FULL_TOML).expect("config");
        let mut setup = SetupState::default();
        let config_path = scratch_path("toml-tools");
        let mut config = NcaConfig::load_from(&config_path);
        let tools = setup.apply_loaded(loaded);
        config.set_tools_for("hash-a", tools.clone());

        assert_eq!(setup.dialect, Dialect::Grbl);
        assert!(!setup.dialect_assumed);
        assert_eq!(setup.stock_label(), "STOCK: SET");
        assert_eq!(
            setup.analysis.stock,
            Some(StockConfig {
                origin_mm: [0.0, 0.0, -20.0],
                size_mm: [100.0, 80.0, 20.0],
            })
        );
        assert!(setup.stock_error.is_none());
        let machine = setup.analysis.machine.as_ref().expect("machine");
        assert_eq!(machine.travel_max_mm, [410.0, 310.0, 10.0]);
        assert!(machine.spindle_manual);
        assert_eq!(
            setup
                .analysis
                .material
                .as_ref()
                .and_then(|material| material.chipload_max_mm),
            Some(0.08)
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools.get(1).and_then(|tool| tool.diameter_mm), Some(6.0));
        assert_eq!(
            config
                .tools_for("hash-a")
                .get(1)
                .and_then(|tool| tool.flutes),
            Some(2)
        );
        let settings = setup.rule_settings("clearance.rapid_at_depth");
        assert!(!settings.enabled);
        assert_eq!(settings.severity, Some(Severity::Warning));
        assert_eq!(settings.thresholds.get("clearance_mm"), Some(&1.5));

        let effective = setup.analysis_config(tools);
        assert_eq!(effective.stock, setup.analysis.stock);
        assert!(effective.machine.is_some());
        assert!(effective.material.is_some());
        assert_eq!(effective.rules, setup.analysis.rules);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn omitted_stock_is_inferred_and_omitted_dialect_is_assumed() {
        let loaded = parse_config("[parse]\n").expect("empty config");
        let mut setup = SetupState::default();
        setup.apply_loaded(loaded);
        assert_eq!(setup.dialect, Dialect::Generic);
        assert!(setup.dialect_assumed);
        assert!(setup.analysis.stock.is_none());
        assert_eq!(setup.stock_label(), "STOCK: INFERRED");
        let inferred = StockModel::explicit([0.0, 0.0, -10.0], [50.0, 40.0, 0.0]).expect("stock");
        assert!(setup.stock_basis_text(Some(&inferred)).contains("top Z"));
    }

    #[test]
    fn explicit_stock_validation_and_revision() {
        let mut setup = SetupState::default();
        let inferred =
            StockModel::infer(&parse_program("G21 G90\nG1 X10 F100\n").expect("program"));
        let before = setup.revision;
        setup.set_stock_explicit(Some(&inferred));
        assert!(setup.revision > before);
        assert_eq!(setup.stock_label(), "STOCK: SET");
        assert!(setup.analysis_config(ToolTable::new()).stock.is_some());

        let revision = setup.revision;
        setup.set_stock_axis(false, 0, 0.0);
        assert!(setup.revision > revision);
        assert_eq!(setup.stock_label(), "STOCK: INVALID");
        assert!(setup.stock_error.is_some());
        assert!(
            setup.analysis_config(ToolTable::new()).stock.is_none(),
            "invalid explicit stock must not reach the engine"
        );

        setup.set_stock_axis(false, 0, 120.0);
        assert_eq!(setup.stock_label(), "STOCK: SET");
        assert!(setup.stock_error.is_none());
        assert!(setup.analysis_config(ToolTable::new()).stock.is_some());

        setup.set_stock_inferred();
        assert_eq!(setup.stock_label(), "STOCK: INFERRED");
        assert!(setup.analysis_config(ToolTable::new()).stock.is_none());
    }

    #[test]
    fn machine_material_and_simulation_edits_reach_their_configs() {
        let mut setup = SetupState::default();
        let inferred = StockModel::explicit([0.0, 0.0, -10.0], [100.0, 80.0, 0.0]).expect("stock");
        setup.set_machine_enabled(true, Some(&inferred));
        setup.set_machine_value(MachineField::MaxFeed, Some(3000.0));
        setup.set_machine_value(MachineField::MaxSpindle, Some(24000.0));
        setup.set_machine_value(MachineField::Rapid, Some(6500.0));
        setup.set_machine_value(MachineField::TravelMax(0), Some(150.0));
        setup.set_spindle_manual(true);
        assert_eq!(setup.sim.rapid_rate_mm_min, 6500.0);
        setup.set_material_preset(Some("aluminum-6061"));
        setup.set_material_value(MaterialField::ChiploadMax, Some(0.09));
        setup.set_material_notes("operator note".to_string());

        let config = setup.analysis_config(ToolTable::new());
        let machine = config.machine.as_ref().expect("machine");
        assert_eq!(machine.max_feed_mm_min, Some(3000.0));
        assert_eq!(machine.max_spindle_rpm, Some(24000.0));
        assert_eq!(machine.travel_max_mm[0], 150.0);
        assert!(machine.spindle_manual);
        let material = config.material.as_ref().expect("material");
        assert_eq!(material.chipload_max_mm, Some(0.09));
        assert!(material.chipload_min_mm.is_some(), "preset values survive");
        assert_eq!(material.notes, "operator note");

        setup.set_sim_value(SimField::CellMm, Some(1.5));
        setup.set_sim_value(SimField::MaxCells, Some(500_000.0));
        setup.set_sim_value(SimField::RapidRate, Some(7000.0));
        setup.set_sim_value(SimField::ToolChangePause, Some(1.0));
        setup.set_sim_value(SimField::AssumedDiameter, Some(3.0));
        setup.set_crash_preview(true);
        assert_eq!(setup.sim.cell_mm, 1.5);
        assert_eq!(setup.sim.max_cells, 500_000);
        assert_eq!(setup.sim.rapid_rate_mm_min, 7000.0);
        assert_eq!(setup.sim.tool_change_pause_s, 1.0);
        assert_eq!(setup.sim.assumed_tool_diameter_mm, Some(3.0));
        assert!(setup.sim.crash_preview);
    }

    #[test]
    fn rule_edits_reach_the_analysis_config() {
        let rule = "clearance.rapid_at_depth";
        let mut setup = SetupState::default();
        setup.set_rule_enabled(rule, false);
        setup.set_rule_severity(rule, Some(Severity::Error));
        setup.set_rule_threshold(rule, "clearance_mm", 1.25);

        let config = setup.analysis_config(ToolTable::new());
        let settings = config.rules.settings(&RuleId::new(rule));
        assert!(!settings.enabled);
        assert_eq!(settings.severity, Some(Severity::Error));
        assert_eq!(settings.thresholds.get("clearance_mm"), Some(&1.25));

        setup.set_rule_enabled(rule, true);
        setup.set_rule_severity(rule, None);
        setup.remove_rule_threshold(rule, "clearance_mm");
        let config = setup.analysis_config(ToolTable::new());
        assert!(
            config.rules.0.is_empty(),
            "default settings must stay sparse"
        );
    }

    #[test]
    fn inventory_is_first_use_ordered_and_skips_t0() {
        let source = "\
G21 G90
T3 M6 (6 mm flat end mill)
G0 X0 Y0
G1 X10 F100
T1 M6 (3 mm ball)
G1 X20
G1 X30
";
        let program = parse_program(source).expect("program");
        let inventory = inventory_for(&program);
        let numbers: Vec<u32> = inventory.iter().map(|tool| tool.number).collect();
        assert_eq!(numbers, [3, 1]);
        assert_eq!(inventory[0].first_line, 4);
        assert_eq!(inventory[0].motion_count, 1);
        assert_eq!(inventory[1].first_line, 6);
        assert_eq!(inventory[1].motion_count, 2);
    }

    #[test]
    fn comment_provenance_is_labeled_with_its_source_line() {
        let source = "T3 M6 (6 mm flat end mill)\nG0 X0 Y0\nG1 X10 F100\n";
        let program = parse_program(source).expect("program");
        let resolved = ToolTable::resolve(&ToolTable::new(), &program.tool_hints);
        let tool = resolved.get(3).expect("comment hint");
        let source = tool
            .sources
            .get(&ToolField::Diameter)
            .copied()
            .expect("diameter from comment");
        assert!(source.is_comment());
        assert_eq!(provenance_label(Some(source)), source.label());
        assert!(provenance_label(Some(source)).starts_with("comment:L"));
        assert_eq!(provenance_label(Some(FieldSource::Config)), "config");
        assert_eq!(provenance_label(None), "unset");
    }

    #[test]
    fn tool_overlay_persists_by_hash_not_path() {
        let path = scratch_path("tools");
        let hash_a = sha256_hex(b"program A");
        let hash_b = sha256_hex(b"program B");
        let mut config = NcaConfig::load_from(&path);
        let mut overlay = Tool::new(3);
        apply_tool_field(&mut overlay, ToolField::Diameter, "6.0").expect("diameter");
        overlay.shape = ToolShape::Flat;
        overlay
            .sources
            .insert(ToolField::Shape, FieldSource::Config);
        store_overlay_tool(&mut config, &hash_a, overlay);
        config.save_to(&path).expect("save");

        let loaded = NcaConfig::load_from(&path);
        assert_eq!(
            loaded
                .tools_for(&hash_a)
                .get(3)
                .and_then(|entry| entry.diameter_mm),
            Some(6.0)
        );
        assert!(loaded.tools_for(&hash_b).get(3).is_none());
        assert_eq!(loaded.tools_for(&hash_a).len(), 1);
        for key in loaded.tool_tables.keys() {
            assert_eq!(key.len(), 64);
            assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(!key.contains('/'));
        }

        // Empty edits remove the entry instead of leaving a hollow record.
        let mut config = NcaConfig::load_from(&path);
        let mut blank = Tool::new(3);
        apply_tool_field(&mut blank, ToolField::Diameter, "").expect("clear");
        store_overlay_tool(&mut config, &hash_a, blank);
        assert!(config.tools_for(&hash_a).is_empty());

        let _ = fs::remove_file(path);
    }

    fn sample_report() -> Report {
        let mut evidence = BTreeMap::new();
        evidence.insert(
            "end_mm".to_string(),
            EvidenceValue::Point3([1.0, 2.0, -3.0]),
        );
        Report {
            schema: "nca.report/1".to_string(),
            ruleset_version: "1.0.0".to_string(),
            input: InputInfo {
                file_name: "part.nc".to_string(),
                sha256: "a".repeat(64),
                line_count: 120,
                dialect: Dialect::Grbl,
                geometry_uncertain_after: None,
            },
            config_digest: "b".repeat(64),
            stock_source: StockSource::Inferred,
            tool_inventory: vec![nc_analysis::ToolInventoryEntry {
                tool: 1,
                first_line: 8,
                motion_count: 412,
                fields: BTreeMap::from([(
                    "diameter_mm".to_string(),
                    nc_analysis::InventoryField {
                        value: EvidenceValue::Float(6.0),
                        source: "comment:L4".to_string(),
                    },
                )]),
            }],
            diagnostics: vec![
                Diagnostic {
                    rule: RuleId::new("clearance.rapid_at_depth"),
                    severity: Severity::Warning,
                    basis: Basis::Model,
                    line: 42,
                    column: 0,
                    line_end: 42,
                    message: "Rapid move below stock top + clearance.".to_string(),
                    evidence,
                },
                Diagnostic {
                    rule: RuleId::new("geometry.duplicate_hole"),
                    severity: Severity::Error,
                    basis: Basis::Exact,
                    line: 55,
                    column: 0,
                    line_end: 55,
                    message: "Two holes share one centre.".to_string(),
                    evidence: BTreeMap::new(),
                },
            ],
            truncated: BTreeMap::from([("process.chipload".to_string(), 3)]),
            coverage: Coverage {
                evaluated: 43,
                total: 45,
                skipped: vec![
                    SkippedRule::new(
                        RuleId::new("process.chipload"),
                        "tool.flutes:T1",
                        "tool T1 has no flute count",
                    ),
                    SkippedRule::new(
                        RuleId::new("travel.stock_bounds"),
                        "stock.explicit",
                        "explicit stock is required",
                    ),
                    SkippedRule::new(
                        RuleId::new("process.chipload"),
                        "tool.flutes:T3",
                        "tool T3 has no flute count",
                    ),
                ],
            },
        }
    }

    #[test]
    fn json_export_has_exactly_one_trailing_newline() {
        let report = sample_report();
        let text = export_text(&report, ExportFormat::Json);
        assert_eq!(text, format!("{}\n", report.to_json_pretty()));
        assert!(text.ends_with('\n'));
        assert!(!text.ends_with("\n\n"));
        assert!(!text.contains("/home/"));
        assert!(!text.contains("/tmp/"));
    }

    #[test]
    fn markdown_lists_counts_every_finding_every_skip_and_no_paths() {
        let report = sample_report();
        let text = export_text(&report, ExportFormat::Markdown);
        for diagnostic in &report.diagnostics {
            assert!(
                text.contains(diagnostic.rule.as_str()),
                "missing rule id {}",
                diagnostic.rule
            );
            assert!(text.contains(diagnostic.message.as_str()));
        }
        for skip in &report.coverage.skipped {
            assert!(text.contains(skip.missing_input.as_str()));
            assert!(text.contains(skip.reason.as_str()));
        }
        assert!(text.contains("43 evaluated"));
        assert!(text.contains("2 skipped rules"));
        assert!(text.contains("3 skip entries"));
        assert!(text.contains("45 registered"));
        assert!(text.contains("Skip entries:"));
        assert_eq!(
            report.coverage.evaluated + report.coverage.skipped_rule_count(),
            report.coverage.total
        );
        assert!(text.contains("Advisory only"));
        assert!(text.contains("T1"));
        assert!(text.contains("comment:L4"));

        assert!(
            !text.contains('%'),
            "coverage text must not be a percentage"
        );
        assert!(!text.contains("SAFE"), "report must not claim safety");
        assert!(!text.contains("/home/"));
        assert!(!text.contains("/tmp/"));
        assert!(!text.contains("C:\\"));
    }

    #[test]
    fn export_writes_one_newline_and_keeps_the_path_out() {
        let report = sample_report();
        let path = scratch_path("export").with_extension("json");
        let outcome =
            write_export(&report, path.to_str().expect("path"), ExportFormat::Json).expect("write");
        let bytes = fs::read(&outcome.path).expect("read");
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.ends_with(b"\n\n"));
        let text = String::from_utf8(bytes).expect("utf8");
        assert_eq!(text, format!("{}\n", report.to_json_pretty()));
        assert!(!text.contains(&path.to_string_lossy().to_string()));
        assert_eq!(outcome.path, path);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn analyze_bare_program_exports_every_skipped_key() {
        let source = "G21 G90\nG1 X10 F100\nG0 X0\n";
        let program = parse_program(source).expect("program");
        let config = AnalysisConfig::default();
        let request = AnalysisRequest::new("bare.nc", source, &config, Dialect::Generic)
            .with_dialect_assumed(true);
        let report = analyze(
            &program,
            &request,
            &RuleRegistry::from_rules(nc_analysis::rules::all()),
        );
        assert!(!report.coverage.skipped.is_empty());
        assert_eq!(
            report.coverage.evaluated + report.coverage.skipped_rule_count(),
            report.coverage.total
        );
        let text = export_text(&report, ExportFormat::Markdown);
        for skip in &report.coverage.skipped {
            assert!(text.contains(skip.missing_input.as_str()));
        }
        assert!(text.contains(&format!(
            "{} skipped rules",
            report.coverage.skipped_rule_count()
        )));
        assert!(!text.contains('%'));
        assert!(!text.contains("SAFE"));
    }

    #[test]
    fn tool_field_edits_record_config_provenance() {
        let mut tool = Tool::new(3);
        apply_tool_field(&mut tool, ToolField::Diameter, "6").expect("diameter");
        apply_tool_field(&mut tool, ToolField::Flutes, "2").expect("flutes");
        apply_tool_field(&mut tool, ToolField::RightHand, "left").expect("hand");
        assert_eq!(
            tool.sources.get(&ToolField::Diameter),
            Some(&FieldSource::Config)
        );
        assert_eq!(tool.right_hand, Some(false));
        assert_eq!(handedness_label(tool.right_hand), "left hand");

        apply_tool_field(&mut tool, ToolField::Diameter, "").expect("clear");
        assert!(tool.diameter_mm.is_none());
        assert!(!tool.sources.contains_key(&ToolField::Diameter));
        assert!(apply_tool_field(&mut tool, ToolField::Diameter, "-1").is_err());
        assert!(apply_tool_field(&mut tool, ToolField::TipAngle, "200").is_err());
    }
}
