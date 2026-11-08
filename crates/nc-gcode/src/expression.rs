//! Bounded `#` parameter storage and arithmetic expression evaluation.
//!
//! The lexer resolves parameter references and `[...]` expressions while it
//! builds the word list. This module owns the value table and the recursive
//! descent evaluator behind that work, so expression syntax is independent of
//! modal state and the planner.
//!
//! # Grammar
//!
//! ```text
//! value       := ['+' | '-']* (number | '#' parameter | group)
//! expression  := term (('+' | '-') term)*
//! term        := unary (('*' | '/') unary)*
//! unary       := ('+' | '-') unary | primary
//! primary     := number | '#' parameter | '(' expression ')' | '[' expression ']'
//! parameter   := digits | '<' name '>'
//! number      := digits ['.' digits] | '.' digits
//! ```
//!
//! A word value is one `value`; compound arithmetic needs brackets, for
//! example `X[[#1+2]*3]`, so a trailing `*` keeps its checksum meaning.
//!
//! # Assignment values
//!
//! An assignment accepts exactly one `value`: a signed decimal (`#1=5`,
//! `#1=-2.5`), a parameter reference (`#1=#2`, `#1=#<feed>`), or a bracketed
//! expression (`#1=[2+3*4]`). Compound arithmetic without brackets
//! (`#1=2+3`) is rejected by the lexer; `*` after an unbracketed value is
//! parsed as the line checksum. A top-level `(` starts a G-code comment, so
//! assignment grouping uses `[...]`.
//!
//! # Bounds
//!
//! Numbered parameters are `#1` through `#100`; named parameters are limited
//! to [`MAX_NAMED_PARAMETERS`] entries of at most
//! [`MAX_PARAMETER_NAME_BYTES`] ASCII bytes. Expressions may nest at most
//! [`MAX_EXPRESSION_DEPTH`] levels (groups and unary sign chains) and are at
//! most [`MAX_EXPRESSION_BYTES`] bytes long. Evaluation uses `f64`; division
//! by zero and non-finite results are errors.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::error::ParseErrorKind;

/// Numbered parameters available as `#1` through `#100`.
pub const MAX_NUMERIC_PARAMETERS: u32 = 100;

/// Maximum number of distinct named `#<name>` parameters.
pub const MAX_NAMED_PARAMETERS: usize = 64;

/// Maximum length of one named parameter, in ASCII bytes.
pub const MAX_PARAMETER_NAME_BYTES: usize = 32;

/// Maximum nesting depth of one expression.
pub const MAX_EXPRESSION_DEPTH: usize = 32;

/// Maximum length of one expression, in bytes.
pub const MAX_EXPRESSION_BYTES: usize = 512;

/// A rejected `#` parameter table operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParameterError {
    /// A numbered parameter is outside `#1`..`#100`.
    #[error("parameter number #{index} must be between 1 and {max}")]
    IndexOutOfRange {
        /// The rejected number.
        index: u32,
        /// Largest supported number.
        max: u32,
    },
    /// A named parameter name is empty, too long, or has invalid characters.
    #[error("named parameter #{name} {reason}")]
    InvalidName {
        /// The rejected name.
        name: String,
        /// Why the name is invalid.
        reason: &'static str,
    },
    /// No room is left for another distinct named parameter.
    #[error("the named parameter table is full ({max} entries)")]
    NamedTableFull {
        /// Maximum number of named entries.
        max: usize,
    },
    /// A value is not a finite number.
    #[error("parameter value is not finite")]
    NonFinite,
}

impl ParameterError {
    /// The public parse-error class for this table error.
    pub(crate) const fn parse_kind(&self) -> ParseErrorKind {
        match self {
            Self::IndexOutOfRange { .. } => ParseErrorKind::ParameterOutOfRange,
            Self::InvalidName { .. } => ParseErrorKind::MalformedExpression,
            Self::NamedTableFull { .. } => ParseErrorKind::TooManyParameters,
            Self::NonFinite => ParseErrorKind::InvalidExpressionValue,
        }
    }
}

/// The bounded `#` parameter table of a parser.
///
/// Numbered entries start unset; reading one before its first assignment is an
/// error. Named entries are created by assignment, normalized to ASCII
/// upper case, and bounded by [`MAX_NAMED_PARAMETERS`].
#[derive(Debug, Clone, PartialEq)]
pub struct Parameters {
    numeric: [Option<f64>; MAX_NUMERIC_PARAMETERS as usize],
    named: BTreeMap<String, f64>,
}

impl Default for Parameters {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameters {
    /// Creates an empty parameter table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            numeric: std::array::from_fn(|_| None),
            named: BTreeMap::new(),
        }
    }

    /// Returns a numbered parameter, or `None` when it is unset or out of
    /// range.
    #[must_use]
    pub fn numeric(&self, index: u32) -> Option<f64> {
        let slot = usize::try_from(index).ok()?.checked_sub(1)?;
        self.numeric.get(slot).copied().flatten()
    }

    /// Returns a named parameter, or `None` when it is unset.
    ///
    /// Names are case-insensitive and stored in upper case.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<f64> {
        if name.len() > MAX_PARAMETER_NAME_BYTES {
            return None;
        }
        self.named.get(&name.to_ascii_uppercase()).copied()
    }

    /// Stores a numbered parameter.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterError::IndexOutOfRange`] outside `#1`..`#100` and
    /// [`ParameterError::NonFinite`] for a non-finite value.
    pub fn set_numeric(&mut self, index: u32, value: f64) -> Result<(), ParameterError> {
        if !(1..=MAX_NUMERIC_PARAMETERS).contains(&index) {
            return Err(ParameterError::IndexOutOfRange {
                index,
                max: MAX_NUMERIC_PARAMETERS,
            });
        }
        if !value.is_finite() {
            return Err(ParameterError::NonFinite);
        }
        let Some(slot) = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|slot| self.numeric.get_mut(slot))
        else {
            return Err(ParameterError::IndexOutOfRange {
                index,
                max: MAX_NUMERIC_PARAMETERS,
            });
        };
        *slot = Some(value);
        Ok(())
    }

    /// Stores a named parameter, creating it when it is new.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterError::InvalidName`] for a bad name,
    /// [`ParameterError::NamedTableFull`] when no room is left for a new name,
    /// and [`ParameterError::NonFinite`] for a non-finite value.
    pub fn set_named(&mut self, name: &str, value: f64) -> Result<(), ParameterError> {
        validate_parameter_name(name).map_err(|reason| ParameterError::InvalidName {
            name: bounded_name(name),
            reason,
        })?;
        if !value.is_finite() {
            return Err(ParameterError::NonFinite);
        }
        let key = name.to_ascii_uppercase();
        if !self.named.contains_key(&key) && self.named.len() >= MAX_NAMED_PARAMETERS {
            return Err(ParameterError::NamedTableFull {
                max: MAX_NAMED_PARAMETERS,
            });
        }
        self.named.insert(key, value);
        Ok(())
    }

    /// Clears every numbered and named parameter.
    pub fn clear(&mut self) {
        self.numeric.fill(None);
        self.named.clear();
    }

    /// Returns the number of assigned parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.numeric.iter().filter(|value| value.is_some()).count() + self.named.len()
    }

    /// Returns `true` when no parameter is assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of assigned named parameters.
    #[must_use]
    pub fn named_len(&self) -> usize {
        self.named.len()
    }
}

/// Validates a named parameter's spelling.
fn validate_parameter_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("must not be empty");
    }
    if name.len() > MAX_PARAMETER_NAME_BYTES {
        return Err("is longer than 32 bytes");
    }
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return Err("must not be empty");
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err("must start with a letter or underscore");
    }
    if bytes.any(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_')) {
        return Err("may contain only ASCII letters, digits, and underscores");
    }
    Ok(())
}

/// Copies at most enough characters of a rejected name for a diagnostic.
fn bounded_name(name: &str) -> String {
    let mut chars = name.chars();
    let mut bounded: String = chars.by_ref().take(MAX_PARAMETER_NAME_BYTES).collect();
    if chars.next().is_some() {
        bounded.push_str("...");
    }
    bounded
}

/// Pending assignments for one block.
///
/// The lexer writes assignments here instead of mutating [`Parameters`], so a
/// block that fails at any stage leaves the real table untouched. Lookups read
/// the newest pending value first, which gives assignments in one block the
/// expected source-order semantics. [`ParameterOverlay::commit`] applies the
/// overlay only after the whole block validates.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ParameterOverlay {
    numeric: BTreeMap<u32, f64>,
    named: BTreeMap<String, f64>,
}

impl ParameterOverlay {
    /// Returns a pending numbered parameter.
    pub(crate) fn numeric(&self, index: u32) -> Option<f64> {
        self.numeric.get(&index).copied()
    }

    /// Returns a pending named parameter. `name` must be upper case.
    pub(crate) fn named(&self, name: &str) -> Option<f64> {
        self.named.get(name).copied()
    }

    /// Stages one numbered assignment.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterError::IndexOutOfRange`] outside `#1`..`#100` and
    /// [`ParameterError::NonFinite`] for a non-finite value.
    pub(crate) fn set_numeric(&mut self, index: u32, value: f64) -> Result<(), ParameterError> {
        if !(1..=MAX_NUMERIC_PARAMETERS).contains(&index) {
            return Err(ParameterError::IndexOutOfRange {
                index,
                max: MAX_NUMERIC_PARAMETERS,
            });
        }
        if !value.is_finite() {
            return Err(ParameterError::NonFinite);
        }
        self.numeric.insert(index, value);
        Ok(())
    }

    /// Stages one named assignment, creating the name when it is new.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterError::InvalidName`] for a bad name,
    /// [`ParameterError::NamedTableFull`] when the union of committed and
    /// pending names would exceed the bound, and [`ParameterError::NonFinite`]
    /// for a non-finite value.
    pub(crate) fn set_named(
        &mut self,
        base: &Parameters,
        name: &str,
        value: f64,
    ) -> Result<(), ParameterError> {
        validate_parameter_name(name).map_err(|reason| ParameterError::InvalidName {
            name: bounded_name(name),
            reason,
        })?;
        if !value.is_finite() {
            return Err(ParameterError::NonFinite);
        }
        let key = name.to_ascii_uppercase();
        if !self.named.contains_key(&key) && !base.named.contains_key(&key) {
            let pending_new = self
                .named
                .keys()
                .filter(|name| !base.named.contains_key(*name))
                .count();
            if base.named.len() + pending_new >= MAX_NAMED_PARAMETERS {
                return Err(ParameterError::NamedTableFull {
                    max: MAX_NAMED_PARAMETERS,
                });
            }
        }
        self.named.insert(key, value);
        Ok(())
    }

    /// Applies every pending assignment to `parameters`.
    ///
    /// Values were validated when staged, so this cannot fail. It runs only
    /// after the block lexes, parses, and plans successfully.
    pub(crate) fn commit(self, parameters: &mut Parameters) {
        for (index, value) in self.numeric {
            if let Some(slot) = usize::try_from(index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|slot| parameters.numeric.get_mut(slot))
            {
                *slot = Some(value);
            }
        }
        parameters.named.extend(self.named);
    }
}

/// A parsed `#` parameter reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParameterKey {
    /// `#1`..`#100`.
    Numeric(u32),
    /// `#<name>`, normalized to upper case.
    Named(String),
}

impl ParameterKey {
    /// A human-readable reference for diagnostics.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Numeric(index) => format!("#{index}"),
            Self::Named(name) => format!("#<{name}>"),
        }
    }
}

/// Why an expression could not be evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpressionErrorKind {
    Malformed,
    UnresolvedParameter,
    UnknownNamedParameter,
    ParameterOutOfRange,
    DivisionByZero,
    NonFinite,
    TooDeep,
    TooLong,
}

/// One expression failure with a byte offset into the expression source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpressionError {
    /// Zero-based byte offset of the failure within the parsed source.
    pub offset: usize,
    /// Machine-readable failure class.
    pub kind: ExpressionErrorKind,
    /// Human-readable explanation.
    pub message: String,
}

impl ExpressionError {
    fn new(offset: usize, kind: ExpressionErrorKind, message: impl Into<String>) -> Self {
        Self {
            offset,
            kind,
            message: message.into(),
        }
    }

    /// Maps to the public parse-error class.
    pub(crate) const fn parse_kind(&self) -> ParseErrorKind {
        match self.kind {
            ExpressionErrorKind::Malformed => ParseErrorKind::MalformedExpression,
            ExpressionErrorKind::UnresolvedParameter => ParseErrorKind::UnresolvedParameter,
            ExpressionErrorKind::UnknownNamedParameter => ParseErrorKind::UnknownNamedParameter,
            ExpressionErrorKind::ParameterOutOfRange => ParseErrorKind::ParameterOutOfRange,
            ExpressionErrorKind::DivisionByZero | ExpressionErrorKind::NonFinite => {
                ParseErrorKind::InvalidExpressionValue
            }
            ExpressionErrorKind::TooDeep => ParseErrorKind::ExpressionTooDeep,
            ExpressionErrorKind::TooLong => ParseErrorKind::ExpressionTooLong,
        }
    }
}

/// Parses a `#` parameter reference starting at `hash_at`.
///
/// `source` is any slice and error offsets are relative to it. Returns the key
/// and the byte index just after the reference.
pub(crate) fn parse_parameter_key(
    source: &str,
    hash_at: usize,
) -> Result<(ParameterKey, usize), ExpressionError> {
    let bytes = source.as_bytes();
    let mut index = hash_at + 1;
    let Some(byte) = bytes.get(index).copied() else {
        return Err(ExpressionError::new(
            hash_at,
            ExpressionErrorKind::Malformed,
            "'#' must be followed by a parameter number or <name>",
        ));
    };
    if byte == b'<' {
        index += 1;
        let name_start = index;
        let Some(close_offset) = source.get(name_start..).and_then(|rest| rest.find('>')) else {
            return Err(ExpressionError::new(
                hash_at,
                ExpressionErrorKind::Malformed,
                "named parameter is missing its closing '>'",
            ));
        };
        let close = name_start + close_offset;
        let name = source.get(name_start..close).unwrap_or_default();
        if let Err(reason) = validate_parameter_name(name) {
            return Err(ExpressionError::new(
                hash_at,
                ExpressionErrorKind::Malformed,
                format!("named parameter {:?} {reason}", bounded_name(name)),
            ));
        }
        Ok((ParameterKey::Named(name.to_ascii_uppercase()), close + 1))
    } else if byte.is_ascii_digit() {
        let digits_start = index;
        while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'.') {
            return Err(ExpressionError::new(
                digits_start,
                ExpressionErrorKind::Malformed,
                "parameter numbers must be integers",
            ));
        }
        let token = source.get(digits_start..index).unwrap_or_default();
        let Ok(number) = token.parse::<u32>() else {
            return Err(ExpressionError::new(
                hash_at,
                ExpressionErrorKind::ParameterOutOfRange,
                format!("parameter number #{token} is too large"),
            ));
        };
        if !(1..=MAX_NUMERIC_PARAMETERS).contains(&number) {
            return Err(ExpressionError::new(
                hash_at,
                ExpressionErrorKind::ParameterOutOfRange,
                format!(
                    "parameter number #{number} must be between 1 and {MAX_NUMERIC_PARAMETERS}"
                ),
            ));
        }
        Ok((ParameterKey::Numeric(number), index))
    } else {
        Err(ExpressionError::new(
            hash_at,
            ExpressionErrorKind::Malformed,
            "'#' must be followed by a parameter number or <name>",
        ))
    }
}

/// Evaluates one word or assignment value: unary signs plus one primary.
///
/// Returns the value and the number of bytes consumed from `source`. Binary
/// operators are deliberately not consumed here, so a trailing `*` outside a
/// bracket keeps its checksum meaning.
pub(crate) fn evaluate_value(
    source: &str,
    base: &Parameters,
    overlay: &ParameterOverlay,
) -> Result<(f64, usize), ExpressionError> {
    let mut parser = Parser {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        depth: 0,
        base,
        overlay,
    };
    let value = parser.parse_unary()?;
    Ok((value, parser.pos))
}

/// Evaluates a complete bracketed expression, requiring only that a closing
/// `]` follows; the caller consumes it.
#[cfg(test)]
pub(crate) fn evaluate_expression(
    source: &str,
    base: &Parameters,
    overlay: &ParameterOverlay,
) -> Result<(f64, usize), ExpressionError> {
    let mut parser = Parser {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        depth: 0,
        base,
        overlay,
    };
    let value = parser.parse_expression()?;
    parser.skip_whitespace()?;
    Ok((value, parser.pos))
}

/// Recursive descent evaluator.
struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
    base: &'a Parameters,
    overlay: &'a ParameterOverlay,
}

impl Parser<'_> {
    /// Parses `term (('+' | '-') term)*`.
    fn parse_expression(&mut self) -> Result<f64, ExpressionError> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_whitespace()?;
            let offset = self.pos;
            match self.bytes.get(self.pos).copied() {
                Some(b'+') => {
                    self.pos += 1;
                    let right = self.parse_term()?;
                    value = finite(value + right, offset)?;
                }
                Some(b'-') => {
                    self.pos += 1;
                    let right = self.parse_term()?;
                    value = finite(value - right, offset)?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    /// Parses `unary (('*' | '/') unary)*`.
    fn parse_term(&mut self) -> Result<f64, ExpressionError> {
        let mut value = self.parse_unary()?;
        loop {
            self.skip_whitespace()?;
            let offset = self.pos;
            match self.bytes.get(self.pos).copied() {
                Some(b'*') => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    value = finite(value * right, offset)?;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    if right == 0.0 {
                        return Err(ExpressionError::new(
                            offset,
                            ExpressionErrorKind::DivisionByZero,
                            "division by zero in expression",
                        ));
                    }
                    value = finite(value / right, offset)?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    /// Parses unary signs and one primary.
    fn parse_unary(&mut self) -> Result<f64, ExpressionError> {
        self.skip_whitespace()?;
        let offset = self.pos;
        match self.bytes.get(self.pos).copied() {
            Some(b'+') => {
                self.pos += 1;
                self.enter(offset)?;
                let value = self.parse_unary()?;
                self.leave();
                Ok(value)
            }
            Some(b'-') => {
                self.pos += 1;
                self.enter(offset)?;
                let value = self.parse_unary()?;
                self.leave();
                Ok(-value)
            }
            _ => self.parse_primary(),
        }
    }

    /// Parses a number, parameter, or parenthesized/bracketed group.
    fn parse_primary(&mut self) -> Result<f64, ExpressionError> {
        self.skip_whitespace()?;
        let offset = self.pos;
        match self.bytes.get(self.pos).copied() {
            Some(b'(' | b'[') => {
                let close = if self.bytes.get(self.pos) == Some(&b'(') {
                    b')'
                } else {
                    b']'
                };
                self.pos += 1;
                self.enter(offset)?;
                let value = self.parse_expression()?;
                self.skip_whitespace()?;
                if self.bytes.get(self.pos) != Some(&close) {
                    return Err(ExpressionError::new(
                        self.pos,
                        ExpressionErrorKind::Malformed,
                        format!("expected '{}' to close the expression", char::from(close)),
                    ));
                }
                self.pos += 1;
                self.leave();
                Ok(value)
            }
            Some(b'#') => self.parse_parameter(),
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => self.parse_number(),
            Some(byte) => Err(ExpressionError::new(
                offset,
                ExpressionErrorKind::Malformed,
                format!(
                    "expected a number, parameter, or group, found {:?}",
                    char::from(byte)
                ),
            )),
            None => Err(ExpressionError::new(
                offset,
                ExpressionErrorKind::Malformed,
                "expected a number, parameter, or group",
            )),
        }
    }

    /// Parses `#1` or `#<name>` and resolves it.
    fn parse_parameter(&mut self) -> Result<f64, ExpressionError> {
        let offset = self.pos;
        let (key, next) = parse_parameter_key(self.source, offset)?;
        self.pos = next;
        self.check_length()?;
        match &key {
            ParameterKey::Numeric(index) => self
                .overlay
                .numeric(*index)
                .or_else(|| self.base.numeric(*index))
                .ok_or_else(|| {
                    ExpressionError::new(
                        offset,
                        ExpressionErrorKind::UnresolvedParameter,
                        format!("parameter {} is not set", key.label()),
                    )
                }),
            ParameterKey::Named(name) => self
                .overlay
                .named(name)
                .or_else(|| self.base.named.get(name).copied())
                .ok_or_else(|| {
                    ExpressionError::new(
                        offset,
                        ExpressionErrorKind::UnknownNamedParameter,
                        format!("named parameter {} is not set", key.label()),
                    )
                }),
        }
    }

    /// Parses a finite decimal literal.
    fn parse_number(&mut self) -> Result<f64, ExpressionError> {
        let start = self.pos;
        let mut digits = 0usize;
        let mut dots = 0usize;
        while let Some(byte) = self.bytes.get(self.pos).copied() {
            match byte {
                byte if byte.is_ascii_digit() => {
                    digits += 1;
                    self.pos += 1;
                }
                b'.' if dots == 0 => {
                    dots += 1;
                    self.pos += 1;
                }
                _ => break,
            }
            self.check_length()?;
        }
        if digits == 0 {
            return Err(ExpressionError::new(
                start,
                ExpressionErrorKind::Malformed,
                "expected a decimal number",
            ));
        }
        let token = self.source.get(start..self.pos).unwrap_or_default();
        match token.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            Ok(_) => Err(ExpressionError::new(
                start,
                ExpressionErrorKind::NonFinite,
                "number is not finite",
            )),
            Err(_) => Err(ExpressionError::new(
                start,
                ExpressionErrorKind::Malformed,
                format!("{token:?} is not a valid decimal number"),
            )),
        }
    }

    /// Skips spaces and tabs, enforcing the expression length bound.
    fn skip_whitespace(&mut self) -> Result<(), ExpressionError> {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
        self.check_length()
    }

    /// Enters one nesting level.
    fn enter(&mut self, offset: usize) -> Result<(), ExpressionError> {
        self.depth += 1;
        if self.depth > MAX_EXPRESSION_DEPTH {
            return Err(ExpressionError::new(
                offset,
                ExpressionErrorKind::TooDeep,
                format!("expression nests deeper than {MAX_EXPRESSION_DEPTH} levels"),
            ));
        }
        Ok(())
    }

    /// Leaves one nesting level.
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Enforces the byte-length bound.
    fn check_length(&self) -> Result<(), ExpressionError> {
        if self.pos > MAX_EXPRESSION_BYTES {
            Err(ExpressionError::new(
                MAX_EXPRESSION_BYTES,
                ExpressionErrorKind::TooLong,
                format!("expression is longer than {MAX_EXPRESSION_BYTES} bytes"),
            ))
        } else {
            Ok(())
        }
    }
}

/// Rejects a non-finite arithmetic result.
fn finite(value: f64, offset: usize) -> Result<f64, ExpressionError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ExpressionError::new(
            offset,
            ExpressionErrorKind::NonFinite,
            "expression result is not finite",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(source: &str, parameters: &Parameters) -> Result<(f64, usize), ExpressionError> {
        evaluate_expression(source, parameters, &ParameterOverlay::default())
    }

    fn eval_value(source: &str, parameters: &Parameters) -> Result<(f64, usize), ExpressionError> {
        evaluate_value(source, parameters, &ParameterOverlay::default())
    }

    #[test]
    fn numeric_parameters_are_bounded_and_reject_non_finite_values() {
        let mut parameters = Parameters::new();
        assert_eq!(
            parameters.set_numeric(0, 1.0),
            Err(ParameterError::IndexOutOfRange {
                index: 0,
                max: MAX_NUMERIC_PARAMETERS
            })
        );
        assert_eq!(
            parameters.set_numeric(MAX_NUMERIC_PARAMETERS + 1, 1.0),
            Err(ParameterError::IndexOutOfRange {
                index: MAX_NUMERIC_PARAMETERS + 1,
                max: MAX_NUMERIC_PARAMETERS
            })
        );
        assert_eq!(
            parameters.set_numeric(1, f64::NAN),
            Err(ParameterError::NonFinite)
        );
        parameters.set_numeric(1, 2.5).expect("set");
        parameters
            .set_numeric(MAX_NUMERIC_PARAMETERS, 7.0)
            .expect("last");
        assert_eq!(parameters.numeric(1), Some(2.5));
        assert_eq!(parameters.numeric(MAX_NUMERIC_PARAMETERS), Some(7.0));
        assert_eq!(parameters.numeric(2), None);
    }

    #[test]
    fn named_parameters_are_case_insensitive_and_bounded() {
        let mut parameters = Parameters::new();
        parameters.set_named("feed", 100.0).expect("set");
        assert_eq!(parameters.named("FEED"), Some(100.0));
        assert_eq!(parameters.named("Feed"), Some(100.0));
        assert_eq!(parameters.named_len(), 1);

        assert!(matches!(
            parameters.set_named("9bad", 1.0),
            Err(ParameterError::InvalidName { .. })
        ));
        assert!(matches!(
            parameters.set_named(&"x".repeat(MAX_PARAMETER_NAME_BYTES + 1), 1.0),
            Err(ParameterError::InvalidName { .. })
        ));

        for index in 1..MAX_NAMED_PARAMETERS {
            parameters
                .set_named(&format!("P{index}"), f64::from(index as u32))
                .expect("named");
        }
        assert_eq!(parameters.named_len(), MAX_NAMED_PARAMETERS);
        assert_eq!(
            parameters.set_named("overflow", 0.0),
            Err(ParameterError::NamedTableFull {
                max: MAX_NAMED_PARAMETERS
            })
        );
    }

    #[test]
    fn evaluation_honours_precedence_and_nesting() {
        let mut parameters = Parameters::new();
        parameters.set_numeric(1, 2.0).expect("set");
        parameters.set_numeric(2, 3.0).expect("set");

        let (value, used) = eval("#1+#2*4", &parameters).expect("eval");
        assert_eq!(value, 14.0);
        assert_eq!(used, 7);

        let (value, _) = eval("[(#1+#2)*2]", &parameters).expect("eval");
        assert_eq!(value, 10.0);

        let (value, _) = eval("-+#1 + +5", &parameters).expect("eval");
        assert_eq!(value, 3.0);
    }

    #[test]
    fn evaluation_rejects_division_by_zero_and_non_finite_results() {
        let parameters = Parameters::new();
        let error = eval("1/0", &parameters).expect_err("zero");
        assert_eq!(error.kind, ExpressionErrorKind::DivisionByZero);

        let huge = "9".repeat(400);
        let error = eval(&huge, &parameters).expect_err("infinite literal");
        assert_eq!(error.kind, ExpressionErrorKind::NonFinite);
    }

    #[test]
    fn evaluation_is_bounded_by_depth_and_length() {
        let parameters = Parameters::new();
        let deep = format!(
            "{}1{}",
            "[".repeat(MAX_EXPRESSION_DEPTH + 1),
            "]".repeat(MAX_EXPRESSION_DEPTH + 1)
        );
        let error = eval(&deep, &parameters).expect_err("deep");
        assert_eq!(error.kind, ExpressionErrorKind::TooDeep);

        let long = format!("{}{}", "1+".repeat(MAX_EXPRESSION_BYTES), "1");
        let error = eval(&long, &parameters).expect_err("long");
        assert_eq!(error.kind, ExpressionErrorKind::TooLong);
    }

    #[test]
    fn value_evaluation_does_not_consume_binary_operators() {
        let mut parameters = Parameters::new();
        parameters.set_numeric(1, 4.0).expect("set");
        let (value, used) = eval_value("[#1+2]*7", &parameters).expect("value");
        assert_eq!(value, 6.0);
        assert_eq!(used, 6);
    }

    #[test]
    fn malformed_parameter_references_are_rejected() {
        let parameters = Parameters::new();
        for text in ["#", "#<", "#<>", "#<9x>", "#1.5", "#abc"] {
            let error = eval(text, &parameters).expect_err(text);
            assert_eq!(
                error.kind,
                ExpressionErrorKind::Malformed,
                "unexpected kind for {text}"
            );
        }
    }

    #[test]
    fn an_overlay_shadows_the_base_and_commits_only_when_asked() {
        let mut base = Parameters::new();
        base.set_numeric(1, 1.0).expect("base");
        base.set_named("feed", 100.0).expect("base");

        let mut overlay = ParameterOverlay::default();
        overlay.set_numeric(1, 2.0).expect("numeric");
        overlay.set_named(&base, "feed", 250.0).expect("named");
        overlay.set_named(&base, "speed", 50.0).expect("new name");

        assert_eq!(overlay.numeric(1), Some(2.0));
        assert_eq!(overlay.named("FEED"), Some(250.0));
        assert_eq!(overlay.named("SPEED"), Some(50.0));
        assert_eq!(base.numeric(1), Some(1.0));
        assert_eq!(base.named("FEED"), Some(100.0));

        let (value, _) =
            evaluate_expression("#1+#<feed>+#<speed>", &base, &overlay).expect("overlay eval");
        assert_eq!(value, 302.0);

        overlay.commit(&mut base);
        assert_eq!(base.numeric(1), Some(2.0));
        assert_eq!(base.named("FEED"), Some(250.0));
        assert_eq!(base.named("SPEED"), Some(50.0));
    }

    #[test]
    fn an_overlay_enforces_the_named_table_bound_before_commit() {
        let mut base = Parameters::new();
        for index in 0..MAX_NAMED_PARAMETERS - 1 {
            base.set_named(&format!("P{index}"), 1.0).expect("base");
        }
        let mut overlay = ParameterOverlay::default();
        overlay.set_named(&base, "LAST", 2.0).expect("last slot");
        assert_eq!(
            overlay.set_named(&base, "OVERFLOW", 3.0),
            Err(ParameterError::NamedTableFull {
                max: MAX_NAMED_PARAMETERS
            })
        );
        assert_eq!(base.named_len(), MAX_NAMED_PARAMETERS - 1);
    }
}
