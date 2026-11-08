//! Validation behavior: unsupported codes and malformed input must produce
//! structured errors with line numbers, never panics or partial blocks.

use nc_gcode::{ParseErrorKind, ParserConfig, parse_program, parse_program_with};
use proptest::prelude::*;

fn kind(text: &str) -> ParseErrorKind {
    parse_program(text)
        .expect_err("expected an error")
        .first()
        .expect("error")
        .kind
}

#[test]
fn unsupported_and_unknown_codes_are_distinct() {
    assert!(parse_program("G41 D1").is_ok());
    assert_eq!(kind("G43 H1"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("G90.1"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("G84 X0 Z-1 R1 F100"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("M98 P1"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("G123"), ParseErrorKind::UnknownCode);
    assert_eq!(kind("M77"), ParseErrorKind::UnknownCode);
    assert_eq!(kind("G17.5"), ParseErrorKind::UnknownCode);
    assert_eq!(kind("G38.25"), ParseErrorKind::UnknownCode);
}

#[test]
fn malformed_words_are_rejected() {
    assert_eq!(kind("G1 X"), ParseErrorKind::MissingValue);
    assert_eq!(kind("G1 X1.2.3"), ParseErrorKind::InvalidNumber);
    assert_eq!(kind("G1 X1 X2 F100"), ParseErrorKind::DuplicateWord);
    assert_eq!(kind("G0 X0 A0"), ParseErrorKind::UnsupportedWord);
    assert_eq!(kind("O1000"), ParseErrorKind::UnsupportedWord);
    assert_eq!(kind("N10 G0 N20"), ParseErrorKind::MisplacedWord);
    assert_eq!(kind("G0 X0 P2"), ParseErrorKind::UnusedWord);
    assert_eq!(kind("G64 P1"), ParseErrorKind::UnusedWord);
    assert_eq!(kind("G0 @"), ParseErrorKind::UnexpectedCharacter);
}

#[test]
fn conflicting_codes_are_rejected() {
    assert_eq!(kind("G0 G1 X0"), ParseErrorKind::ConflictingCodes);
    assert_eq!(kind("M3 M4"), ParseErrorKind::ConflictingCodes);
    assert_eq!(kind("M0 M1"), ParseErrorKind::ConflictingCodes);
    assert_eq!(kind("M0 M30"), ParseErrorKind::ConflictingCodes);
    assert_eq!(kind("G4 P1 G0 X0"), ParseErrorKind::ConflictingCommands);
    assert_eq!(kind("G92 X0 G0 Y1"), ParseErrorKind::ConflictingCommands);
}

#[test]
fn motion_and_parameter_errors_are_specific() {
    assert_eq!(kind("G1 X1"), ParseErrorKind::FeedRateUndefined);
    assert_eq!(kind("G1 X1 F-1"), ParseErrorKind::NegativeValue);
    assert_eq!(kind("G4 P-1"), ParseErrorKind::NegativeValue);
    assert_eq!(kind("S-1"), ParseErrorKind::NegativeValue);
    assert_eq!(kind("S2000000"), ParseErrorKind::ValueOutOfRange);
    assert_eq!(kind("T4294967296"), ParseErrorKind::ToolNumberOutOfRange);
    assert_eq!(kind("G0 X99999999"), ParseErrorKind::ValueOutOfRange);
    assert_eq!(kind("G4"), ParseErrorKind::MissingParameter);
    assert_eq!(kind("G92"), ParseErrorKind::MissingParameter);
    assert_eq!(kind("G10 L2 X0"), ParseErrorKind::MissingParameter);
    assert_eq!(kind("G10 L9 P1 X0"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("G53 G0"), ParseErrorKind::MissingParameter);
    assert_eq!(
        kind("G53 G2 X0 Y0 I1 J0 F100"),
        ParseErrorKind::UnsupportedCode
    );
    assert_eq!(kind("G1 X1 Q2 F100"), ParseErrorKind::UnusedWord);
}

#[test]
fn flow_control_is_deferred_not_silently_accepted() {
    assert_eq!(kind("IF [#1 GT 2]"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(
        kind("WHILE [#1 LT 2]"),
        ParseErrorKind::UnsupportedExpression
    );
    assert_eq!(kind("GOTO 10"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(kind("N10 GOTO 20"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(kind("DO 1"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(kind("ENDIF"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(kind("SUB 100"), ParseErrorKind::UnsupportedExpression);
    assert_eq!(kind("M98 P1"), ParseErrorKind::UnsupportedCode);
    assert_eq!(kind("O1000"), ParseErrorKind::UnsupportedWord);
}

#[test]
fn errors_carry_the_source_line() {
    let errors = parse_program("G0 X1\nG123\nG0 X2\n").expect_err("errors");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 2);
    assert_eq!(errors[0].kind, ParseErrorKind::UnknownCode);

    let errors = parse_program("G123\nG0 X\n").expect_err("errors");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].line, 1);
    assert_eq!(errors[1].line, 2);
}

#[test]
fn error_collection_stops_at_the_configured_limit() {
    let config = ParserConfig {
        max_errors: 2,
        ..ParserConfig::default()
    };
    let errors = parse_program_with("G123\nG123\nG123\nG123\n", &config).expect_err("errors");
    assert_eq!(errors.len(), 3);
    assert_eq!(
        errors.last().expect("last").kind,
        ParseErrorKind::TooManyErrors
    );
}

#[test]
fn overlong_lines_are_rejected_without_hanging() {
    let config = ParserConfig {
        max_line_bytes: 16,
        ..ParserConfig::default()
    };
    let errors = parse_program_with("G0 X1 Y1 Z1 X2 Y2 Z2\n", &config).expect_err("long");
    assert_eq!(errors[0].kind, ParseErrorKind::LineTooLong);
}

#[test]
fn too_many_words_are_rejected() {
    let config = ParserConfig {
        max_words: 3,
        ..ParserConfig::default()
    };
    let errors = parse_program_with("G0 X1 Y1 Z1\n", &config).expect_err("words");
    assert_eq!(errors[0].kind, ParseErrorKind::TooManyWords);
}

#[test]
fn unsupported_codes_do_not_emit_geometry() {
    let errors = parse_program("G0 X1\nM98 P1\n").expect_err("unsupported");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
    // The program is not returned at all when any line fails.
}

#[test]
fn malformed_programs_never_panic() {
    let repeated = "G0 X1 Y2 Z3\n".repeat(10);
    let inputs = [
        "",
        "\0",
        "G",
        "G0 X",
        "(((((",
        ";;;;",
        "****",
        "G0 X1e999999",
        repeated.as_str(),
        "N999999999999999999999999",
        "G0 X1*ZZ",
        "G0 X1 Y2 Z-3 I4 J5 K6 R7 P8 Q9 L10",
    ];
    for input in inputs {
        let _ = parse_program(input);
    }
}

proptest! {
    #[test]
    fn arbitrary_utf8_text_never_panics(text in ".*") {
        let _ = parse_program(&text);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let text = String::from_utf8_lossy(&bytes);
        let _ = parse_program(&text);
    }

    #[test]
    fn gcode_like_lines_never_panic(
        lines in proptest::collection::vec(
            "[A-Za-z0-9 .+\\-*/();#\\[\\]]{0,80}",
            0..20
        )
    ) {
        let text = lines.join("\n");
        let _ = parse_program(&text);
    }
}

#[test]
fn an_arc_word_outside_the_plane_is_unused() {
    let errors = parse_program("G0 X0 Y0\nG2 X10 Y0 I5 J0 K1 F100\n").expect_err("k");
    assert_eq!(errors[0].kind, ParseErrorKind::UnusedWord);
    assert_eq!(errors[0].column, 17);
}

#[test]
fn non_finite_feed_is_rejected() {
    let errors = parse_program("G1 X1 F1e300\n").expect_err("feed");
    // `1e300` lexes as X1 followed by E300, which is not a valid word.
    assert_eq!(errors[0].kind, ParseErrorKind::UnknownWord);

    let huge = format!("G1 X1 F{}\n", "9".repeat(400));
    let errors = parse_program(&huge).expect_err("huge feed");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidNumber);
}

#[test]
fn per_wcs_rotation_through_g10_is_rejected() {
    let errors = parse_program("G10 L2 P1 X0 Y0 R45\n").expect_err("rotation");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
    assert!(errors[0].message.contains("rotation"));
}
