//! Small application state, configuration, and program-loading model.
//!
//! This replaces the parent's 11k-line sender state. There is no controller,
//! session, DRO, tool library, or job history: only the loaded file, the view
//! preferences, and the recent-file list.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::collections::BTreeMap;

use nc_core::{Dialect, ToolTable};
use nc_gcode::{ParseError, ParserConfig, Program, SkippedBlock, parse_program_lossy};
use nc_stock::{InferenceBasis, StockModel, StockSource};
use nc_viewer::MoveKind;
use serde::{Deserialize, Serialize};

use crate::file_browser::{ProgramFile, RecentFiles};
use crate::program::{DEMO_NAME, DEMO_SOURCE, ProgramIndex, SourceLine, build_source_lines};
use crate::setup::{SetupState, sha256_hex};
use crate::theme::ThemeMode;

/// Config directory name.
const APP_DIR: &str = "nca-rs";
/// Config file name.
const CONFIG_FILE: &str = "app.json";

/// One of the two reserved application modes.
///
/// VIEW is the full static toolpath. SIMULATE shares the canvas and adds stock
/// removal, a tool pose, and transport controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppMode {
    /// Full static toolpath view.
    #[default]
    View,
    /// Stock cutting and transport.
    Simulate,
}

impl AppMode {
    /// Short uppercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::View => "VIEW",
            Self::Simulate => "SIMULATE",
        }
    }

    /// Whether this mode is implemented.
    #[must_use]
    pub const fn is_available(self) -> bool {
        true
    }
}

/// Parse status shown in the status strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseState {
    /// No program (never used after startup, which loads the demo).
    Empty,
    /// A read or parse is in flight.
    Loading,
    /// The built-in demo is active.
    Demo,
    /// A user file parsed with no errors.
    Parsed,
    /// A file was read but the parser reported errors.
    ParseErrors(usize),
}

impl ParseState {
    /// Status-strip label. There is deliberately no green "safe" state.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Empty => "NO PROGRAM".to_string(),
            Self::Loading => "PARSING".to_string(),
            Self::Demo => "DEMO".to_string(),
            Self::Parsed => "PARSED".to_string(),
            Self::ParseErrors(count) => format!("PARSE ERRORS {count}"),
        }
    }
}

/// A loaded source file, its optional parsed program, and inferred stock.
#[derive(Clone, Debug)]
pub struct LoadedProgram {
    /// Display name.
    pub name: String,
    /// Absolute path, or `None` for the built-in demo.
    pub path: Option<PathBuf>,
    /// Source text, kept for the program list even when parsing fails.
    pub source: Arc<str>,
    /// SHA-256 of the source bytes; the key for the local tool overlay.
    pub hash: String,
    /// Display lines with line and `N` numbers.
    pub lines: Arc<[SourceLine]>,
    /// Parsed program, possibly partial after lossy parsing.
    pub program: Option<Arc<Program>>,
    /// Parse errors, empty when every block parsed.
    pub errors: Vec<ParseError>,
    /// Blocks the lossy parser skipped, in source order.
    pub skipped_blocks: Vec<SkippedBlock>,
    /// First line after which new geometry is not trusted.
    pub geometry_uncertain_after: Option<usize>,
    /// Inferred stock for the trusted geometry, when a program exists.
    pub stock: Option<StockModel>,
    /// Whether this is the built-in demo.
    pub demo: bool,
}

impl LoadedProgram {
    /// Loads a validated file and parses it with the selected dialect.
    #[must_use]
    pub fn from_file(file: ProgramFile, dialect: Dialect) -> Self {
        Self::from_source_with_dialect(file.name, Some(file.path), file.source, false, dialect)
    }

    /// Parses source text lossily with Generic semantics and infers stock.
    #[must_use]
    pub fn from_source(name: String, path: Option<PathBuf>, source: String, demo: bool) -> Self {
        Self::from_source_with_dialect(name, path, source, demo, Dialect::Generic)
    }

    /// Parses source text lossily with an explicit dialect and infers stock.
    ///
    /// A skipped block never discards the rest of the file: the partial
    /// [`Program`] keeps the trusted prefix, and the errors, skipped blocks,
    /// and geometry-uncertainty line are retained for the UI and analysis.
    #[must_use]
    pub fn from_source_with_dialect(
        name: String,
        path: Option<PathBuf>,
        source: String,
        demo: bool,
        dialect: Dialect,
    ) -> Self {
        // Hash the bytes exactly as loaded. A file-level UTF-8 BOM stays in
        // `source` so the app and CLI report the same SHA-256 for the same
        // file; the parser strips it from each line for text and line mapping.
        let hash = sha256_hex(source.as_bytes());
        let source: Arc<str> = Arc::from(source);
        let parser_config = ParserConfig::default().with_dialect(dialect);
        let lossy = parse_program_lossy(&source, &parser_config);
        let stock = StockModel::infer(&lossy.program);
        let lines = build_source_lines(&source, Some(&lossy.program));
        let geometry_uncertain_after = lossy.program.geometry_uncertain_after();
        Self {
            name,
            path,
            source,
            hash,
            lines,
            program: Some(Arc::new(lossy.program)),
            errors: lossy.errors,
            skipped_blocks: lossy.skipped_blocks,
            geometry_uncertain_after,
            stock: Some(stock),
            demo,
        }
    }

    /// Whether any block was skipped or geometry stopped being trusted.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        !self.errors.is_empty()
            || !self.skipped_blocks.is_empty()
            || self.geometry_uncertain_after.is_some()
    }

    /// Whether new geometry after some line is not trusted.
    #[must_use]
    pub fn geometry_is_uncertain(&self) -> bool {
        self.geometry_uncertain_after.is_some()
    }

    /// The deterministic built-in demo.
    #[must_use]
    pub fn demo(dialect: Dialect) -> Self {
        Self::from_source_with_dialect(
            DEMO_NAME.to_string(),
            None,
            DEMO_SOURCE.to_string(),
            true,
            dialect,
        )
    }

    /// Re-parses the same bytes with a new dialect.
    ///
    /// The SHA-256 is unchanged, so the per-file tool overlay survives. The
    /// caller rebuilds the simulation and re-runs analysis after this.
    pub fn reparse(&mut self, dialect: Dialect) {
        let next = Self::from_source_with_dialect(
            self.name.clone(),
            self.path.clone(),
            self.source.to_string(),
            self.demo,
            dialect,
        );
        *self = next;
    }

    /// Number of source lines.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Distinct tool numbers in the parsed program.
    #[must_use]
    pub fn tool_count(&self) -> usize {
        self.program
            .as_ref()
            .map_or(0, |program| program.stats.tools.len())
    }

    /// Parse status.
    #[must_use]
    pub fn parse_state(&self) -> ParseState {
        if self.demo {
            return ParseState::Demo;
        }
        if self.errors.is_empty() {
            ParseState::Parsed
        } else {
            ParseState::ParseErrors(self.errors.len())
        }
    }

    /// Builds the visible-row mapping for a raw search query.
    #[must_use]
    pub fn index(&self, query: &str) -> ProgramIndex {
        ProgramIndex::new(&self.lines, query)
            .with_error_lines(self.errors.iter().map(|error| error.line))
    }

    /// Stock chip text.
    #[must_use]
    pub fn stock_label(&self) -> &'static str {
        match self.stock.as_ref().map(|stock| stock.source) {
            Some(StockSource::Explicit) => "STOCK: SET",
            Some(StockSource::Inferred) => "STOCK: INFERRED",
            None => "STOCK: NONE",
        }
    }

    /// Plain-language stock inference basis.
    #[must_use]
    pub fn stock_basis(&self) -> &'static str {
        let Some(stock) = self.stock.as_ref() else {
            return "no parsed geometry, so there is no stock box";
        };
        match stock.inference {
            Some(InferenceBasis::ZeroAtTop) => {
                "fitted to cutting bounds; a cut below Z0 puts the stock top at Z0"
            }
            Some(InferenceBasis::AllCutsAboveZero) => {
                "all cuts are above Z0; the top sits 2 mm above the highest cut"
            }
            Some(InferenceBasis::NoCuttingMoves) => {
                "no cutting moves; the box uses the full path bounds"
            }
            None => "entered by the operator, not inferred",
        }
    }

    /// Stock dimensions as display text.
    #[must_use]
    pub fn stock_size_text(&self) -> String {
        let Some(stock) = self.stock.as_ref() else {
            return "--".to_string();
        };
        let [x, y] = stock.xy_extent_mm();
        let height = stock.max_mm[2] - stock.min_mm[2];
        format!("{x:.1} x {y:.1} x {height:.1} mm")
    }
}

/// Persisted portion of the application configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct PersistedConfig {
    theme: String,
    recent: Vec<PathBuf>,
    last_dir: Option<PathBuf>,
    /// Tool overlays keyed by the source SHA-256, never by path.
    tool_tables: BTreeMap<String, ToolTable>,
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default().key().to_string(),
            recent: Vec::new(),
            last_dir: None,
            tool_tables: BTreeMap::new(),
        }
    }
}

/// Local-only application configuration.
#[derive(Clone, Debug)]
pub struct NcaConfig {
    /// Selected theme.
    pub theme: ThemeMode,
    /// Recent files, most recent first.
    pub recent: RecentFiles,
    /// Last browser directory.
    pub last_dir: Option<PathBuf>,
    /// Per-file tool overlays keyed by the source SHA-256.
    ///
    /// Comment hints are merged over one table at simulation and analysis
    /// time; user data always wins for a field it sets.
    pub tool_tables: BTreeMap<String, ToolTable>,
    path: Option<PathBuf>,
}

impl Default for NcaConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            recent: RecentFiles::new(),
            last_dir: None,
            tool_tables: BTreeMap::new(),
            path: None,
        }
    }
}

impl NcaConfig {
    /// Loads the local config, falling back to defaults on any error.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Loads a specific config file. A missing or corrupt file yields the
    /// defaults; the load path never panics or exits.
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        let fallback = Self {
            path: Some(path.to_path_buf()),
            ..Self::default()
        };
        let Ok(text) = fs::read_to_string(path) else {
            return fallback;
        };
        let Ok(persisted) = serde_json::from_str::<PersistedConfig>(&text) else {
            return fallback;
        };
        Self {
            theme: ThemeMode::from_key(&persisted.theme),
            recent: RecentFiles::from_paths(persisted.recent),
            last_dir: persisted.last_dir,
            tool_tables: persisted.tool_tables,
            path: Some(path.to_path_buf()),
        }
    }

    /// Saves the config. Errors are reported to the caller for one tracing
    /// warning; they never interrupt the UI.
    pub fn save(&self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .or_else(config_path)
            .ok_or_else(|| "no config directory is available".to_string())?;
        self.save_to(&path)
    }

    /// Saves to a specific path.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let persisted = PersistedConfig {
            theme: self.theme.key().to_string(),
            recent: self.recent.paths().to_vec(),
            last_dir: self.last_dir.clone(),
            tool_tables: self.tool_tables.clone(),
        };
        let text = serde_json::to_string_pretty(&persisted)
            .map_err(|error| format!("config serialization: {error}"))?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text).map_err(|error| format!("{}: {error}", temporary.display()))?;
        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!("{}: {error}", path.display())
        })
    }

    /// Records a successful open and persists the list.
    pub fn record_recent(&mut self, path: &Path) {
        self.recent.record(path.to_path_buf());
        self.last_dir = path.parent().map(Path::to_path_buf);
        self.persist();
    }

    /// Changes the theme and persists it.
    pub fn set_theme(&mut self, theme: ThemeMode) {
        self.theme = theme;
        self.persist();
    }

    /// Tool overlay for one source hash. Missing hashes yield an empty table.
    #[must_use]
    pub fn tools_for(&self, hash: &str) -> ToolTable {
        self.tool_tables.get(hash).cloned().unwrap_or_default()
    }

    /// Replaces the tool overlay for one source hash and persists it.
    pub fn set_tools_for(&mut self, hash: &str, table: ToolTable) {
        if table.is_empty() {
            self.tool_tables.remove(hash);
        } else {
            self.tool_tables.insert(hash.to_string(), table);
        }
        self.persist();
    }

    /// Removes one recent path and persists the list.
    pub fn remove_recent(&mut self, path: &Path) {
        if self.recent.remove(path) {
            self.persist();
        }
    }

    fn persist(&self) {
        if let Err(error) = self.save() {
            tracing::warn!(%error, "config was not saved");
        }
    }
}

/// `$XDG_CONFIG_HOME/nca-rs/app.json`, else `$HOME/.config/nca-rs/app.json`.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(xdg).join(APP_DIR).join(CONFIG_FILE));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Some(
            PathBuf::from(home)
                .join(".config")
                .join(APP_DIR)
                .join(CONFIG_FILE),
        );
    }
    std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(|appdata| PathBuf::from(appdata).join(APP_DIR).join(CONFIG_FILE))
}

/// UI state that is not part of the viewer camera.
#[derive(Clone, Debug)]
pub struct NcaState {
    /// Local configuration.
    pub config: NcaConfig,
    /// Editable analysis inputs: stock, tools, material, machine, rules, sim.
    pub setup: SetupState,
    /// Reserved mode slot.
    pub mode: AppMode,
    /// Currently displayed program.
    pub loaded: Option<LoadedProgram>,
    /// Parse status override while a background load is running.
    pub loading: bool,
    /// Current source line, 1-based.
    pub selected_line: Option<usize>,
    /// Raw program-search text.
    pub search: String,
    /// Per-kind legend visibility.
    visible_kinds: [bool; MoveKind::ALL.len()],
    /// Whether the 3D stock box is painted.
    pub stock_visible: bool,
    /// Settings sheet visibility.
    pub settings_open: bool,
    /// Cached visible-row mapping for the current source and query.
    index_cache: Option<ProgramIndex>,
    /// Identity of the cached mapping: source pointer, query, error count.
    index_key: Option<(usize, String, usize)>,
}

impl Default for NcaState {
    fn default() -> Self {
        Self::new(NcaConfig::load())
    }
}

impl NcaState {
    /// Creates state and loads the built-in demo.
    #[must_use]
    pub fn new(config: NcaConfig) -> Self {
        let setup = SetupState::default();
        let loaded = LoadedProgram::demo(setup.dialect);
        let mut state = Self {
            config,
            setup,
            mode: AppMode::View,
            loaded: Some(loaded),
            loading: false,
            selected_line: None,
            search: String::new(),
            visible_kinds: [true; MoveKind::ALL.len()],
            stock_visible: true,
            settings_open: false,
            index_cache: None,
            index_key: None,
        };
        state.sync_setup_program();
        state
    }

    /// Rebuilds the search mapping only when the source or query changed.
    ///
    /// The program panel calls this once per frame and then clones the cached
    /// `ProgramIndex`, so canvas gestures never rescan a million-line file.
    pub fn ensure_index(&mut self) {
        let Some(loaded) = self.loaded.as_ref() else {
            self.index_cache = None;
            self.index_key = None;
            return;
        };
        let source = Arc::as_ptr(&loaded.lines) as *const SourceLine as usize;
        let key = (source, self.search.clone(), loaded.errors.len());
        if self.index_key.as_ref() == Some(&key) {
            return;
        }
        self.index_cache = Some(loaded.index(&self.search));
        self.index_key = Some(key);
    }

    /// Cached visible-row mapping, when [`Self::ensure_index`] has run.
    #[must_use]
    pub fn index(&self) -> Option<&ProgramIndex> {
        self.index_cache.as_ref()
    }

    /// Loads the demo program, used when startup parsing of the demo fails.
    pub fn reset_to_demo(&mut self) {
        let dialect = self.setup.dialect;
        self.loaded = Some(LoadedProgram::demo(dialect));
        self.loading = false;
        self.selected_line = None;
        self.search.clear();
        self.sync_setup_program();
    }

    /// Adopts a successfully loaded file.
    pub fn adopt(&mut self, loaded: LoadedProgram) {
        if let Some(path) = loaded.path.as_deref() {
            self.config.record_recent(path);
        }
        self.loaded = Some(loaded);
        self.loading = false;
        self.selected_line = None;
        self.search.clear();
        self.sync_setup_program();
    }

    /// Rebuilds the tool inventory for the current program.
    pub fn sync_setup_program(&mut self) {
        if let Some(loaded) = self.loaded.as_ref() {
            self.setup.sync_program(loaded);
        } else {
            self.setup.inventory.clear();
            self.setup.selected_tool = None;
        }
    }

    /// SHA-256 of the current source, used only as a local tool-table key.
    #[must_use]
    pub fn loaded_hash(&self) -> Option<&str> {
        self.loaded.as_ref().map(|loaded| loaded.hash.as_str())
    }

    /// User tool overlay for the current file.
    #[must_use]
    pub fn user_tools(&self) -> ToolTable {
        self.loaded_hash()
            .map(|hash| self.config.tools_for(hash))
            .unwrap_or_default()
    }

    /// User overlay merged with comment hints for one program.
    #[must_use]
    pub fn resolved_tools(&self, program: &Program) -> ToolTable {
        ToolTable::resolve(&self.user_tools(), &program.tool_hints)
    }

    /// Stock model used by the chips and simulation.
    #[must_use]
    pub fn effective_stock(&self) -> Option<StockModel> {
        let inferred = self.loaded.as_ref().and_then(|loaded| loaded.stock);
        self.setup.effective_stock(inferred)
    }

    /// Truthful stock chip text.
    #[must_use]
    pub fn stock_label(&self) -> &'static str {
        if self
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.stock)
            .is_none()
            && self.setup.analysis.stock.is_none()
        {
            return "STOCK: NONE";
        }
        self.setup.stock_label()
    }

    /// Stock source explanation with inferred basis and top Z.
    #[must_use]
    pub fn stock_basis_text(&self) -> String {
        let inferred = self
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.stock.as_ref());
        self.setup.stock_basis_text(inferred)
    }

    /// Effective stock dimensions as display text.
    #[must_use]
    pub fn stock_size_text(&self) -> String {
        let Some(stock) = self.effective_stock() else {
            return "--".to_string();
        };
        let [x, y] = stock.xy_extent_mm();
        let height = stock.max_mm[2] - stock.min_mm[2];
        format!("{x:.1} x {y:.1} x {height:.1} mm")
    }

    /// Display name of the loaded file.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.loaded
            .as_ref()
            .map_or("no program", |loaded| loaded.name.as_str())
    }

    /// Line count.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.loaded.as_ref().map_or(0, LoadedProgram::line_count)
    }

    /// Parse status for the status strip.
    #[must_use]
    pub fn parse_state(&self) -> ParseState {
        if self.loading {
            return ParseState::Loading;
        }
        self.loaded
            .as_ref()
            .map_or(ParseState::Empty, LoadedProgram::parse_state)
    }

    /// Visibility of one semantic move kind.
    #[must_use]
    pub fn kind_visible(&self, kind: MoveKind) -> bool {
        self.visible_kinds[kind as usize]
    }

    /// Toggles one semantic move kind.
    pub fn toggle_kind(&mut self, kind: MoveKind) {
        let slot = &mut self.visible_kinds[kind as usize];
        *slot = !*slot;
    }

    /// Restores every kind.
    pub fn show_all_kinds(&mut self) {
        self.visible_kinds = [true; MoveKind::ALL.len()];
    }

    /// Number of enabled kinds.
    #[must_use]
    pub fn visible_kind_count(&self) -> usize {
        self.visible_kinds
            .iter()
            .filter(|visible| **visible)
            .count()
    }

    /// Visibility flags indexed by [`MoveKind`].
    ///
    /// The canvas painter gates exactly this kind: hiding `ArcCw` never hides
    /// `ArcCcw`, `HelixCw`, or any other kind.
    #[must_use]
    pub fn visibility(&self) -> [bool; MoveKind::ALL.len()] {
        MoveKind::ALL.map(|kind| self.kind_visible(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::Tool;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "nca-app-config-{tag}-{}-{nanos}.json",
            std::process::id()
        ))
    }

    #[test]
    fn demo_parses_and_infers_stock() {
        let loaded = LoadedProgram::demo(Dialect::Generic);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let program = loaded.program.as_ref().expect("demo parse");
        assert!(program.stats.arc_moves >= 2);
        assert!(program.stats.canned_cycles >= 2);
        assert!(program.stats.tool_changes >= 1);
        assert!(loaded.line_count() > 10);
        assert_eq!(loaded.parse_state(), ParseState::Demo);
        assert!(loaded.stock.is_some());
        assert_eq!(loaded.stock_label(), "STOCK: INFERRED");
        assert!(loaded.stock_basis().contains("cutting bounds"));
    }

    #[test]
    fn search_index_is_cached_between_frames() {
        let mut state = NcaState::new(NcaConfig::default());
        state.ensure_index();
        let first = state.index().expect("cached index");
        assert_eq!(first.visible_count(), state.line_count());
        assert_eq!(first.stored_match_count(), 0);

        state.search = "G1".to_string();
        state.ensure_index();
        let filtered = state.index().expect("cached filtered index");
        assert!(filtered.is_active());
        assert!(filtered.visible_count() < state.line_count());

        // A second frame with the same query reuses the same Arc allocation.
        let matches = filtered.stored_match_count();
        state.ensure_index();
        assert_eq!(
            state
                .index()
                .expect("cached filtered index")
                .stored_match_count(),
            matches
        );
    }

    #[test]
    fn parse_errors_keep_the_partial_program_and_source_list() {
        let source = "G21 G90\nG1 X\nG1 X10 F100\nG1 Y10\n";
        let loaded =
            LoadedProgram::from_source("bad.nc".to_string(), None, source.to_string(), false);
        let program = loaded.program.as_ref().expect("partial program");
        assert_eq!(loaded.line_count(), 4);
        assert!(!loaded.errors.is_empty());
        assert!(!loaded.skipped_blocks.is_empty());
        assert_eq!(loaded.skipped_blocks[0].line, 2);
        assert_eq!(loaded.skipped_blocks[0].source, "G1 X");
        assert_eq!(loaded.geometry_uncertain_after, None);
        assert_eq!(
            loaded.geometry_uncertain_after,
            program.geometry_uncertain_after()
        );
        assert!(!program.motions.is_empty(), "trusted prefix must be kept");
        assert!(matches!(loaded.parse_state(), ParseState::ParseErrors(_)));
        assert_eq!(loaded.stock_label(), "STOCK: INFERRED");
        let index = loaded.index("");
        assert!(index.has_error(2));
    }

    #[test]
    fn unsupported_code_fixture_loads_partial_and_analyzes() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/synthetic/unsupported-code/source.nc");
        let file = crate::file_browser::read_program(&fixture).expect("fixture reads");
        let loaded = LoadedProgram::from_file(file, Dialect::Generic);
        let program = loaded.program.as_ref().expect("partial program");
        assert!(!program.motions.is_empty(), "the prefix before M98 is kept");
        assert_eq!(loaded.geometry_uncertain_after, Some(9));
        assert_eq!(program.geometry_uncertain_after(), Some(9));
        assert!(
            loaded
                .skipped_blocks
                .iter()
                .any(|block| block.line == 9 && block.reason == "unsupported_code")
        );

        let stock = loaded.stock.expect("inferred stock");
        let simulation = nc_sim::Simulation::new(
            program,
            stock,
            &ToolTable::default(),
            &nc_sim::SimConfig::default(),
        );
        assert!(
            simulation.is_ok(),
            "the trusted prefix must simulate: {:?}",
            simulation.err()
        );

        let config = nc_analysis::AnalysisConfig::default();
        let request = nc_analysis::AnalysisRequest::new(
            &loaded.name,
            &loaded.source,
            &config,
            Dialect::Generic,
        )
        .with_dialect_assumed(true);
        let report = nc_analysis::analyze(
            program,
            &request,
            &nc_analysis::RuleRegistry::from_rules(nc_analysis::rules::all()),
        );
        assert_eq!(report.input.geometry_uncertain_after, Some(9));
        let unsupported = report
            .diagnostics
            .iter()
            .find(|finding| finding.rule.as_str() == "parse.unsupported")
            .expect("parse.unsupported finding");
        assert_eq!(unsupported.line, 9);
        assert!(
            unsupported
                .evidence
                .get("raw")
                .is_some_and(|value| value.display_text() == "M98 P1000")
        );
    }

    #[test]
    fn macro_only_file_loads_partial_and_analyzes() {
        let loaded = LoadedProgram::from_source(
            "macro.nc".to_string(),
            None,
            "M98 P1000\n".to_string(),
            false,
        );
        let program = loaded.program.as_ref().expect("partial program");
        assert!(program.motions.is_empty(), "M98 emits no geometry");
        assert_eq!(loaded.geometry_uncertain_after, Some(1));
        assert!(loaded.geometry_is_uncertain());
        assert!(loaded.is_partial());
        assert_eq!(loaded.skipped_blocks.len(), 1);
        assert_eq!(loaded.skipped_blocks[0].line, 1);
        assert!(!loaded.errors.is_empty());
        assert!(matches!(loaded.parse_state(), ParseState::ParseErrors(_)));

        let config = nc_analysis::AnalysisConfig::default();
        let request = nc_analysis::AnalysisRequest::new(
            &loaded.name,
            &loaded.source,
            &config,
            Dialect::Generic,
        )
        .with_dialect_assumed(true);
        let report = nc_analysis::analyze(
            program,
            &request,
            &nc_analysis::RuleRegistry::from_rules(nc_analysis::rules::all()),
        );
        assert_eq!(report.input.geometry_uncertain_after, Some(1));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| finding.rule.as_str() == "parse.unsupported" && finding.line == 1)
        );
    }

    #[test]
    fn bom_file_hash_is_raw_bytes_and_matches_the_report() {
        let path = scratch_path("bom").with_extension("nc");
        let text = "G21 G90\nG0 X0 Y0\nG1 X10 F100\n";
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(text.as_bytes());
        fs::write(&path, &bytes).expect("write bom file");

        let file = crate::file_browser::read_program(&path).expect("read bom file");
        assert!(
            file.source.starts_with('\u{feff}'),
            "the BOM is kept in source"
        );
        let loaded = LoadedProgram::from_file(file, Dialect::Generic);
        let program = loaded.program.as_ref().expect("program");
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        assert_eq!(loaded.lines[0].text, "G21 G90");
        assert_eq!(loaded.hash, sha256_hex(&bytes));
        assert_ne!(
            loaded.hash,
            sha256_hex(text.as_bytes()),
            "the BOM must be part of the hashed bytes"
        );

        let config = nc_analysis::AnalysisConfig::default();
        let request = nc_analysis::AnalysisRequest::new(
            &loaded.name,
            &loaded.source,
            &config,
            Dialect::Generic,
        )
        .with_dialect_assumed(true);
        let report = nc_analysis::analyze(
            program,
            &request,
            &nc_analysis::RuleRegistry::from_rules(nc_analysis::rules::all()),
        );
        assert_eq!(report.input.sha256, loaded.hash);
        assert_eq!(report.input.sha256, sha256_hex(&bytes));
        assert_eq!(report.input.line_count, 3);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn config_round_trips_theme_and_recent_files() {
        let path = scratch_path("round-trip");
        let mut config = NcaConfig::load_from(&path);
        config.theme = ThemeMode::HighContrast;
        config.recent.record(PathBuf::from("/tmp/a.nc"));
        config.recent.record(PathBuf::from("/tmp/b.nc"));
        config.last_dir = Some(PathBuf::from("/tmp"));
        let mut tool = Tool::new(3);
        tool.diameter_mm = Some(6.0);
        tool.shape = nc_core::ToolShape::Flat;
        let mut table = ToolTable::new();
        table.insert(tool);
        config.set_tools_for("hash-three", table);
        config.save_to(&path).expect("save");

        let loaded = NcaConfig::load_from(&path);
        assert_eq!(loaded.theme, ThemeMode::HighContrast);
        assert_eq!(loaded.recent.len(), 2);
        assert_eq!(loaded.recent.paths()[0], PathBuf::from("/tmp/b.nc"));
        assert_eq!(loaded.last_dir, Some(PathBuf::from("/tmp")));
        assert_eq!(
            loaded
                .tools_for("hash-three")
                .get(3)
                .and_then(|tool| tool.diameter_mm),
            Some(6.0)
        );
        assert!(loaded.tools_for("hash-other").is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn corrupt_config_falls_back_without_panicking() {
        let path = scratch_path("corrupt");
        fs::write(&path, "{not json").expect("write");
        let config = NcaConfig::load_from(&path);
        assert_eq!(config.theme, ThemeMode::System);
        assert!(config.recent.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legend_toggles_gate_exactly_one_kind() {
        let mut state = NcaState::new(NcaConfig::default());
        assert_eq!(state.visible_kind_count(), MoveKind::ALL.len());
        assert!(state.visibility().into_iter().all(|visible| visible));

        state.toggle_kind(MoveKind::ArcCw);
        assert!(!state.kind_visible(MoveKind::ArcCw));
        assert_eq!(state.visible_kind_count(), MoveKind::ALL.len() - 1);
        let visibility = state.visibility();
        for kind in MoveKind::ALL {
            assert_eq!(
                visibility[kind as usize],
                kind != MoveKind::ArcCw,
                "{kind:?} changed with ArcCw"
            );
        }

        state.toggle_kind(MoveKind::ArcCw);
        assert!(state.visibility().into_iter().all(|visible| visible));

        for kind in MoveKind::ALL {
            state.toggle_kind(kind);
            assert!(!state.kind_visible(kind), "{kind:?}");
            assert_eq!(state.visible_kind_count(), MoveKind::ALL.len() - 1);
            assert!(
                state
                    .visibility()
                    .into_iter()
                    .enumerate()
                    .all(|(index, visible)| visible == (index != kind as usize)),
                "toggling {kind:?} changed another kind"
            );
            state.toggle_kind(kind);
        }
        state.show_all_kinds();
        assert!(state.visibility().into_iter().all(|visible| visible));
    }
}
