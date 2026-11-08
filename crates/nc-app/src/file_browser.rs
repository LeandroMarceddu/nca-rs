//! In-app, local-only program browser and bounded source reader.
//!
//! The browser lists one directory at a time: visible subdirectories and files
//! whose extension is `.nc`, `.gcode`, `.tap`, or `.ngc` (ASCII
//! case-insensitive). Hidden entries and symlinks are skipped, so navigation
//! cannot loop through a link or surprise the operator with a dotfile.
//!
//! Nothing here connects to a network or sends a command. The selected text is
//! parsed by the app with [`nc_gcode::parse_program`](nc_gcode::parse_program);
//! the browser itself never interprets G-code.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Largest program file accepted: 20 MiB.
pub const MAX_PROGRAM_BYTES: u64 = 20 * 1024 * 1024;

/// Extensions shown by the browser and accepted by [`read_program`].
pub const PROGRAM_EXTENSIONS: [&str; 4] = ["nc", "gcode", "tap", "ngc"];

/// Maximum entries listed from one directory.
///
/// A directory with more entries still opens; the tail is omitted and the
/// path field remains the way to reach a nested folder.
pub const MAX_DIRECTORY_ENTRIES: usize = 4096;

/// Maximum remembered recent files.
pub const MAX_RECENT_FILES: usize = 8;

/// One visible directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserEntry {
    /// Display name (file or directory name only).
    pub name: String,
    /// Absolute path used to enter or load the entry.
    pub path: PathBuf,
    /// `true` for a directory.
    pub is_dir: bool,
    /// File length in bytes; zero for a directory.
    pub size: u64,
}

impl BrowserEntry {
    /// Short kind label for the row badge: `DIR` or the upper-case extension.
    #[must_use]
    pub fn kind_label(&self) -> String {
        if self.is_dir {
            return "DIR".to_string();
        }
        self.path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| "FILE".to_string())
    }
}

/// Why a directory could not be listed or entered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The path is not a directory.
    NotADirectory(String),
    /// A selected file is not a supported program.
    NotAProgramFile(String),
    /// The filesystem returned an error.
    Io(String),
}

impl fmt::Display for BrowseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(path) => write!(formatter, "'{path}' is not a readable folder"),
            Self::NotAProgramFile(name) => write!(
                formatter,
                "'{name}' is not a .nc, .gcode, .tap, or .ngc program"
            ),
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

/// Why a selected program file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramFileError {
    /// The path is not a regular file.
    NotAFile(String),
    /// The extension is outside [`PROGRAM_EXTENSIONS`].
    UnsupportedExtension(String),
    /// The file is larger than [`MAX_PROGRAM_BYTES`].
    TooLarge {
        /// Actual length in bytes.
        bytes: u64,
        /// Configured limit in bytes.
        limit: u64,
    },
    /// The bytes contain a NUL, so this is not a text program.
    Binary,
    /// The bytes are not valid UTF-8.
    NotUtf8,
    /// The file has no non-whitespace text.
    Empty,
    /// The file could not be read.
    Io(String),
}

impl fmt::Display for ProgramFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAFile(path) => write!(formatter, "'{path}' is not a regular file"),
            Self::UnsupportedExtension(name) => write!(
                formatter,
                "'{name}' is not a .nc, .gcode, .tap, or .ngc program"
            ),
            Self::TooLarge { bytes, limit } => write!(
                formatter,
                "file is {}; the browser limit is {}",
                format_size(*bytes),
                format_size(*limit)
            ),
            Self::Binary => formatter.write_str("file contains binary data, not a G-code program"),
            Self::NotUtf8 => formatter.write_str("file is not valid UTF-8 text"),
            Self::Empty => formatter.write_str("file is empty"),
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

/// One validated program file ready for parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramFile {
    /// File name shown in the job header and program list.
    pub name: String,
    /// Absolute path.
    pub path: PathBuf,
    /// Complete UTF-8 source text, including a file-level BOM when present.
    ///
    /// Keeping the BOM makes the app hash and report the raw file bytes, which
    /// matches the CLI. Display and parsing strip it where text is consumed.
    pub source: String,
}

impl ProgramFile {
    /// Source length in bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.source.len()
    }
}

/// True when the path ends in a supported program extension.
#[must_use]
pub fn is_program_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROGRAM_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// Start directory for the first browser open.
#[must_use]
pub fn default_root() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Human size for rows and messages.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= MIB {
        format!("{:.1} MB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Full path as display text, lossily.
#[must_use]
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Abbreviates a path from the left when it is longer than `max_chars`.
#[must_use]
pub fn compact_path(path: &Path, max_chars: usize) -> String {
    let text = display_path(path);
    let count = text.chars().count();
    if count <= max_chars || max_chars == 0 {
        return text;
    }
    let keep = max_chars.saturating_sub(3);
    let skip = count.saturating_sub(keep);
    let tail = text.chars().skip(skip).collect::<String>();
    format!("...{tail}")
}

/// Lists one directory: visible subdirectories and supported program files.
///
/// Directories sort first; each group sorts by ASCII case-insensitive name.
/// Hidden entries, symlinks, and other file types are omitted.
///
/// # Errors
///
/// Returns [`BrowseError::NotADirectory`] for a non-directory and
/// [`BrowseError::Io`] when the directory cannot be read.
pub fn list_directory(directory: &Path) -> Result<Vec<BrowserEntry>, BrowseError> {
    if !directory.is_dir() {
        return Err(BrowseError::NotADirectory(display_path(directory)));
    }
    let read_dir = fs::read_dir(directory)
        .map_err(|error| BrowseError::Io(format!("{}: {error}", display_path(directory))))?;
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            entries.push(BrowserEntry {
                name,
                path,
                is_dir: true,
                size: 0,
            });
        } else if file_type.is_file() && is_program_file(&path) {
            let size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            entries.push(BrowserEntry {
                name,
                path,
                is_dir: false,
                size,
            });
        }
    }
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

/// Reads one program file with a hard size bound.
///
/// # Errors
///
/// Returns a structured reason for a non-file, unsupported extension, oversized
/// file, binary content, invalid UTF-8, empty text, or I/O failure.
pub fn read_program(path: &Path) -> Result<ProgramFile, ProgramFileError> {
    let display = display_path(path);
    let metadata =
        fs::metadata(path).map_err(|error| ProgramFileError::Io(format!("{display}: {error}")))?;
    if !metadata.is_file() {
        return Err(ProgramFileError::NotAFile(display));
    }
    if !is_program_file(path) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(display);
        return Err(ProgramFileError::UnsupportedExtension(name));
    }
    if metadata.len() > MAX_PROGRAM_BYTES {
        return Err(ProgramFileError::TooLarge {
            bytes: metadata.len(),
            limit: MAX_PROGRAM_BYTES,
        });
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".to_string());
    let absolute = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file = fs::File::open(path)
        .map_err(|error| ProgramFileError::Io(format!("{display}: {error}")))?;
    let mut bytes = Vec::new();
    file.take(MAX_PROGRAM_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ProgramFileError::Io(format!("{display}: {error}")))?;
    if bytes.len() as u64 > MAX_PROGRAM_BYTES {
        return Err(ProgramFileError::TooLarge {
            bytes: bytes.len() as u64,
            limit: MAX_PROGRAM_BYTES,
        });
    }
    if bytes.contains(&0) {
        return Err(ProgramFileError::Binary);
    }
    let text = String::from_utf8(bytes).map_err(|_| ProgramFileError::NotUtf8)?;
    // A BOM-only file is empty even though the BOM is not whitespace. Keep the
    // BOM in `source` so hashing covers the raw bytes the CLI also hashes.
    let content = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if content.trim().is_empty() {
        return Err(ProgramFileError::Empty);
    }
    Ok(ProgramFile {
        name,
        path: absolute,
        source: text,
    })
}

/// Resolves an initial browser path.
fn locate(start: &Path) -> (PathBuf, Option<PathBuf>) {
    if start.is_dir() {
        let cwd = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
        return (cwd, None);
    }
    if start.is_file() {
        let parent = start.parent().unwrap_or_else(|| Path::new("/"));
        let cwd = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        let file = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
        return (cwd, Some(file));
    }
    let fallback = default_root();
    (fallback, None)
}

/// State of one open file browser.
///
/// The model is pure data so navigation, filtering, and selection can be
/// tested without a window. The shell owns the GPUI input, scroll handle, and
/// sheet.
#[derive(Clone, Debug)]
pub struct FileBrowser {
    cwd: PathBuf,
    entries: Arc<[BrowserEntry]>,
    selected: Option<usize>,
    path_draft: String,
    error: Option<String>,
}

impl FileBrowser {
    /// Opens `start` (a directory, or a file whose folder is opened).
    #[must_use]
    pub fn open(start: &Path) -> Self {
        let (cwd, preselected) = locate(start);
        let mut browser = Self {
            cwd,
            entries: Arc::from(Vec::new()),
            selected: None,
            path_draft: String::new(),
            error: None,
        };
        browser.reload();
        if let Some(path) = preselected {
            browser.select_path(&path);
        }
        browser
    }

    /// Current directory.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Current directory as display text.
    #[must_use]
    pub fn path_display(&self) -> String {
        display_path(&self.cwd)
    }

    /// Replaces the path field draft.
    pub fn set_path_draft(&mut self, draft: impl Into<String>) {
        self.path_draft = draft.into();
    }

    /// Visible entries.
    #[must_use]
    pub fn entries(&self) -> &[BrowserEntry] {
        &self.entries
    }

    /// Shared entry list for the virtualized list closure.
    #[must_use]
    pub fn entries_arc(&self) -> Arc<[BrowserEntry]> {
        Arc::clone(&self.entries)
    }

    /// Selected entry index, if a file is selected.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// Selected file, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&BrowserEntry> {
        self.selected
            .and_then(|index| self.entries.get(index))
            .filter(|entry| !entry.is_dir)
    }

    /// Current error text shown by the sheet.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Sets an error supplied by the load path.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    /// Number of visible program files.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.is_dir).count()
    }

    /// Selects one visible file. Directories are not selectable.
    pub fn select(&mut self, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        if entry.is_dir {
            return false;
        }
        self.selected = Some(index);
        self.error = None;
        true
    }

    /// Selects a file by path when it is visible in the current directory.
    pub fn select_path(&mut self, path: &Path) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.path == path) else {
            return false;
        };
        self.select(index)
    }

    /// Enters a visible directory.
    pub fn enter(&mut self, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }
        self.cwd = entry.path.clone();
        self.reload();
        true
    }

    /// Moves to the parent directory. Returns `false` at the filesystem root.
    pub fn up(&mut self) -> bool {
        let Some(parent) = self.cwd.parent() else {
            return false;
        };
        let parent = parent.to_path_buf();
        if parent == self.cwd {
            return false;
        }
        self.cwd = parent;
        self.reload();
        true
    }

    /// Navigates to a typed directory, or to a program file's folder and
    /// selects it.
    pub fn navigate_to(&mut self, path: &Path) -> bool {
        if path.as_os_str().is_empty() {
            return false;
        }
        let Ok(metadata) = fs::metadata(path) else {
            self.error = Some(format!("'{}' was not found", display_path(path)));
            return false;
        };
        if metadata.is_dir() {
            let directory = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            self.cwd = directory;
            self.reload();
            return true;
        }
        if metadata.is_file() && is_program_file(path) {
            let Some(parent) = path.parent() else {
                self.error = Some(format!("'{}' has no parent folder", display_path(path)));
                return false;
            };
            let directory = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            let file = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            self.cwd = directory;
            self.reload();
            if self.select_path(&file) {
                return true;
            }
            self.error = Some(format!(
                "'{}' is not visible in {}",
                display_path(path),
                self.path_display()
            ));
            return false;
        }
        self.error = Some(BrowseError::NotAProgramFile(display_path(path)).to_string());
        false
    }

    /// Navigates to the current path draft.
    pub fn commit_path(&mut self) -> bool {
        let draft = self.path_draft.trim().to_string();
        if draft.is_empty() {
            self.error = Some("Enter a folder path first".to_string());
            return false;
        }
        self.navigate_to(Path::new(&draft))
    }

    fn reload(&mut self) {
        match list_directory(&self.cwd) {
            Ok(entries) => {
                self.entries = Arc::from(entries);
                self.error = None;
            }
            Err(error) => {
                self.entries = Arc::from(Vec::new());
                self.error = Some(error.to_string());
            }
        }
        self.selected = None;
        self.path_draft = self.path_display();
    }
}

/// Recent-file model, most recent first, deduplicated and capped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentFiles {
    entries: Vec<PathBuf>,
}

impl RecentFiles {
    /// Creates an empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a list from persisted paths, applying the same dedupe and cap.
    #[must_use]
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut entries = Vec::new();
        for path in paths {
            if path.as_os_str().is_empty() || entries.contains(&path) {
                continue;
            }
            entries.push(path);
            if entries.len() >= MAX_RECENT_FILES {
                break;
            }
        }
        Self { entries }
    }

    /// Records a successful open at the front. A duplicate moves to the front
    /// instead of appearing twice.
    pub fn record(&mut self, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        self.entries.retain(|existing| existing != &path);
        self.entries.insert(0, path);
        self.entries.truncate(MAX_RECENT_FILES);
    }

    /// Removes one entry.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.entries.len();
        self.entries.retain(|existing| existing != path);
        before != self.entries.len()
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Paths, most recent first.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.entries
    }

    /// Number of remembered files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("nca-app-test-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory");
        path
    }

    #[test]
    fn extension_filter_is_ascii_case_insensitive() {
        for name in ["a.nc", "a.GCODE", "a.Tap", "a.NGC"] {
            assert!(is_program_file(Path::new(name)), "{name}");
        }
        for name in ["a.txt", "a.ng", "a.nc.bak", "nc"] {
            assert!(!is_program_file(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn directory_listing_hides_hidden_and_unsupported_entries() {
        let dir = scratch_dir("listing");
        fs::write(dir.join("pocket.nc"), "G0 X0\n").expect("write nc");
        fs::write(dir.join("notes.txt"), "not a program").expect("write txt");
        fs::write(dir.join(".secret.nc"), "G0 X0\n").expect("write hidden");
        fs::create_dir(dir.join("sub")).expect("mkdir");
        fs::create_dir(dir.join(".hidden")).expect("mkdir hidden");

        let entries = list_directory(&dir).expect("list");
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["sub", "pocket.nc"]);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].kind_label(), "NC");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_navigation_and_selection_are_local() {
        let dir = scratch_dir("navigation");
        fs::create_dir(dir.join("nested")).expect("mkdir");
        fs::write(dir.join("part.gcode"), "G0 X0\n").expect("write");

        let mut browser = FileBrowser::open(&dir);
        assert_eq!(browser.file_count(), 1);
        let nested = browser
            .entries()
            .iter()
            .position(|entry| entry.name == "nested")
            .expect("nested");
        assert!(browser.enter(nested));
        assert_eq!(
            browser.cwd(),
            fs::canonicalize(dir.join("nested")).expect("canonical")
        );
        assert!(browser.up());

        let file = browser
            .entries()
            .iter()
            .position(|entry| entry.name == "part.gcode")
            .expect("file");
        assert!(browser.select(file));
        assert!(browser.selected_entry().is_some());
        assert!(!browser.select(0) || browser.selected_entry().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_reader_rejects_bad_inputs() {
        let dir = scratch_dir("reader");
        let oversized = dir.join("large.nc");
        let file = fs::File::create(&oversized).expect("create");
        file.set_len(MAX_PROGRAM_BYTES + 1).expect("sparse length");
        assert!(matches!(
            read_program(&oversized),
            Err(ProgramFileError::TooLarge { .. })
        ));

        let text = dir.join("notes.txt");
        fs::write(&text, "G0 X0\n").expect("write");
        assert!(matches!(
            read_program(&text),
            Err(ProgramFileError::UnsupportedExtension(_))
        ));

        let binary = dir.join("binary.nc");
        fs::write(&binary, [b'G', 0, b'0']).expect("write");
        assert_eq!(read_program(&binary), Err(ProgramFileError::Binary));

        let empty = dir.join("empty.nc");
        fs::write(&empty, "  \n").expect("write");
        assert_eq!(read_program(&empty), Err(ProgramFileError::Empty));

        let good = dir.join("good.nc");
        fs::write(&good, "\u{feff}G21 G90\nG0 X1\n").expect("write");
        let loaded = read_program(&good).expect("read");
        assert_eq!(loaded.name, "good.nc");
        assert!(loaded.path.is_absolute());
        assert_eq!(loaded.source, "\u{feff}G21 G90\nG0 X1\n");

        let bom_only = dir.join("bom-only.nc");
        fs::write(&bom_only, "\u{feff}").expect("write");
        assert_eq!(read_program(&bom_only), Err(ProgramFileError::Empty));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_files_move_to_front_and_cap() {
        let mut recent = RecentFiles::new();
        for index in 0..(MAX_RECENT_FILES + 3) {
            recent.record(PathBuf::from(format!("/tmp/part-{index}.nc")));
        }
        assert_eq!(recent.len(), MAX_RECENT_FILES);
        assert_eq!(
            recent.paths()[0],
            PathBuf::from(format!("/tmp/part-{}.nc", MAX_RECENT_FILES + 2))
        );
        recent.record(PathBuf::from("/tmp/part-5.nc"));
        assert_eq!(recent.paths()[0], PathBuf::from("/tmp/part-5.nc"));
        assert_eq!(
            recent
                .paths()
                .iter()
                .filter(|path| path.as_path() == Path::new("/tmp/part-5.nc"))
                .count(),
            1
        );
        assert!(recent.remove(Path::new("/tmp/part-5.nc")));
        assert!(!recent.remove(Path::new("/tmp/part-5.nc")));
    }

    #[test]
    fn compact_path_keeps_the_tail() {
        let text = compact_path(Path::new("/home/operator/projects/pocket.nc"), 20);
        assert!(text.chars().count() <= 20);
        assert!(text.ends_with("pocket.nc"));
        assert!(text.starts_with("..."));
    }
}
