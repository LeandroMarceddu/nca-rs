//! Lossy parsing: skipped blocks, preserved modal state, uncertainty, and
//! bounded input.

use nc_gcode::{
    ParseErrorKind, ParserConfig, WarningKind, parse_program_lossy, parse_program_with,
};
use proptest::prelude::*;

/// Counts source lines the same way the parser's iterator does.
fn source_line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut count = 1usize;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' || bytes[index] == b'\r' {
            count += 1;
            if index + 1 < bytes.len()
                && matches!(
                    (bytes[index], bytes[index + 1]),
                    (b'\r', b'\n') | (b'\n', b'\r')
                )
            {
                index += 1;
            }
        }
        index += 1;
    }
    count
}

#[test]
fn a_bad_block_is_skipped_and_modal_state_is_kept() {
    let result = parse_program_lossy(
        "G21 G90\nG0 X10\nG1 X20 Q2\nG1 X30 F100\n",
        &ParserConfig::default(),
    );
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::UnusedWord);
    assert_eq!(result.errors[0].line, 3);
    assert_eq!(result.skipped_blocks.len(), 1);
    assert_eq!(result.skipped_blocks[0].line, 3);
    assert_eq!(result.skipped_blocks[0].reason, "unused_word");
    assert_eq!(result.skipped_blocks[0].source, "G1 X20 Q2");
    assert_eq!(result.program.motions.len(), 2);
    assert_eq!(result.program.motions[1].start_mm, [10.0, 0.0, 0.0]);
    assert_eq!(result.program.motions[1].end_mm, [30.0, 0.0, 0.0]);
    assert_eq!(result.program.state.position, [30.0, 0.0, 0.0]);
    assert_eq!(result.trusted_geometry_until, None);
    // The skipped line stays in the program list with its source.
    assert_eq!(result.program.lines[2].line, 3);
    assert_eq!(result.program.lines[2].source, "G1 X20 Q2");
    assert!(result.program.lines[2].segments.is_empty());
}

#[test]
fn lossy_keeps_feedless_cuts_with_unknown_feed() {
    let text = "G21 G90\nG0 Z5\nG1 Z-1\nG1 X10\n";
    let strict = parse_program_with(text, &ParserConfig::default()).expect_err("strict feed");
    assert_eq!(strict[0].kind, ParseErrorKind::FeedRateUndefined);
    assert_eq!(strict[0].line, 3);

    let result = parse_program_lossy(text, &ParserConfig::default());
    assert!(result.errors.is_empty());
    assert!(result.skipped_blocks.is_empty());
    assert!(result.program.skipped_blocks().is_empty());
    assert_eq!(result.trusted_geometry_until, None);
    assert_eq!(result.program.motions.len(), 3);
    assert_eq!(result.program.toolpath.len(), 3);
    assert_eq!(result.program.motions[1].kind, nc_gcode::MoveKind::Plunge);
    assert_eq!(result.program.motions[1].feed_mm_min, None);
    assert_eq!(result.program.motions[2].kind, nc_gcode::MoveKind::Feed);
    assert_eq!(result.program.motions[2].feed_mm_min, None);
    assert_eq!(result.program.motions[2].end_mm, [10.0, 0.0, -1.0]);
    assert!(
        result
            .program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::MissingFeed)
    );
}

#[test]
fn strict_programs_have_no_skipped_blocks() {
    let program = nc_gcode::parse_program("G0 X1\nG1 X2 F100\n").expect("strict");
    assert!(program.skipped_blocks().is_empty());
    assert!(program.skipped_blocks.is_empty());
}

#[test]
fn skipped_blocks_are_stored_on_the_program() {
    let result = parse_program_lossy("G0 X1\nG123\nM98 P1\nG0 X3\n", &ParserConfig::default());
    assert_eq!(result.errors.len(), 2);
    assert_eq!(result.skipped_blocks.len(), 2);
    assert_eq!(result.skipped_blocks[0].line, 2);
    assert_eq!(result.skipped_blocks[0].source, "G123");
    assert_eq!(result.skipped_blocks[0].reason, "unknown_code");
    assert_eq!(result.skipped_blocks[1].line, 3);
    assert_eq!(result.skipped_blocks[1].source, "M98 P1");
    assert_eq!(result.skipped_blocks[1].reason, "unsupported_code");

    assert_eq!(result.program.skipped_blocks.len(), 2);
    assert_eq!(
        result.program.skipped_blocks(),
        result.skipped_blocks.as_slice()
    );
    assert_eq!(result.program.skipped_blocks()[1].line, 3);
    assert_eq!(result.trusted_geometry_until, Some(3));
}

#[test]
fn unknown_codes_are_diagnostics_not_geometry_stops() {
    let result = parse_program_lossy("G0 X1\nG123\nG0 X2\n", &ParserConfig::default());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::UnknownCode);
    assert_eq!(result.program.motions.len(), 2);
    assert_eq!(result.trusted_geometry_until, None);
    assert!(result.skipped_blocks.iter().all(|block| block.line == 2));
}

#[test]
fn m98_stops_new_geometry() {
    let result = parse_program_lossy("G0 X1\nM98 P1000\nG0 X2\n", &ParserConfig::default());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::UnsupportedCode);
    assert_eq!(result.program.motions.len(), 1);
    assert_eq!(result.program.motions[0].end_mm, [1.0, 0.0, 0.0]);
    assert_eq!(result.trusted_geometry_until, Some(2));
    assert_eq!(result.program.geometry_uncertain_after(), Some(2));
}

#[test]
fn flow_control_stops_new_geometry() {
    let result = parse_program_lossy("G0 X1\nIF [#1 GT 2]\nG0 X2\n", &ParserConfig::default());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::UnsupportedExpression);
    assert_eq!(result.program.motions.len(), 1);
    assert_eq!(result.trusted_geometry_until, Some(2));
}

#[test]
fn macro_calls_stop_new_geometry() {
    let result = parse_program_lossy("G0 X1\nG65 P1000\nG0 X2\n", &ParserConfig::default());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::UnknownCode);
    assert_eq!(result.program.motions.len(), 1);
    assert_eq!(result.trusted_geometry_until, Some(2));
}

#[test]
fn cutter_compensation_keeps_geometry_and_trusted_until_stays_none() {
    let result = parse_program_lossy(
        "G0 X1\nG41 D1\nG0 X2\nG42 D1\nG0 X3\nG40\nG0 X4\n",
        &ParserConfig::default(),
    );
    assert!(result.errors.is_empty());
    assert!(result.skipped_blocks.is_empty());
    assert_eq!(result.trusted_geometry_until, None);
    assert_eq!(result.program.geometry_uncertain_after(), None);
    assert_eq!(result.program.motions.len(), 4);
    let compensation: Vec<nc_gcode::CutterComp> = result
        .program
        .motions
        .iter()
        .map(|motion| result.program.modal(motion.modal).cutter_comp)
        .collect();
    assert_eq!(
        compensation,
        vec![
            nc_gcode::CutterComp::Off,
            nc_gcode::CutterComp::Left,
            nc_gcode::CutterComp::Right,
            nc_gcode::CutterComp::Off,
        ]
    );
    assert_eq!(result.program.motions[3].end_mm, [4.0, 0.0, 0.0]);
    assert!(
        result
            .program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::CutterCompNotApplied)
    );
}

#[test]
fn overlong_lines_are_skipped_cleanly() {
    let config = ParserConfig {
        max_line_bytes: 16,
        ..ParserConfig::default()
    };
    let long = format!("G0 X{}", "1".repeat(64));
    let text = format!("G0 X1\n{long}\nG0 X3\n");
    let result = parse_program_lossy(&text, &config);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::LineTooLong);
    assert_eq!(result.errors[0].line, 2);
    assert_eq!(result.skipped_blocks.len(), 1);
    assert_eq!(result.program.motions.len(), 2);
    assert_eq!(result.program.motions[1].end_mm, [3.0, 0.0, 0.0]);
    assert!(result.errors.iter().all(|error| error.line >= 1));
    assert!(
        result
            .errors
            .iter()
            .all(|error| error.line <= source_line_count(&text))
    );
}

#[test]
fn oversized_files_are_refused() {
    let config = ParserConfig {
        max_file_bytes: 32,
        ..ParserConfig::default()
    };
    let text = "G0 X1\n".repeat(100);
    let errors = parse_program_with(&text, &config).expect_err("file limit");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ParseErrorKind::FileTooLarge);
    assert_eq!(errors[0].line, 1);

    let result = parse_program_lossy(&text, &config);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ParseErrorKind::FileTooLarge);
    assert!(result.program.motions.is_empty());
    assert_eq!(result.trusted_geometry_until, Some(1));
}

#[test]
fn too_many_errors_stop_the_lossy_scan() {
    let config = ParserConfig {
        max_errors: 2,
        ..ParserConfig::default()
    };
    let result = parse_program_lossy("G123\nG123\nG123\nG123\n", &config);
    assert_eq!(result.errors.len(), 3);
    assert_eq!(
        result.errors.last().expect("last").kind,
        ParseErrorKind::TooManyErrors
    );
    assert_eq!(result.trusted_geometry_until, Some(2));
    assert!(result.program.motions.is_empty());
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic_and_lines_stay_in_range(
        bytes in proptest::collection::vec(any::<u8>(), 0..1024)
    ) {
        let text = String::from_utf8_lossy(&bytes);
        let result = parse_program_lossy(&text, &ParserConfig::default());
        let line_count = source_line_count(&text);
        for error in &result.errors {
            prop_assert!(error.line >= 1);
            prop_assert!(error.line <= line_count);
        }
        for block in &result.skipped_blocks {
            prop_assert!(block.line >= 1);
            prop_assert!(block.line <= line_count);
        }
        for motion in &result.program.motions {
            prop_assert!(motion.line >= 1);
            prop_assert!(motion.line <= line_count);
        }
        for comment in &result.program.comments {
            prop_assert!(comment.line >= 1);
            prop_assert!(comment.line <= line_count);
        }
    }

    #[test]
    fn parsing_twice_is_deterministic(
        lines in proptest::collection::vec(
            "[A-Za-z0-9 .+\\-*/();#\\[\\]]{0,60}",
            0..16
        )
    ) {
        let text = lines.join("\n");
        let config = ParserConfig::default();
        let first = parse_program_lossy(&text, &config);
        let second = parse_program_lossy(&text, &config);
        prop_assert_eq!(&first.errors, &second.errors);
        prop_assert_eq!(&first.skipped_blocks, &second.skipped_blocks);
        prop_assert_eq!(&first.program.motions, &second.program.motions);
        prop_assert_eq!(&first.program.annotations, &second.program.annotations);
        prop_assert_eq!(&first.program.comments, &second.program.comments);
        prop_assert_eq!(&first.program.tool_hints, &second.program.tool_hints);
        prop_assert_eq!(&first.program.modal_table, &second.program.modal_table);
        prop_assert_eq!(
            first.program.geometry_uncertain_after_line,
            second.program.geometry_uncertain_after_line
        );
    }
}

#[test]
fn empty_input_has_no_diagnostics() {
    let result = parse_program_lossy("", &ParserConfig::default());
    assert!(result.errors.is_empty());
    assert!(result.skipped_blocks.is_empty());
    assert!(result.program.motions.is_empty());
    assert_eq!(result.trusted_geometry_until, None);
}
