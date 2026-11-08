//! The rule contract and the shared analysis model.

use std::cell::OnceCell;

use nc_core::{MachineConfig, MaterialPreset, ToolField, ToolTable};
use nc_gcode::Program;
use nc_stock::{Heightmap, StockModel};

use crate::config::{RuleSettings, Tolerances};
use crate::report::Diagnostic;
use crate::types::{Basis, RuleId, Severity};

/// What kind of stock a rule needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockRequirement {
    /// The rule does not use stock.
    None,
    /// The rule runs on inferred stock and reports the inference.
    InferredOk,
    /// The rule is skipped when the stock is inferred.
    Explicit,
}

/// Inputs the engine must confirm before calling a rule.
#[derive(Debug, Clone, Copy)]
pub struct Requirements {
    /// Tool fields the rule needs for every tool it evaluates.
    pub tool_fields: &'static [ToolField],
    /// `true` when a machine configuration is required.
    pub machine: bool,
    /// `true` when a material preset is required.
    pub material: bool,
    /// Stock requirement.
    pub stock: StockRequirement,
}

impl Requirements {
    /// No requirements at all.
    pub const NONE: Self = Self {
        tool_fields: &[],
        machine: false,
        material: false,
        stock: StockRequirement::None,
    };

    /// Requirements with only a stock level.
    #[must_use]
    pub const fn with_stock(stock: StockRequirement) -> Self {
        Self {
            stock,
            ..Self::NONE
        }
    }

    /// Requirements for a rule that reads one or more tool fields.
    #[must_use]
    pub const fn with_tools(tool_fields: &'static [ToolField]) -> Self {
        Self {
            tool_fields,
            ..Self::NONE
        }
    }
}

impl Default for Requirements {
    fn default() -> Self {
        Self::NONE
    }
}

/// The result of evaluating one rule.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleOutcome {
    /// The rule ran and produced zero or more findings.
    Findings(Vec<Diagnostic>),
    /// The rule could not run for a data-dependent reason.
    Skipped {
        /// Stable missing-input key.
        missing_input: String,
        /// Human reason.
        reason: String,
    },
}

/// A rule in the deterministic preflight engine.
pub trait Rule {
    /// Stable rule id.
    fn id(&self) -> RuleId;

    /// Default severity before configuration overrides.
    fn default_severity(&self) -> Severity;

    /// Exact or model basis.
    fn basis(&self) -> Basis;

    /// Inputs that must exist before evaluation.
    fn requirements(&self) -> Requirements;

    /// Evaluates the rule against the shared model.
    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome;

    /// Plain-language explanation shown in the UI details panel.
    fn why_it_matters(&self) -> &'static str {
        ""
    }
}

/// One cutting pass's engagement estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct PassEngagement {
    /// Index into the unexpanded motion list.
    pub motion_index: usize,
    /// Source line.
    pub line: usize,
    /// Axial depth of cut in millimetres.
    pub axial_doc_mm: f64,
    /// Estimated radial depth of cut in millimetres.
    pub radial_doc_mm: f64,
    /// Estimated contact angle in radians.
    pub contact_angle_rad: f64,
    /// Material removal rate in cubic millimetres per minute.
    pub mrr_mm3_min: f64,
}

/// The one shared z-map pass used by `process.*` rules.
#[derive(Debug, Clone, PartialEq)]
pub struct EngagementData {
    /// Heightmap after walking every cutting motion in program order.
    pub final_heightmap: Heightmap,
    /// Per-pass estimates.
    pub passes: Vec<PassEngagement>,
    /// `true` when the pass was approximated because the grid was coarsened.
    pub coarse: bool,
}

/// One shared, lazily built model per analysis run.
pub struct AnalysisModel<'a> {
    /// Parsed program.
    pub program: &'a Program,
    /// Inferred or explicit stock.
    pub stock: StockModel,
    /// Resolved tool table.
    pub tools: &'a ToolTable,
    /// Machine limits, when configured.
    pub machine: Option<&'a MachineConfig>,
    /// Material preset, when configured.
    pub material: Option<&'a MaterialPreset>,
    /// Rule tolerances.
    pub tolerances: &'a Tolerances,
    /// One shared engagement pass, computed at most once.
    pub engagement: OnceCell<EngagementData>,
    /// `true` when the dialect was not configured and `Generic` was assumed.
    pub dialect_assumed: bool,
}

impl<'a> AnalysisModel<'a> {
    /// Builds a model without an engagement pass.
    #[must_use]
    pub fn new(
        program: &'a Program,
        stock: StockModel,
        tools: &'a ToolTable,
        machine: Option<&'a MachineConfig>,
        material: Option<&'a MaterialPreset>,
        tolerances: &'a Tolerances,
    ) -> Self {
        Self {
            program,
            stock,
            tools,
            machine,
            material,
            tolerances,
            engagement: OnceCell::new(),
            dialect_assumed: false,
        }
    }

    /// Records whether `Generic` was assumed instead of configured.
    #[must_use]
    pub fn with_dialect_assumed(mut self, assumed: bool) -> Self {
        self.dialect_assumed = assumed;
        self
    }
}
