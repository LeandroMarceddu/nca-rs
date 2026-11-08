//! Deterministic vendor dialect detection from program text.
//!
//! The detector reports a dialect only when the source itself carries
//! evidence for it. It never guesses from file names, paths, or machine
//! settings, and a [`None`] result means the caller should keep its own
//! default (normally [`Dialect::Generic`]) and mark the dialect as assumed.
//!
//! # Evidence order
//!
//! 1. Strong code markers anywhere outside a comment: `M851` and `M852` are
//!    Makera auxiliary-output codes. The first one found is the evidence and
//!    wins over every text marker.
//! 2. Header comments before the first motion: comments on lines before the
//!    first motion word, plus comments that appear before the motion word on
//!    the same line. Text matching is ASCII case-insensitive.
//!
//! If the header mentions two different vendor dialects, the result is
//! [`None`]: the program is ambiguous rather than one vendor being chosen.
//! Repeating the same dialect is fine; its first occurrence is the evidence.
//!
//! The scan is linear in the source byte length and stops as soon as a strong
//! code marker is found. Evidence text is bounded to 120 bytes and no
//! per-line collection is allocated.
//!
//! [`Dialect::Generic`]: nc_core::Dialect::Generic

use nc_core::Dialect;

use crate::parser::SourceLines;

/// Maximum number of source bytes retained in [`DialectEvidence::text`].
pub const EVIDENCE_TEXT_MAX_BYTES: usize = 120;

/// One source line that supports a detected dialect.
///
/// The text is the trimmed program line, truncated on a UTF-8 boundary to at
/// most [`EVIDENCE_TEXT_MAX_BYTES`] bytes. It never contains a file path added
/// by the detector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectEvidence {
    /// 1-based source line number.
    pub line: usize,
    /// Stable short marker such as `"M851"`, `"makera-header"`, or
    /// `"grbl-header"`.
    pub marker: &'static str,
    /// Bounded source excerpt suitable for display.
    pub text: String,
}

/// A detected dialect together with the source evidence that supports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectDetection {
    /// Detected dialect.
    pub dialect: Dialect,
    /// Selected source evidence for `dialect`; a strong code marker wins over
    /// header text.
    pub evidence: DialectEvidence,
}

/// Detects a vendor dialect from program text.
///
/// Strong code markers are searched across the whole program outside comments.
/// Text markers are read only from comments before the first motion. The
/// caller still decides whether to apply the result; an explicit program or
/// configuration dialect always takes precedence.
///
/// Returns [`None`] when the source has no recognizable vendor evidence or
/// when header comments name two different dialects.
///
/// # Example
///
/// ```
/// use nc_gcode::Dialect;
/// use nc_gcode::dialect_detect::detect_dialect;
///
/// let source = "(  vendor: Makera)\nG0 X0\n";
/// let detected = detect_dialect(source).expect("vendor header");
/// assert_eq!(detected.dialect, Dialect::Makera);
/// assert_eq!(detected.evidence.line, 1);
/// assert_eq!(detected.evidence.marker, "makera-header");
/// assert_eq!(detected.evidence.text, "(  vendor: Makera)");
/// ```
#[must_use]
pub fn detect_dialect(source: &str) -> Option<DialectDetection> {
    let mut header = HeaderText::default();
    let mut header_open = true;

    for (index, line) in SourceLines::new(source).enumerate() {
        let scan = scan_line(line, header_open);

        if let Some(marker) = scan.code {
            return Some(DialectDetection {
                dialect: Dialect::Makera,
                evidence: DialectEvidence {
                    line: index + 1,
                    marker,
                    text: evidence_text(line),
                },
            });
        }

        if header_open {
            header.observe(&scan, index + 1, line);
            if scan.motion {
                header_open = false;
            }
        }
    }

    header.finish()
}

/// A stable text marker in a header comment.
#[derive(Clone, Copy)]
struct TextMarker {
    dialect: Dialect,
    marker: &'static str,
    needle: &'static str,
}

/// Header phrases accepted for each dialect, in a fixed order for ties.
const TEXT_MARKERS: [TextMarker; 11] = [
    TextMarker {
        dialect: Dialect::Makera,
        marker: "makera-header",
        needle: "makera",
    },
    TextMarker {
        dialect: Dialect::Makera,
        marker: "makera-header",
        needle: "carvera",
    },
    TextMarker {
        dialect: Dialect::Makera,
        marker: "makera-header",
        needle: "model: z1",
    },
    TextMarker {
        dialect: Dialect::Makera,
        marker: "makera-header",
        needle: "community post process utility",
    },
    TextMarker {
        dialect: Dialect::Grbl,
        marker: "grbl-header",
        needle: "grblhal",
    },
    TextMarker {
        dialect: Dialect::Grbl,
        marker: "grbl-header",
        needle: "grbl",
    },
    TextMarker {
        dialect: Dialect::LinuxCnc,
        marker: "linuxcnc-header",
        needle: "linuxcnc",
    },
    TextMarker {
        dialect: Dialect::LinuxCnc,
        marker: "linuxcnc-header",
        needle: "linux cnc",
    },
    TextMarker {
        dialect: Dialect::Mach3,
        marker: "mach3-header",
        needle: "mach3",
    },
    TextMarker {
        dialect: Dialect::Mach3,
        marker: "mach3-header",
        needle: "mach4",
    },
    TextMarker {
        dialect: Dialect::Fanuc,
        marker: "fanuc-header",
        needle: "fanuc",
    },
];

/// Result of scanning one source line outside comments.
#[derive(Default)]
struct LineScan {
    /// First Makera code word on the line, if any.
    code: Option<&'static str>,
    /// `true` when the line contains a motion word.
    motion: bool,
    /// Byte offset of the first motion word, when [`Self::motion`] is set.
    motion_offset: Option<usize>,
    /// Bit mask of text-marker dialects seen before the first motion.
    text_mask: u8,
    /// First text marker on the line and its byte offset.
    first_text: Option<(Dialect, &'static str, usize)>,
}

/// Accumulated header evidence and ambiguity state.
#[derive(Default)]
struct HeaderText {
    mask: u8,
    evidence: Option<(Dialect, DialectEvidence)>,
}

impl HeaderText {
    /// Records the text markers found on one header line.
    fn observe(&mut self, scan: &LineScan, line: usize, source: &str) {
        if scan.text_mask == 0 {
            return;
        }
        self.mask |= scan.text_mask;
        if self.evidence.is_some() {
            return;
        }
        if let Some((dialect, marker, _offset)) = scan.first_text {
            self.evidence = Some((
                dialect,
                DialectEvidence {
                    line,
                    marker,
                    text: evidence_text(source),
                },
            ));
        }
    }

    /// Returns the result, or `None` when the header names two dialects.
    fn finish(self) -> Option<DialectDetection> {
        if self.mask.count_ones() > 1 {
            return None;
        }
        let (dialect, evidence) = self.evidence?;
        Some(DialectDetection { dialect, evidence })
    }
}

/// Scans one line for code markers, motion words, and header text markers.
///
/// Code words are scanned first so a setup block such as `G92 X0` is not
/// mistaken for motion. Comment text is only searched when `collect_text` is
/// set, and only before the first motion word on the line.
fn scan_line(line: &str, collect_text: bool) -> LineScan {
    let mut scan = scan_code(line);
    if collect_text && scan.code.is_none() {
        let limit = scan.motion_offset.unwrap_or(line.len());
        scan_comments(line, limit, &mut scan);
    }
    scan
}

/// Scans code words outside comments for markers and the first motion.
fn scan_code(line: &str) -> LineScan {
    let bytes = line.as_bytes();
    let mut scan = LineScan::default();
    let mut saw_axis_setup_g = false;
    let mut first_motion_g = None;
    let mut first_axis = None;
    let mut index = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\r' => index += 1,
            b'(' => {
                let start = index + 1;
                let end = match line.get(start..).and_then(|rest| rest.find(')')) {
                    Some(offset) => start + offset,
                    None => bytes.len(),
                };
                index = if end < bytes.len() {
                    end + 1
                } else {
                    bytes.len()
                };
            }
            b';' | b'*' => break,
            b'#' => index = skip_parameter(bytes, index + 1),
            b'[' => index = skip_bracket(bytes, index + 1),
            b'/' | b'%' => index += 1,
            _ if byte.is_ascii_alphabetic() => {
                let letter = byte.to_ascii_uppercase();
                let word_start = index;
                match scan_number(bytes, index + 1) {
                    Some((value, end)) => {
                        if letter == b'M'
                            && let Some(marker) = makera_code_marker(value)
                        {
                            scan.code = Some(marker);
                            return scan;
                        }
                        if letter == b'G' {
                            if is_motion_code(value) {
                                first_motion_g.get_or_insert(word_start);
                            } else if is_axis_setup_code(value) {
                                saw_axis_setup_g = true;
                            }
                        } else if matches!(letter, b'X' | b'Y' | b'Z') {
                            first_axis.get_or_insert(word_start);
                        }
                        index = end;
                    }
                    None => index += 1,
                }
            }
            _ => index += 1,
        }
    }

    // A bare axis word is motion under the active modal mode. An axis word on
    // a block that also carries a setup command such as G92 belongs to that
    // command and must not close the header.
    let motion = first_motion_g.is_some() || (first_axis.is_some() && !saw_axis_setup_g);
    if motion {
        scan.motion = true;
        scan.motion_offset = [first_motion_g, first_axis].into_iter().flatten().min();
    }
    scan
}

/// Searches comments that start before `limit`.
fn scan_comments(line: &str, limit: usize, scan: &mut LineScan) {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && index < limit {
        match bytes[index] {
            b'(' => {
                let start = index + 1;
                let end = match line.get(start..).and_then(|rest| rest.find(')')) {
                    Some(offset) => start + offset,
                    None => bytes.len(),
                };
                note_comment(line.get(start..end).unwrap_or_default(), start, scan);
                index = if end < bytes.len() {
                    end + 1
                } else {
                    bytes.len()
                };
            }
            b';' => {
                note_comment(line.get(index + 1..).unwrap_or_default(), index + 1, scan);
                break;
            }
            b'*' => break,
            b'#' => index = skip_parameter(bytes, index + 1),
            b'[' => index = skip_bracket(bytes, index + 1),
            _ => index += 1,
        }
    }
}

/// Searches one comment body for header markers and records the earliest hit.
fn note_comment(comment: &str, offset: usize, scan: &mut LineScan) {
    if comment.is_empty() {
        return;
    }
    for marker in &TEXT_MARKERS {
        if let Some(position) = find_ignore_ascii_case(comment, marker.needle) {
            scan.text_mask |= dialect_bit(marker.dialect);
            let absolute = offset + position;
            if scan.first_text.is_none_or(|(_, _, best)| absolute < best) {
                scan.first_text = Some((marker.dialect, marker.marker, absolute));
            }
        }
    }
}

/// Returns the stable marker for a Makera auxiliary-output code.
fn makera_code_marker(value: f64) -> Option<&'static str> {
    if (value - 851.0).abs() <= 1.0e-9 {
        Some("M851")
    } else if (value - 852.0).abs() <= 1.0e-9 {
        Some("M852")
    } else {
        None
    }
}

/// Returns `true` for a G-code that moves the machine.
///
/// Canned cycles, probing moves, and homing moves count as motion; setup codes
/// such as `G4`, `G10`, `G80`, and `G92` do not.
fn is_motion_code(value: f64) -> bool {
    const PROBE_CODES: [f64; 4] = [38.2, 38.3, 38.4, 38.5];
    if PROBE_CODES
        .iter()
        .any(|probe| (value - probe).abs() <= 1.0e-9)
    {
        return true;
    }
    let rounded = value.round();
    if (value - rounded).abs() > 1.0e-9 {
        return false;
    }
    rounded == 0.0
        || rounded == 1.0
        || rounded == 2.0
        || rounded == 3.0
        || rounded == 5.0
        || rounded == 28.0
        || rounded == 30.0
        || rounded == 73.0
        || rounded == 74.0
        || rounded == 76.0
        || (81.0..=89.0).contains(&rounded)
}

/// Returns `true` for a setup G-code that consumes axis words without moving.
fn is_axis_setup_code(value: f64) -> bool {
    [10.0, 28.1, 30.1, 43.1, 50.0, 51.0, 92.0, 92.1, 92.2, 92.3]
        .iter()
        .any(|code| (value - code).abs() <= 1.0e-9)
}

/// Maps a dialect to one bit without hashing.
const fn dialect_bit(dialect: Dialect) -> u8 {
    match dialect {
        Dialect::Generic => 0,
        Dialect::Grbl => 1 << 0,
        Dialect::LinuxCnc => 1 << 1,
        Dialect::Mach3 => 1 << 2,
        Dialect::Fanuc => 1 << 3,
        Dialect::Makera => 1 << 4,
    }
}

/// Case-insensitive ASCII substring search.
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Scans a decimal word value starting just after its letter.
///
/// Returns the value and the byte index just after the token. A value without
/// at least one digit is not a word.
fn scan_number(bytes: &[u8], start: usize) -> Option<(f64, usize)> {
    let mut index = start;
    if matches!(bytes.get(index), Some(b'+' | b'-')) {
        index += 1;
    }
    let mut has_digit = false;
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
        has_digit = true;
        index += 1;
    }
    if matches!(bytes.get(index), Some(b'.')) {
        index += 1;
        while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
            has_digit = true;
            index += 1;
        }
    }
    if !has_digit {
        return None;
    }
    let token = bytes.get(start..index)?;
    let token = std::str::from_utf8(token).ok()?;
    let value = token.parse::<f64>().ok()?;
    value.is_finite().then_some((value, index))
}

/// Skips a numbered or named parameter key after its `#` marker.
fn skip_parameter(bytes: &[u8], mut index: usize) -> usize {
    if bytes.get(index) == Some(&b'<') {
        index += 1;
        while index < bytes.len() && bytes[index] != b'>' {
            index += 1;
        }
        if index < bytes.len() {
            index += 1;
        }
        return index;
    }
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
        index += 1;
    }
    index
}

/// Skips a bracketed expression, including nested brackets.
fn skip_bracket(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1u32;
    while index < bytes.len() {
        match bytes[index] {
            b'[' => depth = depth.saturating_add(1),
            b']' => {
                depth = depth.saturating_sub(1);
                index += 1;
                if depth == 0 {
                    return index;
                }
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    index
}

/// Returns the trimmed source line bounded to [`EVIDENCE_TEXT_MAX_BYTES`].
fn evidence_text(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.len() <= EVIDENCE_TEXT_MAX_BYTES {
        return trimmed.to_string();
    }
    let mut end = EVIDENCE_TEXT_MAX_BYTES;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed.get(..end).unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_search_is_ascii_only() {
        assert_eq!(find_ignore_ascii_case("GrblHAL 1.1", "grbl"), Some(0));
        assert_eq!(find_ignore_ascii_case("x Mach3", "MACH3"), Some(2));
        assert_eq!(find_ignore_ascii_case("", "grbl"), None);
        assert_eq!(find_ignore_ascii_case("grbl", ""), None);
        assert_eq!(find_ignore_ascii_case("gr", "grbl"), None);
    }

    #[test]
    fn evidence_text_is_capped_on_a_char_boundary() {
        let line = format!("{}x", "\u{4e2d}".repeat(100));
        let text = evidence_text(&line);
        assert!(text.len() <= EVIDENCE_TEXT_MAX_BYTES);
        assert!(line.starts_with(&text));
        assert_eq!(evidence_text("  G0 X0  "), "G0 X0");
    }

    #[test]
    fn parameter_names_are_not_words() {
        let scan = scan_line("#<m851>=1", true);
        assert!(scan.code.is_none());
        let scan = scan_line("X[#<m852>+1]", true);
        assert!(scan.code.is_none());
    }

    #[test]
    fn commented_markers_are_not_code() {
        let scan = scan_line("(M851 S100) M400", true);
        assert!(scan.code.is_none());
        let scan = scan_line("G0 X0 ; M852", true);
        assert!(scan.code.is_none());
    }

    #[test]
    fn motion_words_are_recognized() {
        for line in [
            "G0 X0",
            "g1x1",
            "G2 X1 Y1 I1 J0",
            "G38.2 Z-1",
            "X10",
            "G21 X10",
            "G53 X10",
        ] {
            assert!(scan_line(line, true).motion, "{line}");
        }
        for line in [
            "G21 G90",
            "G4 P1",
            "G80",
            "G92 X0",
            "G10 L2 P1 X0",
            "G43.1 Z2",
            "G50 X0",
            "M3 S1000",
        ] {
            assert!(!scan_line(line, true).motion, "{line}");
        }
    }
}
