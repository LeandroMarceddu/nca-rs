//! The compact read-only desktop shell.
//!
//! One workspace, two reserved modes, and three sheets: file open, move
//! legend, and settings. The viewer dominates the window. There is no
//! controller panel, DRO, jog, MDI, console, connect, reset, or start control;
//! the only file mutation is reading a local program into memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, ExternalPaths, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, ScrollStrategy,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use nc_analysis::config_file::load_config;
use nc_analysis::{Report, Severity as AnalysisSeverity};
use nc_core::{Dialect, FieldSource, Tool, ToolField, ToolShape, ToolTable};
use nc_gcode::Program;
use nc_sim::Simulation;
use nc_stock::{Cutter, StockModel};
use nc_viewer::{MoveFamily, MoveKind, StockBox};

use crate::analysis_ui::{
    AnalysisGeneration, AnalysisPanel, AnalysisState, CoverageGroup, CoverageTarget,
    DiagnosticFilter, OverlayLines, ReviewOptions, RuleCatalog, SeverityCounts, SeverityFilter,
    coverage_groups, coverage_summary, focus_bounds, line_severities, overlay_lines,
};
use crate::analysis_worker::{AnalysisWorker, DialectChoice};

use crate::file_browser::{
    BrowserEntry, FileBrowser, compact_path, default_root, display_path, format_size, read_program,
};
use crate::layout::{STATUS_BAR_HEIGHT, ShellLayout, TOUCH_TARGET};
use crate::program::SourceLine;
use crate::setup::{
    ADVISORY_NOTE, ExportFormat, MachineField, MaterialField, SimField, StatusMessage,
    ToleranceField, apply_tool_field, file_name_of, format_number, format_optional, overlay_tool,
    parse_non_negative, parse_optional_count, parse_optional_number, parse_optional_positive,
    provenance_label, store_overlay_tool, write_export,
};
use crate::sim::{
    FrameClock, HeightfieldCache, HeightfieldMesh, PlaybackSpeed, SegmentLines, SimCanvas,
    SimToggles, ToolChip, Transport, follow_visible_row, format_diameter, format_time,
    fraction_for_time, is_long_seek, parse_tool_edit, time_for_fraction, tool_chip,
};
use crate::sim_worker::SeekWorker;
use crate::state::{AppMode, LoadedProgram, NcaConfig, NcaState, ParseState};
use crate::theme::{Palette, ThemeMode, apply as apply_theme, move_color, move_style};
use crate::viewer::{self, HasViewer, ViewerHandle, ViewerMode, ViewerOverlay};

/// Height of the SIMULATE transport bar, in logical pixels.
const TRANSPORT_HEIGHT: f32 = 76.0;

/// One editable setup-sheet field.
///
/// Every field has a stable text input; structural choices (shape, dialect,
/// stock source, rule severity) are buttons instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SetupField {
    ConfigPath,
    StockOrigin(usize),
    StockSize(usize),
    ToolName,
    ToolDiameter,
    ToolFlutes,
    ToolStickout,
    ToolFluteLength,
    ToolTipAngle,
    ToolCornerRadius,
    ToolShankDiameter,
    ToolHolderDiameter,
    MaterialChiploadMin,
    MaterialChiploadMax,
    MaterialSurfaceSpeedMin,
    MaterialSurfaceSpeedMax,
    MaterialMaxMrr,
    MaterialNotes,
    MachineTravelMin(usize),
    MachineTravelMax(usize),
    MachineMaxFeed,
    MachineMaxSpindle,
    MachineRapid,
    ToleranceArcRadius,
    ToleranceMinSegment,
    ToleranceHoleXy,
    ToleranceClearance,
    SimCellMm,
    SimMaxCells,
    SimRapidRate,
    SimToolChangePause,
    SimAssumedDiameter,
    RuleThresholdKey,
    RuleThresholdValue,
    ExportPath,
}

impl SetupField {
    const ALL: &'static [Self] = &[
        Self::ConfigPath,
        Self::StockOrigin(0),
        Self::StockOrigin(1),
        Self::StockOrigin(2),
        Self::StockSize(0),
        Self::StockSize(1),
        Self::StockSize(2),
        Self::ToolName,
        Self::ToolDiameter,
        Self::ToolFlutes,
        Self::ToolStickout,
        Self::ToolFluteLength,
        Self::ToolTipAngle,
        Self::ToolCornerRadius,
        Self::ToolShankDiameter,
        Self::ToolHolderDiameter,
        Self::MaterialChiploadMin,
        Self::MaterialChiploadMax,
        Self::MaterialSurfaceSpeedMin,
        Self::MaterialSurfaceSpeedMax,
        Self::MaterialMaxMrr,
        Self::MaterialNotes,
        Self::MachineTravelMin(0),
        Self::MachineTravelMin(1),
        Self::MachineTravelMin(2),
        Self::MachineTravelMax(0),
        Self::MachineTravelMax(1),
        Self::MachineTravelMax(2),
        Self::MachineMaxFeed,
        Self::MachineMaxSpindle,
        Self::MachineRapid,
        Self::ToleranceArcRadius,
        Self::ToleranceMinSegment,
        Self::ToleranceHoleXy,
        Self::ToleranceClearance,
        Self::SimCellMm,
        Self::SimMaxCells,
        Self::SimRapidRate,
        Self::SimToolChangePause,
        Self::SimAssumedDiameter,
        Self::RuleThresholdKey,
        Self::RuleThresholdValue,
        Self::ExportPath,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ConfigPath => "CONFIG TOML",
            Self::StockOrigin(0) => "ORIGIN X",
            Self::StockOrigin(1) => "ORIGIN Y",
            Self::StockOrigin(2) => "ORIGIN Z",
            Self::StockSize(0) => "SIZE X",
            Self::StockSize(1) => "SIZE Y",
            Self::StockSize(2) => "SIZE Z",
            Self::ToolName => "NAME",
            Self::ToolDiameter => "DIAMETER MM",
            Self::ToolFlutes => "FLUTES",
            Self::ToolStickout => "STICKOUT MM",
            Self::ToolFluteLength => "FLUTE LENGTH",
            Self::ToolTipAngle => "TIP ANGLE DEG",
            Self::ToolCornerRadius => "CORNER RADIUS",
            Self::ToolShankDiameter => "SHANK DIA MM",
            Self::ToolHolderDiameter => "HOLDER DIA MM",
            Self::MaterialChiploadMin => "CHIPLOAD MIN",
            Self::MaterialChiploadMax => "CHIPLOAD MAX",
            Self::MaterialSurfaceSpeedMin => "SURFACE SPD MIN",
            Self::MaterialSurfaceSpeedMax => "SURFACE SPD MAX",
            Self::MaterialMaxMrr => "MAX MRR",
            Self::MaterialNotes => "NOTES",
            Self::MachineTravelMin(0) => "TRAVEL MIN X",
            Self::MachineTravelMin(1) => "TRAVEL MIN Y",
            Self::MachineTravelMin(2) => "TRAVEL MIN Z",
            Self::MachineTravelMax(0) => "TRAVEL MAX X",
            Self::MachineTravelMax(1) => "TRAVEL MAX Y",
            Self::MachineTravelMax(2) => "TRAVEL MAX Z",
            Self::MachineMaxFeed => "MAX FEED MM/MIN",
            Self::MachineMaxSpindle => "MAX SPINDLE RPM",
            Self::MachineRapid => "RAPID MM/MIN",
            Self::ToleranceArcRadius => "ARC RADIUS MM",
            Self::ToleranceMinSegment => "MIN SEGMENT MM",
            Self::ToleranceHoleXy => "HOLE XY MM",
            Self::ToleranceClearance => "CLEARANCE MM",
            Self::SimCellMm => "CELL MM",
            Self::SimMaxCells => "MAX CELLS",
            Self::SimRapidRate => "RAPID MM/MIN",
            Self::SimToolChangePause => "TOOL CHANGE S",
            Self::SimAssumedDiameter => "ASSUMED DIA MM",
            Self::RuleThresholdKey => "THRESHOLD KEY",
            Self::RuleThresholdValue => "THRESHOLD VALUE",
            Self::ExportPath => "EXPORT PATH",
            Self::StockOrigin(_) => "ORIGIN",
            Self::StockSize(_) => "SIZE",
            Self::MachineTravelMin(_) => "TRAVEL MIN",
            Self::MachineTravelMax(_) => "TRAVEL MAX",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::ConfigPath => "/path/to/config.toml",
            Self::ToolName => "optional",
            Self::MaterialNotes => "optional notes",
            Self::RuleThresholdKey => "e.g. clearance_mm",
            Self::RuleThresholdValue => "number",
            Self::ExportPath => "nca-report.json",
            _ => "empty = unknown",
        }
    }
}

/// One active simulation and the data the shell needs to paint it.
struct SimSession {
    simulation: Simulation,
    program: Arc<Program>,
    stock: StockModel,
    tools: ToolTable,
    segment_lines: Arc<SegmentLines>,
}

impl SimSession {
    fn active_tool(&self) -> u32 {
        let snapshot = self.simulation.snapshot();
        if snapshot.tool > 0 {
            snapshot.tool
        } else {
            self.program.stats.tools.first().copied().unwrap_or(0)
        }
    }

    fn active_cutter(&self) -> Option<Cutter> {
        self.simulation
            .cutter_for_tool(self.active_tool())
            .cloned()
            .or_else(|| self.simulation.current_cutter().cloned())
    }

    fn canvas(&self, mesh: Arc<HeightfieldMesh>, toggles: SimToggles) -> Option<SimCanvas> {
        let stock = self.stock.to_stock_box()?;
        let snapshot = self.simulation.snapshot();
        let cutter = self.active_cutter();
        let tool_assumed = cutter
            .as_ref()
            .is_some_and(|cutter| cutter.assumed || cutter.is_centerline());
        Some(SimCanvas {
            snapshot,
            mesh,
            stock,
            cutter,
            segment_lines: Arc::clone(&self.segment_lines),
            toggles,
            tool_assumed,
        })
    }
}

/// The shell view.
pub struct NcaShell {
    state: NcaState,
    viewer: ViewerHandle,
    mono: SharedString,
    program_scroll: UniformListScrollHandle,
    program_search_input: Entity<InputState>,
    browser_path_input: Entity<InputState>,
    setup_inputs: BTreeMap<SetupField, Entity<InputState>>,
    pending_setup_sync: bool,
    export_open: bool,
    browser: Option<FileBrowser>,
    legend_open: bool,
    load_error: Option<String>,
    pending_search_reset: bool,
    _load_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    sim: Option<SimSession>,
    transport: Transport,
    frame_clock: FrameClock,
    toggles: SimToggles,
    mesh_cache: HeightfieldCache,
    mesh_updated_at: Option<Instant>,
    seek_worker: Option<SeekWorker>,
    seeking: bool,
    seek_generation: Option<u64>,
    scrub_target: Option<f64>,
    sim_error: Option<String>,
    last_follow_line: Option<usize>,
    view_selected_line: Option<usize>,
    tool_popover_open: bool,
    tool_edit_number: Option<u32>,
    tool_shape_draft: ToolShape,
    tool_diameter_input: Entity<InputState>,
    tool_flutes_input: Entity<InputState>,
    tool_edit_error: Option<String>,
    scrub_slider: Entity<SliderState>,
    analysis: AnalysisState,
    analysis_worker: Option<AnalysisWorker>,
    analysis_run: u64,
    program_generation: u64,
    selected_diagnostic: Option<usize>,
    analysis_panel: Option<AnalysisPanel>,
    severity_filter: SeverityFilter,
    diagnostic_rule: String,
    diagnostic_search: String,
    diagnostic_rule_input: Entity<InputState>,
    diagnostic_search_input: Entity<InputState>,
    diagnostics_scroll: UniformListScrollHandle,
    rule_catalog: RuleCatalog,
    line_severity_map: Arc<BTreeMap<usize, AnalysisSeverity>>,
    overlay_lines: OverlayLines,
    pending_select_rule: Option<String>,
    settings_focus: Option<CoverageTarget>,
}

impl NcaShell {
    /// Builds the shell and loads the built-in demo.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        window: &mut Window,
        open_path: Option<PathBuf>,
        view_3d: bool,
        open_browser: bool,
        simulate: bool,
        sim_at: Option<f32>,
        autoplay: bool,
        review: ReviewOptions,
        initial_theme: ThemeMode,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = NcaState::new(NcaConfig::load());
        // A launch flag overrides the persisted mode for this run without
        // rewriting app.json; the settings selector persists explicit changes.
        state.config.theme = initial_theme;
        apply_theme(initial_theme, cx);

        let program_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.search.clone())
                .placeholder("Text, N120, or line 120")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let browser_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/path/to/folder")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let tool_diameter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("6.0")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let tool_flutes_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("2")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let diagnostic_rule_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Rule id or category")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let diagnostic_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Message, rule, or line")
                .clean_on_escape()
                .submit_on_enter(true)
        });
        let setup_inputs = create_setup_inputs(window, cx);
        let scrub_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.001)
                .default_value(0.0)
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(
            &program_search_input,
            |this, input, event, cx| match event {
                InputEvent::Change => {
                    this.state.search = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.select_first_match(cx);
                }
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(&browser_path_input, |this, input, event, cx| {
            if let InputEvent::Change = event {
                let draft = input.read(cx).value().to_string();
                if let Some(browser) = this.browser.as_mut() {
                    browser.set_path_draft(draft);
                }
            }
            if let InputEvent::PressEnter { .. } = event {
                this.navigate_browser(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&scrub_slider, |this, _, event, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => *value,
            };
            let SliderValue::Single(fraction) = value else {
                return;
            };
            this.scrub_to_fraction(f64::from(fraction), cx);
        }));
        subscriptions.push(cx.subscribe(
            &diagnostic_rule_input,
            |this, input, event, cx| match event {
                InputEvent::Change => {
                    this.diagnostic_rule = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.select_first_diagnostic(cx),
                _ => {}
            },
        ));
        subscriptions.push(
            cx.subscribe(
                &diagnostic_search_input,
                |this, input, event, cx| match event {
                    InputEvent::Change => {
                        this.diagnostic_search = input.read(cx).value().to_string();
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.select_first_diagnostic(cx),
                    _ => {}
                },
            ),
        );
        for (field, input) in setup_inputs.clone() {
            subscriptions.push(
                cx.subscribe(&input, move |this, input, event, cx| match event {
                    InputEvent::Change => {
                        this.setup_field_changed(field, input.read(cx).value().to_string(), cx);
                    }
                    InputEvent::PressEnter { .. } | InputEvent::Blur => {
                        this.commit_setup_field(field, cx);
                    }
                    InputEvent::Focus => {}
                }),
            );
        }

        let mono = cx.theme().mono_font_family.clone();
        let mut shell = Self {
            state,
            viewer: ViewerHandle::new(),
            mono,
            program_scroll: UniformListScrollHandle::new(),
            program_search_input,
            browser_path_input,
            setup_inputs,
            pending_setup_sync: false,
            export_open: review.export,
            browser: None,
            legend_open: false,
            load_error: None,
            pending_search_reset: false,
            _load_task: None,
            _subscriptions: subscriptions,
            sim: None,
            transport: Transport::new(),
            frame_clock: FrameClock::default(),
            toggles: SimToggles::default(),
            mesh_cache: HeightfieldCache::default(),
            mesh_updated_at: None,
            seek_worker: None,
            seeking: false,
            seek_generation: None,
            scrub_target: None,
            sim_error: None,
            last_follow_line: None,
            view_selected_line: None,
            tool_popover_open: false,
            tool_edit_number: None,
            tool_shape_draft: ToolShape::Flat,
            tool_diameter_input,
            tool_flutes_input,
            tool_edit_error: None,
            scrub_slider,
            analysis: AnalysisState::Idle,
            analysis_worker: None,
            analysis_run: 0,
            program_generation: 1,
            selected_diagnostic: None,
            analysis_panel: None,
            severity_filter: SeverityFilter::All,
            diagnostic_rule: String::new(),
            diagnostic_search: String::new(),
            diagnostic_rule_input,
            diagnostic_search_input,
            diagnostics_scroll: UniformListScrollHandle::new(),
            rule_catalog: RuleCatalog::load(),
            line_severity_map: Arc::new(BTreeMap::new()),
            overlay_lines: OverlayLines::default(),
            pending_select_rule: review.select_rule.clone(),
            settings_focus: None,
        };
        shell.analysis_panel = review.panel;
        if review.analyze && shell.analysis_panel.is_none() {
            shell.analysis_panel = Some(AnalysisPanel::Diagnostics);
        }
        if review.setup {
            shell.state.settings_open = true;
        }
        shell.sync_setup_inputs(window, cx);
        if let Some(path) = open_path {
            shell.load_initial(path);
        }
        if view_3d {
            shell.viewer.set_mode(ViewerMode::ThreeD);
        }
        if open_browser {
            shell.open_browser(window, cx);
        }
        if simulate {
            shell.enter_simulate(cx);
            if let Some(fraction) = sim_at {
                shell.startup_seek(window, f64::from(fraction), cx);
            }
            if autoplay {
                shell.start_playback(cx);
            }
        }
        shell.begin_analysis(cx);
        shell
    }

    /// Builds a deterministic simulation for the loaded program.
    fn build_sim_session(&mut self) -> Result<(), String> {
        let loaded = self
            .state
            .loaded
            .as_ref()
            .ok_or_else(|| "No program is open.".to_string())?;
        let program = loaded.program.as_ref().cloned().ok_or_else(|| {
            "The file has parse errors, so there is no geometry to simulate.".to_string()
        })?;
        let inferred = loaded.stock;
        let stock = self
            .state
            .setup
            .effective_stock(inferred)
            .ok_or_else(|| "No stock box is available for this file.".to_string())?;
        let tools = self.state.resolved_tools(&program);
        let config = self.state.setup.sim.sanitized();
        let simulation = Simulation::new(&program, stock, &tools, &config)
            .map_err(|error| format!("Cannot simulate this program: {error}"))?;
        let segment_lines = Arc::new(SegmentLines::from_program(&program));
        self.sim = Some(SimSession {
            simulation,
            program,
            stock,
            tools,
            segment_lines,
        });
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        Ok(())
    }

    fn enter_simulate(&mut self, cx: &mut Context<Self>) {
        if self.state.mode == AppMode::Simulate {
            return;
        }
        let previous_selection = self.state.selected_line;
        match self.build_sim_session() {
            Ok(()) => {
                self.view_selected_line = previous_selection;
                self.state.mode = AppMode::Simulate;
                self.sim_error = None;
                self.transport.stop();
                self.frame_clock.reset();
                self.toggles.stock = self.viewer.stock_visible();
                self.refresh_sim_mesh(true);
                self.follow_current_line(true, cx);
            }
            Err(message) => {
                self.sim = None;
                self.mesh_cache.clear();
                self.sim_error = Some(message);
                self.state.mode = AppMode::View;
            }
        }
        cx.notify();
    }

    fn leave_simulate(&mut self, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            return;
        }
        self.cancel_seek();
        self.transport.pause();
        self.frame_clock.reset();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.state.mode = AppMode::View;
        self.state.selected_line = self.view_selected_line.take();
        cx.notify();
    }

    /// Drops playback and rebuilds if SIMULATE is still the active mode.
    fn program_changed(&mut self, cx: &mut Context<Self>) {
        self.pending_setup_sync = true;
        self.cancel_seek();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.view_selected_line = None;
        self.frame_clock.reset();
        self.transport.stop();
        self.sim_error = None;
        if self.state.mode == AppMode::Simulate {
            match self.build_sim_session() {
                Ok(()) => {
                    self.toggles.stock = self.viewer.stock_visible();
                    self.refresh_sim_mesh(true);
                    self.follow_current_line(true, cx);
                }
                Err(message) => {
                    self.sim = None;
                    self.mesh_cache.clear();
                    self.sim_error = Some(message);
                    self.state.mode = AppMode::View;
                }
            }
        }
        cx.notify();
    }

    fn cancel_seek(&mut self) {
        if let Some(worker) = self.seek_worker.as_mut() {
            worker.cancel();
        }
        self.seeking = false;
        self.seek_generation = None;
        self.scrub_target = None;
    }

    fn diagnostic_filter(&self) -> DiagnosticFilter {
        DiagnosticFilter::new(
            self.severity_filter,
            &self.diagnostic_rule,
            &self.diagnostic_search,
        )
    }

    /// Drops the report-specific view caches and the selected finding.
    fn clear_analysis_views(&mut self) {
        self.selected_diagnostic = None;
        self.line_severity_map = Arc::new(BTreeMap::new());
        self.overlay_lines = OverlayLines::default();
    }

    /// Starts analysis on the worker thread for the currently loaded program.
    ///
    /// The resolved tool table is the only optional input the app has in v0.1:
    /// stock stays inferred, machine and material stay unset, tolerances and
    /// rules stay at their defaults, and `Generic` is explicitly labeled as an
    /// assumed dialect.
    fn begin_analysis(&mut self, cx: &mut Context<Self>) {
        self.state.setup.export_status = None;
        self.analysis_run = self.analysis_run.wrapping_add(1);
        if let Some(worker) = self.analysis_worker.as_mut() {
            worker.cancel(self.analysis_run);
        }
        let generation = AnalysisGeneration {
            run: self.analysis_run,
            program: self.program_generation,
        };
        self.clear_analysis_views();
        self.analysis.begin(generation);

        let Some(loaded) = self.state.loaded.as_ref() else {
            self.analysis = AnalysisState::Failed {
                generation,
                message: "No program is open, so there is nothing to analyze.".to_string(),
            };
            cx.notify();
            return;
        };
        let Some(program) = loaded.program.as_ref().cloned() else {
            self.analysis = AnalysisState::Failed {
                generation,
                message: "The file has parse errors, so no program reached the analysis engine."
                    .to_string(),
            };
            cx.notify();
            return;
        };
        let file_name = loaded.name.clone();
        let source = Arc::clone(&loaded.source);
        let tools = self.state.resolved_tools(&program);
        let config = self.state.setup.analysis_config(tools);
        let dialect = DialectChoice {
            dialect: self.state.setup.dialect,
            assumed: self.state.setup.dialect_assumed,
        };

        if self.analysis_worker.is_none() {
            match AnalysisWorker::spawn() {
                Ok(worker) => self.analysis_worker = Some(worker),
                Err(message) => {
                    self.analysis = AnalysisState::Failed {
                        generation,
                        message,
                    };
                    cx.notify();
                    return;
                }
            }
        }
        let result = self
            .analysis_worker
            .as_mut()
            .map(|worker| worker.request(generation, program, file_name, source, config, dialect));
        match result {
            Some(Ok(())) => {}
            Some(Err(message)) => {
                self.analysis = AnalysisState::Failed {
                    generation,
                    message,
                };
            }
            None => {
                self.analysis = AnalysisState::Failed {
                    generation,
                    message: "The analysis worker is unavailable.".to_string(),
                };
            }
        }
        cx.notify();
    }

    /// Cancels the current request. A worker already inside the engine finishes
    /// its computation, but its stale result is discarded.
    fn cancel_analysis(&mut self, cx: &mut Context<Self>) {
        self.analysis_run = self.analysis_run.wrapping_add(1);
        if let Some(worker) = self.analysis_worker.as_mut() {
            worker.cancel(self.analysis_run);
        }
        self.analysis = AnalysisState::Canceled;
        self.clear_analysis_views();
        cx.notify();
    }

    /// Polls completed analyses. Only the current generation can replace state.
    fn poll_analysis(&mut self, cx: &mut Context<Self>) {
        let mut completed = Vec::new();
        if let Some(worker) = self.analysis_worker.as_ref() {
            while let Some(response) = worker.try_recv() {
                completed.push(response);
            }
        }
        let mut changed = false;
        for response in completed {
            if self.analysis.complete(response.generation, response.result) {
                changed = true;
                if let Some(report) = self.analysis.report().cloned() {
                    self.apply_report_views(&report, cx);
                }
            }
        }
        if self.analysis.is_running()
            && self
                .analysis_worker
                .as_ref()
                .is_some_and(|worker| !worker.is_alive())
            && let AnalysisState::Running(generation) = self.analysis
        {
            self.analysis = AnalysisState::Failed {
                generation,
                message: "The analysis worker stopped unexpectedly.".to_string(),
            };
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Rebuilds gutter and overlay caches and applies a review selection.
    fn apply_report_views(&mut self, report: &Arc<Report>, cx: &mut Context<Self>) {
        self.line_severity_map = Arc::new(line_severities(report));
        self.overlay_lines = overlay_lines(report);
        let pending = self.pending_select_rule.take();
        if let Some(rule) = pending
            && let Some(index) = report
                .diagnostics
                .iter()
                .position(|diagnostic| diagnostic.rule.as_str() == rule)
        {
            self.analysis_panel = Some(AnalysisPanel::Diagnostics);
            self.select_diagnostic(index, cx);
        }
    }

    fn select_first_diagnostic(&mut self, cx: &mut Context<Self>) {
        let Some(report) = self.analysis.report().cloned() else {
            return;
        };
        let Some(index) = self.diagnostic_filter().indices(&report).first().copied() else {
            return;
        };
        self.analysis_panel = Some(AnalysisPanel::Diagnostics);
        self.select_diagnostic(index, cx);
    }

    /// Selects one finding: highlights and scrolls its source line and frames
    /// the segments it generated. The viewer stays interactive.
    fn select_diagnostic(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(report) = self.analysis.report().cloned() else {
            return;
        };
        let Some(diagnostic) = report.diagnostics.get(index) else {
            return;
        };
        let line = diagnostic.line;
        let line_end = diagnostic.line_end;
        self.selected_diagnostic = Some(index);
        self.state.selected_line = Some(line);
        self.state.ensure_index();
        let mut visible = self
            .state
            .index()
            .and_then(|index| index.visible_index(line.saturating_sub(1)));
        if visible.is_none() && !self.state.search.trim().is_empty() {
            // A diagnostic click must reveal its source line, so drop a search
            // filter that hides it. The input is cleared on the next frame.
            self.state.search.clear();
            self.pending_search_reset = true;
            self.state.ensure_index();
            visible = self
                .state
                .index()
                .and_then(|index| index.visible_index(line.saturating_sub(1)));
        }
        if let Some(visible) = visible {
            self.program_scroll
                .scroll_to_item(visible, ScrollStrategy::Center);
        }
        self.focus_line(line, line_end);
        cx.notify();
    }

    /// Frames the world bounds of a source line in the active projection.
    fn focus_line(&mut self, line: usize, line_end: usize) {
        let Some(program) = self
            .state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.program.as_ref().cloned())
        else {
            return;
        };
        let Some(bounds) = focus_bounds(&program, line, line_end) else {
            return;
        };
        if self.viewer.mode().is_three_d() {
            self.viewer.focus_3d(bounds);
        } else {
            self.viewer.focus_2d(bounds.xy_bounds());
        }
    }

    /// Applies a startup seek synchronously so a screenshot is reproducible.
    fn startup_seek(&mut self, window: &mut Window, fraction: f64, cx: &mut Context<Self>) {
        let Some(actual) = self.sim.as_mut().map(|session| {
            let total = session.simulation.timeline().total_time_s();
            let target = time_for_fraction(fraction, total);
            session.simulation.seek_to_time(target);
            session.simulation.snapshot().progress
        }) else {
            return;
        };
        self.transport.pause();
        self.frame_clock.reset();
        self.cancel_seek();
        self.mesh_cache.clear();
        self.refresh_sim_mesh(true);
        self.last_follow_line = None;
        self.follow_current_line(true, cx);
        self.set_scrub_slider(actual, window, cx);
        cx.notify();
    }

    fn refresh_sim_mesh(&mut self, force: bool) {
        let now = Instant::now();
        let playing = self.transport.is_playing();
        if !force
            && playing
            && let Some(last) = self.mesh_updated_at
            && now.duration_since(last) < Duration::from_millis(33)
        {
            return;
        }
        let Some(session) = self.sim.as_ref() else {
            return;
        };
        let base = session.simulation.config().render_resolution;
        let resolution = if playing {
            base
        } else {
            (base.saturating_mul(2)).clamp(8, 512)
        };
        let _ = self
            .mesh_cache
            .ensure(session.simulation.heightmap(), &session.stock, resolution);
        self.mesh_updated_at = Some(now);
    }

    fn sim_canvas(&self) -> Option<SimCanvas> {
        let session = self.sim.as_ref()?;
        let mesh = self.mesh_cache.current()?;
        session.canvas(mesh, self.toggles)
    }

    fn follow_current_line(&mut self, scroll: bool, cx: &mut Context<Self>) {
        let Some(line) = self
            .sim
            .as_ref()
            .map(|session| session.simulation.snapshot().current_line)
        else {
            return;
        };
        if line == 0 {
            return;
        }
        let line_changed = self.last_follow_line != Some(line);
        if line_changed {
            self.last_follow_line = Some(line);
        }
        if self.state.selected_line != Some(line) {
            self.state.selected_line = Some(line);
        }
        if scroll && line_changed {
            self.state.ensure_index();
            if let Some(index) = self.state.index().cloned()
                && let Some(row) = follow_visible_row(&index, line)
            {
                self.program_scroll
                    .scroll_to_item(row, ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    fn set_scrub_slider(&mut self, fraction: f64, window: &mut Window, cx: &mut Context<Self>) {
        #[allow(clippy::cast_possible_truncation)]
        let value = fraction.clamp(0.0, 1.0) as f32;
        self.scrub_slider.update(cx, |state, cx| {
            state.set_value(SliderValue::Single(value), window, cx);
        });
    }

    fn sync_scrub_slider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sim.as_ref() else {
            return;
        };
        #[allow(clippy::cast_possible_truncation)]
        let fraction =
            self.scrub_target
                .unwrap_or_else(|| session.simulation.snapshot().progress) as f32;
        let current = match self.scrub_slider.read(cx).value() {
            SliderValue::Single(value) => value,
            SliderValue::Range(_, end) => end,
        };
        if (current - fraction).abs() > 0.0005 {
            self.set_scrub_slider(f64::from(fraction), window, cx);
        }
    }

    fn tick_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            self.frame_clock.reset();
            return;
        }
        if self.seeking {
            window.request_animation_frame();
            return;
        }
        if !self.transport.is_playing() {
            self.frame_clock.reset();
            return;
        }
        let now = Instant::now();
        let dt = self.frame_clock.step(now);
        if dt > 0.0 {
            let factor = self.transport.speed().factor();
            let mut finished = false;
            if let Some(session) = self.sim.as_mut() {
                session.simulation.advance(dt * factor);
                let snapshot = session.simulation.snapshot();
                let total = session.simulation.timeline().total_time_s();
                finished = total > 0.0 && snapshot.time_s + 1.0e-9 >= total;
            }
            if finished {
                self.transport.pause();
                self.frame_clock.reset();
            }
            self.follow_current_line(true, cx);
            self.refresh_sim_mesh(false);
        }
        window.request_animation_frame();
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        if self.state.mode == AppMode::Simulate && self.sim.is_some() {
            self.transport.play();
            self.frame_clock.reset();
            cx.notify();
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate || self.sim.is_none() || self.seeking {
            return;
        }
        if self.transport.is_playing() {
            self.transport.pause();
            self.frame_clock.reset();
        } else {
            let at_end = self
                .sim
                .as_ref()
                .is_some_and(|session| session.simulation.snapshot().progress >= 1.0);
            if at_end && let Some(session) = self.sim.as_mut() {
                session.simulation.reset();
                self.mesh_cache.clear();
                self.mesh_updated_at = None;
            }
            self.transport.play();
            self.frame_clock.reset();
            self.follow_current_line(true, cx);
        }
        cx.notify();
    }

    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.stop();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.reset();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn step_motion(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.step();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.step_motion();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn step_line(&mut self, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.transport.step();
        self.frame_clock.reset();
        if let Some(session) = self.sim.as_mut() {
            session.simulation.step_line();
        }
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.follow_current_line(true, cx);
        self.refresh_sim_mesh(true);
        cx.notify();
    }

    fn scrub_to_fraction(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let Some(total) = self
            .sim
            .as_ref()
            .map(|session| session.simulation.timeline().total_time_s())
        else {
            return;
        };
        let target = time_for_fraction(fraction, total);
        self.scrub_target = Some(fraction.clamp(0.0, 1.0));
        self.request_seek_to_time(target, cx);
    }

    fn seek_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if self.state.mode != AppMode::Simulate {
            self.state.selected_line = Some(line);
            cx.notify();
            return;
        }
        let Some((target, total)) = self.sim.as_ref().map(|session| {
            let total = session.simulation.timeline().total_time_s();
            let target = session
                .simulation
                .timeline()
                .time_for_line_at_or_after(line)
                .unwrap_or(total);
            (target, total)
        }) else {
            self.state.selected_line = Some(line);
            cx.notify();
            return;
        };
        self.scrub_target = Some(fraction_for_time(target, total));
        self.request_seek_to_time(target, cx);
    }

    fn request_seek_to_time(&mut self, target_s: f64, cx: &mut Context<Self>) {
        let Some((total, current, program, stock, tools, config)) =
            self.sim.as_ref().map(|session| {
                (
                    session.simulation.timeline().total_time_s(),
                    session.simulation.snapshot().time_s,
                    Arc::clone(&session.program),
                    session.stock,
                    session.tools.clone(),
                    session.simulation.config().clone(),
                )
            })
        else {
            return;
        };
        let target = target_s.clamp(0.0, total);
        self.transport.pause();
        self.frame_clock.reset();
        if !is_long_seek(current, target) {
            if let Some(session) = self.sim.as_mut() {
                session.simulation.seek_to_time(target);
            }
            self.seeking = false;
            self.seek_generation = None;
            self.scrub_target = None;
            self.mesh_cache.clear();
            self.mesh_updated_at = None;
            self.follow_current_line(true, cx);
            self.refresh_sim_mesh(true);
            cx.notify();
            return;
        }

        if self.seek_worker.is_none() {
            match SeekWorker::spawn() {
                Ok(worker) => self.seek_worker = Some(worker),
                Err(message) => {
                    self.sim_error = Some(message);
                    self.seeking = false;
                    self.scrub_target = None;
                    cx.notify();
                    return;
                }
            }
        }
        self.seeking = true;
        if let Some(worker) = self.seek_worker.as_mut() {
            match worker.request(program, stock, tools, config, target) {
                Ok(generation) => self.seek_generation = Some(generation),
                Err(message) => {
                    self.sim_error = Some(message);
                    self.seeking = false;
                    self.seek_generation = None;
                    self.scrub_target = None;
                }
            }
        }
        cx.notify();
    }

    fn poll_seek(&mut self, cx: &mut Context<Self>) {
        let Some(worker) = self.seek_worker.as_ref() else {
            return;
        };
        let mut response = None;
        while let Some(candidate) = worker.try_recv() {
            if Some(candidate.generation) == self.seek_generation {
                response = Some(candidate);
                break;
            }
        }
        let Some(response) = response else {
            if self.seeking {
                let dead = self
                    .seek_worker
                    .as_ref()
                    .is_some_and(|worker| !worker.is_alive());
                if dead {
                    self.seeking = false;
                    self.seek_generation = None;
                    self.scrub_target = None;
                    self.sim_error = Some("The seek worker stopped unexpectedly.".to_string());
                    cx.notify();
                }
            }
            return;
        };
        self.seeking = false;
        self.seek_generation = None;
        self.scrub_target = None;
        match response.result {
            Ok(simulation) => {
                if let Some(session) = self.sim.as_mut() {
                    session.simulation = simulation;
                }
                self.transport.pause();
                self.frame_clock.reset();
                self.mesh_cache.clear();
                self.mesh_updated_at = None;
                self.follow_current_line(true, cx);
                self.refresh_sim_mesh(true);
            }
            Err(message) => {
                self.sim_error = Some(format!("Seek failed: {message}"));
            }
        }
        cx.notify();
    }

    fn resolved_tools(&self) -> ToolTable {
        self.state
            .loaded
            .as_ref()
            .and_then(|loaded| {
                loaded
                    .program
                    .as_ref()
                    .map(|program| self.state.resolved_tools(program))
            })
            .unwrap_or_default()
    }

    fn active_tool_number(&self) -> u32 {
        if let Some(session) = self.sim.as_ref() {
            return session.active_tool();
        }
        self.state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.program.as_ref())
            .and_then(|program| {
                program
                    .stats
                    .tools
                    .iter()
                    .copied()
                    .find(|tool| *tool > 0)
                    .or_else(|| program.stats.tools.first().copied())
            })
            .unwrap_or(0)
    }

    fn current_tool_chip(&self) -> ToolChip {
        if let Some(session) = self.sim.as_ref() {
            let tool = session.active_tool();
            let cutter = session.active_cutter();
            return tool_chip(tool, session.tools.get(tool), cutter.as_ref());
        }
        let Some(program) = self
            .state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.program.as_ref())
        else {
            return ToolChip {
                tool: 0,
                text: "NO TOOL".to_string(),
                assumed: false,
            };
        };
        let tool = program
            .stats
            .tools
            .iter()
            .copied()
            .find(|tool| *tool > 0)
            .or_else(|| program.stats.tools.first().copied())
            .unwrap_or(0);
        if tool == 0 {
            return ToolChip {
                tool: 0,
                text: "NO TOOL".to_string(),
                assumed: false,
            };
        }
        let tools = self.resolved_tools();
        let cutter = tools
            .get(tool)
            .map(|entry| Cutter::from_tool(entry, None))
            .unwrap_or_else(Cutter::centerline);
        tool_chip(tool, tools.get(tool), Some(&cutter))
    }

    fn open_tool_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tool = self.active_tool_number();
        self.open_tool_popover_for(tool, window, cx);
    }

    /// Opens the quick tool editor for one tool number.
    fn open_tool_popover_for(&mut self, tool: u32, window: &mut Window, cx: &mut Context<Self>) {
        if tool == 0 {
            self.tool_edit_number = None;
            self.tool_edit_error = Some("No tool is named by this program.".to_string());
            self.tool_popover_open = true;
            cx.notify();
            return;
        }
        let tools = if let Some(session) = self.sim.as_ref() {
            session.tools.clone()
        } else {
            self.resolved_tools()
        };
        let entry = tools.get(tool).cloned().unwrap_or_else(|| Tool::new(tool));
        self.tool_shape_draft = entry.shape;
        let diameter = entry.diameter_mm.map(format_diameter).unwrap_or_default();
        let flutes = entry
            .flutes
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.tool_diameter_input.update(cx, |input, cx| {
            input.set_value(diameter, window, cx);
        });
        self.tool_flutes_input.update(cx, |input, cx| {
            input.set_value(flutes, window, cx);
        });
        self.tool_edit_number = Some(tool);
        self.tool_edit_error = None;
        self.tool_popover_open = true;
        cx.notify();
    }

    fn apply_tool_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(number) = self.tool_edit_number else {
            self.tool_popover_open = false;
            cx.notify();
            return;
        };
        let diameter_text = self.tool_diameter_input.read(cx).value().to_string();
        let flutes_text = self.tool_flutes_input.read(cx).value().to_string();
        let Some(hash) = self.state.loaded_hash().map(str::to_string) else {
            self.tool_edit_error = Some("No program is open.".to_string());
            cx.notify();
            return;
        };
        let table = self.state.config.tools_for(&hash);
        let existing = table.get(number);
        let tool = match parse_tool_edit(
            number,
            &diameter_text,
            &flutes_text,
            self.tool_shape_draft,
            existing,
        ) {
            Ok(tool) => tool,
            Err(message) => {
                self.tool_edit_error = Some(message);
                cx.notify();
                return;
            }
        };
        store_overlay_tool(&mut self.state.config, &hash, tool);
        self.state.sync_setup_program();
        self.tool_popover_open = false;
        self.tool_edit_error = None;
        self.sync_setup_inputs(window, cx);
        if self.state.mode == AppMode::Simulate {
            self.rebuild_after_tool_edit(window, cx);
        } else {
            cx.notify();
        }
        self.begin_analysis(cx);
    }

    fn rebuild_after_tool_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_seek();
        self.sim = None;
        self.mesh_cache.clear();
        self.mesh_updated_at = None;
        self.last_follow_line = None;
        self.transport.stop();
        self.frame_clock.reset();
        match self.build_sim_session() {
            Ok(()) => {
                self.sim_error = None;
                self.toggles.stock = self.viewer.stock_visible();
                self.refresh_sim_mesh(true);
                self.follow_current_line(true, cx);
            }
            Err(message) => {
                self.sim = None;
                self.mesh_cache.clear();
                self.sim_error = Some(message);
                self.state.mode = AppMode::View;
            }
        }
        cx.notify();
    }

    fn load_initial(&mut self, path: PathBuf) {
        match read_program(&path) {
            Ok(file) => {
                self.program_generation = self.program_generation.wrapping_add(1);
                let dialect = self.state.setup.dialect;
                self.state.adopt(LoadedProgram::from_file(file, dialect));
                self.pending_setup_sync = true;
                self.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.viewer.fit();
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
            }
        }
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.load_error = None;
        self.state.loading = true;
        cx.notify();
        let task_path = path.clone();
        let dialect = self.state.setup.dialect;
        self._load_task = Some(cx.spawn(async move |weak, cx| {
            let task_dialect = dialect;
            let result = cx
                .background_executor()
                .spawn(async move {
                    match read_program(&task_path) {
                        Ok(file) => Ok(LoadedProgram::from_file(file, task_dialect)),
                        Err(error) => Err(error.to_string()),
                    }
                })
                .await;
            let _ = weak.update(cx, |shell, cx| {
                match result {
                    Ok(mut loaded) => {
                        if shell.state.setup.dialect != dialect {
                            loaded.reparse(shell.state.setup.dialect);
                        }
                        shell.program_generation = shell.program_generation.wrapping_add(1);
                        shell.state.adopt(loaded);
                        shell.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        shell.browser = None;
                        shell.load_error = None;
                        shell.pending_search_reset = true;
                        shell.viewer.fit();
                        shell.program_changed(cx);
                        shell.begin_analysis(cx);
                    }
                    Err(message) => {
                        shell.state.loading = false;
                        shell.load_error = Some(message);
                        if let Some(browser) = shell.browser.as_mut() {
                            browser.set_error("Could not load the selected file");
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    fn open_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .state
            .config
            .last_dir
            .clone()
            .or_else(|| {
                self.state
                    .loaded
                    .as_ref()
                    .and_then(|loaded| loaded.path.as_deref())
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(default_root);
        self.browser = Some(FileBrowser::open(&start));
        self.sync_browser_input(window, cx);
        cx.notify();
    }

    fn sync_browser_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .browser
            .as_ref()
            .map_or_else(String::new, FileBrowser::path_display);
        self.browser_path_input
            .update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn navigate_browser(&mut self, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        if !browser.commit_path() {
            cx.notify();
            return;
        }
        cx.notify();
    }

    fn browser_click(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        let is_dir = browser
            .entries()
            .get(index)
            .is_some_and(|entry| entry.is_dir);
        if is_dir {
            if browser.enter(index) {
                self.sync_browser_input(window, cx);
            }
        } else {
            browser.select(index);
        }
        cx.notify();
    }

    fn browser_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        if browser.up() {
            self.sync_browser_input(window, cx);
        }
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let path = self
            .browser
            .as_ref()
            .and_then(FileBrowser::selected_entry)
            .map(|entry| entry.path.clone());
        if let Some(path) = path {
            self.open_path(path, cx);
        }
    }

    fn close_browser(&mut self, cx: &mut Context<Self>) {
        self.browser = None;
        cx.notify();
    }

    fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.state.config.set_theme(mode);
        apply_theme(mode, cx);
        cx.notify();
    }

    fn select_first_match(&mut self, cx: &mut Context<Self>) {
        self.state.ensure_index();
        let Some(loaded) = self.state.loaded.as_ref() else {
            return;
        };
        let index = self
            .state
            .index()
            .cloned()
            .unwrap_or_else(|| loaded.index(&self.state.search));
        let Some(source) = index.first_match() else {
            return;
        };
        let Some(line) = loaded.lines.get(source) else {
            return;
        };
        self.state.selected_line = Some(line.number);
        if let Some(visible) = index.visible_index(source) {
            self.program_scroll
                .scroll_to_item(visible, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.search.clear();
        self.program_search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.program_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn drop_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        match paths {
            [path] if path.is_file() => self.open_path(path.clone(), cx),
            [] => {
                self.load_error =
                    Some("Drop one .nc, .gcode, .tap, or .ngc program file".to_string());
                cx.notify();
            }
            [path] => {
                self.load_error = Some(format!("'{}' is not a regular file", display_path(path)));
                cx.notify();
            }
            _ => {
                self.load_error = Some("Drop exactly one program file".to_string());
                cx.notify();
            }
        }
    }

    /// Reads one setup input, when it exists.
    fn setup_value(&self, field: SetupField, cx: &Context<Self>) -> String {
        self.setup_inputs
            .get(&field)
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Updates one setup input from its model value.
    fn set_setup_value(
        &self,
        field: SetupField,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = self.setup_inputs.get(&field).cloned() {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    fn sync_setup_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inferred = self.state.loaded.as_ref().and_then(|loaded| loaded.stock);
        let resolved = self.resolved_tools();
        let selected_tool = self
            .state
            .setup
            .selected_tool
            .and_then(|number| resolved.get(number).cloned());
        let (origin, size) = self.state.setup.stock_values(inferred.as_ref());
        let material = self.state.setup.analysis.material.clone();
        let machine = self.state.setup.analysis.machine.clone();
        let tolerances = self.state.setup.analysis.tolerances;
        let sim = self.state.setup.sim.clone();
        for &field in SetupField::ALL {
            let value = match field {
                SetupField::ConfigPath => self.state.setup.config_path_text.clone(),
                SetupField::ExportPath => self.state.setup.export_path.clone(),
                SetupField::StockOrigin(axis) => format_number(origin[axis]),
                SetupField::StockSize(axis) => format_number(size[axis]),
                SetupField::ToolName => selected_tool
                    .as_ref()
                    .and_then(|tool| tool.name.clone())
                    .unwrap_or_default(),
                SetupField::ToolDiameter => {
                    format_optional(selected_tool.as_ref().and_then(|tool| tool.diameter_mm))
                }
                SetupField::ToolFlutes => selected_tool
                    .as_ref()
                    .and_then(|tool| tool.flutes)
                    .map_or_else(String::new, |value| value.to_string()),
                SetupField::ToolStickout => {
                    format_optional(selected_tool.as_ref().and_then(|tool| tool.stickout_mm))
                }
                SetupField::ToolFluteLength => {
                    format_optional(selected_tool.as_ref().and_then(|tool| tool.flute_length_mm))
                }
                SetupField::ToolTipAngle => {
                    format_optional(selected_tool.as_ref().and_then(|tool| tool.tip_angle_deg))
                }
                SetupField::ToolCornerRadius => format_optional(
                    selected_tool
                        .as_ref()
                        .and_then(|tool| tool.corner_radius_mm),
                ),
                SetupField::ToolShankDiameter => format_optional(
                    selected_tool
                        .as_ref()
                        .and_then(|tool| tool.shank_diameter_mm),
                ),
                SetupField::ToolHolderDiameter => format_optional(
                    selected_tool
                        .as_ref()
                        .and_then(|tool| tool.holder_diameter_mm),
                ),
                SetupField::MaterialChiploadMin => {
                    format_optional(material.as_ref().and_then(|preset| preset.chipload_min_mm))
                }
                SetupField::MaterialChiploadMax => {
                    format_optional(material.as_ref().and_then(|preset| preset.chipload_max_mm))
                }
                SetupField::MaterialSurfaceSpeedMin => format_optional(
                    material
                        .as_ref()
                        .and_then(|preset| preset.surface_speed_min_mpm),
                ),
                SetupField::MaterialSurfaceSpeedMax => format_optional(
                    material
                        .as_ref()
                        .and_then(|preset| preset.surface_speed_max_mpm),
                ),
                SetupField::MaterialMaxMrr => {
                    format_optional(material.as_ref().and_then(|preset| preset.max_mrr_mm3_min))
                }
                SetupField::MaterialNotes => material
                    .as_ref()
                    .map(|preset| preset.notes.clone())
                    .unwrap_or_default(),
                SetupField::MachineTravelMin(axis) => {
                    format_optional(machine.as_ref().map(|machine| machine.travel_min_mm[axis]))
                }
                SetupField::MachineTravelMax(axis) => {
                    format_optional(machine.as_ref().map(|machine| machine.travel_max_mm[axis]))
                }
                SetupField::MachineMaxFeed => {
                    format_optional(machine.as_ref().and_then(|machine| machine.max_feed_mm_min))
                }
                SetupField::MachineMaxSpindle => {
                    format_optional(machine.as_ref().and_then(|machine| machine.max_spindle_rpm))
                }
                SetupField::MachineRapid => format_optional(
                    machine
                        .as_ref()
                        .and_then(|machine| machine.rapid_rate_mm_min),
                ),
                SetupField::ToleranceArcRadius => format_number(tolerances.arc_radius_mm),
                SetupField::ToleranceMinSegment => format_number(tolerances.min_segment_mm),
                SetupField::ToleranceHoleXy => format_number(tolerances.hole_xy_mm),
                SetupField::ToleranceClearance => format_number(tolerances.clearance_mm),
                SetupField::SimCellMm => format_number(sim.cell_mm),
                SetupField::SimMaxCells => sim.max_cells.to_string(),
                SetupField::SimRapidRate => format_number(sim.rapid_rate_mm_min),
                SetupField::SimToolChangePause => format_number(sim.tool_change_pause_s),
                SetupField::SimAssumedDiameter => format_optional(sim.assumed_tool_diameter_mm),
                SetupField::RuleThresholdKey | SetupField::RuleThresholdValue => String::new(),
            };
            // Transient rule-entry fields are cleared, never seeded.
            self.set_setup_value(field, value, window, cx);
        }
    }

    /// Live text changes: only the two path drafts update as the operator types.
    fn setup_field_changed(&mut self, field: SetupField, value: String, cx: &mut Context<Self>) {
        match field {
            SetupField::ConfigPath => self.state.setup.config_path_text = value,
            SetupField::ExportPath => {
                self.state.setup.export_path = value;
                self.state.setup.export_status = None;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Commits one field on Enter or focus loss.
    fn commit_setup_field(&mut self, field: SetupField, cx: &mut Context<Self>) {
        let raw = self.setup_value(field, cx);
        let inferred = self.state.loaded.as_ref().and_then(|loaded| loaded.stock);
        let mut tool_edit = false;
        match field {
            SetupField::ConfigPath | SetupField::ExportPath => {
                self.setup_field_changed(field, raw, cx);
                return;
            }
            SetupField::StockOrigin(axis) => match parse_optional_number(&raw, field.label()) {
                Ok(Some(value)) => {
                    if self.state.setup.analysis.stock.is_none() {
                        self.state.setup.set_stock_explicit(inferred.as_ref());
                    }
                    self.state.setup.set_stock_axis(true, axis, value);
                }
                Ok(None) => {
                    self.state.setup.message =
                        Some(StatusMessage::error("Stock origin values cannot be empty."));
                }
                Err(message) => self.state.setup.message = Some(StatusMessage::error(message)),
            },
            SetupField::StockSize(axis) => match parse_optional_positive(&raw, field.label()) {
                Ok(Some(value)) => {
                    if self.state.setup.analysis.stock.is_none() {
                        self.state.setup.set_stock_explicit(inferred.as_ref());
                    }
                    self.state.setup.set_stock_axis(false, axis, value);
                }
                Ok(None) => {
                    self.state.setup.message =
                        Some(StatusMessage::error("Stock size values cannot be empty."));
                }
                Err(message) => self.state.setup.message = Some(StatusMessage::error(message)),
            },
            SetupField::ToolName
            | SetupField::ToolDiameter
            | SetupField::ToolFlutes
            | SetupField::ToolStickout
            | SetupField::ToolFluteLength
            | SetupField::ToolTipAngle
            | SetupField::ToolCornerRadius
            | SetupField::ToolShankDiameter
            | SetupField::ToolHolderDiameter => {
                tool_edit = true;
                self.commit_tool_field(field, &raw);
            }
            SetupField::MaterialChiploadMin
            | SetupField::MaterialChiploadMax
            | SetupField::MaterialSurfaceSpeedMin
            | SetupField::MaterialSurfaceSpeedMax
            | SetupField::MaterialMaxMrr => {
                let material_field = match field {
                    SetupField::MaterialChiploadMin => MaterialField::ChiploadMin,
                    SetupField::MaterialChiploadMax => MaterialField::ChiploadMax,
                    SetupField::MaterialSurfaceSpeedMin => MaterialField::SurfaceSpeedMin,
                    SetupField::MaterialSurfaceSpeedMax => MaterialField::SurfaceSpeedMax,
                    _ => MaterialField::MaxMrr,
                };
                match parse_optional_positive(&raw, field.label()) {
                    Ok(value) => self.state.setup.set_material_value(material_field, value),
                    Err(message) => {
                        self.state.setup.message = Some(StatusMessage::error(message));
                    }
                }
            }
            SetupField::MaterialNotes => {
                self.state.setup.set_material_notes(raw);
            }
            SetupField::MachineTravelMin(axis) | SetupField::MachineTravelMax(axis) => {
                let machine_field = match field {
                    SetupField::MachineTravelMin(axis) => MachineField::TravelMin(axis),
                    SetupField::MachineTravelMax(axis) => MachineField::TravelMax(axis),
                    _ => MachineField::TravelMax(axis),
                };
                match parse_optional_number(&raw, field.label()) {
                    Ok(Some(value)) => {
                        if self.state.setup.analysis.machine.is_none() {
                            self.state
                                .setup
                                .set_machine_enabled(true, inferred.as_ref());
                        }
                        self.state
                            .setup
                            .set_machine_value(machine_field, Some(value));
                    }
                    Ok(None) => {
                        self.state.setup.message = Some(StatusMessage::error(
                            "Machine travel values cannot be empty.",
                        ));
                    }
                    Err(message) => {
                        self.state.setup.message = Some(StatusMessage::error(message));
                    }
                }
            }
            SetupField::MachineMaxFeed
            | SetupField::MachineMaxSpindle
            | SetupField::MachineRapid => {
                let machine_field = match field {
                    SetupField::MachineMaxFeed => MachineField::MaxFeed,
                    SetupField::MachineMaxSpindle => MachineField::MaxSpindle,
                    _ => MachineField::Rapid,
                };
                match parse_optional_positive(&raw, field.label()) {
                    Ok(Some(value)) => {
                        if self.state.setup.analysis.machine.is_none() {
                            self.state
                                .setup
                                .set_machine_enabled(true, inferred.as_ref());
                        }
                        self.state
                            .setup
                            .set_machine_value(machine_field, Some(value));
                    }
                    Ok(None) => {
                        self.state.setup.set_machine_value(machine_field, None);
                    }
                    Err(message) => {
                        self.state.setup.message = Some(StatusMessage::error(message));
                    }
                }
            }
            SetupField::ToleranceArcRadius
            | SetupField::ToleranceMinSegment
            | SetupField::ToleranceHoleXy
            | SetupField::ToleranceClearance => {
                let tolerance = match field {
                    SetupField::ToleranceArcRadius => ToleranceField::ArcRadius,
                    SetupField::ToleranceMinSegment => ToleranceField::MinSegment,
                    SetupField::ToleranceHoleXy => ToleranceField::HoleXy,
                    _ => ToleranceField::Clearance,
                };
                match parse_non_negative(&raw, field.label()) {
                    Ok(value) => self.state.setup.set_tolerance(tolerance, value),
                    Err(message) => {
                        self.state.setup.message = Some(StatusMessage::error(message));
                    }
                }
            }
            SetupField::SimCellMm
            | SetupField::SimMaxCells
            | SetupField::SimRapidRate
            | SetupField::SimToolChangePause
            | SetupField::SimAssumedDiameter => {
                let sim_field = match field {
                    SetupField::SimCellMm => SimField::CellMm,
                    SetupField::SimMaxCells => SimField::MaxCells,
                    SetupField::SimRapidRate => SimField::RapidRate,
                    SetupField::SimToolChangePause => SimField::ToolChangePause,
                    _ => SimField::AssumedDiameter,
                };
                let parsed = match sim_field {
                    SimField::MaxCells => {
                        parse_optional_count(&raw, field.label()).map(|value| value.map(f64::from))
                    }
                    SimField::ToolChangePause => parse_optional_number(&raw, field.label()),
                    _ => parse_optional_positive(&raw, field.label()),
                };
                match parsed {
                    Ok(value) => self.state.setup.set_sim_value(sim_field, value),
                    Err(message) => {
                        self.state.setup.message = Some(StatusMessage::error(message));
                    }
                }
            }
            SetupField::RuleThresholdKey | SetupField::RuleThresholdValue => return,
        }
        let sim_changed = matches!(
            field,
            SetupField::SimCellMm
                | SetupField::SimMaxCells
                | SetupField::SimRapidRate
                | SetupField::SimToolChangePause
                | SetupField::SimAssumedDiameter
        );
        let analysis_changed = !sim_changed;
        self.pending_setup_sync = true;
        if sim_changed {
            self.program_changed(cx);
            return;
        }
        if analysis_changed {
            if self.state.mode == AppMode::Simulate {
                self.program_changed(cx);
            }
            self.begin_analysis(cx);
        }
        if tool_edit {
            self.state.sync_setup_program();
        }
        cx.notify();
    }

    /// Edits one field of the selected tool overlay.
    fn commit_tool_field(&mut self, field: SetupField, raw: &str) {
        let Some(number) = self.state.setup.selected_tool else {
            self.state.setup.message = Some(StatusMessage::error(
                "Select a tool in the inventory first.",
            ));
            return;
        };
        let Some(hash) = self.state.loaded_hash().map(str::to_string) else {
            self.state.setup.message = Some(StatusMessage::error("No program is open."));
            return;
        };
        let mut tool = overlay_tool(&self.state.config, &hash, number);
        let tool_field = match field {
            SetupField::ToolName => ToolField::Name,
            SetupField::ToolDiameter => ToolField::Diameter,
            SetupField::ToolFlutes => ToolField::Flutes,
            SetupField::ToolStickout => ToolField::Stickout,
            SetupField::ToolFluteLength => ToolField::FluteLength,
            SetupField::ToolTipAngle => ToolField::TipAngle,
            SetupField::ToolCornerRadius => ToolField::CornerRadius,
            SetupField::ToolShankDiameter => ToolField::ShankDiameter,
            SetupField::ToolHolderDiameter => ToolField::HolderDiameter,
            _ => return,
        };
        match apply_tool_field(&mut tool, tool_field, raw) {
            Ok(()) => {
                store_overlay_tool(&mut self.state.config, &hash, tool);
                self.state.setup.message = None;
            }
            Err(message) => {
                self.state.setup.message = Some(StatusMessage::error(message));
            }
        }
    }

    /// Sets the selected tool's shape.
    fn set_tool_shape(&mut self, shape: ToolShape, window: &mut Window, cx: &mut Context<Self>) {
        let Some(number) = self.state.setup.selected_tool else {
            return;
        };
        let Some(hash) = self.state.loaded_hash().map(str::to_string) else {
            return;
        };
        let mut tool = overlay_tool(&self.state.config, &hash, number);
        tool.shape = shape;
        if shape == ToolShape::Unknown {
            tool.sources.remove(&ToolField::Shape);
        } else {
            tool.sources.insert(ToolField::Shape, FieldSource::Config);
        }
        store_overlay_tool(&mut self.state.config, &hash, tool);
        self.after_analysis_edit(window, cx);
    }

    /// Sets the selected tool's handedness.
    fn set_tool_handedness(
        &mut self,
        right_hand: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(number) = self.state.setup.selected_tool else {
            return;
        };
        let Some(hash) = self.state.loaded_hash().map(str::to_string) else {
            return;
        };
        let mut tool = overlay_tool(&self.state.config, &hash, number);
        let text = match right_hand {
            Some(true) => "right",
            Some(false) => "left",
            None => "",
        };
        if let Err(message) = apply_tool_field(&mut tool, ToolField::RightHand, text) {
            self.state.setup.message = Some(StatusMessage::error(message));
            cx.notify();
            return;
        }
        store_overlay_tool(&mut self.state.config, &hash, tool);
        self.after_analysis_edit(window, cx);
    }

    /// Toggles the current material's coolant requirement.
    fn toggle_coolant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let required = self
            .state
            .setup
            .analysis
            .material
            .as_ref()
            .is_some_and(|preset| preset.requires_coolant);
        self.state.setup.set_requires_coolant(!required);
        self.after_analysis_edit(window, cx);
    }

    /// Toggles manual-spindle mode on configured machine limits.
    fn toggle_spindle_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let manual = self
            .state
            .setup
            .analysis
            .machine
            .as_ref()
            .is_some_and(|machine| machine.spindle_manual);
        self.state.setup.set_spindle_manual(!manual);
        self.after_analysis_edit(window, cx);
    }

    /// Toggles crash preview and rebuilds the simulation.
    fn toggle_crash_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.state.setup.sim.crash_preview;
        self.state.setup.set_crash_preview(!enabled);
        self.sync_setup_inputs(window, cx);
        if self.state.mode == AppMode::Simulate {
            self.rebuild_after_tool_edit(window, cx);
        }
        cx.notify();
    }

    /// Loads the TOML path typed into the setup sheet.
    fn load_config_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.setup_value(SetupField::ConfigPath, cx);
        let text = raw.trim();
        if text.is_empty() {
            self.state.setup.message =
                Some(StatusMessage::error("Enter a configuration path first."));
            cx.notify();
            return;
        }
        self.state.setup.config_path_text = text.to_string();
        let path = crate::setup::expand_home(text);
        match load_config(&path) {
            Ok(loaded) => self.apply_loaded_config(path, loaded, window, cx),
            Err(error) => {
                let message = error.to_string();
                self.state.setup.message = Some(StatusMessage::error(format!(
                    "CONFIG NOT LOADED: {message}"
                )));
                self.load_error = Some(format!("CONFIG NOT LOADED: {message}"));
                cx.notify();
            }
        }
    }

    /// Applies an already-parsed TOML configuration to the current session.
    fn apply_loaded_config(
        &mut self,
        path: PathBuf,
        loaded: nc_analysis::LoadedConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tool_count = loaded.analysis.tools.len();
        let stock_text = match loaded.analysis.stock {
            Some(_) => "explicit stock",
            None => "inferred stock",
        };
        let dialect = loaded.dialect;
        let assumed = loaded.dialect_assumed;
        let tools = self.state.setup.apply_loaded(loaded);
        if let Some(hash) = self.state.loaded_hash().map(str::to_string) {
            self.state.config.set_tools_for(&hash, tools);
        }
        let name = file_name_of(&path);
        self.state.setup.config_name = Some(name.clone());
        self.reparse_current(dialect, cx);
        self.state.setup.message = Some(StatusMessage::info(format!(
            "LOADED {name}: {}{}, {stock_text}, {tool_count} configured tools.",
            dialect.label(),
            if assumed { " (assumed)" } else { "" },
        )));
        self.load_error = None;
        self.sync_setup_inputs(window, cx);
        self.begin_analysis(cx);
    }

    /// Re-parses the current source with `dialect` and rebuilds derived state.
    fn reparse_current(&mut self, dialect: Dialect, cx: &mut Context<Self>) {
        let Some(mut loaded) = self.state.loaded.take() else {
            return;
        };
        loaded.reparse(dialect);
        self.program_generation = self.program_generation.wrapping_add(1);
        self.state.loaded = Some(loaded);
        self.state.selected_line = None;
        self.state.sync_setup_program();
        self.program_changed(cx);
    }

    /// Selects a dialect and re-parses, re-analyzes, and rebuilds simulation.
    fn set_dialect_action(
        &mut self,
        dialect: Dialect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.setup.dialect == dialect && !self.state.setup.dialect_assumed {
            return;
        }
        self.state.setup.set_dialect(dialect);
        self.reparse_current(dialect, cx);
        self.sync_setup_inputs(window, cx);
        self.begin_analysis(cx);
    }

    /// Writes the current report to the export path.
    fn export_action(&mut self, cx: &mut Context<Self>) {
        let Some(report) = self.analysis.report().cloned() else {
            self.state.setup.export_status =
                Some(StatusMessage::error("Run analysis before exporting."));
            cx.notify();
            return;
        };
        let path_text = self.setup_value(SetupField::ExportPath, cx);
        self.state.setup.export_path = path_text.clone();
        match write_export(&report, &path_text, self.state.setup.export_format) {
            Ok(outcome) => {
                self.state.setup.export_status = Some(StatusMessage::info(format!(
                    "WROTE {} ({} bytes)",
                    outcome.path.display(),
                    outcome.bytes
                )));
            }
            Err(message) => {
                self.state.setup.export_status = Some(StatusMessage::error(message));
            }
        }
        cx.notify();
    }

    fn after_analysis_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_setup_inputs(window, cx);
        if self.state.mode == AppMode::Simulate {
            self.rebuild_after_tool_edit(window, cx);
        }
        self.begin_analysis(cx);
    }

    /// Switches explicit/inferred stock and re-runs analysis and simulation.
    fn stock_mode_changed(&mut self, explicit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let inferred = self.state.loaded.as_ref().and_then(|loaded| loaded.stock);
        if explicit {
            self.state.setup.set_stock_explicit(inferred.as_ref());
        } else {
            self.state.setup.set_stock_inferred();
        }
        self.after_analysis_edit(window, cx);
    }

    /// Enables or disables machine limits.
    fn machine_mode_changed(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        let inferred = self.state.loaded.as_ref().and_then(|loaded| loaded.stock);
        self.state
            .setup
            .set_machine_enabled(enabled, inferred.as_ref());
        self.after_analysis_edit(window, cx);
    }

    /// Chooses a built-in material preset or clears it.
    fn material_preset_changed(
        &mut self,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.setup.set_material_preset(name);
        self.after_analysis_edit(window, cx);
    }

    /// Selects one inventory tool and seeds its fields.
    fn select_tool(&mut self, number: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.setup.selected_tool = Some(number);
        self.sync_setup_inputs(window, cx);
        cx.notify();
    }

    /// Selects one rule for threshold editing.
    fn select_rule(&mut self, rule: String, cx: &mut Context<Self>) {
        self.state.setup.selected_rule = Some(rule);
        cx.notify();
    }

    /// Enables or disables one rule.
    fn toggle_rule(&mut self, rule: String, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.state.setup.rule_settings(&rule);
        self.state.setup.set_rule_enabled(&rule, !settings.enabled);
        self.state.setup.selected_rule = Some(rule);
        self.after_analysis_edit(window, cx);
    }

    /// Cycles DEFAULT -> ERROR -> WARNING -> INFO -> DEFAULT.
    fn cycle_rule_severity(&mut self, rule: String, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.state.setup.rule_settings(&rule).severity;
        let next = match current {
            None => Some(AnalysisSeverity::Error),
            Some(AnalysisSeverity::Error) => Some(AnalysisSeverity::Warning),
            Some(AnalysisSeverity::Warning) => Some(AnalysisSeverity::Info),
            Some(AnalysisSeverity::Info) => None,
        };
        self.state.setup.set_rule_severity(&rule, next);
        self.state.setup.selected_rule = Some(rule);
        self.after_analysis_edit(window, cx);
    }

    /// Adds the threshold typed into the selected rule's key/value fields.
    fn add_rule_threshold(&mut self, cx: &mut Context<Self>) {
        let Some(rule) = self.state.setup.selected_rule.clone() else {
            self.state.setup.message = Some(StatusMessage::error(
                "Select a rule before adding a threshold.",
            ));
            cx.notify();
            return;
        };
        let key = self.setup_value(SetupField::RuleThresholdKey, cx);
        let value = self.setup_value(SetupField::RuleThresholdValue, cx);
        if key.trim().is_empty() {
            self.state.setup.message = Some(StatusMessage::error("Enter a threshold key."));
            cx.notify();
            return;
        }
        match parse_optional_number(&value, "Threshold") {
            Ok(Some(value)) => {
                self.state.setup.set_rule_threshold(&rule, &key, value);
                self.pending_setup_sync = true;
                self.begin_analysis(cx);
            }
            Ok(None) => {
                self.state.setup.message = Some(StatusMessage::error("Enter a threshold value."));
                cx.notify();
            }
            Err(message) => {
                self.state.setup.message = Some(StatusMessage::error(message));
                cx.notify();
            }
        }
    }

    fn remove_rule_threshold(&mut self, rule: String, key: String, cx: &mut Context<Self>) {
        self.state.setup.remove_rule_threshold(&rule, &key);
        self.begin_analysis(cx);
    }

    /// Switches export format, keeping the path only when it is not a default.
    fn set_export_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let previous = self.state.setup.export_format;
        if previous == format {
            return;
        }
        let previous_default = previous.default_path();
        if self.state.setup.export_path.trim().is_empty()
            || self.state.setup.export_path == previous_default
        {
            self.state.setup.export_path = format.default_path().to_string();
            self.pending_setup_sync = true;
        }
        self.state.setup.export_format = format;
        self.state.setup.export_status = None;
        cx.notify();
    }

    /// The clickable tool chip shared by the top bar and the SIMULATE HUD.
    fn tool_chip_button(&self, id: &'static str, p: Palette, view: Entity<Self>) -> AnyElement {
        let info = self.current_tool_chip();
        div()
            .id(id)
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_lg()
            .border_1()
            .border_color(if info.assumed { p.warning } else { p.border })
            .bg(p.panel_alt)
            .text_size(px(9.5))
            .font_weight(FontWeight::BOLD)
            .text_color(if info.assumed { p.warning } else { p.text })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| shell.open_tool_popover(window, cx));
            })
            .child(info.text)
            .into_any_element()
    }

    /// Top-bar analysis state and counts. Clicking it opens the diagnostics panel.
    fn analysis_chip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let report = self.analysis.report();
        let counts = report.map(|report| SeverityCounts::from_report(report));
        let (text, color, border) = match (&self.analysis, counts) {
            (AnalysisState::Ready { .. }, Some(counts)) => {
                let color = if counts.errors > 0 {
                    p.danger
                } else if counts.warnings > 0 {
                    p.warning
                } else {
                    p.text_dim
                };
                let border = if counts.errors > 0 {
                    p.danger
                } else if counts.warnings > 0 {
                    p.warning
                } else {
                    p.border
                };
                (
                    format!("{} {}", self.analysis.label(), counts.compact()),
                    color,
                    border,
                )
            }
            (AnalysisState::Running(_), _) => ("ANALYZING".to_string(), p.info, p.info),
            (AnalysisState::Failed { .. }, _) => {
                (self.analysis.label().to_string(), p.danger, p.danger)
            }
            _ => (self.analysis.label().to_string(), p.text_faint, p.border),
        };
        let open_view = view;
        let tip = format!(
            "Analysis: {}. Click for diagnostics.",
            self.analysis.label()
        );
        chip(&text, p)
            .id("bar-analysis-state")
            .cursor_pointer()
            .border_color(border)
            .text_color(color)
            .hover(|style| style.bg(p.raised_hover))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, _, cx| {
                open_view.update(cx, |shell, cx| {
                    shell.analysis_panel = Some(AnalysisPanel::Diagnostics);
                    cx.notify();
                });
            })
            .into_any_element()
    }

    fn top_bar(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let target = layout.touch_target();
        let view_chip = action_button(
            "mode-view",
            "VIEW",
            true,
            self.state.mode == AppMode::View,
            p,
            view.clone(),
            move |shell, _, cx| shell.leave_simulate(cx),
        );
        let simulate_chip = action_button(
            "mode-simulate",
            "SIMULATE",
            true,
            self.state.mode == AppMode::Simulate,
            p,
            view.clone(),
            move |shell, _, cx| shell.enter_simulate(cx),
        );

        let brand = h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(px(21.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.accent)
                    .child("NCA"),
            )
            .child(
                v_flex()
                    .gap_0()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_dim)
                            .child("READ-ONLY NC VIEWER"),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .text_color(p.text_faint)
                            .child("3-AXIS MILL | LOCAL ONLY"),
                    ),
            );

        let open = action_button(
            "bar-open",
            "OPEN",
            true,
            false,
            p,
            view.clone(),
            move |shell, window, cx| {
                shell.open_browser(window, cx);
            },
        );

        let fit = action_button(
            "bar-fit",
            "FIT",
            true,
            false,
            p,
            view.clone(),
            move |shell, _window, cx| {
                shell.viewer.fit();
                cx.notify();
            },
        );

        let mode_2d = self.viewer.mode() == ViewerMode::Top2D;
        let two_d = action_button(
            "bar-2d",
            "2D",
            true,
            mode_2d,
            p,
            view.clone(),
            move |shell, _, cx| {
                if shell.viewer.set_mode(ViewerMode::Top2D) {
                    cx.notify();
                }
            },
        );
        let mode_3d = self.viewer.mode() == ViewerMode::ThreeD;
        let three_d = action_button(
            "bar-3d",
            "3D",
            true,
            mode_3d,
            p,
            view.clone(),
            move |shell, _, cx| {
                if shell.viewer.set_mode(ViewerMode::ThreeD) {
                    cx.notify();
                }
            },
        );

        let stock_on = self.viewer.stock_visible();
        let stock = action_button(
            "bar-stock",
            if stock_on { "STOCK ON" } else { "STOCK OFF" },
            true,
            stock_on,
            p,
            view.clone(),
            move |shell, _, cx| {
                let next = !shell.viewer.stock_visible();
                if shell.viewer.set_stock_visible(next) {
                    shell.state.stock_visible = next;
                    shell.toggles.stock = next;
                    cx.notify();
                }
            },
        );

        let settings = action_button(
            "bar-settings",
            "SETUP",
            true,
            self.state.settings_open,
            p,
            view.clone(),
            move |shell, window, cx| {
                shell.settings_focus = None;
                shell.state.settings_open = true;
                shell.export_open = false;
                shell.sync_setup_inputs(window, cx);
                cx.notify();
            },
        );

        let analyzing = self.analysis.is_running();
        let can_analyze = self
            .state
            .loaded
            .as_ref()
            .is_some_and(|loaded| loaded.program.is_some())
            && !analyzing;
        let analyze = action_button(
            "bar-analyze",
            if analyzing { "ANALYZING" } else { "ANALYZE" },
            can_analyze,
            analyzing,
            p,
            view.clone(),
            move |shell, _, cx| shell.begin_analysis(cx),
        );
        let analysis_chip = self.analysis_chip(p, view.clone());
        let export = action_button(
            "bar-export",
            "EXPORT",
            true,
            self.export_open,
            p,
            view.clone(),
            move |shell, _, cx| {
                shell.export_open = true;
                shell.state.settings_open = false;
                cx.notify();
            },
        );
        let tool_button = self.tool_chip_button("bar-tool", p, view.clone());

        let actions = h_flex()
            .flex_1()
            .min_w(px(0.0))
            .justify_end()
            .items_center()
            .gap_1()
            .child(open)
            .child(fit)
            .child(two_d)
            .child(three_d)
            .child(stock)
            .child(tool_button)
            .child(analysis_chip)
            .child(analyze)
            .child(export)
            .child(settings);

        if layout.top_bar_rows() == 2 {
            return v_flex()
                .flex_shrink_0()
                .gap_1()
                .p_1()
                .border_b_1()
                .border_color(p.border)
                .bg(p.panel)
                .child(
                    h_flex()
                        .h(px(target))
                        .items_center()
                        .gap_2()
                        .child(brand)
                        .child(div().flex_1())
                        .child(view_chip)
                        .child(simulate_chip),
                )
                .child(h_flex().h(px(target)).items_center().gap_1().child(actions))
                .into_any_element();
        }

        h_flex()
            .h(px(target + 8.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(brand)
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(view_chip)
                    .child(simulate_chip),
            )
            .child(actions)
            .into_any_element()
    }

    fn tool_chip_compact(&self, id: &'static str, p: Palette, view: Entity<Self>) -> AnyElement {
        let info = self.current_tool_chip();
        div()
            .id(id)
            .h(px(26.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(if info.assumed { p.warning } else { p.border })
            .bg(p.panel_alt)
            .text_size(px(9.0))
            .font_weight(FontWeight::BOLD)
            .text_color(if info.assumed { p.warning } else { p.text })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| shell.open_tool_popover(window, cx));
            })
            .child(info.text)
            .into_any_element()
    }

    fn transport_bar(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let playing = self.transport.is_playing();
        let state = if self.seeking {
            "SEEKING"
        } else {
            self.transport.state().label()
        };
        let state_color = if self.seeking {
            p.info
        } else if playing {
            p.accent
        } else {
            p.text_dim
        };

        let play = transport_button(
            "sim-play",
            if playing { "PAUSE" } else { "PLAY" },
            true,
            playing,
            p,
            view.clone(),
            move |shell, _, cx| shell.toggle_playback(cx),
        );
        let stop = transport_button(
            "sim-stop",
            "STOP",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.stop_playback(cx),
        );
        let step_motion = transport_button(
            "sim-step-motion",
            "STEP MOVE",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.step_motion(cx),
        );
        let step_line = transport_button(
            "sim-step-line",
            "STEP LINE",
            true,
            false,
            p,
            view.clone(),
            move |shell, _, cx| shell.step_line(cx),
        );

        let mut speed_row = h_flex().items_center().gap_1();
        for speed in PlaybackSpeed::ALL {
            speed_row = speed_row.child(transport_button(
                speed_button_id(speed),
                speed.label(),
                true,
                self.transport.speed() == speed,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.transport.set_speed(speed);
                    cx.notify();
                },
            ));
        }

        let (time_text, tick_text, line_text) = self.sim.as_ref().map_or_else(
            || {
                (
                    "0:00.0 / 0:00.0".to_string(),
                    "TICK 0".to_string(),
                    "L-".to_string(),
                )
            },
            |session| {
                let snapshot = session.simulation.snapshot();
                let total = session.simulation.timeline().total_time_s();
                let kind = snapshot
                    .current_kind
                    .map_or("--", nc_viewer::MoveKind::label);
                (
                    format!("{} / {}", format_time(snapshot.time_s), format_time(total)),
                    format!("TICK {}", snapshot.tick),
                    format!("L{} {}", snapshot.current_line, kind),
                )
            },
        );

        let row_one = h_flex()
            .h(px(30.0))
            .items_center()
            .gap_1()
            .child(
                chip(state, p)
                    .text_color(state_color)
                    .border_color(if self.seeking { p.info } else { p.border }),
            )
            .child(play)
            .child(stop)
            .child(step_motion)
            .child(step_line)
            .child(div().w(px(4.0)))
            .child(speed_row)
            .child(div().flex_1())
            .child(chip(&time_text, p))
            .child(chip(&tick_text, p))
            .child(chip(&line_text, p));

        let slider = div()
            .flex_1()
            .min_w(px(140.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .child(
                Slider::new(&self.scrub_slider)
                    .horizontal()
                    .w_full()
                    .h(px(20.0)),
            );

        let mut toggles = h_flex().items_center().gap_1();
        toggles = toggles
            .child(compact_toggle(
                "sim-toggle-stock",
                "STOCK",
                self.toggles.stock,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.stock = !shell.toggles.stock;
                    shell.viewer.set_stock_visible(shell.toggles.stock);
                    shell.state.stock_visible = shell.toggles.stock;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-rapids",
                "RAPIDS",
                self.toggles.rapids,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.rapids = !shell.toggles.rapids;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-air",
                "AIR",
                self.toggles.air_cuts,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.air_cuts = !shell.toggles.air_cuts;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-future",
                "FUTURE",
                self.toggles.future_path,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.future_path = !shell.toggles.future_path;
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "sim-toggle-tool",
                "TOOL",
                self.toggles.tool,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.toggles.tool = !shell.toggles.tool;
                    cx.notify();
                },
            ))
            .child(
                compact_toggle(
                    "sim-toggle-warnings",
                    "WARN",
                    self.toggles.warnings,
                    p,
                    view.clone(),
                    move |shell, _, cx| {
                        shell.toggles.warnings = !shell.toggles.warnings;
                        cx.notify();
                    },
                )
                .tooltip(|window, cx| {
                    Tooltip::new("Draw analysis warning and error segments over the toolpath.")
                        .build(window, cx)
                }),
            );

        let row_two = h_flex()
            .h(px(28.0))
            .items_center()
            .gap_2()
            .child(slider)
            .child(toggles)
            .child(self.tool_chip_compact("sim-tool-chip", p, view));

        v_flex()
            .h(px(TRANSPORT_HEIGHT))
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel_alt)
            .child(row_one)
            .child(row_two)
            .into_any_element()
    }

    fn sim_hud(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(session) = self.sim.as_ref() else {
            return div().into_any_element();
        };
        let snapshot = session.simulation.snapshot();
        let stats = session.simulation.stats();
        let total = session.simulation.timeline().total_time_s();
        let tool = self.current_tool_chip();
        let mut rows = v_flex().gap(px(2.0));
        let row = |label: &'static str, value: String, color: Hsla| {
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(76.0))
                        .flex_shrink_0()
                        .text_size(px(8.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_faint)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(value),
                )
        };
        rows = rows
            .child(row("TICK", snapshot.tick.to_string(), p.text))
            .child(row(
                "TIME",
                format!("{} / {}", format_time(snapshot.time_s), format_time(total)),
                p.text,
            ))
            .child(row(
                "LINE",
                format!(
                    "L{} {}",
                    snapshot.current_line,
                    snapshot
                        .current_kind
                        .map_or("--", nc_viewer::MoveKind::label)
                ),
                p.text,
            ))
            .child(row(
                "PROGRESS",
                format!("{:.1}%", snapshot.progress * 100.0),
                p.text,
            ))
            .child(row(
                "CUT VOLUME",
                if stats.volume_assumed {
                    format!("{:.1} mm3 (assumed)", stats.cut_volume_mm3)
                } else {
                    format!("{:.1} mm3", stats.cut_volume_mm3)
                },
                if stats.volume_assumed {
                    p.warning
                } else {
                    p.text
                },
            ))
            .child(row(
                "MAX DEPTH",
                format!("{:.2} mm", stats.max_depth_mm),
                p.text,
            ))
            .child(row(
                "CUT / RAPID",
                format!(
                    "{} / {}",
                    format_time(stats.cut_time_s),
                    format_time(stats.rapid_time_s)
                ),
                p.text_dim,
            ))
            .child(row("AIR TIME", format_time(stats.air_time_s), p.text_dim));

        let mut badges = h_flex().gap_1().items_center();
        if tool.assumed {
            badges = badges.child(
                chip("TOOL SIZE ASSUMED", p)
                    .text_color(p.warning)
                    .border_color(p.warning),
            );
        }
        if session.simulation.is_coarse() {
            badges = badges.child(
                chip("COARSE GRID", p)
                    .text_color(p.warning)
                    .border_color(p.warning),
            );
        }
        if self.seeking {
            badges = badges.child(chip("SEEKING", p).text_color(p.info).border_color(p.info));
        }

        v_flex()
            .absolute()
            .top(px(40.0))
            .left(px(8.0))
            .w(px(258.0))
            .gap_1()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border_strong)
            .bg(p.panel.alpha(0.94))
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_dim)
                            .child("SIMULATION"),
                    )
                    .child(div().flex_1())
                    .child(self.tool_chip_compact("hud-tool-chip", p, view)),
            )
            .child(badges)
            .child(rows)
            .into_any_element()
    }

    fn sim_error_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(message) = self.sim_error.clone() else {
            return div().into_any_element();
        };
        h_flex()
            .min_h(px(36.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.warning)
            .bg(p.warning.alpha(0.10))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.warning)
                    .child("SIMULATION UNAVAILABLE"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text)
                    .child(message),
            )
            .child(
                div()
                    .id("sim-error-dismiss")
                    .h(px(28.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, _, cx| {
                        view.update(cx, |shell, cx| {
                            shell.sim_error = None;
                            cx.notify();
                        });
                    })
                    .child("DISMISS"),
            )
            .into_any_element()
    }

    fn tool_popover(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        if !self.tool_popover_open {
            return div().into_any_element();
        }
        let number = self.tool_edit_number;
        let title = number.map_or_else(|| "TOOL".to_string(), |number| format!("TOOL T{number}"));
        let mut shapes = h_flex().gap_1().flex_wrap();
        for shape in [
            ToolShape::Flat,
            ToolShape::Ball,
            ToolShape::Bull,
            ToolShape::Drill,
            ToolShape::VBit,
            ToolShape::Unknown,
        ] {
            shapes = shapes.child(compact_toggle(
                tool_shape_id(shape),
                shape.label().to_ascii_uppercase().as_str(),
                self.tool_shape_draft == shape,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.tool_shape_draft = shape;
                    cx.notify();
                },
            ));
        }
        let mut body = v_flex().gap_2();
        if let Some(number) = number {
            body = body
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("DIAMETER MM"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(34.0))
                                .rounded_lg()
                                .border_1()
                                .border_color(p.border_strong)
                                .bg(p.panel_alt)
                                .px_1()
                                .child(
                                    Input::new(&self.tool_diameter_input)
                                        .h(px(32.0))
                                        .w_full()
                                        .text_size(px(12.0)),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("FLUTES"),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .h(px(34.0))
                                .rounded_lg()
                                .border_1()
                                .border_color(p.border_strong)
                                .bg(p.panel_alt)
                                .px_1()
                                .child(
                                    Input::new(&self.tool_flutes_input)
                                        .h(px(32.0))
                                        .w_full()
                                        .text_size(px(12.0)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(8.5))
                                .text_color(p.text_faint)
                                .child("Empty = unknown"),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("SHAPE"),
                        )
                        .child(shapes),
                );
            let apply_view = view.clone();
            body = body.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(8.5))
                            .text_color(p.text_faint)
                            .child(format!(
                                "T{number}: local tool data, never written to the G-code."
                            )),
                    )
                    .child(action_button(
                        "tool-apply",
                        "APPLY",
                        true,
                        true,
                        p,
                        apply_view,
                        move |shell, window, cx| shell.apply_tool_edit(window, cx),
                    )),
            );
        }
        if let Some(error) = &self.tool_edit_error {
            body = body.child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.warning)
                    .child(error.clone()),
            );
        }
        let top = layout.top_bar_height
            + STATUS_BAR_HEIGHT
            + if self.state.mode == AppMode::Simulate {
                TRANSPORT_HEIGHT
            } else {
                0.0
            }
            + 8.0;
        div()
            .absolute()
            .top(px(top))
            .right(px(12.0))
            .w(px(330.0))
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border_strong)
            .bg(p.panel)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .child(section_label(&title, p))
                    .child(chip("QUICK EDIT", p))
                    .child(div().flex_1())
                    .child(action_button(
                        "tool-popover-close",
                        "CLOSE",
                        true,
                        false,
                        p,
                        view,
                        move |shell, _, cx| {
                            shell.tool_popover_open = false;
                            cx.notify();
                        },
                    )),
            )
            .child(body)
            .into_any_element()
    }

    fn status_strip(&self, p: Palette) -> AnyElement {
        let loaded = self.state.loaded.as_ref();
        let mut strip = h_flex()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.panel_alt);

        let name = self.state.file_name().to_string();
        let full_name = loaded
            .and_then(|loaded| loaded.path.as_deref())
            .map_or_else(|| name.clone(), display_path);
        strip = strip.child(
            div()
                .id("status-file")
                .h(px(22.0))
                .max_w(px(260.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .overflow_hidden()
                .rounded(px(5.0))
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .px_2()
                .text_size(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text)
                .truncate()
                .tooltip(move |window, cx| Tooltip::new(full_name.clone()).build(window, cx))
                .child(name),
        );

        strip = strip.child(chip(&format!("{} LINES", self.state.line_count()), p));
        let tools = self.state.setup.inventory.len();
        let tools_text = if tools == 1 {
            "1 TOOL".to_string()
        } else {
            format!("{tools} TOOLS")
        };
        strip = strip.child(chip(&tools_text, p));

        let stock_label = self.state.stock_label();
        let stock_tip = format!(
            "{}\n{}",
            self.state.stock_size_text(),
            self.state.stock_basis_text()
        );
        strip = strip.child(
            chip(stock_label, p)
                .id("status-stock")
                .tooltip(move |window, cx| Tooltip::new(stock_tip.clone()).build(window, cx)),
        );
        let dialect_label = if self.state.setup.dialect_assumed {
            "GENERIC ASSUMED".to_string()
        } else {
            format!(
                "DIALECT: {}",
                self.state.setup.dialect.label().to_uppercase()
            )
        };
        let dialect_tip = if self.state.setup.dialect_assumed {
            "No dialect is configured, so parsing and analysis use Generic semantics and \
             label the choice assumed."
                .to_string()
        } else {
            format!(
                "The setup sheet selected the {} dialect for parsing and analysis.",
                self.state.setup.dialect.label()
            )
        };
        strip = strip.child(
            chip(&dialect_label, p)
                .id("status-dialect")
                .tooltip(move |window, cx| Tooltip::new(dialect_tip.clone()).build(window, cx)),
        );

        let parse = self.state.parse_state();
        let parse_color = match parse {
            ParseState::ParseErrors(_) => p.danger,
            ParseState::Loading => p.info,
            _ => p.text_dim,
        };
        let parse_border = if matches!(parse, ParseState::ParseErrors(_)) {
            p.danger
        } else {
            p.border
        };
        strip = strip.child(
            chip(&parse.label(), p)
                .text_color(parse_color)
                .border_color(parse_border),
        );

        if let Some(line) = self.state.selected_line {
            strip = strip.child(chip(&format!("L{line}"), p));
        }

        strip = strip.child(div().flex_1());
        strip = strip.child(
            div()
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child("ADVISORY ONLY | NO CONTROLLER"),
        );
        strip.into_any_element()
    }

    fn analysis_dock(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let mut dock = v_flex()
            .flex_shrink_0()
            .border_t_1()
            .border_color(p.border)
            .bg(p.panel_alt);
        if let Some(panel) = self.analysis_panel {
            dock = dock.child(self.analysis_panel_body(layout, panel, p, view.clone()));
        }
        dock.child(self.analysis_tab_strip(p, view))
            .into_any_element()
    }

    fn analysis_tab_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let counts = self
            .analysis
            .report()
            .map(|report| SeverityCounts::from_report(report));
        let diagnostics_label = counts.map_or_else(
            || "DIAGNOSTICS".to_string(),
            |counts| format!("DIAGNOSTICS {}", counts.compact()),
        );
        let coverage_label = self.analysis.report().map_or_else(
            || "COVERAGE".to_string(),
            |report| format!("COVERAGE {}", coverage_summary(&report.coverage)),
        );
        let diag_active = self.analysis_panel == Some(AnalysisPanel::Diagnostics);
        let coverage_active = self.analysis_panel == Some(AnalysisPanel::Coverage);

        let mut strip = h_flex()
            .h(px(34.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_t_1()
            .border_color(p.border)
            .bg(p.panel_alt)
            .child(compact_toggle(
                "analysis-tab-diagnostics",
                &diagnostics_label,
                diag_active,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.analysis_panel = if diag_active {
                        None
                    } else {
                        Some(AnalysisPanel::Diagnostics)
                    };
                    cx.notify();
                },
            ))
            .child(compact_toggle(
                "analysis-tab-coverage",
                &coverage_label,
                coverage_active,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.analysis_panel = if coverage_active {
                        None
                    } else {
                        Some(AnalysisPanel::Coverage)
                    };
                    cx.notify();
                },
            ))
            .child(chip(self.analysis.label(), p))
            .child(div().flex_1());
        if let Some(report) = self.analysis.report()
            && !report.truncated.is_empty()
        {
            strip = strip.child(
                chip("TRUNCATED", p)
                    .id("analysis-truncated")
                    .text_color(p.warning)
                    .border_color(p.warning)
                    .tooltip(|window, cx| {
                        Tooltip::new(
                            "The report cap dropped findings; the report lists the count per rule.",
                        )
                        .build(window, cx)
                    }),
            );
        }
        strip = strip.child(
            div()
                .text_size(px(8.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child("COVERAGE IS NOT A SAFETY VERDICT"),
        );
        if self.analysis.is_running() {
            strip = strip.child(compact_toggle(
                "analysis-cancel",
                "CANCEL",
                false,
                p,
                view,
                move |shell, _, cx| shell.cancel_analysis(cx),
            ));
        }
        strip.into_any_element()
    }

    fn analysis_panel_body(
        &self,
        layout: ShellLayout,
        panel: AnalysisPanel,
        p: Palette,
        view: Entity<Self>,
    ) -> AnyElement {
        let body = match panel {
            AnalysisPanel::Diagnostics => self.diagnostics_panel_body(layout, p, view),
            AnalysisPanel::Coverage => self.coverage_panel_body(p, view),
        };
        div()
            .h(px(layout.analysis_panel_height()))
            .flex_shrink_0()
            .overflow_hidden()
            .child(body)
            .into_any_element()
    }

    fn analysis_empty_state(&self, p: Palette) -> Div {
        let (title, detail) = match &self.analysis {
            AnalysisState::Idle => (
                "NO ANALYSIS YET",
                "Press ANALYZE to run the rule registry against this program.",
            ),
            AnalysisState::Running(_) => (
                "ANALYZING",
                "Rules run off the UI thread; the viewer stays interactive.",
            ),
            AnalysisState::Failed { message, .. } => ("ANALYSIS FAILED", message.as_str()),
            AnalysisState::Canceled => (
                "ANALYSIS CANCELED",
                "The previous result was discarded and no report is shown.",
            ),
            AnalysisState::Ready { .. } => ("NO FINDINGS", "The report has no diagnostics."),
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .p_3()
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(560.0))
                    .text_size(px(9.5))
                    .text_color(p.text_faint)
                    .child(detail.to_string()),
            )
    }

    fn diagnostics_panel_body(
        &self,
        layout: ShellLayout,
        p: Palette,
        view: Entity<Self>,
    ) -> AnyElement {
        let Some(report) = self.analysis.report().cloned() else {
            return self.analysis_empty_state(p).into_any_element();
        };
        let counts = SeverityCounts::from_report(&report);
        let filter = self.diagnostic_filter();
        let indices = Arc::new(filter.indices(&report));
        let filtered_count = indices.len();
        let selected = self.selected_diagnostic;
        let total = counts.total();

        let mut severities = h_flex().items_center().gap_1();
        for severity in SeverityFilter::ALL {
            let active = self.severity_filter == severity;
            severities = severities.child(compact_toggle(
                severity_filter_id(severity),
                severity.label(),
                active,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.severity_filter = severity;
                    cx.notify();
                },
            ));
        }
        let rule_input = self.diagnostic_rule_input.clone();
        let search_input = self.diagnostic_search_input.clone();
        let header = v_flex()
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(section_label("DIAGNOSTICS", p))
                    .child(
                        chip(&format!("{} ERRORS", counts.errors), p)
                            .text_color(if counts.errors > 0 {
                                p.danger
                            } else {
                                p.text_faint
                            })
                            .border_color(if counts.errors > 0 {
                                p.danger
                            } else {
                                p.border
                            }),
                    )
                    .child(
                        chip(&format!("{} WARNINGS", counts.warnings), p)
                            .text_color(if counts.warnings > 0 {
                                p.warning
                            } else {
                                p.text_faint
                            })
                            .border_color(if counts.warnings > 0 {
                                p.warning
                            } else {
                                p.border
                            }),
                    )
                    .child(chip(&format!("{} INFO", counts.info), p))
                    .child(chip(&format!("{filtered_count} OF {total}"), p))
                    .child(div().flex_1())
                    .child(severities),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(compact_input(
                        "diagnostic-rule",
                        &rule_input,
                        "RULE OR CATEGORY",
                        p,
                    ))
                    .child(compact_input(
                        "diagnostic-search",
                        &search_input,
                        "MESSAGE, RULE, OR LINE",
                        p,
                    )),
            );

        let list_report = Arc::clone(&report);
        let list_view = view.clone();
        let list = if filtered_count == 0 {
            let message = if total == 0 {
                "No findings in this report."
            } else {
                "No finding matches this filter."
            };
            v_flex()
                .flex_1()
                .min_h(px(0.0))
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(p.text_faint)
                        .child(message),
                )
                .into_any_element()
        } else {
            uniform_list("diagnostic-list", filtered_count, move |range, _, _| {
                range
                    .filter_map(|visible| {
                        let index = *indices.get(visible)?;
                        let diagnostic = list_report.diagnostics.get(index)?;
                        Some(
                            diagnostic_row(
                                index,
                                diagnostic,
                                selected == Some(index),
                                p,
                                list_view.clone(),
                            )
                            .into_any_element(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(&self.diagnostics_scroll)
            .into_any_element()
        };

        let detail = self.diagnostic_detail(&report, p);
        let detail_width = (layout.width * 0.34).clamp(300.0, 430.0);

        h_flex()
            .size_full()
            .min_h(px(0.0))
            .items_stretch()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(header)
                    .child(list),
            )
            .child(
                div()
                    .w(px(detail_width))
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(p.border)
                    .bg(p.panel)
                    .child(detail),
            )
            .into_any_element()
    }

    fn diagnostic_detail(&self, report: &Report, p: Palette) -> AnyElement {
        let Some(index) = self.selected_diagnostic else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_3()
                .child(
                    div().text_size(px(9.5)).text_color(p.text_faint).child(
                        "Select a finding for its rule, basis, evidence, and why it matters.",
                    ),
                )
                .into_any_element();
        };
        let Some(diagnostic) = report.diagnostics.get(index) else {
            return div().into_any_element();
        };
        let note = self.rule_catalog.note(diagnostic.rule.as_str());
        let mut detail = v_flex()
            .id("diagnostic-detail")
            .size_full()
            .min_h(px(0.0))
            .overflow_y_scrollbar()
            .gap_1()
            .p_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .flex_wrap()
                    .child(severity_chip(diagnostic.severity, p))
                    .child(chip(diagnostic.rule.as_str(), p))
                    .child(chip(
                        diagnostic.basis.label().to_ascii_uppercase().as_str(),
                        p,
                    ))
                    .child(div().flex_1())
                    .child(chip(
                        &if diagnostic.line_end > diagnostic.line {
                            format!("L{}-{}", diagnostic.line, diagnostic.line_end)
                        } else {
                            format!("L{}:{}", diagnostic.line, diagnostic.column)
                        },
                        p,
                    )),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.text)
                    .child(diagnostic.message.clone()),
            );
        if let Some(note) = note {
            detail = detail.child(
                v_flex()
                    .gap_0()
                    .child(
                        div()
                            .text_size(px(8.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .child("WHY IT MATTERS"),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(p.text_dim)
                            .child(note.why_it_matters.clone()),
                    ),
            );
        } else {
            detail = detail.child(
                div()
                    .text_size(px(9.0))
                    .text_color(p.warning)
                    .child("No description for this rule id in the current registry."),
            );
        }
        if !diagnostic.evidence.is_empty() {
            let mut table = v_flex().gap_0().child(
                div()
                    .text_size(px(8.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("EVIDENCE"),
            );
            for (key, value) in &diagnostic.evidence {
                table = table.child(
                    h_flex()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .w(px(128.0))
                                .flex_shrink_0()
                                .truncate()
                                .text_size(px(8.5))
                                .font_family(self.mono.clone())
                                .text_color(p.text_dim)
                                .child(key.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(px(8.5))
                                .font_family(self.mono.clone())
                                .text_color(p.text)
                                .child(value.display_text()),
                        ),
                );
            }
            detail = detail.child(table);
        }
        detail.into_any_element()
    }

    fn coverage_panel_body(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(report) = self.analysis.report().cloned() else {
            return v_flex()
                .size_full()
                .gap_2()
                .p_2()
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child(crate::analysis_ui::COVERAGE_EMPTY_NOTE),
                )
                .child(self.analysis_empty_state(p))
                .into_any_element();
        };
        let coverage = &report.coverage;
        let groups = coverage_groups(coverage);
        let header = v_flex()
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(section_label("COVERAGE", p))
                    .child(chip(&coverage_summary(coverage), p))
                    .child(chip(&format!("{} ENTRIES", coverage.skipped_count()), p))
                    .child(chip(
                        &format!("{} RULES REGISTERED", self.rule_catalog.len()),
                        p,
                    ))
                    .child(div().flex_1())
                    .child(
                        chip("NOT A SAFETY VERDICT", p)
                            .text_color(p.warning)
                            .border_color(p.warning),
                    ),
            )
            .child(
                div()
                    .max_w(px(980.0))
                    .text_size(px(9.0))
                    .text_color(p.text_faint)
                    .child(crate::analysis_ui::COVERAGE_NOTE),
            );

        let mut rows = v_flex().gap_1().p_2();
        if groups.is_empty() {
            rows = rows.child(
                div()
                    .text_size(px(10.0))
                    .text_color(p.text_dim)
                    .child("Every registered rule was evaluated for this program."),
            );
        }
        for group in &groups {
            rows = rows.child(coverage_group_card(group, p, view.clone()));
        }
        v_flex()
            .size_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .into_any_element()
    }

    fn coverage_action(
        &mut self,
        target: CoverageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            CoverageTarget::Tool(number) => self.open_tool_popover_for(number, window, cx),
            CoverageTarget::Machine | CoverageTarget::Material | CoverageTarget::Stock => {
                self.settings_focus = Some(target);
                self.state.settings_open = true;
                cx.notify();
            }
            CoverageTarget::RuleDisabled | CoverageTarget::Other => {}
        }
    }

    fn workspace(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        match layout.columns {
            crate::layout::ColumnLayout::Three => h_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .child(self.program_panel(p, view.clone(), Some(layout.left_width), None))
                .child(self.canvas_card(layout, p, view.clone()))
                .child(self.legend_panel(layout, p, view))
                .into_any_element(),
            crate::layout::ColumnLayout::Two => h_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .child(self.program_panel(p, view.clone(), Some(layout.left_width), None))
                .child(self.canvas_card(layout, p, view))
                .into_any_element(),
            crate::layout::ColumnLayout::One => v_flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(crate::layout::GAP))
                .p_3()
                .overflow_y_scrollbar()
                .child(self.canvas_card_portrait(layout, p, view.clone()))
                .child(self.collapsed_legend(p, view.clone()))
                .child(self.program_panel(p, view, None, Some(portrait_program_height(layout))))
                .into_any_element(),
        }
    }

    fn program_panel(
        &self,
        p: Palette,
        view: Entity<Self>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> AnyElement {
        let Some(loaded) = self.state.loaded.as_ref() else {
            let mut panel = v_flex()
                .rounded_lg()
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .p_3()
                .gap_2()
                .child(section_label("PROGRAM", p))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(p.text_faint)
                        .child("No program is open. Use OPEN to choose a local file."),
                );
            if let Some(width) = width {
                panel = panel.w(px(width)).h_full();
            }
            if let Some(height) = height {
                panel = panel.w_full().h(px(height));
            }
            return panel.into_any_element();
        };

        let index = self
            .state
            .index()
            .cloned()
            .unwrap_or_else(|| loaded.index(&self.state.search));
        let visible_count = index.visible_count();
        let line_count = loaded.line_count();
        let selected = self.state.selected_line;
        let palette = p;
        let list_view = view.clone();
        let row_lines = Arc::clone(&loaded.lines);
        let mono = self.mono.clone();
        let row_index = index.clone();
        let row_severities = Arc::clone(&self.line_severity_map);

        let body = if visible_count == 0 {
            v_flex()
                .flex_1()
                .min_h(px(0.0))
                .items_center()
                .justify_center()
                .p_3()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child("NO MATCHING LINES"),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(p.text_faint)
                        .child("Try text, N120, or line 120."),
                )
                .into_any_element()
        } else {
            uniform_list("program-list", visible_count, move |range, _, _| {
                range
                    .filter_map(|visible| {
                        let source = row_index.source_index(visible)?;
                        let line = row_lines.get(source)?;
                        Some(
                            program_row(
                                line,
                                row_index.has_error(line.number),
                                row_severities.get(&line.number).copied(),
                                selected == Some(line.number),
                                palette,
                                mono.clone(),
                                list_view.clone(),
                            )
                            .into_any_element(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(&self.program_scroll)
            .into_any_element()
        };

        let header = h_flex()
            .h(px(32.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("PROGRAM", p))
            .child(div().flex_1())
            .child(chip(&format!("{line_count} LINES"), p))
            .child(if index.is_active() {
                chip(&format!("{visible_count} MATCH"), p).into_any_element()
            } else {
                chip("ALL", p).into_any_element()
            });

        let clear_view = view.clone();
        let search_row = h_flex()
            .h(px(TOUCH_TARGET + 8.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel_alt)
                    .on_mouse_down(MouseButton::Left, {
                        let input = self.program_search_input.clone();
                        move |_, window, cx| {
                            input.update(cx, |state, cx| state.focus(window, cx));
                        }
                    })
                    .child(
                        Input::new(&self.program_search_input)
                            .h(px(TOUCH_TARGET - 2.0))
                            .w_full()
                            .text_size(px(12.0)),
                    ),
            )
            .child(
                div()
                    .id("program-search-clear")
                    .h(px(TOUCH_TARGET))
                    .min_w(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if index.is_active() {
                        p.text
                    } else {
                        p.text_faint
                    })
                    .hover(|style| style.bg(p.raised_hover).text_color(p.text))
                    .on_click(move |_, window, cx| {
                        clear_view.update(cx, |shell, cx| shell.clear_search(window, cx));
                    })
                    .child("CLEAR"),
            );

        let list_header = h_flex()
            .h(px(22.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(div().w(px(18.0)))
            .child(
                div()
                    .w(px(48.0))
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("LINE"),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("N"),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child("SOURCE"),
            );

        let mut panel = v_flex()
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(header)
            .child(search_row)
            .child(list_header)
            .child(body);
        if let Some(width) = width {
            panel = panel.w(px(width)).h_full();
        }
        if let Some(height) = height {
            panel = panel.w_full().h(px(height));
        }
        panel.into_any_element()
    }

    fn canvas_card(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let card = div()
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .h_full()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel);
        self.canvas_contents(card, p, view, layout.legend_inline())
    }

    /// Analysis overlays for the current frame.
    ///
    /// The warning/error line sets come from the accepted report; the highlight
    /// is the SIMULATE current line or the selected view line. In SIMULATE the
    /// current line is emphasized after the overlay so it keeps priority.
    fn viewer_overlay(&self) -> Option<ViewerOverlay> {
        let highlight_line = if self.state.mode == AppMode::Simulate {
            self.sim
                .as_ref()
                .map(|session| session.simulation.snapshot().current_line)
                .filter(|line| *line > 0)
        } else {
            self.state.selected_line
        };
        let has_report = self.analysis.report().is_some();
        if !has_report && highlight_line.is_none() {
            return None;
        }
        let (error_lines, warning_lines) =
            if has_report && self.toggles.warnings && !self.overlay_lines.is_empty() {
                (
                    Arc::from(self.overlay_lines.errors.clone()),
                    Arc::from(self.overlay_lines.warnings.clone()),
                )
            } else {
                (
                    Arc::from(Vec::<usize>::new()),
                    Arc::from(Vec::<usize>::new()),
                )
            };
        Some(ViewerOverlay {
            error_lines,
            warning_lines,
            highlight_line,
        })
    }

    fn canvas_card_portrait(
        &self,
        layout: ShellLayout,
        p: Palette,
        view: Entity<Self>,
    ) -> AnyElement {
        let card = div()
            .relative()
            .w_full()
            .h(px(layout.viewer_height))
            .flex_shrink_0()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel);
        self.canvas_contents(card, p, view, false)
    }

    fn canvas_contents(
        &self,
        mut card: Div,
        p: Palette,
        view: Entity<Self>,
        legend_inline: bool,
    ) -> AnyElement {
        let loaded = self.state.loaded.as_ref();
        let program: Option<Arc<Program>> = loaded.and_then(|loaded| loaded.program.clone());
        let stock: Option<StockBox> = self
            .state
            .effective_stock()
            .and_then(|stock| stock.to_stock_box());
        let sim = self.sim_canvas();
        let marker = if sim.is_none() {
            viewer::program_start_marker(program.as_deref())
        } else {
            None
        };
        let visibility = self.state.visibility();
        let mode = self.viewer.mode();
        let zoom = self.viewer.zoom_percent();
        let parse_errors = loaded.is_some_and(|loaded| !loaded.errors.is_empty());
        let preview = loaded.is_none();
        let overlay = self.viewer_overlay();

        card = card.child(viewer::view(
            &self.viewer,
            p,
            program,
            stock,
            marker,
            visibility,
            preview,
            sim.clone(),
            overlay,
            view.clone(),
        ));

        let mut top_left = h_flex()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .gap_1()
            .child(chip(mode.label(), p))
            .child(chip(&format!("{zoom}%"), p));
        if self.analysis.report().is_some() {
            top_left = top_left.child(
                compact_toggle(
                    "view-toggle-warnings",
                    if self.toggles.warnings {
                        "WARN ON"
                    } else {
                        "WARN OFF"
                    },
                    self.toggles.warnings,
                    p,
                    view.clone(),
                    move |shell, _, cx| {
                        shell.toggles.warnings = !shell.toggles.warnings;
                        cx.notify();
                    },
                )
                .tooltip(|window, cx| {
                    Tooltip::new("Toggle analysis warning and error overlays.").build(window, cx)
                }),
            );
        }
        if self.state.mode == AppMode::Simulate
            && let Some(sim) = &sim
        {
            top_left = top_left
                .child(
                    chip(self.transport.state().label(), p)
                        .text_color(if self.transport.is_playing() {
                            p.accent
                        } else {
                            p.text_dim
                        })
                        .border_color(if self.transport.is_playing() {
                            p.accent
                        } else {
                            p.border
                        }),
                )
                .child(chip(&format!("L{}", sim.snapshot.current_line), p));
        }
        if parse_errors {
            top_left = top_left.child(
                chip("GEOMETRY UNAVAILABLE", p)
                    .text_color(p.danger)
                    .border_color(p.danger),
            );
        }
        card = card.child(top_left);

        card = card.child(
            h_flex()
                .absolute()
                .top(px(8.0))
                .right(px(8.0))
                .gap_1()
                .child(chip("READ-ONLY", p)),
        );

        if self.state.mode == AppMode::Simulate {
            card = card.child(self.sim_hud(p, view.clone()));
            if let Some(sim) = &sim
                && sim.tool_assumed
            {
                let pose = sim.snapshot.pose_mm;
                if let Some((x, y)) = self.viewer.project_pose(pose, mode) {
                    card = card.child(
                        chip("\u{00d8}?", p)
                            .absolute()
                            .left(px(x + 10.0))
                            .top(px((y - 22.0).max(4.0)))
                            .text_color(p.warning)
                            .border_color(p.warning),
                    );
                }
            }
        }

        let hint = if mode.is_three_d() {
            "DRAG ORBIT | SHIFT+DRAG PAN | WHEEL ZOOM | DOUBLE-CLICK FIT"
        } else {
            "DRAG PAN | WHEEL ZOOM | DOUBLE-CLICK FIT"
        };
        card = card.child(
            div()
                .absolute()
                .bottom(px(8.0))
                .left(px(10.0))
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child(hint),
        );

        if legend_inline {
            return card.into_any_element();
        }
        let legend_view = view;
        card.child(
            div()
                .id("legend-collapsed-button")
                .absolute()
                .bottom(px(8.0))
                .right(px(8.0))
                .h(px(TOUCH_TARGET))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .rounded_lg()
                .border_1()
                .border_color(p.border_strong)
                .bg(p.panel)
                .cursor_pointer()
                .hover(|style| style.bg(p.raised_hover))
                .on_click(move |_, _, cx| {
                    legend_view.update(cx, |shell, cx| {
                        shell.legend_open = true;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child("LEGEND"),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.accent)
                        .child("OPEN"),
                ),
        )
        .into_any_element()
    }

    fn legend_panel(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let reset_view = view.clone();
        let header = h_flex()
            .h(px(32.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("MOVE LEGEND", p))
            .child(div().flex_1())
            .child(chip(&format!("{} KINDS", MoveKind::ALL.len()), p))
            .child(
                div()
                    .id("legend-all")
                    .h(px(22.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover).text_color(p.text))
                    .on_click(move |_, _, cx| {
                        reset_view.update(cx, |shell, cx| {
                            shell.state.show_all_kinds();
                            cx.notify();
                        });
                    })
                    .child("ALL"),
            );

        v_flex()
            .w(px(layout.right_width))
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(self.legend_rows(p, view)),
            )
            .into_any_element()
    }

    fn legend_rows(&self, p: Palette, view: Entity<Self>) -> Div {
        let mut column = v_flex().gap_1().p_2().child(cut_depth_ramp(p));
        for (family, kinds) in legend_groups() {
            let floor = div()
                .h(px(20.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(family_color(family, p)),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_faint)
                        .child(family.label().to_ascii_uppercase()),
                );
            column = column.child(floor);
            for kind in kinds {
                column = column.child(self.legend_row(kind, p, view.clone()));
            }
        }
        column
    }

    fn legend_row(&self, kind: MoveKind, p: Palette, view: Entity<Self>) -> Stateful<Div> {
        let visible = self.state.kind_visible(kind);
        let style = move_style(kind);
        let color = move_color(kind, p);
        let id = format!("legend-{kind:?}");
        div()
            .id(id)
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(if visible { p.border } else { p.panel })
            .bg(if visible { p.panel_alt } else { p.panel })
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, _, cx| {
                view.update(cx, |shell, cx| {
                    shell.state.toggle_kind(kind);
                    cx.notify();
                });
            })
            .child(legend_swatch(style, color))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.5))
                    .font_weight(if visible {
                        FontWeight::NORMAL
                    } else {
                        FontWeight::BOLD
                    })
                    .text_color(if visible { p.text } else { p.text_faint })
                    .child(style.label),
            )
            .child(
                div()
                    .w(px(22.0))
                    .text_size(px(8.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if visible { p.accent } else { p.text_faint })
                    .child(if visible { "ON" } else { "OFF" }),
            )
    }

    fn collapsed_legend(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let mut bar = h_flex()
            .id("legend-collapsed")
            .w_full()
            .h(px(TOUCH_TARGET))
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .px_2()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover))
            .on_click(move |_, _, cx| {
                view.update(cx, |shell, cx| {
                    shell.legend_open = true;
                    cx.notify();
                });
            })
            .child(section_label("MOVE LEGEND", p))
            .child(chip(
                &format!(
                    "{} / {}",
                    self.state.visible_kind_count(),
                    MoveKind::ALL.len()
                ),
                p,
            ));
        for family in MoveFamily::ALL {
            bar = bar.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(family_color(family, p)),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .child(family.label().to_ascii_uppercase()),
                    ),
            );
        }
        bar.child(div().flex_1())
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.accent)
                    .child("OPEN"),
            )
            .into_any_element()
    }

    fn legend_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(620.0))
                    .max_w_full()
                    .h_full()
                    .max_h(px(680.0))
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(
                        h_flex()
                            .h(px(52.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(section_label("MOVE LEGEND", p))
                            .child(chip(&format!("{} KINDS", MoveKind::ALL.len()), p))
                            .child(div().flex_1())
                            .child(action_button(
                                "legend-sheet-close",
                                "CLOSE",
                                true,
                                false,
                                p,
                                view.clone(),
                                move |shell, _, cx| {
                                    shell.legend_open = false;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scrollbar()
                            .child(self.legend_rows(p, view)),
                    ),
            )
            .into_any_element()
    }

    fn setup_section(&self, title: &str, chips: Vec<Div>, focused: bool, p: Palette) -> Div {
        let mut head = h_flex().items_center().gap_2().child(
            div()
                .text_size(px(10.5))
                .font_weight(FontWeight::BOLD)
                .text_color(if focused { p.accent } else { p.text_dim })
                .child(title.to_string()),
        );
        for chip_element in chips {
            head = head.child(chip_element);
        }
        if focused {
            head = head.child(
                chip("REQUESTED FROM COVERAGE", p)
                    .text_color(p.accent)
                    .border_color(p.accent),
            );
        }
        v_flex()
            .gap_2()
            .p_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(if focused { p.accent } else { p.border })
            .bg(if focused { p.accent_muted } else { p.panel_alt })
            .child(head)
    }

    fn setup_field_box(&self, field: SetupField, p: Palette) -> Div {
        let input = self.setup_inputs.get(&field).cloned();
        let mut box_element = div()
            .flex_1()
            .min_w(px(0.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(5.0))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.panel)
            .px_1();
        if let Some(input) = input {
            box_element =
                box_element.child(Input::new(&input).h(px(26.0)).w_full().text_size(px(10.0)));
        }
        box_element
    }

    fn setup_compact_field(&self, field: SetupField, p: Palette) -> Div {
        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .gap_1()
            .child(
                div()
                    .text_size(px(7.8))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .truncate()
                    .child(field.label()),
            )
            .child(self.setup_field_box(field, p))
    }

    fn setup_input_row(&self, field: SetupField, p: Palette) -> Div {
        h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(108.0))
                    .flex_shrink_0()
                    .text_size(px(8.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_faint)
                    .child(field.label()),
            )
            .child(self.setup_field_box(field, p))
    }

    fn setup_tool_field(
        &self,
        field: SetupField,
        tool_field: ToolField,
        resolved: Option<&Tool>,
        p: Palette,
    ) -> Div {
        let source = resolved.and_then(|tool| tool.sources.get(&tool_field).copied());
        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(7.8))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .truncate()
                            .child(field.label()),
                    )
                    .child(provenance_chip(source, p)),
            )
            .child(self.setup_field_box(field, p))
    }

    fn setup_stock_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let focused = self.settings_focus == Some(CoverageTarget::Stock);
        let mut section = self.setup_section(
            "STOCK",
            vec![chip(self.state.stock_label(), p), chip("MILLIMETRES", p)],
            focused,
            p,
        );
        let explicit = self.state.setup.analysis.stock.is_some();
        section = section.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(compact_toggle(
                    "stock-mode-inferred",
                    "INFERRED",
                    !explicit,
                    p,
                    view.clone(),
                    move |shell, window, cx| shell.stock_mode_changed(false, window, cx),
                ))
                .child(compact_toggle(
                    "stock-mode-explicit",
                    "EXPLICIT",
                    explicit,
                    p,
                    view.clone(),
                    move |shell, window, cx| shell.stock_mode_changed(true, window, cx),
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(8.5))
                        .text_color(p.text_faint)
                        .child(self.state.stock_basis_text()),
                ),
        );
        if let Some(error) = self.state.setup.stock_error.clone() {
            section = section.child(
                div()
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.danger)
                    .child(format!("INVALID STOCK: {error}")),
            );
        }
        section = section
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::StockOrigin(0), p))
                    .child(self.setup_compact_field(SetupField::StockOrigin(1), p))
                    .child(self.setup_compact_field(SetupField::StockOrigin(2), p)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::StockSize(0), p))
                    .child(self.setup_compact_field(SetupField::StockSize(1), p))
                    .child(self.setup_compact_field(SetupField::StockSize(2), p)),
            );
        section
    }

    fn setup_tools_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let mut section = self.setup_section(
            "TOOLS",
            vec![
                chip(
                    &format!("{} INVENTORY", self.state.setup.inventory.len()),
                    p,
                ),
                chip("KEYED BY SHA-256", p),
            ],
            false,
            p,
        );
        let selected_number = self.state.setup.selected_tool;
        let resolved = self.resolved_tools();
        let resolved_tool = selected_number.and_then(|number| resolved.get(number).cloned());
        let mut rows = v_flex().gap_1();
        if self.state.setup.inventory.is_empty() {
            rows = rows.child(
                div()
                    .text_size(px(9.0))
                    .text_color(p.text_dim)
                    .child("No T number is used by this program."),
            );
        }
        for tool in &self.state.setup.inventory {
            let number = tool.number;
            let row_view = view.clone();
            let selected = selected_number == Some(number);
            let diameter = resolved
                .get(number)
                .and_then(|entry| entry.diameter_mm)
                .map(format_diameter)
                .unwrap_or_else(|| "?".to_string());
            let comment = resolved
                .get(number)
                .and_then(|entry| entry.sources.get(&ToolField::Diameter).copied())
                .and_then(|source| match source {
                    FieldSource::Comment { line } => Some(format!("comment L{line}")),
                    FieldSource::Config => None,
                })
                .or_else(|| tool.hint_line.map(|line| format!("comment L{line}")));
            let mut label = format!(
                "T{number}  \u{00d8}{diameter}  L{}  {} motions",
                tool.first_line, tool.motion_count
            );
            if let Some(comment) = comment {
                label.push_str(&format!("  {comment}"));
            }
            rows = rows.child(
                div()
                    .id(format!("setup-tool-{number}"))
                    .h(px(30.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(if selected { p.accent } else { p.border })
                    .bg(if selected { p.accent_muted } else { p.panel })
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if selected { p.accent } else { p.text })
                    .cursor_pointer()
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, window, cx| {
                        row_view.update(cx, |shell, cx| shell.select_tool(number, window, cx));
                    })
                    .child(label),
            );
        }
        section = section.child(div().max_h(px(116.0)).overflow_y_scrollbar().child(rows));
        if let Some(number) = selected_number {
            let shape = resolved_tool
                .as_ref()
                .map_or(ToolShape::Unknown, |tool| tool.shape);
            let mut shapes = h_flex().gap_1().flex_wrap();
            for shape_option in [
                ToolShape::Flat,
                ToolShape::Ball,
                ToolShape::Bull,
                ToolShape::Drill,
                ToolShape::VBit,
                ToolShape::Unknown,
            ] {
                let shape_view = view.clone();
                shapes = shapes.child(sheet_toggle(
                    format!("setup-shape-{shape_option:?}").to_ascii_lowercase(),
                    shape_option.label().to_uppercase(),
                    shape == shape_option,
                    p,
                    shape_view,
                    move |shell, window, cx| {
                        shell.set_tool_shape(shape_option, window, cx);
                    },
                ));
            }
            let right_hand = resolved_tool.as_ref().and_then(|tool| tool.right_hand);
            let hand_row = h_flex()
                .gap_1()
                .child(sheet_toggle(
                    "setup-hand-right".to_string(),
                    "RIGHT HAND".to_string(),
                    right_hand == Some(true),
                    p,
                    view.clone(),
                    move |shell, window, cx| {
                        shell.set_tool_handedness(Some(true), window, cx);
                    },
                ))
                .child(sheet_toggle(
                    "setup-hand-left".to_string(),
                    "LEFT HAND".to_string(),
                    right_hand == Some(false),
                    p,
                    view.clone(),
                    move |shell, window, cx| {
                        shell.set_tool_handedness(Some(false), window, cx);
                    },
                ))
                .child(sheet_toggle(
                    "setup-hand-unknown".to_string(),
                    "UNKNOWN".to_string(),
                    right_hand.is_none(),
                    p,
                    view.clone(),
                    move |shell, window, cx| {
                        shell.set_tool_handedness(None, window, cx);
                    },
                ))
                .child(provenance_chip(
                    resolved_tool
                        .as_ref()
                        .and_then(|tool| tool.sources.get(&ToolField::RightHand).copied()),
                    p,
                ));
            section = section
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(section_label(&format!("TOOL T{number}"), p))
                        .child(chip(
                            &resolved_tool.as_ref().map_or("blank".to_string(), |tool| {
                                tool.name.clone().unwrap_or_else(|| "blank".to_string())
                            }),
                            p,
                        ))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(8.0))
                                .text_color(p.text_faint)
                                .child("Every field is optional; empty means unknown."),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(108.0))
                                .text_size(px(8.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.text_faint)
                                .child("SHAPE"),
                        )
                        .child(shapes)
                        .child(provenance_chip(
                            resolved_tool
                                .as_ref()
                                .and_then(|tool| tool.sources.get(&ToolField::Shape).copied()),
                            p,
                        )),
                )
                .child(hand_row)
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(self.setup_tool_field(
                            SetupField::ToolName,
                            ToolField::Name,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolDiameter,
                            ToolField::Diameter,
                            resolved_tool.as_ref(),
                            p,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(self.setup_tool_field(
                            SetupField::ToolFlutes,
                            ToolField::Flutes,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolStickout,
                            ToolField::Stickout,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolFluteLength,
                            ToolField::FluteLength,
                            resolved_tool.as_ref(),
                            p,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(self.setup_tool_field(
                            SetupField::ToolTipAngle,
                            ToolField::TipAngle,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolCornerRadius,
                            ToolField::CornerRadius,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolShankDiameter,
                            ToolField::ShankDiameter,
                            resolved_tool.as_ref(),
                            p,
                        ))
                        .child(self.setup_tool_field(
                            SetupField::ToolHolderDiameter,
                            ToolField::HolderDiameter,
                            resolved_tool.as_ref(),
                            p,
                        )),
                );
        }
        section
    }

    fn setup_material_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let focused = self.settings_focus == Some(CoverageTarget::Material);
        let material = self.state.setup.analysis.material.clone();
        let mut section = self.setup_section(
            "MATERIAL",
            vec![
                chip(
                    &material
                        .as_ref()
                        .map_or_else(|| "UNSET".to_string(), |preset| preset.name.to_uppercase()),
                    p,
                ),
                chip("OPTIONAL RANGES", p),
            ],
            focused,
            p,
        );
        let mut presets = h_flex().gap_1().flex_wrap();
        presets = presets.child(sheet_toggle(
            "setup-material-none".to_string(),
            "NONE".to_string(),
            material.is_none(),
            p,
            view.clone(),
            move |shell, window, cx| {
                shell.material_preset_changed(None, window, cx);
            },
        ));
        for name in nc_core::MaterialPreset::builtin_names() {
            let name_static: &'static str = name;
            let active = material.as_ref().is_some_and(|preset| preset.name == *name);
            presets = presets.child(sheet_toggle(
                format!("setup-material-{name}"),
                name.to_uppercase(),
                active,
                p,
                view.clone(),
                move |shell, window, cx| {
                    shell.material_preset_changed(Some(name_static), window, cx);
                },
            ));
        }
        section = section.child(presets);
        section = section
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::MaterialChiploadMin, p))
                    .child(self.setup_compact_field(SetupField::MaterialChiploadMax, p))
                    .child(self.setup_compact_field(SetupField::MaterialSurfaceSpeedMin, p))
                    .child(self.setup_compact_field(SetupField::MaterialSurfaceSpeedMax, p))
                    .child(self.setup_compact_field(SetupField::MaterialMaxMrr, p)),
            )
            .child(self.setup_input_row(SetupField::MaterialNotes, p));
        if let Some(preset) = material.as_ref() {
            section = section.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(sheet_toggle(
                        "setup-material-coolant".to_string(),
                        "REQUIRES COOLANT".to_string(),
                        preset.requires_coolant,
                        p,
                        view.clone(),
                        move |shell, window, cx| {
                            shell.toggle_coolant(window, cx);
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(8.5))
                            .text_color(p.text_faint)
                            .child(preset.notes.clone()),
                    ),
            );
        }
        section
    }

    fn setup_machine_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let focused = self.settings_focus == Some(CoverageTarget::Machine);
        let machine = self.state.setup.analysis.machine.clone();
        let configured = machine.is_some();
        let valid = machine
            .as_ref()
            .is_some_and(|machine| machine.validate().is_ok());
        let mut section = self.setup_section(
            "MACHINE",
            vec![
                chip(
                    if !configured {
                        "UNSET"
                    } else if valid {
                        "SET"
                    } else {
                        "INVALID"
                    },
                    p,
                ),
                chip("TRAVEL AND LIMITS", p),
            ],
            focused,
            p,
        );
        section = section.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(sheet_toggle(
                    "setup-machine-off".to_string(),
                    "UNSET".to_string(),
                    !configured,
                    p,
                    view.clone(),
                    move |shell, window, cx| {
                        shell.machine_mode_changed(false, window, cx);
                    },
                ))
                .child(sheet_toggle(
                    "setup-machine-on".to_string(),
                    "SET".to_string(),
                    configured,
                    p,
                    view.clone(),
                    move |shell, window, cx| {
                        shell.machine_mode_changed(true, window, cx);
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(8.5))
                        .text_color(p.text_faint)
                        .child(
                            "Travel is required for machine rules; feed and spindle limits are optional.",
                        ),
                ),
        );
        if let Some(error) = self.state.setup.machine_error.clone() {
            section = section.child(
                div()
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.danger)
                    .child(format!("INVALID MACHINE: {error}")),
            );
        }
        section = section
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::MachineTravelMin(0), p))
                    .child(self.setup_compact_field(SetupField::MachineTravelMin(1), p))
                    .child(self.setup_compact_field(SetupField::MachineTravelMin(2), p))
                    .child(self.setup_compact_field(SetupField::MachineTravelMax(0), p))
                    .child(self.setup_compact_field(SetupField::MachineTravelMax(1), p))
                    .child(self.setup_compact_field(SetupField::MachineTravelMax(2), p)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::MachineMaxFeed, p))
                    .child(self.setup_compact_field(SetupField::MachineMaxSpindle, p))
                    .child(self.setup_compact_field(SetupField::MachineRapid, p)),
            );
        if configured {
            section = section.child(sheet_toggle(
                "setup-machine-manual".to_string(),
                "SPINDLE MANUAL (NO M3 EXPECTED)".to_string(),
                machine
                    .as_ref()
                    .is_some_and(|machine| machine.spindle_manual),
                p,
                view,
                move |shell, window, cx| shell.toggle_spindle_manual(window, cx),
            ));
        }
        section
    }

    fn setup_dialect_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let section = self.setup_section(
            "DIALECT",
            vec![
                chip(
                    if self.state.setup.dialect_assumed {
                        "GENERIC ASSUMED"
                    } else {
                        "EXPLICIT"
                    },
                    p,
                ),
                chip("REPARSES THE PROGRAM", p),
            ],
            false,
            p,
        );
        let mut buttons = h_flex().gap_1().flex_wrap();
        for dialect in Dialect::ALL {
            let active = self.state.setup.dialect == dialect;
            buttons = buttons.child(sheet_toggle(
                format!("setup-dialect-{}", dialect.label()),
                dialect.label().to_uppercase(),
                active,
                p,
                view.clone(),
                move |shell, window, cx| shell.set_dialect_action(dialect, window, cx),
            ));
        }
        section.child(buttons)
    }

    fn setup_simulation_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let crash = self.state.setup.sim.crash_preview;
        let mut section = self.setup_section(
            "SIMULATION",
            vec![chip("STOCK REMOVAL PREVIEW", p), chip("LOCAL ONLY", p)],
            false,
            p,
        );
        section = section
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::SimCellMm, p))
                    .child(self.setup_compact_field(SetupField::SimMaxCells, p))
                    .child(self.setup_compact_field(SetupField::SimRapidRate, p))
                    .child(self.setup_compact_field(SetupField::SimToolChangePause, p))
                    .child(self.setup_compact_field(SetupField::SimAssumedDiameter, p)),
            )
            .child(sheet_toggle(
                "setup-sim-crash".to_string(),
                "CRASH PREVIEW".to_string(),
                crash,
                p,
                view,
                move |shell, window, cx| shell.toggle_crash_preview(window, cx),
            ));
        section
    }

    fn setup_tolerances_section(&self, p: Palette) -> Div {
        self.setup_section("TOLERANCES", vec![chip("ANALYSIS THRESHOLDS", p)], false, p)
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(self.setup_compact_field(SetupField::ToleranceArcRadius, p))
                    .child(self.setup_compact_field(SetupField::ToleranceMinSegment, p))
                    .child(self.setup_compact_field(SetupField::ToleranceHoleXy, p))
                    .child(self.setup_compact_field(SetupField::ToleranceClearance, p)),
            )
    }

    fn setup_rules_section(&self, p: Palette, view: Entity<Self>) -> Div {
        let selected_rule = self.state.setup.selected_rule.clone();
        let mut section = self.setup_section(
            "RULES",
            vec![
                chip(&format!("{} REGISTERED", self.rule_catalog.len()), p),
                chip("THRESHOLDS FLATTENED FROM CONFIG", p),
            ],
            false,
            p,
        );
        let mut rows = v_flex().gap_1();
        for note in self.rule_catalog.iter() {
            let id = note.rule.clone();
            let settings = self.state.setup.rule_settings(&id);
            let selected = selected_rule.as_ref() == Some(&id);
            let severity = settings.severity.unwrap_or(note.severity);
            let severity_label = if settings.severity.is_some() {
                severity.label().to_uppercase()
            } else {
                format!("{} (DEFAULT)", severity.label().to_uppercase())
            };
            let threshold_count = settings.thresholds.len();
            let select_view = view.clone();
            let select_id = id.clone();
            let toggle_view = view.clone();
            let toggle_id = id.clone();
            let severity_view = view.clone();
            let severity_id = id.clone();
            rows = rows.child(
                h_flex()
                    .id(format!("setup-rule-{id}"))
                    .items_center()
                    .gap_2()
                    .px_1()
                    .p_1()
                    .rounded(px(4.0))
                    .bg(if selected { p.accent_muted } else { p.panel })
                    .cursor_pointer()
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, _, cx| {
                        select_view.update(cx, |shell, cx| {
                            shell.select_rule(select_id.clone(), cx);
                        });
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(8.5))
                            .font_weight(FontWeight::BOLD)
                            .font_family("monospace")
                            .text_color(if selected { p.accent } else { p.text_dim })
                            .child(id),
                    )
                    .child(sheet_toggle(
                        format!("rule-enabled-{toggle_id}"),
                        if settings.enabled {
                            "ENABLED".to_string()
                        } else {
                            "DISABLED".to_string()
                        },
                        settings.enabled,
                        p,
                        toggle_view,
                        move |shell, window, cx| {
                            shell.toggle_rule(toggle_id.clone(), window, cx);
                        },
                    ))
                    .child(sheet_button(
                        format!("rule-severity-{severity_id}"),
                        severity_label,
                        settings.severity.is_some(),
                        p,
                        severity_view,
                        move |shell, window, cx| {
                            shell.cycle_rule_severity(severity_id.clone(), window, cx);
                        },
                    ))
                    .child(chip(&format!("{threshold_count} THR"), p)),
            );
        }
        section = section.child(div().max_h(px(220.0)).overflow_y_scrollbar().child(rows));
        if let Some(rule) = selected_rule {
            let settings = self.state.setup.rule_settings(&rule);
            let mut detail = v_flex().gap_1().child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(section_label(&format!("THRESHOLDS: {rule}"), p))
                    .child(chip(&format!("{} SET", settings.thresholds.len()), p)),
            );
            if settings.thresholds.is_empty() {
                detail =
                    detail.child(div().text_size(px(8.5)).text_color(p.text_faint).child(
                        "No numeric threshold is set; the rule uses its documented default.",
                    ));
            }
            for (key, value) in &settings.thresholds {
                let remove_rule = rule.clone();
                let remove_key = key.clone();
                let remove_view = view.clone();
                detail = detail.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_weight(FontWeight::BOLD)
                                .font_family("monospace")
                                .text_color(p.text_dim)
                                .child(format!("{key} = {value}")),
                        )
                        .child(div().flex_1())
                        .child(sheet_button(
                            format!("rule-threshold-remove-{rule}-{key}"),
                            "REMOVE".to_string(),
                            false,
                            p,
                            remove_view,
                            move |shell, _, cx| {
                                shell.remove_rule_threshold(
                                    remove_rule.clone(),
                                    remove_key.clone(),
                                    cx,
                                );
                            },
                        )),
                );
            }
            let add_view = view;
            detail = detail.child(
                h_flex()
                    .items_end()
                    .gap_2()
                    .child(self.setup_compact_field(SetupField::RuleThresholdKey, p))
                    .child(self.setup_compact_field(SetupField::RuleThresholdValue, p))
                    .child(sheet_button(
                        "rule-threshold-add".to_string(),
                        "SET THRESHOLD".to_string(),
                        true,
                        p,
                        add_view,
                        move |shell, _, cx| shell.add_rule_threshold(cx),
                    )),
            );
            section = section.child(detail);
        }
        section
    }

    fn settings_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let mut theme_row = h_flex().gap_2().items_center();
        for mode in ThemeMode::ALL {
            let active = self.state.config.theme == mode;
            theme_row = theme_row.child(action_button(
                theme_button_id(mode),
                mode.label(),
                true,
                active,
                p,
                view.clone(),
                move |shell, _, cx| shell.set_theme(mode, cx),
            ));
        }

        let mut config_section = self.setup_section(
            "CONFIG",
            vec![
                chip("STRICT TOML", p),
                self.state
                    .setup
                    .config_name
                    .as_ref()
                    .map_or_else(|| chip("NO CONFIG", p), |name| chip(name, p)),
            ],
            false,
            p,
        );
        config_section = config_section
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(self.setup_field_box(SetupField::ConfigPath, p))
                    .child(action_button(
                        "setup-load-config",
                        "LOAD CONFIG",
                        true,
                        false,
                        p,
                        view.clone(),
                        move |shell, window, cx| shell.load_config_action(window, cx),
                    )),
            )
            .child(div().text_size(px(8.5)).text_color(p.text_faint).child(
                "Unknown keys are errors. Loading applies stock, machine, material, \
                         tolerances, rules, dialect, and tools to this file by SHA-256.",
            ));

        let mut message_row = v_flex();
        if let Some(message) = self.state.setup.message.as_ref() {
            let color = match message.kind {
                crate::setup::MessageKind::Info => p.info,
                crate::setup::MessageKind::Warning => p.warning,
                crate::setup::MessageKind::Error => p.danger,
            };
            message_row = message_row.child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(color)
                    .child(message.text.clone()),
            );
        }

        let mut body =
            v_flex()
                .gap_2()
                .child(message_row)
                .child(config_section)
                .child(self.setup_stock_section(p, view.clone()))
                .child(self.setup_tools_section(p, view.clone()))
                .child(self.setup_material_section(p, view.clone()))
                .child(self.setup_machine_section(p, view.clone()))
                .child(self.setup_dialect_section(p, view.clone()))
                .child(self.setup_simulation_section(p, view.clone()))
                .child(self.setup_tolerances_section(p))
                .child(self.setup_rules_section(p, view.clone()))
                .child(
                    v_flex()
                        .gap_2()
                        .child(section_label("APPEARANCE", p))
                        .child(theme_row),
                )
                .child(div().text_size(px(8.5)).text_color(p.text_faint).child(
                    "Never writes G-code or connects to a controller. Every edit is local.",
                ));

        body = body.child(
            div()
                .text_size(px(8.5))
                .text_color(p.text_faint)
                .child(ADVISORY_NOTE),
        );

        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(980.0))
                    .max_w_full()
                    .h(px(760.0))
                    .max_h_full()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(
                        h_flex()
                            .h(px(48.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(section_label("SETUP", p))
                            .child(chip("LOCAL ONLY", p))
                            .child(chip(self.state.stock_label(), p))
                            .child(chip(
                                &if self.state.setup.dialect_assumed {
                                    "GENERIC ASSUMED".to_string()
                                } else {
                                    format!(
                                        "DIALECT: {}",
                                        self.state.setup.dialect.label().to_uppercase()
                                    )
                                },
                                p,
                            ))
                            .child(div().flex_1())
                            .child(action_button(
                                "settings-close",
                                "CLOSE",
                                true,
                                false,
                                p,
                                view.clone(),
                                move |shell, _, cx| {
                                    shell.state.settings_open = false;
                                    shell.settings_focus = None;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scrollbar()
                            .p_2()
                            .child(body),
                    ),
            )
            .into_any_element()
    }

    fn export_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let ready = self.analysis.report().is_some();
        let mut format_row = h_flex().gap_1();
        for format in ExportFormat::ALL {
            let active = self.state.setup.export_format == format;
            let format_view = view.clone();
            format_row = format_row.child(sheet_toggle(
                format!("export-format-{}", format.label()),
                format.label().to_string(),
                active,
                p,
                format_view,
                move |shell, _, cx| shell.set_export_format(format, cx),
            ));
        }
        let mut body = v_flex().gap_3();
        if let Some(status) = self.state.setup.export_status.as_ref() {
            let color = match status.kind {
                crate::setup::MessageKind::Info => p.info,
                crate::setup::MessageKind::Warning => p.warning,
                crate::setup::MessageKind::Error => p.danger,
            };
            body = body.child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(color)
                    .child(status.text.clone()),
            );
        }
        body = body
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .child("FORMAT"),
                    )
                    .child(format_row),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_faint)
                            .child("PATH - RELATIVE TO THE CURRENT DIRECTORY, OR ABSOLUTE"),
                    )
                    .child(self.setup_input_row(SetupField::ExportPath, p)),
            )
            .child(div().text_size(px(8.5)).text_color(p.text_faint).child(
                "JSON is the canonical pretty report plus one newline. Markdown lists \
                         every diagnostic and every coverage skip. The report body never contains \
                         an absolute input path.",
            ))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        chip(
                            if ready {
                                "REPORT READY"
                            } else {
                                "ANALYZE FIRST"
                            },
                            p,
                        )
                        .text_color(if ready { p.info } else { p.warning })
                        .border_color(if ready {
                            p.info
                        } else {
                            p.warning
                        }),
                    )
                    .child(div().flex_1())
                    .child(action_button(
                        "export-write",
                        "WRITE REPORT",
                        ready,
                        false,
                        p,
                        view.clone(),
                        move |shell, _, cx| shell.export_action(cx),
                    )),
            )
            .child(
                div()
                    .text_size(px(8.5))
                    .text_color(p.text_faint)
                    .child(ADVISORY_NOTE),
            );

        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(640.0))
                    .max_w_full()
                    .max_h_full()
                    .overflow_y_scrollbar()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(section_label("EXPORT REPORT", p))
                            .child(chip("LOCAL FILE", p))
                            .child(div().flex_1())
                            .child(action_button(
                                "export-close",
                                "CLOSE",
                                true,
                                false,
                                p,
                                view,
                                move |shell, _, cx| {
                                    shell.export_open = false;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn browser_sheet(&self, layout: ShellLayout, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(browser) = self.browser.as_ref() else {
            return div().into_any_element();
        };
        let entries = browser.entries_arc();
        let selected = browser.selected_index();
        let path_text = browser.path_display();
        let error = browser.error().map(str::to_string);
        let entry_count = entries.len();
        let program_count = browser.file_count();
        let folder = browser
            .cwd()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| browser.path_display());
        let recent = self.state.config.recent.clone();
        let selected_name = browser.selected_entry().map_or_else(
            || "No file selected".to_string(),
            |entry| entry.name.clone(),
        );

        let list_view = view.clone();
        let recent_view = view.clone();

        let header = h_flex()
            .h(px(52.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(section_label("OPEN PROGRAM", p))
            .child(chip("LOCAL ONLY", p))
            .child(chip(&format!("{entry_count} ITEMS"), p))
            .child(chip(&format!("{program_count} PROGRAMS"), p))
            .child(chip(&folder, p))
            .child(div().flex_1())
            .child(action_button(
                "browser-close",
                "CLOSE",
                true,
                false,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.close_browser(cx);
                },
            ));

        let path_row = h_flex()
            .h(px(TOUCH_TARGET + 12.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(p.border)
            .child(action_button(
                "browser-up",
                "UP",
                true,
                false,
                p,
                view.clone(),
                move |shell, window, cx| {
                    shell.browser_up(window, cx);
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(TOUCH_TARGET))
                    .flex()
                    .items_center()
                    .on_mouse_down(MouseButton::Left, {
                        let input = self.browser_path_input.clone();
                        move |_, window, cx| {
                            input.update(cx, |state, cx| state.focus(window, cx));
                        }
                    })
                    .child(
                        Input::new(&self.browser_path_input)
                            .h(px(TOUCH_TARGET))
                            .w_full()
                            .text_size(px(11.0)),
                    ),
            )
            .child(action_button(
                "browser-go",
                "GO",
                true,
                false,
                p,
                view.clone(),
                move |shell, window, cx| {
                    shell.navigate_browser(cx);
                    shell.sync_browser_input(window, cx);
                },
            ));

        let mut list = v_flex()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scrollbar()
            .gap_0();
        if entries.is_empty() {
            list = list.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .p_3()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.text_dim)
                            .child("No folders or program files here"),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(p.text_faint)
                            .child("Tap UP or type another folder path."),
                    ),
            );
        } else {
            for (index, entry) in entries.iter().enumerate() {
                list = list.child(browser_row(
                    entry,
                    index,
                    selected == Some(index),
                    p,
                    list_view.clone(),
                ));
            }
        }

        let mut recent_rows = v_flex().gap_1().p_1();
        if recent.is_empty() {
            recent_rows = recent_rows.child(
                div()
                    .text_size(px(9.5))
                    .text_color(p.text_faint)
                    .child("No recent files yet."),
            );
        }
        for (index, path) in recent.paths().iter().enumerate() {
            recent_rows = recent_rows.child(recent_row(path, index, p, recent_view.clone()));
        }

        let body = h_flex()
            .flex_1()
            .min_h(px(0.0))
            .items_stretch()
            .gap_0()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .h_full()
                    .child(list),
            )
            .child(
                v_flex()
                    .w(px(230.0))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .border_l_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .child(
                        h_flex()
                            .h(px(30.0))
                            .flex_shrink_0()
                            .items_center()
                            .px_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(section_label("RECENT", p)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scrollbar()
                            .child(recent_rows),
                    ),
            );

        let footer = h_flex()
            .h(px(TOUCH_TARGET + 12.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_t_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text_dim)
                    .child(selected_name),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(9.0))
                    .text_color(if error.is_some() {
                        p.danger
                    } else {
                        p.text_faint
                    })
                    .child(error.unwrap_or(path_text)),
            )
            .child(action_button(
                "browser-open",
                "OPEN",
                browser.selected_entry().is_some(),
                true,
                p,
                view.clone(),
                move |shell, _, cx| {
                    shell.open_selected(cx);
                },
            ));

        overlay_layer(layout, p)
            .child(
                v_flex()
                    .w(px(920.0))
                    .max_w_full()
                    .h(px(660.0))
                    .max_h_full()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.panel)
                    .child(header)
                    .child(path_row)
                    .child(body)
                    .child(footer),
            )
            .into_any_element()
    }

    fn analysis_error_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(message) = self.analysis.failure() else {
            return div().into_any_element();
        };
        let dismiss_view = view;
        h_flex()
            .min_h(px(36.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.danger)
            .bg(p.danger.alpha(0.12))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.danger)
                    .child("ANALYSIS FAILED"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text)
                    .child(message.to_string()),
            )
            .child(
                div()
                    .id("analysis-error-dismiss")
                    .h(px(28.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, _, cx| {
                        dismiss_view.update(cx, |shell, cx| {
                            shell.analysis = AnalysisState::Idle;
                            cx.notify();
                        });
                    })
                    .child("DISMISS"),
            )
            .into_any_element()
    }

    fn error_strip(&self, p: Palette, view: Entity<Self>) -> AnyElement {
        let Some(message) = self.load_error.clone() else {
            return div().into_any_element();
        };
        let dismiss_view = view;
        h_flex()
            .min_h(px(36.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(p.danger)
            .bg(p.danger.alpha(0.12))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.danger)
                    .child("LOAD FAILED"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(p.text)
                    .child(message),
            )
            .child(
                div()
                    .id("load-error-dismiss")
                    .h(px(28.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel_alt)
                    .text_size(px(8.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_dim)
                    .hover(|style| style.bg(p.raised_hover))
                    .on_click(move |_, _, cx| {
                        dismiss_view.update(cx, |shell, cx| {
                            shell.load_error = None;
                            cx.notify();
                        });
                    })
                    .child("DISMISS"),
            )
            .into_any_element()
    }
}

impl HasViewer for NcaShell {
    fn viewer_handle(&self) -> &ViewerHandle {
        &self.viewer
    }
}

impl Render for NcaShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_analysis(cx);
        self.state.ensure_index();
        if self.state.mode == AppMode::Simulate {
            self.poll_seek(cx);
            self.tick_playback(window, cx);
            self.refresh_sim_mesh(false);
            self.sync_scrub_slider(window, cx);
        }
        let viewport = window.viewport_size();
        let layout =
            ShellLayout::for_viewport(f32::from(viewport.width), f32::from(viewport.height));
        let p = Palette::for_mode(self.state.config.theme);
        if self.pending_search_reset {
            self.pending_search_reset = false;
            self.program_search_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if self.pending_setup_sync {
            self.pending_setup_sync = false;
            self.sync_setup_inputs(window, cx);
        }
        let view = cx.entity();
        let font = cx.theme().font_family.clone();

        let mut root = v_flex()
            .size_full()
            .bg(p.bg)
            .text_color(p.text)
            .font_family(font)
            .on_drop::<ExternalPaths>({
                let drop_view = view.clone();
                move |paths, _, cx| {
                    let paths = paths.paths().to_vec();
                    drop_view.update(cx, |shell, cx| shell.drop_paths(&paths, cx));
                }
            })
            .child(self.top_bar(layout, p, view.clone()))
            .child(self.status_strip(p))
            .child(self.sim_error_strip(p, view.clone()))
            .child(self.analysis_error_strip(p, view.clone()))
            .child(self.error_strip(p, view.clone()));
        if self.state.mode == AppMode::Simulate {
            root = root.child(self.transport_bar(p, view.clone()));
        }
        root = root.child(self.workspace(layout, p, view.clone()));
        root = root.child(self.analysis_dock(layout, p, view.clone()));

        if self.tool_popover_open {
            root = root.child(self.tool_popover(layout, p, view.clone()));
        }
        if self.browser.is_some() {
            root = root.child(self.browser_sheet(layout, p, view.clone()));
        }
        if self.state.settings_open {
            root = root.child(self.settings_sheet(layout, p, view.clone()));
        }
        if self.export_open {
            root = root.child(self.export_sheet(layout, p, view.clone()));
        }
        if self.legend_open {
            root = root.child(self.legend_sheet(layout, p, view));
        }

        root
    }
}

/// Height of the portrait program card after the viewer, legend, gaps, and
/// page padding are reserved. The viewer keeps at least half the viewport and
/// the program card fills the rest of the scroll column instead of leaving an
/// empty band at the bottom.
fn portrait_program_height(layout: ShellLayout) -> f32 {
    let reserved = layout.viewer_height + TOUCH_TARGET + 4.0 * crate::layout::GAP;
    (layout.content_height() - reserved).max(320.0)
}

/// Creates one text input for every setup field.
fn create_setup_inputs(
    window: &mut Window,
    cx: &mut Context<NcaShell>,
) -> BTreeMap<SetupField, Entity<InputState>> {
    let mut inputs = BTreeMap::new();
    for &field in SetupField::ALL {
        let placeholder = field.placeholder();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .clean_on_escape()
                .submit_on_enter(true)
        });
        inputs.insert(field, input);
    }
    inputs
}

/// A compact toggle with a dynamic element id.
fn sheet_toggle(
    id: String,
    label: String,
    on: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    sheet_button(id, label, on, p, view, action)
}

/// A compact button with a dynamic element id.
fn sheet_button(
    id: String,
    label: String,
    active: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(if active { p.accent } else { p.border })
        .bg(if active { p.accent_muted } else { p.panel })
        .text_size(px(8.0))
        .font_weight(FontWeight::BOLD)
        .text_color(if active { p.accent } else { p.text_faint })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised_hover))
        .on_click(move |_, window, cx| {
            view.update(cx, |shell, cx| action(shell, window, cx));
        })
        .child(label)
}

/// Provenance chip for one tool field.
fn provenance_chip(source: Option<FieldSource>, p: Palette) -> Div {
    let label = provenance_label(source);
    let color = match source {
        Some(FieldSource::Config) => p.accent,
        Some(FieldSource::Comment { .. }) => p.warning,
        None => p.text_faint,
    };
    chip(&label, p).text_color(color).border_color(color)
}

/// A filled chip.
fn chip(text: &str, p: Palette) -> Div {
    div()
        .h(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .px_2()
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(p.text_dim)
        .child(text.to_string())
}

/// A section label in the parent's 11 px uppercase style.
fn section_label(text: &str, p: Palette) -> Div {
    div()
        .text_size(px(11.0))
        .font_weight(FontWeight::BOLD)
        .text_color(p.text_dim)
        .child(text.to_string())
}

/// A 34 px transport action.
fn transport_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    active: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(30.0))
        .min_w(px(44.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(if active { p.accent } else { p.border })
        .bg(if active { p.accent_muted } else { p.panel })
        .text_size(px(9.0))
        .font_weight(FontWeight::BOLD)
        .text_color(if active {
            p.accent
        } else if enabled {
            p.text
        } else {
            p.text_faint
        });
    if enabled {
        button
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover).text_color(p.text))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| action(shell, window, cx));
            })
            .child(label)
    } else {
        button.opacity(0.45).child(label)
    }
}

/// A compact SIMULATE toggle chip.
fn compact_toggle(
    id: &'static str,
    label: &str,
    on: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(if on { p.accent } else { p.border })
        .bg(if on { p.accent_muted } else { p.panel })
        .text_size(px(8.0))
        .font_weight(FontWeight::BOLD)
        .text_color(if on { p.accent } else { p.text_faint })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised_hover))
        .on_click(move |_, window, cx| {
            view.update(cx, |shell, cx| action(shell, window, cx));
        })
        .child(label.to_string())
}

fn speed_button_id(speed: PlaybackSpeed) -> &'static str {
    match speed {
        PlaybackSpeed::Half => "sim-speed-half",
        PlaybackSpeed::One => "sim-speed-one",
        PlaybackSpeed::Two => "sim-speed-two",
        PlaybackSpeed::Four => "sim-speed-four",
    }
}

fn tool_shape_id(shape: ToolShape) -> &'static str {
    match shape {
        ToolShape::Flat => "tool-shape-flat",
        ToolShape::Ball => "tool-shape-ball",
        ToolShape::Bull => "tool-shape-bull",
        ToolShape::Drill => "tool-shape-drill",
        ToolShape::VBit => "tool-shape-vbit",
        ToolShape::Unknown => "tool-shape-unknown",
    }
}

/// A 48 px top-bar action.
fn action_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    active: bool,
    p: Palette,
    view: Entity<NcaShell>,
    action: impl Fn(&mut NcaShell, &mut Window, &mut Context<NcaShell>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(TOUCH_TARGET))
        .min_w(px(52.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .rounded_lg()
        .border_1()
        .border_color(if active { p.accent } else { p.border })
        .bg(if active { p.accent_muted } else { p.panel_alt })
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(if active {
            p.accent
        } else if enabled {
            p.text
        } else {
            p.text_faint
        });
    if enabled {
        button
            .cursor_pointer()
            .hover(|style| style.bg(p.raised_hover).text_color(p.text))
            .on_click(move |_, window, cx| {
                view.update(cx, |shell, cx| action(shell, window, cx));
            })
            .child(label)
    } else {
        button.opacity(0.45).child(label)
    }
}

fn theme_button_id(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::System => "theme-system",
        ThemeMode::Dark => "theme-dark",
        ThemeMode::Light => "theme-light",
        ThemeMode::HighContrast => "theme-high-contrast",
    }
}

/// Full-window sheet layer below the top bar.
fn overlay_layer(layout: ShellLayout, p: Palette) -> Div {
    div()
        .absolute()
        .top(px(layout.overlay_top()))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .bg(p.scrim)
}

fn severity_filter_id(filter: SeverityFilter) -> &'static str {
    match filter {
        SeverityFilter::All => "diagnostic-filter-all",
        SeverityFilter::Errors => "diagnostic-filter-errors",
        SeverityFilter::Warnings => "diagnostic-filter-warnings",
        SeverityFilter::Info => "diagnostic-filter-info",
    }
}

fn severity_color(severity: AnalysisSeverity, p: Palette) -> Hsla {
    match severity {
        AnalysisSeverity::Error => p.danger,
        AnalysisSeverity::Warning => p.warning,
        AnalysisSeverity::Info => p.info,
    }
}

fn severity_chip(severity: AnalysisSeverity, p: Palette) -> Div {
    let color = severity_color(severity, p);
    chip(severity.label().to_ascii_uppercase().as_str(), p)
        .text_color(color)
        .border_color(color)
}

/// One compact input row used by the diagnostics filter header.
fn compact_input(
    id: &'static str,
    input: &Entity<InputState>,
    placeholder: &'static str,
    p: Palette,
) -> Stateful<Div> {
    let input = input.clone();
    let focus_input = input.clone();
    div()
        .id(id)
        .flex_1()
        .min_w(px(0.0))
        .h(px(30.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(px(5.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .px_1()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            focus_input.update(cx, |state, cx| state.focus(window, cx));
        })
        .child(Input::new(&input).h(px(28.0)).w_full().text_size(px(10.0)))
        .tooltip({
            let text = placeholder.to_string();
            move |window, cx| Tooltip::new(text.clone()).build(window, cx)
        })
}

/// One 48 px finding row.
fn diagnostic_row(
    index: usize,
    diagnostic: &nc_analysis::Diagnostic,
    selected: bool,
    p: Palette,
    view: Entity<NcaShell>,
) -> Stateful<Div> {
    let color = severity_color(diagnostic.severity, p);
    let rule = diagnostic.rule.as_str().to_string();
    let message = diagnostic.message.clone();
    let line = diagnostic.line;
    div()
        .id(format!("diagnostic-row-{index}"))
        .h(px(TOUCH_TARGET))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .bg(if selected { p.accent_muted } else { p.panel })
        .border_l(px(3.0))
        .border_color(if selected { p.accent } else { p.panel })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised))
        .on_click(move |_, _, cx| {
            view.update(cx, |shell, cx| shell.select_diagnostic(index, cx));
        })
        .child(
            div()
                .w(px(9.0))
                .h(px(9.0))
                .flex_shrink_0()
                .rounded(px(5.0))
                .bg(color),
        )
        .child(
            div()
                .w(px(158.0))
                .flex_shrink_0()
                .truncate()
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(rule),
        )
        .child(
            div()
                .w(px(44.0))
                .flex_shrink_0()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(p.text_faint)
                .child(format!("L{line}")),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(10.0))
                .text_color(p.text)
                .child(message),
        )
}

fn coverage_group_card(group: &CoverageGroup, p: Palette, view: Entity<NcaShell>) -> Div {
    let target = group.target;
    let missing = group.missing_input.clone();
    let mut entries = v_flex().gap_0();
    for entry in &group.entries {
        entries = entries.child(
            h_flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .w(px(190.0))
                        .flex_shrink_0()
                        .truncate()
                        .text_size(px(8.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text_dim)
                        .child(entry.rule.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(9.0))
                        .text_color(p.text_faint)
                        .child(entry.reason.clone()),
                ),
        );
    }
    let action = match target.action_label() {
        Some(label) => {
            let action_view = view;
            let id = format!("coverage-action-{}", group.missing_input);
            div()
                .id(id)
                .h(px(24.0))
                .px_2()
                .flex_shrink_0()
                .flex()
                .items_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(p.accent)
                .bg(p.accent_muted)
                .text_size(px(8.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.accent)
                .cursor_pointer()
                .hover(|style| style.bg(p.raised_hover))
                .on_click(move |_, window, cx| {
                    action_view.update(cx, |shell, cx| {
                        shell.coverage_action(target, window, cx);
                    });
                })
                .child(label)
                .into_any_element()
        }
        None => chip(
            match target {
                CoverageTarget::RuleDisabled => "DISABLED",
                _ => "NO EDITOR",
            },
            p,
        )
        .into_any_element(),
    };
    v_flex()
        .gap_1()
        .p_2()
        .rounded(px(8.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .font_family("monospace")
                        .text_color(p.text)
                        .child(missing.clone()),
                )
                .child(chip(&format!("{} RULES", group.entries.len()), p))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .text_color(p.text_faint)
                        .child(target.description()),
                )
                .child(action),
        )
        .child(entries)
}

/// One 48 px program row.
fn program_row(
    line: &SourceLine,
    has_error: bool,
    severity: Option<AnalysisSeverity>,
    selected: bool,
    p: Palette,
    mono: SharedString,
    view: Entity<NcaShell>,
) -> impl IntoElement {
    let number = line.number;
    let n_word = line.n_word;
    let text = line.text.clone();
    let trimmed = text.trim_start();
    let is_comment = trimmed.starts_with('(') || trimmed.starts_with(';');
    let text_color = if selected {
        p.accent
    } else if has_error {
        p.danger
    } else if is_comment {
        p.text_faint
    } else {
        p.text
    };
    let background = if selected { p.accent_muted } else { p.panel };
    let border = if selected {
        p.accent
    } else if has_error {
        p.danger
    } else {
        p.panel
    };
    let gutter = match severity {
        Some(AnalysisSeverity::Error) => p.danger,
        Some(AnalysisSeverity::Warning) => p.warning,
        Some(AnalysisSeverity::Info) => p.info,
        None if has_error => p.danger,
        None => p.border_strong,
    };
    let filled = severity.is_some() || has_error;
    h_flex()
        .id(format!("program-line-{number}"))
        .h(px(TOUCH_TARGET))
        .items_center()
        .gap_2()
        .px_2()
        .bg(background)
        .border_l(px(3.0))
        .border_color(border)
        .cursor_pointer()
        .hover(|style| style.bg(p.raised))
        .on_click(move |_, _, cx| {
            view.update(cx, |shell, cx| {
                shell.seek_to_line(number, cx);
            });
        })
        .child(
            div()
                .w(px(10.0))
                .h(px(10.0))
                .flex_shrink_0()
                .rounded(px(5.0))
                .border_1()
                .border_color(gutter)
                .bg(if filled { gutter } else { p.panel }),
        )
        .child(
            div()
                .w(px(48.0))
                .flex_shrink_0()
                .text_size(px(9.5))
                .font_weight(if selected {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if selected { p.accent } else { p.text_faint })
                .child(number.to_string()),
        )
        .child(
            div()
                .w(px(38.0))
                .flex_shrink_0()
                .text_size(px(9.0))
                .text_color(p.text_faint)
                .child(n_word.map_or_else(String::new, |word| format!("N{word}"))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.5))
                .font_family(mono)
                .text_color(text_color)
                .child(text),
        )
}

/// One 48 px browser row.
fn browser_row(
    entry: &BrowserEntry,
    index: usize,
    selected: bool,
    p: Palette,
    view: Entity<NcaShell>,
) -> Stateful<Div> {
    let is_dir = entry.is_dir;
    let name = entry.name.clone();
    let kind = entry.kind_label();
    let meta = if is_dir {
        "FOLDER".to_string()
    } else {
        format_size(entry.size)
    };
    div()
        .id(format!("browser-entry-{index}"))
        .h(px(TOUCH_TARGET))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .bg(if selected { p.accent_muted } else { p.panel })
        .border_l(px(3.0))
        .border_color(if selected { p.accent } else { p.panel })
        .cursor_pointer()
        .hover(|style| style.bg(p.raised))
        .on_click(move |_, window, cx| {
            view.update(cx, |shell, cx| shell.browser_click(index, window, cx));
        })
        .child(
            div()
                .w(px(42.0))
                .flex_shrink_0()
                .text_size(px(8.0))
                .font_weight(FontWeight::BOLD)
                .text_color(if is_dir { p.warning } else { p.text_faint })
                .child(kind),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.0))
                .text_color(if is_dir { p.info } else { p.text })
                .child(name),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(9.0))
                .text_color(p.text_faint)
                .child(meta),
        )
}

/// One 48 px recent-file row with its own remove action.
fn recent_row(path: &Path, index: usize, p: Palette, view: Entity<NcaShell>) -> Stateful<Div> {
    let open_path = path.to_path_buf();
    let remove_path = path.to_path_buf();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_path(path));
    let tail = compact_path(path, 34);
    let missing = !path.is_file();
    let open_view = view.clone();
    let remove_view = view;
    h_flex()
        .id(format!("recent-{index}"))
        .h(px(TOUCH_TARGET))
        .items_center()
        .gap_2()
        .px_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .cursor_pointer()
        .hover(|style| style.bg(p.raised_hover))
        .on_click(move |_, _, cx| {
            let path = open_path.clone();
            open_view.update(cx, |shell, cx| shell.open_path(path, cx));
        })
        .child(
            v_flex()
                .flex_1()
                .min_w(px(32.0))
                .gap_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if missing { p.text_faint } else { p.text })
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(7.5))
                        .text_color(if missing { p.danger } else { p.text_faint })
                        .child(if missing {
                            format!("MISSING | {tail}")
                        } else {
                            tail
                        }),
                ),
        )
        .child(
            div()
                .id(format!("recent-remove-{index}"))
                .w(px(32.0))
                .h(px(32.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_size(px(9.0))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .hover(|style| style.bg(p.panel_alt).text_color(p.danger))
                .on_click(move |_, _, cx| {
                    remove_view.update(cx, |shell, cx| {
                        shell.state.config.remove_recent(&remove_path);
                        cx.notify();
                    });
                })
                .child("X"),
        )
}

/// One legend swatch: colour plus the stroke pattern from [`MoveKind::style`].
fn legend_swatch(style: nc_viewer::MoveStyle, color: Hsla) -> Div {
    let mut swatch = h_flex().w(px(30.0)).h(px(16.0)).items_center().gap_1();
    match style.dash {
        None => {
            swatch = swatch.child(div().w(px(22.0)).h(px(2.0)).bg(color));
        }
        Some([on, off]) => {
            let width = (on * 0.5).clamp(2.0, 5.0);
            let count = if on > 3.0 { 3 } else { 4 };
            let gap = if off > 2.5 { 3.0 } else { 2.0 };
            for _ in 0..count {
                swatch = swatch
                    .child(div().w(px(width)).h(px(2.0)).bg(color))
                    .child(div().w(px(gap)));
            }
        }
    }
    if let Some(marker) = style.marker {
        let text = match marker {
            nc_viewer::HoleMarker::Ring => "O",
            nc_viewer::HoleMarker::DashedRing => "~O",
            nc_viewer::HoleMarker::DoubleRing => "OO",
        };
        swatch = swatch.child(
            div()
                .text_size(px(7.5))
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(text),
        );
    }
    swatch
}

/// Five-step cut-depth ramp shown under the kinds.
fn cut_depth_ramp(p: Palette) -> Div {
    let mut ramp = h_flex().gap_1().items_center();
    for index in 0..5 {
        #[allow(clippy::cast_precision_loss)]
        let fraction = index as f32 / 4.0;
        ramp = ramp.child(
            div()
                .flex_1()
                .h(px(10.0))
                .rounded(px(2.0))
                .bg(crate::theme::cut_depth_color(fraction, p)),
        );
    }
    v_flex()
        .gap_1()
        .p_2()
        .border_t_1()
        .border_color(p.border)
        .child(section_label("CUT DEPTH", p))
        .child(ramp)
        .child(
            div()
                .text_size(px(7.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.text_faint)
                .child("SHALLOW -> DEEP"),
        )
}

/// All 13 kinds grouped by family, in legend order.
fn legend_groups() -> Vec<(MoveFamily, Vec<MoveKind>)> {
    MoveFamily::ALL
        .into_iter()
        .map(|family| {
            let kinds = MoveKind::ALL
                .into_iter()
                .filter(|kind| kind.family() == family)
                .collect::<Vec<_>>();
            (family, kinds)
        })
        .collect()
}

/// Representative colour for a collapsed family summary.
fn family_color(family: MoveFamily, p: Palette) -> Hsla {
    match family {
        MoveFamily::Travel => move_color(MoveKind::Rapid, p),
        MoveFamily::Cutting => move_color(MoveKind::Feed, p),
        MoveFamily::Arcs => move_color(MoveKind::ArcCw, p),
        MoveFamily::Holes => move_color(MoveKind::Drill, p),
        MoveFamily::Special => move_color(MoveKind::Probe, p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_groups_cover_every_kind_once() {
        let groups = legend_groups();
        assert_eq!(groups.len(), MoveFamily::ALL.len());
        let mut seen = 0;
        for (family, kinds) in groups {
            assert!(!kinds.is_empty(), "{family:?}");
            for kind in kinds {
                assert_eq!(kind.family(), family);
                seen += 1;
            }
        }
        assert_eq!(seen, MoveKind::ALL.len());
        assert_eq!(seen, 13);
    }

    #[test]
    fn every_legend_row_has_label_and_style() {
        for kind in MoveKind::ALL {
            let style = move_style(kind);
            assert!(!style.label.is_empty(), "{kind:?}");
            assert!(!kind.family().label().is_empty());
            assert!(!kind.short_label().is_empty());
        }
    }

    #[test]
    fn family_colors_are_deterministic() {
        let palette = Palette::dark();
        for family in MoveFamily::ALL {
            assert_eq!(family_color(family, palette), family_color(family, palette));
        }
    }
}
