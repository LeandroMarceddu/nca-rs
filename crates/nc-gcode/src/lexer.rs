//! Line lexer for the grblHAL G-code dialect.
//!
//! The lexer turns one source line into ordered [`Word`]s, comment text, a
//! block-delete flag, and an optional `*` checksum. Modal interpretation lives
//! in the planner. Parameter references and `[...]` expressions are evaluated
//! while lexing against a [`Parameters`] table.
//!
//! Accepted syntax:
//!
//! - `G`, `M`, axis, and parameter words with optional sign and decimal point.
//! - `N` line numbers, `T`/`S`/`F` words.
//! - Numbered and named `#` parameters, including assignments. Simple values
//!   (`#1=5`, `#1=#2`) are accepted; compound arithmetic must be bracketed
//!   (`#1=[2+3*4]`).
//! - `[...]` arithmetic expressions in numeric word values.
//! - `( ... )` and `;` comments.
//! - A leading `/` block-delete marker.
//! - A trailing `*nn` XOR checksum over the bytes before `*`.
//! - A lone `%` program marker.
//!
//! Flow control (`IF`, `WHILE`, `GOTO`, `DO`, `SUB`, ...) is still rejected
//! with [`ParseErrorKind::UnsupportedExpression`]; it is a documented
//! follow-up (VIEW-02), not silently accepted.
//!
//! [`lex_line`] uses a fresh, empty parameter table for each call.
//! [`lex_line_with_parameters`] persists assignments in the caller's table, so
//! a whole program can share one parameter environment.

use crate::error::{ParseError, ParseErrorKind};
use crate::expression::{ParameterOverlay, Parameters, evaluate_value, parse_parameter_key};

/// Safety cap on words per line when the caller does not set one.
pub const DEFAULT_MAX_WORDS: usize = 256;

/// One letter word, for example `G1` or `X-12.5`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Word {
    /// Upper-case word letter, `A` through `Z`.
    pub letter: char,
    /// Numeric value after evaluating any parameter reference or expression.
    pub value: f64,
    /// 1-based byte column of the letter in the source line.
    pub column: usize,
}

impl Word {
    /// Creates a word.
    #[must_use]
    pub const fn new(letter: char, value: f64, column: usize) -> Self {
        Self {
            letter,
            value,
            column,
        }
    }
}

/// The result of lexing one line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LexedLine {
    /// Words in source order.
    pub words: Vec<Word>,
    /// Comment text without the surrounding delimiters.
    pub comments: Vec<String>,
    /// `true` when the line starts with the `/` block-delete marker.
    pub block_delete: bool,
    /// Parsed `*` checksum, when present and valid.
    pub checksum: Option<u8>,
    /// `true` for a lone `%` program marker line.
    pub percent: bool,
}

impl LexedLine {
    /// Returns `true` when the line has no executable words.
    ///
    /// Parameter-assignment and comment-only lines have no words.
    #[must_use]
    pub fn has_words(&self) -> bool {
        !self.words.is_empty()
    }
}

/// Flow-control keywords that are recognised only to produce a clear deferral
/// message. They are not G-code words.
const CONTROL_KEYWORDS: [&str; 11] = [
    "IF", "WHILE", "GOTO", "DO", "ENDIF", "ENDWHILE", "ELSE", "SUB", "ENDSUB", "CALL", "RETURN",
];

/// Lexes one source line with a fresh, empty parameter table.
///
/// Assignments on this line are visible to later words on the same line but do
/// not persist to another call. Use [`lex_line_with_parameters`] when a
/// program needs a shared parameter environment.
///
/// `line` is used only to label errors. `max_words` is a safety bound; values
/// below one are treated as [`DEFAULT_MAX_WORDS`].
///
/// # Errors
///
/// Returns every lexing error found on the line. The line is not partially
/// returned.
pub fn lex_line(line: usize, source: &str, max_words: usize) -> Result<LexedLine, Vec<ParseError>> {
    let parameters = Parameters::new();
    let mut overlay = ParameterOverlay::default();
    lex_line_with_overlay(line, source, max_words, &parameters, &mut overlay)
}

/// Lexes one source line against a shared parameter table.
///
/// Assignments on the line update `parameters` in source order only when the
/// whole line lexes successfully. A failed line leaves the table untouched.
/// [`Parser`](crate::Parser) stages the same overlay through planning, so a
/// block that fails after lexing also leaves the table untouched.
///
/// # Errors
///
/// Returns every lexing and expression error found on the line. The line is
/// not partially returned.
pub fn lex_line_with_parameters(
    line: usize,
    source: &str,
    max_words: usize,
    parameters: &mut Parameters,
) -> Result<LexedLine, Vec<ParseError>> {
    let mut overlay = ParameterOverlay::default();
    let lexed = lex_line_with_overlay(line, source, max_words, parameters, &mut overlay)?;
    overlay.commit(parameters);
    Ok(lexed)
}

/// Lexes one line, staging assignments in `overlay` instead of `base`.
///
/// The caller commits the overlay after the block validates.
pub(crate) fn lex_line_with_overlay(
    line: usize,
    source: &str,
    max_words: usize,
    base: &Parameters,
    overlay: &mut ParameterOverlay,
) -> Result<LexedLine, Vec<ParseError>> {
    let max_words = max_words.max(1);
    let mut errors = Vec::new();

    if let Some((column, keyword)) = leading_control_keyword(source) {
        errors.push(ParseError::new(
            line,
            column,
            ParseErrorKind::UnsupportedExpression,
            format!(
                "{keyword} flow control is not supported yet; parameters and expressions are implemented without it (VIEW-02 follow-up)"
            ),
        ));
        return Err(errors);
    }

    let mut lexed = LexedLine::default();
    let bytes = source.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\r' => index += 1,
            b'/' if lexed.words.is_empty() && !lexed.percent => {
                lexed.block_delete = true;
                index += 1;
            }
            b'(' => match source[index + 1..].find(')') {
                Some(offset) => {
                    let end = index + 1 + offset;
                    lexed.comments.push(source[index + 1..end].to_string());
                    index = end + 1;
                }
                None => {
                    errors.push(ParseError::new(
                        line,
                        index + 1,
                        ParseErrorKind::UnterminatedComment,
                        "unterminated '(' comment",
                    ));
                    break;
                }
            },
            b';' => {
                lexed.comments.push(source[index + 1..].trim().to_string());
                index = bytes.len();
            }
            b'*' => match parse_checksum(&source[index + 1..]) {
                Ok((checksum, consumed)) => {
                    let computed = checksum_xor(&bytes[..index]);
                    if computed != checksum {
                        errors.push(ParseError::new(
                            line,
                            index + 1,
                            ParseErrorKind::ChecksumMismatch,
                            format!(
                                "checksum mismatch: line has {computed}, program says {checksum}"
                            ),
                        ));
                    } else {
                        lexed.checksum = Some(checksum);
                    }
                    index += 1 + consumed;
                }
                Err(message) => {
                    errors.push(ParseError::new(
                        line,
                        index + 1,
                        ParseErrorKind::InvalidChecksum,
                        message,
                    ));
                    index = bytes.len();
                }
            },
            b'%' => {
                if lexed.words.is_empty() && !lexed.percent && is_lone_percent(&source[index..]) {
                    lexed.percent = true;
                    index = bytes.len();
                } else {
                    errors.push(ParseError::new(
                        line,
                        index + 1,
                        ParseErrorKind::UnexpectedCharacter,
                        "'%' must be alone on its line",
                    ));
                    index += 1;
                }
            }
            b'#' => match parse_assignment(line, source, index, base, overlay) {
                Ok(next) => index = next,
                Err(error) => {
                    errors.push(error);
                    break;
                }
            },
            b'[' => {
                errors.push(ParseError::new(
                    line,
                    index + 1,
                    ParseErrorKind::ExpressionNotAllowed,
                    "an expression must be the value of a word, for example X[1+2]",
                ));
                break;
            }
            b']' => {
                errors.push(ParseError::new(
                    line,
                    index + 1,
                    ParseErrorKind::MalformedExpression,
                    "unexpected ']' without a matching '['",
                ));
                break;
            }
            _ if byte.is_ascii_alphabetic() => {
                let letter = char::from(byte).to_ascii_uppercase();
                let column = index + 1;
                index += 1;
                if index >= bytes.len() {
                    errors.push(ParseError::new(
                        line,
                        column,
                        ParseErrorKind::MissingValue,
                        format!("'{letter}' is not followed by a number"),
                    ));
                    break;
                }
                if let Some(expression_start) = expression_value_start(bytes, index) {
                    if matches!(letter, 'G' | 'M' | 'O' | 'N') {
                        errors.push(ParseError::new(
                            line,
                            column,
                            ParseErrorKind::ExpressionNotAllowed,
                            format!(
                                "expressions are not allowed in {letter} words; use a plain number"
                            ),
                        ));
                        break;
                    }
                    match lex_value_expression(line, source, expression_start, base, overlay) {
                        Ok((value, consumed)) => {
                            index = expression_start + consumed;
                            lexed.words.push(Word::new(letter, value, column));
                        }
                        Err(error) => {
                            errors.push(error);
                            break;
                        }
                    }
                } else {
                    let next = bytes[index];
                    if !(next.is_ascii_digit() || next == b'+' || next == b'-' || next == b'.') {
                        errors.push(ParseError::new(
                            line,
                            column,
                            ParseErrorKind::MissingValue,
                            format!("'{letter}' is not followed by a number"),
                        ));
                        break;
                    }
                    let start = index;
                    while index < bytes.len() {
                        let candidate = bytes[index];
                        if candidate.is_ascii_digit()
                            || candidate == b'.'
                            || candidate == b'+'
                            || candidate == b'-'
                        {
                            index += 1;
                        } else {
                            break;
                        }
                    }
                    let token = &source[start..index];
                    match parse_number(token) {
                        Some(value) => lexed.words.push(Word::new(letter, value, column)),
                        None => {
                            errors.push(ParseError::new(
                                line,
                                column,
                                ParseErrorKind::InvalidNumber,
                                format!("'{token}' is not a valid decimal number"),
                            ));
                            break;
                        }
                    }
                }
                if lexed.words.len() > max_words {
                    errors.push(ParseError::new(
                        line,
                        column,
                        ParseErrorKind::TooManyWords,
                        format!("more than {max_words} words on one line"),
                    ));
                    break;
                }
            }
            _ => {
                let ch = source[index..].chars().next().unwrap_or('\u{fffd}');
                errors.push(ParseError::new(
                    line,
                    index + 1,
                    ParseErrorKind::UnexpectedCharacter,
                    format!("unexpected character {ch:?}"),
                ));
                index += ch.len_utf8();
            }
        }
    }

    if errors.is_empty() {
        Ok(lexed)
    } else {
        Err(errors)
    }
}

/// Returns the start of an expression value after a word letter.
///
/// A value expression starts with `#`, `[`, or one or more signs followed by
/// one of those. A plain signed decimal is handled by the numeric path.
fn expression_value_start(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index).copied()? {
        b'#' | b'[' => Some(index),
        b'+' | b'-' => {
            let mut probe = index;
            while matches!(bytes.get(probe), Some(b'+' | b'-')) {
                probe += 1;
            }
            matches!(bytes.get(probe), Some(b'#' | b'[')).then_some(index)
        }
        _ => None,
    }
}

/// Evaluates one `#` parameter or `[...]` word value.
fn lex_value_expression(
    line: usize,
    source: &str,
    start: usize,
    base: &Parameters,
    overlay: &ParameterOverlay,
) -> Result<(f64, usize), ParseError> {
    let slice = source.get(start..).unwrap_or_default();
    evaluate_value(slice, base, overlay).map_err(|error| {
        ParseError::new(
            line,
            start + error.offset + 1,
            error.parse_kind(),
            error.message,
        )
    })
}

/// Parses one `#` parameter assignment and stages it in `overlay`.
///
/// Returns the byte index just after the assigned value. Simple values are a
/// signed number or a parameter reference; compound arithmetic must be
/// bracketed. A trailing `*` is left for the checksum parser.
fn parse_assignment(
    line: usize,
    source: &str,
    start: usize,
    base: &Parameters,
    overlay: &mut ParameterOverlay,
) -> Result<usize, ParseError> {
    let bytes = source.as_bytes();
    let (key, mut index) = parse_parameter_key(source, start).map_err(|error| {
        ParseError::new(line, error.offset + 1, error.parse_kind(), error.message)
    })?;
    while matches!(bytes.get(index), Some(b' ' | b'\t' | b'\r')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'=') {
        return Err(ParseError::new(
            line,
            index + 1,
            ParseErrorKind::MissingAssignment,
            format!("parameter {} assignment requires '='", key.label()),
        ));
    }
    index += 1;
    while matches!(bytes.get(index), Some(b' ' | b'\t' | b'\r')) {
        index += 1;
    }
    if index >= bytes.len() {
        return Err(ParseError::new(
            line,
            index + 1,
            ParseErrorKind::MalformedExpression,
            format!("parameter {} assignment needs a value", key.label()),
        ));
    }
    // In G-code a top-level '(' starts a comment, so it cannot introduce an
    // assignment value. Brackets are the expression form.
    let mut first = index;
    loop {
        while matches!(bytes.get(first), Some(b' ' | b'\t' | b'\r')) {
            first += 1;
        }
        match bytes.get(first) {
            Some(b'+' | b'-') => first += 1,
            Some(b'(') => {
                return Err(ParseError::new(
                    line,
                    first + 1,
                    ParseErrorKind::MalformedExpression,
                    "parentheses start a comment in G-code; use brackets for an assignment expression, for example #1=[2+3]",
                ));
            }
            _ => break,
        }
    }
    let slice = source.get(index..).unwrap_or_default();
    let (value, consumed) = evaluate_value(slice, base, overlay).map_err(|error| {
        ParseError::new(
            line,
            index + error.offset + 1,
            error.parse_kind(),
            error.message,
        )
    })?;
    let end = index + consumed;
    let mut probe = end;
    while matches!(bytes.get(probe), Some(b' ' | b'\t' | b'\r')) {
        probe += 1;
    }
    if let Some(operator @ (b'+' | b'-' | b'/')) = bytes.get(probe).copied() {
        let label = key.label();
        return Err(ParseError::new(
            line,
            probe + 1,
            ParseErrorKind::MalformedExpression,
            format!(
                "compound assignment value for {label} must be bracketed, for example {label}=[2+3*4]; found '{}'",
                char::from(operator)
            ),
        ));
    }
    let assignment = match &key {
        crate::expression::ParameterKey::Numeric(number) => overlay.set_numeric(*number, value),
        crate::expression::ParameterKey::Named(name) => overlay.set_named(base, name, value),
    };
    assignment
        .map_err(|error| ParseError::new(line, start + 1, error.parse_kind(), error.to_string()))?;
    Ok(end)
}

/// Parses a value token that has already been restricted to digits, `.`, `+`,
/// and `-`. Returns `None` for repeated signs, multiple decimal points, a bare
/// sign, a bare dot, and non-finite results.
fn parse_number(token: &str) -> Option<f64> {
    let unsigned = token
        .strip_prefix('-')
        .or_else(|| token.strip_prefix('+'))
        .unwrap_or(token);
    if unsigned.is_empty() {
        return None;
    }
    let mut digits = 0usize;
    let mut dots = 0usize;
    for byte in unsigned.bytes() {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' => {
                dots += 1;
                if dots > 1 {
                    return None;
                }
            }
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    token.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Parses one to three decimal digits after `*`.
///
/// Returns the checksum and the number of bytes consumed (including leading
/// and trailing whitespace that belongs to the checksum field).
fn parse_checksum(rest: &str) -> Result<(u8, usize), String> {
    let trimmed_end = rest.trim_end();
    let trimmed = trimmed_end.trim_start();
    if trimmed.is_empty() {
        return Err("'*' is not followed by a checksum".to_string());
    }
    if !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("checksum must be decimal digits only".to_string());
    }
    if trimmed.len() > 3 {
        return Err("checksum has more than three digits".to_string());
    }
    let value: u8 = trimmed
        .parse()
        .map_err(|_| format!("checksum {trimmed} is out of range"))?;
    Ok((value, rest.len()))
}

/// XOR of every byte in `bytes`, matching grbl's line checksum.
#[must_use]
pub fn checksum_xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum ^ byte)
}

/// Returns the column and name of a leading flow-control keyword.
///
/// An optional `/` block-delete marker and `N` line number are skipped so a
/// numbered flow-control line still produces the clear deferral message.
fn leading_control_keyword(source: &str) -> Option<(usize, &'static str)> {
    let trimmed = source.trim_start();
    let mut offset = source.len() - trimmed.len();
    let mut rest = trimmed;
    if let Some(after) = rest.strip_prefix('/') {
        offset += 1;
        rest = after;
    }
    let rest_trimmed = rest.trim_start();
    offset += rest.len() - rest_trimmed.len();
    rest = rest_trimmed;
    if let Some(skip) = line_number_prefix_len(rest) {
        offset += skip;
        rest = &rest[skip..];
        let rest_trimmed = rest.trim_start();
        offset += rest.len() - rest_trimmed.len();
        rest = rest_trimmed;
    }
    let mut end = 0usize;
    for (index, ch) in rest.char_indices() {
        if ch.is_ascii_alphabetic() {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let candidate = rest[..end].to_ascii_uppercase();
    let tail = &rest[end..];
    let boundary = tail.is_empty()
        || tail.starts_with(char::is_whitespace)
        || tail.starts_with('[')
        || tail.starts_with('(');
    if !boundary {
        return None;
    }
    CONTROL_KEYWORDS
        .iter()
        .find(|keyword| **keyword == candidate)
        .map(|keyword| (offset + 1, *keyword))
}

/// Returns the byte length of a leading `N` line number, if present.
fn line_number_prefix_len(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    if !matches!(bytes.first(), Some(b'N' | b'n')) {
        return None;
    }
    let mut end = 1usize;
    while matches!(bytes.get(end), Some(byte) if byte.is_ascii_digit()) {
        end += 1;
    }
    (end > 1).then_some(end)
}

/// Returns `true` when the only non-space, non-comment content is `%`.
fn is_lone_percent(source: &str) -> bool {
    let mut rest = source.trim();
    if let Some(stripped) = rest.strip_prefix('%') {
        rest = stripped;
    } else {
        return false;
    }
    let rest = match rest.find(';') {
        Some(index) => &rest[..index],
        None => rest,
    };
    let mut rest = rest.trim();
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else {
            return false;
        };
        rest = rest[open + close + 1..].trim();
    }
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::{MAX_EXPRESSION_DEPTH, MAX_NUMERIC_PARAMETERS};

    #[test]
    fn words_and_comments() {
        let lexed = lex_line(1, "N10 G0 X1 (move) ; tail", DEFAULT_MAX_WORDS).expect("lex");
        assert_eq!(lexed.words.len(), 3);
        assert_eq!(lexed.words[0], Word::new('N', 10.0, 1));
        assert_eq!(lexed.words[1], Word::new('G', 0.0, 5));
        assert_eq!(lexed.words[2], Word::new('X', 1.0, 8));
        assert_eq!(lexed.comments, vec!["move", "tail"]);
        assert!(!lexed.block_delete);
    }

    #[test]
    fn lowercase_and_compact_words() {
        let lexed = lex_line(1, "g1x-2.5y.5", DEFAULT_MAX_WORDS).expect("lex");
        assert_eq!(lexed.words[0], Word::new('G', 1.0, 1));
        assert_eq!(lexed.words[1], Word::new('X', -2.5, 3));
        assert_eq!(lexed.words[2], Word::new('Y', 0.5, 8));
    }

    #[test]
    fn block_delete_and_percent() {
        let lexed = lex_line(1, "/G0 X0", DEFAULT_MAX_WORDS).expect("lex");
        assert!(lexed.block_delete);
        assert_eq!(lexed.words.len(), 2);

        let mark = lex_line(2, "%", DEFAULT_MAX_WORDS).expect("lex");
        assert!(mark.percent);
        assert!(mark.words.is_empty());
    }

    #[test]
    fn checksums_round_trip() {
        let body = "G0 X1";
        let sum = checksum_xor(body.as_bytes());
        let line = format!("{body}*{sum:02}");
        let lexed = lex_line(1, &line, DEFAULT_MAX_WORDS).expect("lex");
        assert_eq!(lexed.checksum, Some(sum));

        let bad = lex_line(1, "G0 X1*99", DEFAULT_MAX_WORDS).expect_err("mismatch");
        assert_eq!(bad[0].kind, ParseErrorKind::ChecksumMismatch);
    }

    #[test]
    fn malformed_input_reports_kind() {
        assert_eq!(
            lex_line(1, "G0 X", DEFAULT_MAX_WORDS).expect_err("missing")[0].kind,
            ParseErrorKind::MissingValue
        );
        assert_eq!(
            lex_line(1, "G0 X1.2.3", DEFAULT_MAX_WORDS).expect_err("number")[0].kind,
            ParseErrorKind::InvalidNumber
        );
        assert_eq!(
            lex_line(1, "(open", DEFAULT_MAX_WORDS).expect_err("comment")[0].kind,
            ParseErrorKind::UnterminatedComment
        );
        assert_eq!(
            lex_line(1, "G0 X[#1]", DEFAULT_MAX_WORDS).expect_err("unresolved")[0].kind,
            ParseErrorKind::UnresolvedParameter
        );
        assert_eq!(
            lex_line(1, "#1 5", DEFAULT_MAX_WORDS).expect_err("assignment")[0].kind,
            ParseErrorKind::MissingAssignment
        );
        assert_eq!(
            lex_line(1, "[1+2]", DEFAULT_MAX_WORDS).expect_err("bare expression")[0].kind,
            ParseErrorKind::ExpressionNotAllowed
        );
        assert_eq!(
            lex_line(1, "IF [#1 GT 2]", DEFAULT_MAX_WORDS).expect_err("flow")[0].kind,
            ParseErrorKind::UnsupportedExpression
        );
        assert_eq!(
            lex_line(1, "G0 X1*", DEFAULT_MAX_WORDS).expect_err("checksum")[0].kind,
            ParseErrorKind::InvalidChecksum
        );
        assert_eq!(
            lex_line(1, "G0 @", DEFAULT_MAX_WORDS).expect_err("char")[0].kind,
            ParseErrorKind::UnexpectedCharacter
        );
    }

    #[test]
    fn assignments_and_expressions_are_lexed() {
        let lexed = lex_line(1, "#1=5 X[#1*2] Y#1", DEFAULT_MAX_WORDS).expect("lex");
        assert_eq!(
            lexed.words,
            vec![Word::new('X', 10.0, 6), Word::new('Y', 5.0, 14)]
        );

        let named = lex_line(1, "#<feed>=100 F[#<feed>/2]", DEFAULT_MAX_WORDS).expect("lex");
        assert_eq!(named.words, vec![Word::new('F', 50.0, 13)]);
    }

    #[test]
    fn a_multiplication_inside_brackets_leaves_a_trailing_checksum_alone() {
        let body = "#1=3 X[[#1*2]]";
        let sum = checksum_xor(body.as_bytes());
        let line = format!("{body}*{sum:02}");
        let mut parameters = Parameters::new();
        let lexed =
            lex_line_with_parameters(1, &line, DEFAULT_MAX_WORDS, &mut parameters).expect("lex");
        assert_eq!(lexed.checksum, Some(sum));
        assert_eq!(lexed.words, vec![Word::new('X', 6.0, 6)]);
        assert_eq!(parameters.numeric(1), Some(3.0));
    }

    #[test]
    fn an_assignment_line_keeps_its_checksum() {
        let body = "#1=5";
        let sum = checksum_xor(body.as_bytes());
        let line = format!("{body}*{sum:02}");
        let mut parameters = Parameters::new();
        let lexed =
            lex_line_with_parameters(1, &line, DEFAULT_MAX_WORDS, &mut parameters).expect("lex");
        assert_eq!(lexed.checksum, Some(sum));
        assert!(lexed.words.is_empty());
        assert_eq!(parameters.numeric(1), Some(5.0));
    }

    #[test]
    fn expressions_are_rejected_in_code_and_line_numbers() {
        for line in ["G[1+2]", "M[3]", "N[1] G0", "O[1]"] {
            let errors = lex_line(1, line, DEFAULT_MAX_WORDS).expect_err(line);
            assert_eq!(errors[0].kind, ParseErrorKind::ExpressionNotAllowed);
        }
    }

    #[test]
    fn expression_bounds_are_reported_by_the_lexer() {
        let deep = format!(
            "G0 X{}1{}",
            "[".repeat(MAX_EXPRESSION_DEPTH + 1),
            "]".repeat(MAX_EXPRESSION_DEPTH + 1)
        );
        let errors = lex_line(1, &deep, DEFAULT_MAX_WORDS).expect_err("deep");
        assert_eq!(errors[0].kind, ParseErrorKind::ExpressionTooDeep);

        let errors = lex_line(1, "#101=1", DEFAULT_MAX_WORDS).expect_err("range");
        assert_eq!(errors[0].kind, ParseErrorKind::ParameterOutOfRange);
        assert_eq!(MAX_NUMERIC_PARAMETERS, 100);
    }

    #[test]
    fn word_limit_is_enforced() {
        let line = "G1 X1 Y1 Z1";
        let errors = lex_line(1, line, 2).expect_err("too many");
        assert_eq!(errors[0].kind, ParseErrorKind::TooManyWords);
    }

    #[test]
    fn numbers_reject_non_finite_and_repeated_signs() {
        assert_eq!(parse_number("1.5"), Some(1.5));
        assert_eq!(parse_number("-.5"), Some(-0.5));
        assert_eq!(parse_number("+7."), Some(7.0));
        assert_eq!(parse_number("1e3"), None);
        assert_eq!(parse_number("--1"), None);
        assert_eq!(parse_number("1-2"), None);
        assert_eq!(parse_number("."), None);
        assert_eq!(parse_number("+"), None);
    }
}
