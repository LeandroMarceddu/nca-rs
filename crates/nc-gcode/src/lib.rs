//! G-code lexer, parser, modal-state emulation, and motion records.
//!
//! The crate is independent of the UI, transport, and controller session. It
//! turns program text into validated lines, modal parser state, display
//! geometry, and a semantic [`MotionRecord`] stream. Safety-critical command
//! gating stays in `nc-analysis`.
//!
//! # Pipeline
//!
//! ```text
//! source line -> lexer (parameters + expressions) -> Block
//!             -> Planner (validate + plan) -> Parser::execute
//!             (modal state + Toolpath + MotionRecord + Annotation)
//! ```
//!
//! A block is validated in full before it is applied, so an error never leaves
//! the modal state, the toolpath, or the record stream half-updated. The
//! planner is the only place that interprets words; [`parse_program_lossy`]
//! wraps the same parser and skips failed blocks instead of returning early.
//!
//! # Example
//!
//! ```
//! use nc_gcode::{MoveKind, parse_program};
//!
//! let program = parse_program(
//!     "G21 G90\n\
//!      G0 X0 Y0 Z5\n\
//!      G1 Z-1 F100\n\
//!      G1 X10\n",
//! )
//! .expect("valid program");
//!
//! assert_eq!(program.stats.rapid_segments, 1);
//! assert_eq!(program.stats.plunge_segments, 1);
//! assert_eq!(program.stats.feed_segments, 1);
//! assert_eq!(program.bounds.min().z, -1.0);
//! assert_eq!(program.motions.len(), 3);
//! assert_eq!(program.motions[0].kind, MoveKind::Retract);
//! assert_eq!(program.motions[1].kind, MoveKind::Plunge);
//! assert_eq!(program.motions[2].kind, MoveKind::Feed);
//! assert_eq!(program.toolpath.segments()[0].kind, MoveKind::Retract);
//! ```
//!
//! The display toolpath carries the same semantic kinds as the records.
//!
//! # Motion records
//!
//! [`Program::motions`] is the analysis and simulation source of truth. Every
//! record is in machine millimetres and carries the semantic [`MoveKind`], the
//! active tool as `u32`, feed and spindle state, the plane, an interned
//! [`ModalId`], and exact arc or canned-cycle data when applicable. Canned
//! cycles store one record per hole; [`Program::expanded_motions`] generates
//! their rapid/feed/retract sub-moves on demand.
//!
//! The display [`Toolpath`](nc_viewer::Toolpath) is still produced. It remains
//! the renderer's compact `f32` chord stream and a fallback for very large
//! programs. Every display segment carries the same semantic [`MoveKind`] as
//! the motion record it came from, so the canvas paints `ArcCw` separately
//! from `ArcCcw`, and canned-cycle cutting moves keep `Drill`, `Peck`, or
//! `Bore` instead of collapsing to a generic feed or plunge.
//!
//! # Lossy parsing
//!
//! ```
//! use nc_gcode::{ParserConfig, parse_program_lossy};
//!
//! let result = parse_program_lossy("G0 X1\nG1 X2 Q3\nG1 X3 F100\n", &ParserConfig::default());
//! assert_eq!(result.errors.len(), 1);
//! assert_eq!(result.skipped_blocks[0].line, 2);
//! // The failed block kept the previous modal state and did not stop the file.
//! assert_eq!(result.program.motions.len(), 2);
//! ```
//!
//! A failed block is recorded and skipped. Unknown words and codes are
//! diagnostics, never fatal. Lossy parsing keeps a cutting move whose feed is
//! undefined with `feed_mm_min = None` and a `MissingFeed` warning; strict
//! parsing still reports `FeedRateUndefined`. Truly unsupported state-changing
//! constructs
//! (`M98`/`M99`, macros, flow control) stop new geometry after their line and
//! set [`LossyProgram::trusted_geometry_until`] and
//! [`Program::geometry_uncertain_after_line`]. Cutter compensation (`G41`/
//! `G42`/`G40`) is accepted and recorded in [`CutterComp`] and [`ModalState`];
//! geometry continues on the programmed centerline and never applies an offset.
//!
//! # Input limits
//!
//! [`ParserConfig::max_file_bytes`], [`ParserConfig::max_line_bytes`], and
//! [`ParserConfig::max_words`] bound allocation and work. An over-long file is
//! refused with [`ParseErrorKind::FileTooLarge`]; an over-long line is refused
//! with [`ParseErrorKind::LineTooLong`] and, in lossy mode, skipped.
//!
//! # Supported commands
//!
//! `G0`, `G1`, `G2`, `G3`, `G4`, `G10 L2/L20`, `G17`-`G19`, `G20`/`G21`,
//! `G28`/`G28.1`, `G30`/`G30.1`, `G38.2`-`G38.5`, `G40`-`G42`,
//! `G43.1`, `G49`, `G50`/`G51` (Fanuc-style `P` scale and `XYZ` pivot),
//! `G53`-`G59.3`, `G61`/`G64`, `G73`, `G80`/`G81`/`G82`/`G83`/`G85`/`G86`/
//! `G89`, `G90`/`G91`, `G92`/`G92.1`/`G92.2`/`G92.3`, `G93`/`G94`,
//! `G98`/`G99`; `M0`-`M9`, `M30`; `N`, `T`, `S`, `F`, `D`; `( )` and `;`
//! comments; `/` block delete; `*` checksums; and a lone `%` marker.
//!
//! # Parameters and expressions
//!
//! Numbered parameters `#1` through `#100` and named parameters `#<name>` are
//! supported. Named parameters are case-insensitive, ASCII, at most 32 bytes,
//! and limited to 64 distinct names. An assignment such as `#1=5`,
//! `#<feed>=250`, or `#2=[#1*3]` updates the parser's table and produces no
//! motion. Reading an unassigned numbered parameter or an unknown named
//! parameter is a structured error.
//!
//! A numeric word value may be a parameter or a bracketed arithmetic
//! expression, for example `X[#1+2.5]`, `F[#<feed>*2]`, and `S[#100]`.
//! Expressions support `+`, `-`, `*`, `/`, unary signs, parentheses, nested
//! `[`/`]` groups, decimal literals, and parameter references. They are
//! evaluated with `f64`; division by zero, non-finite results, unresolved
//! parameters, expressions deeper than 32 nesting levels, and expressions
//! longer than 512 bytes are rejected. Expressions are not allowed in `G`,
//! `M`, `N`, or `O` words.
//!
//! An assignment value is deliberately one of:
//!
//! - a signed decimal: `#1=5`, `#1=-2.5`;
//! - a parameter reference: `#1=#2`, `#1=#<feed>`;
//! - a bracketed expression: `#1=[2+3*4]`.
//!
//! Compound arithmetic must be bracketed. `#1=2+3` is a
//! [`ParseErrorKind::MalformedExpression`], not an implicit expression.
//! Because `*` after a value is the line checksum, always put the whole
//! calculation inside brackets. A top-level `(` starts a G-code comment, so
//! assignment grouping also uses `[...]`. Assignments are staged while a block
//! is lexed and committed only after the block validates; a failed block
//! leaves the parameter table untouched.
//!
//! # Deferred (rejected, not emulated)
//!
//! Flow control (`IF`/`WHILE`/`GOTO`/`DO`/`ENDIF`/`SUB`/`M98`/`M99`), `O`-word
//! subprograms, true cutter-radius offset geometry, lathe cycles,
//! `G84`/`G87`/`G88` tapping, `G90.1`/`G91.1` absolute arc centers, and extra
//! axes (`A`, `B`, `C`, `U`, `V`, `W`) remain unsupported. Flow control returns
//! [`ParseErrorKind::UnsupportedExpression`]; subprograms and the other forms
//! return [`ParseErrorKind::UnsupportedCode`] or
//! [`ParseErrorKind::UnsupportedWord`].
//!
//! # Coordinates
//!
//! Stored coordinates are millimetres. The display toolpath applies `G51`
//! scaling, caller-supplied rotation, the active work offset, `G92`, and the
//! `G43.1` tool-length offset, so with zero offsets it equals the program
//! coordinates. [`MotionRecord`] points are in machine coordinates.
//! `G53` moves and `G28`/`G30` home moves target absolute machine coordinates
//! regardless of `G90`/`G91`. Rapids, acceleration, `G93` timing, dwell, and
//! firmware compensation are not part of the display geometry.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod block;
mod config;
mod error;
mod expression;
mod lexer;
mod motion;
mod parser;
mod planner;
mod program;
mod state;

#[cfg(any(test, feature = "test-util"))]
mod builder;

#[cfg(any(test, feature = "test-util"))]
pub use builder::ProgramBuilder;
pub use config::{DEFAULT_MAX_FILE_BYTES, MAX_COORDINATE_MM, MIN_ARC_RADIUS_MM, ParserConfig};
pub use error::{ParseError, ParseErrorKind, ParseWarning, WarningKind};
pub use expression::{
    MAX_EXPRESSION_BYTES, MAX_EXPRESSION_DEPTH, MAX_NAMED_PARAMETERS, MAX_NUMERIC_PARAMETERS,
    MAX_PARAMETER_NAME_BYTES, ParameterError, Parameters,
};
pub use lexer::{
    DEFAULT_MAX_WORDS, LexedLine, Word, checksum_xor, lex_line, lex_line_with_parameters,
};
pub use motion::{
    Annotation, ArcFormat, ArcRecord, CannedCycle, CannedRecord, Comment, CoolantState, CutterComp,
    LossyProgram, ModalId, ModalState, MotionRecord, ProgramToolHint, SkippedBlock,
};
pub use nc_core::{Dialect, ToolHint};
pub use nc_viewer::{AnnotationKind, Bounds3, MoveKind};
pub use parser::{Parser, parse_program, parse_program_lossy, parse_program_with};
pub use program::{Program, ProgramLine, ProgramStats};
pub use state::{
    CannedCycleKind, CannedCycleParams, CoolantMode, DistanceMode, FeedMode, MotionMode,
    ParserState, PathControl, Plane, ProbeKind, RetractMode, Rotation, Scaling, SpindleMode,
    ToolLength,
};
