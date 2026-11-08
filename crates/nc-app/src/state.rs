//! Small application state, configuration, and program-loading model.
//!
//! This replaces the parent's 11k-line sender state. There is no controller,
//! session, DRO, tool library, or job history: only the loaded file, the view
//! preferences, and the recent-file list.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_gcode::{ParseError, Program, parse_program};
use nc_stock::{InferenceBasis, StockModel, StockSource};
use nc_viewer::{MoveKind, SegmentKind};
use serde::{Deserialize, Serialize};

use crate::file_browser::{ProgramFile, RecentFiles};
use crate::program::{DEMO_NAME, DEMO_SOURCE, ProgramIndex, SourceLine, build_source_lines};
use crate::theme::ThemeMode;

/// Config directory name.
const APP_DIR: &str = "nca-rs";
/// Config file name.
const CONFIG_FILE: &str = "app.json";

/// One of the two reserved application modes.
///
/// Only [`AppMode::View`] is implemented in this milestone. SIMULATE keeps its
/// top-bar slot so the later transport milestone does not reflow the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppMode {
    /// Full static toolpath view.
    #[default]
    View,
    /// Reserved for stock cutting and transport.
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
        matches!(self, Self::View)
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
    /// Display lines with line and `N` numbers.
    pub lines: Arc<[SourceLine]>,
    /// Parsed program, when the parser accepted the file.
    pub program: Option<Arc<Program>>,
    /// Parse errors, empty on success.
    pub errors: Vec<ParseError>,
    /// Inferred stock for a successful parse.
    pub stock: Option<StockModel>,
    /// Whether this is the built-in demo.
    pub demo: bool,
}

impl LoadedProgram {
    /// Loads a validated file and parses it.
    #[must_use]
    pub fn from_file(file: ProgramFile) -> Self {
        Self::from_source(file.name, Some(file.path), file.source, false)
    }

    /// Parses source text and infers stock.
    #[must_use]
    pub fn from_source(name: String, path: Option<PathBuf>, source: String, demo: bool) -> Self {
        let source: Arc<str> = Arc::from(source);
        match parse_program(&source) {
            Ok(program) => {
                let stock = StockModel::infer(&program);
                let lines = build_source_lines(&source, Some(&program));
                Self {
                    name,
                    path,
                    source,
                    lines,
                    program: Some(Arc::new(program)),
                    errors: Vec::new(),
                    stock: Some(stock),
                    demo,
                }
            }
            Err(errors) => {
                let lines = build_source_lines(&source, None);
                Self {
                    name,
                    path,
                    source,
                    lines,
                    program: None,
                    errors,
                    stock: None,
                    demo,
                }
            }
        }
    }

    /// The deterministic built-in demo.
    #[must_use]
    pub fn demo() -> Self {
        Self::from_source(DEMO_NAME.to_string(), None, DEMO_SOURCE.to_string(), true)
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
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default().key().to_string(),
            recent: Vec::new(),
            last_dir: None,
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
    path: Option<PathBuf>,
}

impl Default for NcaConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            recent: RecentFiles::new(),
            last_dir: None,
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
        Self {
            config: NcaConfig::load(),
            mode: AppMode::View,
            loaded: Some(LoadedProgram::demo()),
            loading: false,
            selected_line: None,
            search: String::new(),
            visible_kinds: [true; MoveKind::ALL.len()],
            stock_visible: true,
            settings_open: false,
            index_cache: None,
            index_key: None,
        }
    }
}

impl NcaState {
    /// Creates state and loads the built-in demo.
    #[must_use]
    pub fn new(config: NcaConfig) -> Self {
        Self {
            config,
            mode: AppMode::View,
            loaded: Some(LoadedProgram::demo()),
            loading: false,
            selected_line: None,
            search: String::new(),
            visible_kinds: [true; MoveKind::ALL.len()],
            stock_visible: true,
            settings_open: false,
            index_cache: None,
            index_key: None,
        }
    }

    /// Rebuilds the search mapping only when the source or query changed.
    ///
    /// The program panel calls this once per frame and then clones the cached
    /// [`ProgramIndex`], so canvas gestures never rescan a million-line file.
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
        self.loaded = Some(LoadedProgram::demo());
        self.loading = false;
        self.selected_line = None;
        self.search.clear();
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

    /// Whether a display segment group should be painted.
    ///
    /// The parser still emits the five display [`SegmentKind`]s, so a group is
    /// painted when any semantic kind it can represent is enabled. This keeps
    /// the legend honest while the upstream taxonomy migration is pending.
    #[must_use]
    pub fn segment_visible(&self, kind: SegmentKind) -> bool {
        match kind {
            SegmentKind::Rapid => {
                self.kind_visible(MoveKind::Rapid) || self.kind_visible(MoveKind::Retract)
            }
            SegmentKind::Feed => [
                MoveKind::Feed,
                MoveKind::Ramp,
                MoveKind::Drill,
                MoveKind::Peck,
                MoveKind::Bore,
            ]
            .into_iter()
            .any(|kind| self.kind_visible(kind)),
            SegmentKind::Plunge => self.kind_visible(MoveKind::Plunge),
            SegmentKind::Arc => [
                MoveKind::ArcCw,
                MoveKind::ArcCcw,
                MoveKind::HelixCw,
                MoveKind::HelixCcw,
            ]
            .into_iter()
            .any(|kind| self.kind_visible(kind)),
            SegmentKind::Probe => self.kind_visible(MoveKind::Probe),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let loaded = LoadedProgram::demo();
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
    fn parse_errors_keep_the_source_list() {
        let loaded = LoadedProgram::from_source(
            "bad.nc".to_string(),
            None,
            "G21 G90\nG1 X\n".to_string(),
            false,
        );
        assert!(loaded.program.is_none());
        assert_eq!(loaded.line_count(), 2);
        assert!(!loaded.errors.is_empty());
        assert!(matches!(loaded.parse_state(), ParseState::ParseErrors(_)));
        assert_eq!(loaded.stock_label(), "STOCK: NONE");
        let index = loaded.index("");
        assert!(index.has_error(2));
    }

    #[test]
    fn config_round_trips_theme_and_recent_files() {
        let path = scratch_path("round-trip");
        let mut config = NcaConfig {
            theme: ThemeMode::HighContrast,
            ..NcaConfig::default()
        };
        config.recent.record(PathBuf::from("/tmp/a.nc"));
        config.recent.record(PathBuf::from("/tmp/b.nc"));
        config.last_dir = Some(PathBuf::from("/tmp"));
        config.save_to(&path).expect("save");

        let loaded = NcaConfig::load_from(&path);
        assert_eq!(loaded.theme, ThemeMode::HighContrast);
        assert_eq!(loaded.recent.len(), 2);
        assert_eq!(loaded.recent.paths()[0], PathBuf::from("/tmp/b.nc"));
        assert_eq!(loaded.last_dir, Some(PathBuf::from("/tmp")));
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
    fn legend_toggles_gate_display_groups() {
        let mut state = NcaState::new(NcaConfig::default());
        assert_eq!(state.visible_kind_count(), 13);
        state.toggle_kind(MoveKind::Rapid);
        assert!(!state.kind_visible(MoveKind::Rapid));
        // Retract is still enabled, so the combined rapid display group stays.
        assert!(state.segment_visible(SegmentKind::Rapid));
        state.toggle_kind(MoveKind::Retract);
        assert!(!state.segment_visible(SegmentKind::Rapid));
        state.show_all_kinds();
        assert!(state.segment_visible(SegmentKind::Arc));
    }
}
