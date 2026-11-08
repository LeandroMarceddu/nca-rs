//! Program-list source model and search/filter mapping.
//!
//! The list is virtualized by `uniform_list`: this module owns only the
//! source-line array and a compact mapping from visible row to source index.
//! An empty query is the identity mapping and allocates nothing, so opening a
//! million-line program does not create a widget or a match vector per line.

use std::collections::BTreeSet;
use std::sync::Arc;

use nc_gcode::Program;

/// Stable built-in demo file name.
pub const DEMO_NAME: &str = "demo-pocket.nc";

/// Deterministic demo program used when no file is open.
///
/// It exercises rapids, feeds, plunge, retract, arcs, a helical full circle,
/// drilling, pecking, and a ramp, so the initial canvas and legend are not
/// empty. It is parsed through the same pipeline as a user file.
pub const DEMO_SOURCE: &str = "\
(demo pocket - deterministic built-in sample)
G21 G90 G17
G0 Z5
T1 M6 (6 mm flat end mill)
S12000 M3
G0 X10 Y10
G1 Z-2 F300
G1 X50 F800
G1 Y20
G2 X60 Y30 I10 J0
G1 Y40
G3 X50 Y50 I-10 J0
G1 X10
G1 Y10
G0 Z5
G0 X20 Y20
G81 Z-3 R2 F250
G80
G0 X35 Y20
G83 Z-4 R2 Q1 F200
G80
G0 X80 Y20
G1 Z-2 F300
G2 X80 Y20 I0 J5 Z-3 F500
G0 Z5
G0 X20 Y35
G1 Z-1 F300
G1 X40 Z-2 F600
G1 X40 Y45
G0 Z5
M5
M30
";

/// One source line with its display number and optional `N` word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLine {
    /// 1-based source line number.
    pub number: usize,
    /// The `N` word value when the line has one.
    pub n_word: Option<u32>,
    /// Source text without the terminator or a UTF-8 BOM.
    pub text: String,
}

/// Splits program text on LF, CRLF, or a lone CR.
///
/// The result matches the parser's [`SourceLines`] iteration: a trailing
/// terminator does not create an extra empty line, and an empty source has no
/// lines.
#[must_use]
pub fn split_source_lines(source: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = source;
    if rest.is_empty() {
        return lines;
    }
    while !rest.is_empty() {
        let bytes = rest.as_bytes();
        match bytes
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        {
            Some(index) => {
                lines.push(&rest[..index]);
                let mut next = index + 1;
                let paired = matches!(
                    (bytes[index], bytes.get(index + 1)),
                    (b'\r', Some(b'\n')) | (b'\n', Some(b'\r'))
                );
                if paired {
                    next += 1;
                }
                rest = &rest[next..];
            }
            None => {
                lines.push(rest);
                rest = "";
            }
        }
    }
    lines
}

/// Reads a leading `N` word from plain source text.
///
/// The parser supplies the authoritative value. This fallback keeps the
/// program list useful when the file did not parse and `Program` is absent.
fn n_word_from_text(text: &str) -> Option<u32> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 2 || !bytes[0].eq_ignore_ascii_case(&b'n') {
        return None;
    }
    let digits = bytes[1..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    trimmed[1..1 + digits].parse::<u32>().ok()
}

/// Builds the source-line display model, taking `N` words from a parsed
/// program when available.
#[must_use]
pub fn build_source_lines(source: &str, program: Option<&Program>) -> Arc<[SourceLine]> {
    let lines = split_source_lines(source);
    let mut out = Vec::with_capacity(lines.len());
    for (index, text) in lines.into_iter().enumerate() {
        let number = index.saturating_add(1);
        // The file-level BOM belongs to the raw bytes the report hashes, not
        // to the first display line.
        let display = if index == 0 {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        let n_word = program
            .and_then(|program| program.lines.get(index))
            .and_then(|line| line.number)
            .or_else(|| n_word_from_text(display));
        out.push(SourceLine {
            number,
            n_word,
            text: display.to_string(),
        });
    }
    Arc::from(out)
}

/// The parsed shape of a search field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchQuery {
    /// No query; the list shows every source line.
    Empty,
    /// A bare source line number.
    Line(usize),
    /// An `N` word, for example `N120`.
    NWord(u32),
    /// Case-insensitive source text.
    Text(String),
}

impl SearchQuery {
    /// Parses `N<number>`, a bare line number, or source text.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::Empty;
        }
        if trimmed.len() > 1
            && trimmed.as_bytes()[0].eq_ignore_ascii_case(&b'n')
            && trimmed[1..].bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(value) = trimmed[1..].parse::<u32>()
        {
            return Self::NWord(value);
        }
        if trimmed.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(value) = trimmed.parse::<usize>()
        {
            return Self::Line(value);
        }
        Self::Text(trimmed.to_ascii_lowercase())
    }

    /// Whether this query filters the list.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self, Self::Empty)
    }

    /// The original display text, reconstructed for a chip.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Line(line) => line.to_string(),
            Self::NWord(word) => format!("N{word}"),
            Self::Text(text) => text.clone(),
        }
    }

    fn matches(&self, line: &SourceLine) -> bool {
        match self {
            Self::Empty => true,
            Self::Line(number) => {
                line.number == *number || line.n_word == u32::try_from(*number).ok()
            }
            Self::NWord(word) => line.n_word == Some(*word) || line.number == *word as usize,
            Self::Text(text) => line.text.to_ascii_lowercase().contains(text),
        }
    }
}

/// Visible-row to source-line mapping for the program list.
#[derive(Clone, Debug)]
pub struct ProgramIndex {
    query: SearchQuery,
    line_count: usize,
    matches: Option<Arc<[usize]>>,
    error_lines: BTreeSet<usize>,
}

impl ProgramIndex {
    /// Builds the identity index without scanning or allocating per line.
    #[must_use]
    pub fn identity(line_count: usize) -> Self {
        Self {
            query: SearchQuery::Empty,
            line_count,
            matches: None,
            error_lines: BTreeSet::new(),
        }
    }

    /// Builds an index for a query over the source-line model.
    #[must_use]
    pub fn new(lines: &[SourceLine], raw: &str) -> Self {
        let query = SearchQuery::parse(raw);
        let matches = if query.is_active() {
            Some(Arc::from(
                lines
                    .iter()
                    .enumerate()
                    .filter_map(|(index, line)| query.matches(line).then_some(index))
                    .collect::<Vec<_>>(),
            ))
        } else {
            None
        };
        Self {
            query,
            line_count: lines.len(),
            matches,
            error_lines: BTreeSet::new(),
        }
    }

    /// Adds parse-error line numbers for the severity gutter.
    #[must_use]
    pub fn with_error_lines(mut self, lines: impl IntoIterator<Item = usize>) -> Self {
        self.error_lines = lines.into_iter().collect();
        self
    }

    /// Parsed query.
    #[must_use]
    pub fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// Number of source lines.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// Number of visible rows.
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.matches
            .as_ref()
            .map_or(self.line_count, |matches| matches.len())
    }

    /// Number of stored matches; zero for the identity mapping.
    #[must_use]
    pub fn stored_match_count(&self) -> usize {
        self.matches.as_ref().map_or(0, |matches| matches.len())
    }

    /// Whether the query filters the list.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.query.is_active()
    }

    /// Maps a visible row to a zero-based source index.
    #[must_use]
    pub fn source_index(&self, visible: usize) -> Option<usize> {
        match self.matches.as_ref() {
            Some(matches) => matches.get(visible).copied(),
            None => (visible < self.line_count).then_some(visible),
        }
    }

    /// Maps a source index back to a visible row.
    #[must_use]
    pub fn visible_index(&self, source: usize) -> Option<usize> {
        match self.matches.as_ref() {
            Some(matches) => matches.binary_search(&source).ok(),
            None => (source < self.line_count).then_some(source),
        }
    }

    /// The first matching source index, used by ENTER in the search field.
    #[must_use]
    pub fn first_match(&self) -> Option<usize> {
        self.source_index(0)
    }

    /// True when this 1-based source line carries a parse error.
    #[must_use]
    pub fn has_error(&self, line: usize) -> bool {
        self.error_lines.contains(&line)
    }

    /// Error line numbers, for status text.
    #[must_use]
    pub fn error_lines(&self) -> &BTreeSet<usize> {
        &self.error_lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Arc<[SourceLine]> {
        build_source_lines(source, None)
    }

    #[test]
    fn source_split_matches_parser_terminators() {
        assert!(split_source_lines("").is_empty());
        assert_eq!(split_source_lines("one"), vec!["one"]);
        assert_eq!(split_source_lines("one\r\ntwo\n"), vec!["one", "two"]);
        assert_eq!(split_source_lines("one\rtwo\r"), vec!["one", "two"]);
        assert_eq!(split_source_lines("one\n\ntwo"), vec!["one", "", "two"]);
    }

    #[test]
    fn search_query_parses_three_forms() {
        assert_eq!(SearchQuery::parse(""), SearchQuery::Empty);
        assert_eq!(SearchQuery::parse("  "), SearchQuery::Empty);
        assert_eq!(SearchQuery::parse("N120"), SearchQuery::NWord(120));
        assert_eq!(SearchQuery::parse("n7"), SearchQuery::NWord(7));
        assert_eq!(SearchQuery::parse("120"), SearchQuery::Line(120));
        assert_eq!(SearchQuery::parse("G1 X"), SearchQuery::Text("g1 x".into()));
        assert_eq!(SearchQuery::parse("Nx").display(), "nx");
    }

    #[test]
    fn empty_query_is_a_zero_allocation_identity() {
        let index = ProgramIndex::identity(1_000_000);
        assert_eq!(index.visible_count(), 1_000_000);
        assert_eq!(index.stored_match_count(), 0);
        assert_eq!(index.source_index(999_999), Some(999_999));
        assert_eq!(index.visible_index(123_456), Some(123_456));
        assert_eq!(index.source_index(1_000_000), None);
    }

    #[test]
    fn line_and_n_word_queries_filter_and_map() {
        let source = lines("G21\nN120 G0 X0\nG1 X10\nN220 G1 Y10\n");
        let by_line = ProgramIndex::new(&source, "2");
        assert_eq!(by_line.visible_count(), 1);
        assert_eq!(by_line.source_index(0), Some(1));

        let by_n = ProgramIndex::new(&source, "N220");
        assert_eq!(by_n.visible_count(), 1);
        assert_eq!(by_n.source_index(0), Some(3));
        assert_eq!(by_n.visible_index(3), Some(0));

        let text = ProgramIndex::new(&source, "g1");
        assert_eq!(text.visible_count(), 2);
        assert_eq!(text.source_index(0), Some(2));
        assert_eq!(text.source_index(1), Some(3));
    }

    #[test]
    fn no_match_is_reported_without_panicking() {
        let source = lines("G21\nG90\n");
        let index = ProgramIndex::new(&source, "nope");
        assert_eq!(index.visible_count(), 0);
        assert_eq!(index.source_index(0), None);
        assert_eq!(index.first_match(), None);
    }

    #[test]
    fn error_lines_are_available_to_the_gutter() {
        let source = lines("G21\nG1 X\n");
        let index = ProgramIndex::new(&source, "").with_error_lines([2]);
        assert!(index.has_error(2));
        assert!(!index.has_error(1));
        assert_eq!(index.error_lines().len(), 1);
    }
}
