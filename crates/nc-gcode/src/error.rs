//! Structured parse errors and non-fatal warnings.
//!
//! A block is validated as a unit and never partially applied, so one bad word
//! cannot leave the modal state half-updated. Every diagnostic carries the
//! source line and a 1-based column so a program-list view can highlight it.

use thiserror::Error;

/// A fatal G-code parse error.
///
/// `parse_program` returns all errors it collected (up to
/// [`ParserConfig::max_errors`](crate::ParserConfig::max_errors)); a single
/// bad block does not stop the rest of the file from being checked.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("line {line}, column {column}: {message}")]
pub struct ParseError {
    /// 1-based source line in the program text.
    pub line: usize,
    /// 1-based byte column in the source line.
    pub column: usize,
    /// Machine-readable error class.
    pub kind: ParseErrorKind,
    /// Human-readable explanation with context.
    pub message: String,
}

impl ParseError {
    /// Creates an error with a message.
    #[must_use]
    pub fn new(
        line: usize,
        column: usize,
        kind: ParseErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            line,
            column,
            kind,
            message: message.into(),
        }
    }
}

/// Classification of a [`ParseError`].
///
/// The set is intentionally small and stable enough for validation UI to group
/// messages without string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseErrorKind {
    /// The source line exceeds the configured byte limit.
    LineTooLong,
    /// The source file exceeds the configured byte limit.
    FileTooLarge,
    /// The line contains more words than the configured limit.
    TooManyWords,
    /// A number could not be parsed as a finite decimal value.
    InvalidNumber,
    /// A character is not valid outside a comment.
    UnexpectedCharacter,
    /// A `(` comment is missing its closing `)`.
    UnterminatedComment,
    /// A `*` checksum is not one to three decimal digits.
    InvalidChecksum,
    /// The `*` checksum does not match the line content.
    ChecksumMismatch,
    /// A letter word is not followed by a number.
    MissingValue,
    /// A line number (`N`) is not the first word.
    MisplacedWord,
    /// A letter that is not a valid word in this dialect.
    UnknownWord,
    /// A valid word that this 3-axis grblHAL dialect does not support.
    UnsupportedWord,
    /// A word is present but the block's command does not use it.
    UnusedWord,
    /// The same word appears twice in one block.
    DuplicateWord,
    /// Two G or M codes belong to the same modal group.
    ConflictingCodes,
    /// Two commands cannot be used in the same block.
    ConflictingCommands,
    /// A syntactically valid G or M code that is not a grblHAL code.
    UnknownCode,
    /// A known GRBL/RS274 code that grblHAL or this parser does not support.
    UnsupportedCode,
    /// A required parameter word is missing or inconsistent.
    MissingParameter,
    /// A parameter value is invalid for the command.
    InvalidParameter,
    /// A value that must be non-negative is negative.
    NegativeValue,
    /// A value is outside the supported range.
    ValueOutOfRange,
    /// A feed move has no usable feed rate.
    FeedRateUndefined,
    /// Axis or arc words are present but no motion mode is active.
    NoMotionMode,
    /// Arc geometry is invalid (radius mismatch, zero radius, bad sweep).
    InvalidArc,
    /// Flow control (`IF`, `WHILE`, `GOTO`, `DO`, `SUB`, ...) is not supported.
    UnsupportedExpression,
    /// A numbered `#` parameter was read before it was assigned.
    UnresolvedParameter,
    /// A named `#<...>` parameter was never assigned.
    UnknownNamedParameter,
    /// A `#` parameter number is outside `#1`..`#100`.
    ParameterOutOfRange,
    /// A `#` parameter assignment is missing its `=`.
    MissingAssignment,
    /// The bounded named-parameter table is full.
    TooManyParameters,
    /// A `[...]` expression is malformed, or a compound assignment value was
    /// not bracketed (for example `#1=2+3` instead of `#1=[2+3]`). A
    /// top-level `(` in an assignment is a comment, not an expression.
    MalformedExpression,
    /// An expression nests deeper than the supported limit.
    ExpressionTooDeep,
    /// An expression is longer than the supported limit.
    ExpressionTooLong,
    /// An expression divided by zero or produced a non-finite value.
    InvalidExpressionValue,
    /// An expression appeared where only a plain number is allowed.
    ExpressionNotAllowed,
    /// A tool number cannot be represented by the viewer.
    ToolNumberOutOfRange,
    /// Error collection stopped at the configured limit.
    TooManyErrors,
}

impl ParseErrorKind {
    /// Stable snake-case key used in skipped-block records and findings.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::LineTooLong => "line_too_long",
            Self::FileTooLarge => "file_too_large",
            Self::TooManyWords => "too_many_words",
            Self::InvalidNumber => "invalid_number",
            Self::UnexpectedCharacter => "unexpected_character",
            Self::UnterminatedComment => "unterminated_comment",
            Self::InvalidChecksum => "invalid_checksum",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::MissingValue => "missing_value",
            Self::MisplacedWord => "misplaced_word",
            Self::UnknownWord => "unknown_word",
            Self::UnsupportedWord => "unsupported_word",
            Self::UnusedWord => "unused_word",
            Self::DuplicateWord => "duplicate_word",
            Self::ConflictingCodes => "conflicting_codes",
            Self::ConflictingCommands => "conflicting_commands",
            Self::UnknownCode => "unknown_code",
            Self::UnsupportedCode => "unsupported_code",
            Self::MissingParameter => "missing_parameter",
            Self::InvalidParameter => "invalid_parameter",
            Self::NegativeValue => "negative_value",
            Self::ValueOutOfRange => "value_out_of_range",
            Self::FeedRateUndefined => "feed_rate_undefined",
            Self::NoMotionMode => "no_motion_mode",
            Self::InvalidArc => "invalid_arc",
            Self::UnsupportedExpression => "unsupported_expression",
            Self::UnresolvedParameter => "unresolved_parameter",
            Self::UnknownNamedParameter => "unknown_named_parameter",
            Self::ParameterOutOfRange => "parameter_out_of_range",
            Self::MissingAssignment => "missing_assignment",
            Self::TooManyParameters => "too_many_parameters",
            Self::MalformedExpression => "malformed_expression",
            Self::ExpressionTooDeep => "expression_too_deep",
            Self::ExpressionTooLong => "expression_too_long",
            Self::InvalidExpressionValue => "invalid_expression_value",
            Self::ExpressionNotAllowed => "expression_not_allowed",
            Self::ToolNumberOutOfRange => "tool_number_out_of_range",
            Self::TooManyErrors => "too_many_errors",
        }
    }
}

/// A non-fatal issue found while parsing.
///
/// Warnings never change the toolpath; they flag assumptions a validation UI
/// should show before the program is streamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    /// 1-based source line that produced the warning.
    pub line: usize,
    /// Machine-readable warning class.
    pub kind: WarningKind,
    /// Human-readable explanation.
    pub message: String,
}

impl ParseWarning {
    /// Creates a warning with a message.
    #[must_use]
    pub fn new(line: usize, kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            line,
            kind,
            message: message.into(),
        }
    }
}

/// Classification of a [`ParseWarning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// `G28`/`G30` used a stored position this parser does not know; the
    /// machine origin was assumed.
    AssumedHomePosition,
    /// The block reaches motion after `M2`/`M30`, which the controller will
    /// never execute.
    MotionAfterProgramEnd,
    /// A canned-cycle block has a depth above the R plane, so it removes no
    /// material.
    EmptyCannedCycle,
    /// `G41`/`G42` activated cutter compensation; the toolpath still shows the
    /// programmed centerline and the offset was not applied.
    CutterCompNotApplied,
    /// A tool-comment dimension had no determinable unit and was dropped.
    AmbiguousToolHint,
    /// A cutting motion had no programmable feed; the lossy parser kept the
    /// motion with `feed_mm_min` unset so feed-based rules can skip it.
    MissingFeed,
}
