//! One block of words, plus the supported G- and M-code tables.
//!
//! The lexer is syntax-only; this module gives words their meaning. A block is
//! parsed into typed fields and validated for duplicate and conflicting words
//! before the planner sees it.

use nc_core::{Wcs, math};

use crate::error::{ParseError, ParseErrorKind};
use crate::lexer::LexedLine;
use crate::state::{CannedCycleKind, ProbeKind};

/// A numeric word with its source column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Value {
    /// Numeric value as written (program units until converted).
    pub value: f64,
    /// 1-based byte column of the word letter.
    pub column: usize,
}

/// A recognised G-code and where it appeared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GWord {
    /// Parsed code.
    pub code: GCode,
    /// 1-based byte column of the `G` letter.
    pub column: usize,
    /// Numeric value as written, used only in messages.
    pub value: f64,
}

/// A recognised M-code and where it appeared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MWord {
    /// Parsed code.
    pub code: MCode,
    /// 1-based byte column of the `M` letter.
    pub column: usize,
    /// Numeric value as written, used only in messages.
    pub value: f64,
}

/// The supported G-code commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GCode {
    Rapid,
    Linear,
    ArcCw,
    ArcCcw,
    Probe(ProbeKind),
    Canned(CannedCycleKind),
    CancelCanned,
    Dwell,
    SetCoordinateSystem,
    PlaneXy,
    PlaneZx,
    PlaneYz,
    UnitsInch,
    UnitsMm,
    Home,
    SetHome,
    SecondHome,
    SetSecondHome,
    CutterCancel,
    CutterLeft,
    CutterRight,
    ToolLengthDynamic,
    ToolLengthCancel,
    ScalingSet,
    ScalingCancel,
    MachineCoords,
    Wcs(Wcs),
    PathExact,
    PathContinuous,
    Absolute,
    Incremental,
    FeedInverseTime,
    FeedPerMinute,
    RetractInitial,
    RetractR,
    SetOffsets,
    ClearOffsets,
    SuspendOffsets,
    RestoreOffsets,
}

/// The supported M-code commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MCode {
    ProgramStop,
    OptionalStop,
    ProgramEnd,
    SpindleClockwise,
    SpindleCounterClockwise,
    SpindleStop,
    ToolChange,
    MistOn,
    FloodOn,
    CoolantOff,
    ProgramEndRewind,
}

/// Modal group of a G-code, used to reject two codes that fight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GGroup {
    Motion,
    Plane,
    Distance,
    Units,
    FeedMode,
    Wcs,
    PathControl,
    Retract,
    CutterComp,
    ToolLength,
    Scaling,
    NonModal,
}

/// Modal group of an M-code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MGroup {
    ProgramFlow,
    Spindle,
    ToolChange,
    Coolant,
}

impl GCode {
    /// The modal group this code belongs to.
    pub(crate) const fn group(self) -> GGroup {
        match self {
            Self::Rapid
            | Self::Linear
            | Self::ArcCw
            | Self::ArcCcw
            | Self::Probe(_)
            | Self::Canned(_)
            | Self::CancelCanned => GGroup::Motion,
            Self::PlaneXy | Self::PlaneZx | Self::PlaneYz => GGroup::Plane,
            Self::Absolute | Self::Incremental => GGroup::Distance,
            Self::UnitsInch | Self::UnitsMm => GGroup::Units,
            Self::FeedInverseTime | Self::FeedPerMinute => GGroup::FeedMode,
            Self::Wcs(_) => GGroup::Wcs,
            Self::PathExact | Self::PathContinuous => GGroup::PathControl,
            Self::RetractInitial | Self::RetractR => GGroup::Retract,
            Self::CutterCancel | Self::CutterLeft | Self::CutterRight => GGroup::CutterComp,
            Self::ToolLengthDynamic | Self::ToolLengthCancel => GGroup::ToolLength,
            Self::ScalingSet | Self::ScalingCancel => GGroup::Scaling,
            Self::Dwell
            | Self::SetCoordinateSystem
            | Self::Home
            | Self::SetHome
            | Self::SecondHome
            | Self::SetSecondHome
            | Self::MachineCoords
            | Self::SetOffsets
            | Self::ClearOffsets
            | Self::SuspendOffsets
            | Self::RestoreOffsets => GGroup::NonModal,
        }
    }

    /// Returns `true` for the non-modal actions handled as one exclusive block.
    pub(crate) const fn is_non_modal_action(self) -> bool {
        matches!(
            self,
            Self::Dwell
                | Self::SetCoordinateSystem
                | Self::Home
                | Self::SetHome
                | Self::SecondHome
                | Self::SetSecondHome
                | Self::SetOffsets
                | Self::ClearOffsets
                | Self::SuspendOffsets
                | Self::RestoreOffsets
        )
    }
}

impl MCode {
    /// The modal group this code belongs to.
    pub(crate) const fn group(self) -> MGroup {
        match self {
            Self::ProgramStop | Self::OptionalStop | Self::ProgramEnd | Self::ProgramEndRewind => {
                MGroup::ProgramFlow
            }
            Self::SpindleClockwise | Self::SpindleCounterClockwise | Self::SpindleStop => {
                MGroup::Spindle
            }
            Self::ToolChange => MGroup::ToolChange,
            Self::MistOn | Self::FloodOn | Self::CoolantOff => MGroup::Coolant,
        }
    }
}

/// Words of one block, typed and ready for modal interpretation.
#[derive(Debug, Clone, Default)]
pub(crate) struct Block {
    /// `N` line number.
    pub number: Option<u32>,
    /// G-codes in source order.
    pub g: Vec<GWord>,
    /// M-codes in source order.
    pub m: Vec<MWord>,
    /// X word.
    pub x: Option<Value>,
    /// Y word.
    pub y: Option<Value>,
    /// Z word.
    pub z: Option<Value>,
    /// I word (arc center X offset).
    pub i: Option<Value>,
    /// J word (arc center Y offset).
    pub j: Option<Value>,
    /// K word (arc center Z offset).
    pub k: Option<Value>,
    /// R word (arc radius or cycle clearance).
    pub r: Option<Value>,
    /// P word (dwell, offset index, or scaling factor).
    pub p: Option<Value>,
    /// `D` word (cutter-compensation offset number).
    pub d: Option<Value>,
    /// Q word (peck increment).
    pub q: Option<Value>,
    /// L word (offset operation).
    pub l: Option<Value>,
    /// F word.
    pub f: Option<Value>,
    /// S word.
    pub s: Option<Value>,
    /// T word.
    pub t: Option<Value>,
}

impl Block {
    /// Returns `true` when the block has an axis word.
    pub(crate) fn has_axis_words(&self) -> bool {
        self.x.is_some() || self.y.is_some() || self.z.is_some()
    }

    /// Returns `true` when the block has an I, J, or K word.
    pub(crate) fn has_center_words(&self) -> bool {
        self.i.is_some() || self.j.is_some() || self.k.is_some()
    }

    /// Returns `true` when the block has a word that can drive a canned cycle.
    pub(crate) fn has_cycle_words(&self) -> bool {
        self.has_axis_words() || self.r.is_some() || self.q.is_some() || self.p.is_some()
    }

    /// Converts a lexed line into typed words.
    ///
    /// # Errors
    ///
    /// Returns every word-level error on the line: unknown letters, missing or
    /// duplicate values, unsupported codes, and conflicting modal groups.
    pub(crate) fn parse(line: usize, lexed: &LexedLine) -> Result<Self, Vec<ParseError>> {
        let mut block = Self::default();
        let mut errors = Vec::new();

        for (index, word) in lexed.words.iter().enumerate() {
            match word.letter {
                'N' => {
                    if index != 0 {
                        errors.push(ParseError::new(
                            line,
                            word.column,
                            ParseErrorKind::MisplacedWord,
                            "the line number (N) must be the first word",
                        ));
                    } else {
                        block.number = match line_number(word.value) {
                            Some(number) => Some(number),
                            None => {
                                errors.push(ParseError::new(
                                    line,
                                    word.column,
                                    ParseErrorKind::InvalidParameter,
                                    format!(
                                        "line number N{} must be a non-negative integer",
                                        word.value
                                    ),
                                ));
                                None
                            }
                        };
                    }
                }
                'G' => match parse_g_code(word.value) {
                    Ok(code) => block.g.push(GWord {
                        code,
                        column: word.column,
                        value: word.value,
                    }),
                    Err((kind, message)) => {
                        errors.push(ParseError::new(line, word.column, kind, message));
                    }
                },
                'M' => match parse_m_code(word.value) {
                    Ok(code) => block.m.push(MWord {
                        code,
                        column: word.column,
                        value: word.value,
                    }),
                    Err((kind, message)) => {
                        errors.push(ParseError::new(line, word.column, kind, message));
                    }
                },
                'X' => put(
                    &mut block.x,
                    word.value,
                    word.column,
                    line,
                    'X',
                    &mut errors,
                ),
                'Y' => put(
                    &mut block.y,
                    word.value,
                    word.column,
                    line,
                    'Y',
                    &mut errors,
                ),
                'Z' => put(
                    &mut block.z,
                    word.value,
                    word.column,
                    line,
                    'Z',
                    &mut errors,
                ),
                'I' => put(
                    &mut block.i,
                    word.value,
                    word.column,
                    line,
                    'I',
                    &mut errors,
                ),
                'J' => put(
                    &mut block.j,
                    word.value,
                    word.column,
                    line,
                    'J',
                    &mut errors,
                ),
                'K' => put(
                    &mut block.k,
                    word.value,
                    word.column,
                    line,
                    'K',
                    &mut errors,
                ),
                'R' => put(
                    &mut block.r,
                    word.value,
                    word.column,
                    line,
                    'R',
                    &mut errors,
                ),
                'P' => put(
                    &mut block.p,
                    word.value,
                    word.column,
                    line,
                    'P',
                    &mut errors,
                ),
                'D' => put(
                    &mut block.d,
                    word.value,
                    word.column,
                    line,
                    'D',
                    &mut errors,
                ),
                'Q' => put(
                    &mut block.q,
                    word.value,
                    word.column,
                    line,
                    'Q',
                    &mut errors,
                ),
                'L' => put(
                    &mut block.l,
                    word.value,
                    word.column,
                    line,
                    'L',
                    &mut errors,
                ),
                'F' => put(
                    &mut block.f,
                    word.value,
                    word.column,
                    line,
                    'F',
                    &mut errors,
                ),
                'S' => put(
                    &mut block.s,
                    word.value,
                    word.column,
                    line,
                    'S',
                    &mut errors,
                ),
                'T' => put(
                    &mut block.t,
                    word.value,
                    word.column,
                    line,
                    'T',
                    &mut errors,
                ),
                'O' => errors.push(ParseError::new(
                    line,
                    word.column,
                    ParseErrorKind::UnsupportedWord,
                    "O-words (program or subprogram numbers) are not supported",
                )),
                'A' | 'B' | 'C' | 'U' | 'V' | 'W' => errors.push(ParseError::new(
                    line,
                    word.column,
                    ParseErrorKind::UnsupportedWord,
                    format!(
                        "the {} axis is not supported by this 3-axis parser",
                        word.letter
                    ),
                )),
                other => errors.push(ParseError::new(
                    line,
                    word.column,
                    ParseErrorKind::UnknownWord,
                    format!("'{other}' is not a valid G-code word"),
                )),
            }
        }

        errors.extend(check_g_groups(line, &block.g));
        errors.extend(check_m_groups(line, &block.m));

        if errors.is_empty() {
            Ok(block)
        } else {
            Err(errors)
        }
    }
}

/// Stores a numeric word, reporting a duplicate instead of overwriting.
fn put(
    slot: &mut Option<Value>,
    value: f64,
    column: usize,
    line: usize,
    letter: char,
    errors: &mut Vec<ParseError>,
) {
    if slot.is_some() {
        errors.push(ParseError::new(
            line,
            column,
            ParseErrorKind::DuplicateWord,
            format!("'{letter}' appears more than once in this block"),
        ));
    } else {
        *slot = Some(Value { value, column });
    }
}

/// Parses an `N` value as an unsigned 32-bit line number.
fn line_number(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }
    if math::abs(value.fract()) > f64::EPSILON {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(value as u32)
}

/// Reports two G-codes in the same modal group.
fn check_g_groups(line: usize, words: &[GWord]) -> Vec<ParseError> {
    let mut errors = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if let Some(first) = words[..index]
            .iter()
            .find(|first| first.code.group() == word.code.group())
        {
            errors.push(ParseError::new(
                line,
                word.column,
                ParseErrorKind::ConflictingCodes,
                format!(
                    "G{} and G{} are in the same modal group; only one may be used per block",
                    trim_number(first.value),
                    trim_number(word.value)
                ),
            ));
        }
    }
    errors
}

/// Reports two M-codes in the same modal group.
fn check_m_groups(line: usize, words: &[MWord]) -> Vec<ParseError> {
    let mut errors = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if let Some(first) = words[..index]
            .iter()
            .find(|first| first.code.group() == word.code.group())
        {
            errors.push(ParseError::new(
                line,
                word.column,
                ParseErrorKind::ConflictingCodes,
                format!(
                    "M{} and M{} are in the same modal group; only one may be used per block",
                    trim_number(first.value),
                    trim_number(word.value)
                ),
            ));
        }
    }
    errors
}

/// Formats a code value without a trailing `.0`.
fn trim_number(value: f64) -> String {
    format!("{value}")
}

/// Parses a G-code value into a supported command.
///
/// Values must be exact tenths so that `G38.25` and `G1.04` cannot round into
/// a different command.
fn parse_g_code(value: f64) -> Result<GCode, (ParseErrorKind, String)> {
    let label = format!("G{}", trim_number(value));
    if !value.is_finite() || value < 0.0 {
        return Err((
            ParseErrorKind::InvalidParameter,
            format!("{label} is not a valid G-code"),
        ));
    }
    let scaled = value * 10.0;
    let rounded = math::round(scaled);
    if !rounded.is_finite()
        || math::abs(scaled - rounded) > 1.0e-6
        || !(0.0..=1_000_000.0).contains(&rounded)
    {
        return Err((
            ParseErrorKind::UnknownCode,
            format!("{label} is not a supported G-code"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let key = rounded as u32;
    // Keys are tenths of the G number: G1 -> 10, G38.2 -> 382.
    let code = match key {
        0 => GCode::Rapid,
        10 => GCode::Linear,
        20 => GCode::ArcCw,
        30 => GCode::ArcCcw,
        40 => GCode::Dwell,
        100 => GCode::SetCoordinateSystem,
        170 => GCode::PlaneXy,
        180 => GCode::PlaneZx,
        190 => GCode::PlaneYz,
        200 => GCode::UnitsInch,
        210 => GCode::UnitsMm,
        280 => GCode::Home,
        300 => GCode::SecondHome,
        400 => GCode::CutterCancel,
        410 => GCode::CutterLeft,
        420 => GCode::CutterRight,
        490 => GCode::ToolLengthCancel,
        500 => GCode::ScalingCancel,
        510 => GCode::ScalingSet,
        530 => GCode::MachineCoords,
        540 => GCode::Wcs(Wcs::G54),
        550 => GCode::Wcs(Wcs::G55),
        560 => GCode::Wcs(Wcs::G56),
        570 => GCode::Wcs(Wcs::G57),
        580 => GCode::Wcs(Wcs::G58),
        590 => GCode::Wcs(Wcs::G59),
        610 => GCode::PathExact,
        640 => GCode::PathContinuous,
        730 => GCode::Canned(CannedCycleKind::ChipBreak),
        800 => GCode::CancelCanned,
        810 => GCode::Canned(CannedCycleKind::Drill),
        820 => GCode::Canned(CannedCycleKind::DrillDwell),
        830 => GCode::Canned(CannedCycleKind::PeckDrill),
        850 => GCode::Canned(CannedCycleKind::BoreFeedOut),
        860 => GCode::Canned(CannedCycleKind::BoreStop),
        890 => GCode::Canned(CannedCycleKind::BoreDwell),
        900 => GCode::Absolute,
        910 => GCode::Incremental,
        920 => GCode::SetOffsets,
        922 => GCode::SuspendOffsets,
        923 => GCode::RestoreOffsets,
        930 => GCode::FeedInverseTime,
        940 => GCode::FeedPerMinute,
        980 => GCode::RetractInitial,
        990 => GCode::RetractR,
        281 => GCode::SetHome,
        301 => GCode::SetSecondHome,
        382 => GCode::Probe(ProbeKind::Toward),
        383 => GCode::Probe(ProbeKind::TowardNoError),
        384 => GCode::Probe(ProbeKind::Away),
        385 => GCode::Probe(ProbeKind::AwayNoError),
        431 => GCode::ToolLengthDynamic,
        591 => GCode::Wcs(Wcs::G59_1),
        592 => GCode::Wcs(Wcs::G59_2),
        593 => GCode::Wcs(Wcs::G59_3),
        921 => GCode::ClearOffsets,
        282 | 302 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} homing-cycle variants are not supported"),
            ));
        }
        430 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                "G43 (tool length compensation with H) is not supported; use G43.1".to_string(),
            ));
        }
        700 | 710 | 740 | 760 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} is a lathe cycle and is out of scope"),
            ));
        }
        840 | 870 | 880 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} tapping or back-boring is not supported"),
            ));
        }
        950 | 960 | 970 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} constant-surface-speed is a lathe feature"),
            ));
        }
        680 | 690 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!(
                    "{label} coordinate rotation is not a grblHAL G-code; rotation is a per-WCS setting"
                ),
            ));
        }
        901 | 911 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} absolute arc centers are not supported; use I/J/K offsets"),
            ));
        }
        _ => {
            return Err((
                ParseErrorKind::UnknownCode,
                format!("{label} is not a supported G-code"),
            ));
        }
    };
    Ok(code)
}

/// Parses an M-code value into a supported command.
fn parse_m_code(value: f64) -> Result<MCode, (ParseErrorKind, String)> {
    let label = format!("M{}", trim_number(value));
    if !value.is_finite() || value < 0.0 {
        return Err((
            ParseErrorKind::InvalidParameter,
            format!("{label} is not a valid M-code"),
        ));
    }
    let rounded = math::round(value);
    if !rounded.is_finite() || math::abs(value - rounded) > 1.0e-9 || rounded > 1_000.0 {
        return Err((
            ParseErrorKind::UnknownCode,
            format!("{label} is not a supported M-code"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let key = rounded as u32;
    let code = match key {
        0 => MCode::ProgramStop,
        1 => MCode::OptionalStop,
        2 => MCode::ProgramEnd,
        3 => MCode::SpindleClockwise,
        4 => MCode::SpindleCounterClockwise,
        5 => MCode::SpindleStop,
        6 => MCode::ToolChange,
        7 => MCode::MistOn,
        8 => MCode::FloodOn,
        9 => MCode::CoolantOff,
        30 => MCode::ProgramEndRewind,
        10 | 11 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} clamp control is not supported"),
            ));
        }
        19 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} spindle orientation is not supported"),
            ));
        }
        48 | 49 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} override control is not supported"),
            ));
        }
        60 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} pallet change is not supported"),
            ));
        }
        62..=68 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} digital I/O control is not supported"),
            ));
        }
        98 | 99 => {
            return Err((
                ParseErrorKind::UnsupportedCode,
                format!("{label} subprogram calls are not supported"),
            ));
        }
        _ => {
            return Err((
                ParseErrorKind::UnknownCode,
                format!("{label} is not a supported M-code"),
            ));
        }
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_codes_map_to_commands() {
        assert_eq!(parse_g_code(0.0), Ok(GCode::Rapid));
        assert_eq!(parse_g_code(1.0), Ok(GCode::Linear));
        assert_eq!(parse_g_code(38.2), Ok(GCode::Probe(ProbeKind::Toward)));
        assert_eq!(parse_g_code(59.3), Ok(GCode::Wcs(Wcs::G59_3)));
        assert_eq!(
            parse_g_code(83.0),
            Ok(GCode::Canned(CannedCycleKind::PeckDrill))
        );
        assert_eq!(parse_g_code(41.0), Ok(GCode::CutterLeft));
        assert_eq!(parse_g_code(42.0), Ok(GCode::CutterRight));
        assert_eq!(parse_g_code(92.1), Ok(GCode::ClearOffsets));
        assert_eq!(parse_m_code(6.0), Ok(MCode::ToolChange));
        assert_eq!(parse_m_code(30.0), Ok(MCode::ProgramEndRewind));
    }

    #[test]
    fn non_exact_values_are_not_rounded_into_another_code() {
        assert!(parse_g_code(1.04).is_err());
        assert!(parse_g_code(38.25).is_err());
        assert!(parse_g_code(-1.0).is_err());
        assert!(parse_m_code(3.5).is_err());
    }

    #[test]
    fn known_unsupported_codes_have_targeted_messages() {
        let (kind, message) = parse_g_code(43.0).expect_err("tool length");
        assert_eq!(kind, ParseErrorKind::UnsupportedCode);
        assert!(message.contains("tool length"));

        let (kind, message) = parse_g_code(84.0).expect_err("tapping");
        assert_eq!(kind, ParseErrorKind::UnsupportedCode);
        assert!(message.contains("tapping"));
    }

    #[test]
    fn group_conflicts_are_reported() {
        let line = "G0 G1 X0";
        let lexed = crate::lexer::lex_line(1, line, 64).expect("lex");
        let errors = Block::parse(1, &lexed).expect_err("conflict");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == ParseErrorKind::ConflictingCodes)
        );

        let lexed = crate::lexer::lex_line(1, "M3 M4", 64).expect("lex");
        let errors = Block::parse(1, &lexed).expect_err("conflict");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == ParseErrorKind::ConflictingCodes)
        );
    }

    #[test]
    fn duplicate_axis_words_are_reported() {
        let lexed = crate::lexer::lex_line(1, "X1 X2", 64).expect("lex");
        let errors = Block::parse(1, &lexed).expect_err("duplicate");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == ParseErrorKind::DuplicateWord)
        );
    }

    #[test]
    fn line_number_must_come_first() {
        let lexed = crate::lexer::lex_line(1, "G0 N10 X0", 64).expect("lex");
        let errors = Block::parse(1, &lexed).expect_err("misplaced");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == ParseErrorKind::MisplacedWord)
        );
    }

    #[test]
    fn unsupported_axes_are_rejected() {
        let lexed = crate::lexer::lex_line(1, "G0 A10", 64).expect("lex");
        let errors = Block::parse(1, &lexed).expect_err("axis");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == ParseErrorKind::UnsupportedWord)
        );
    }
}
