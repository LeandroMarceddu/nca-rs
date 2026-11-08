//! Makera Z1 / Smoothieware-derived dialect behavior.
//!
//! Makera firmware is based on Smoothieware, but adds auxiliary-output codes.
//! These tests pin the codes this viewer understands, the dialect gate, and
//! the fact that a Makera `G28` is not turned into an invented home target.

use nc_core::Dialect;
use nc_gcode::{ParseErrorKind, ParserConfig, parse_program_lossy, parse_program_with};

fn makera() -> ParserConfig {
    ParserConfig::default().with_dialect(Dialect::Makera)
}

#[test]
fn makera_accepts_smoothieware_and_auxiliary_codes() {
    let source =
        "G21 G90\nT1 M6\nM400\nM851 S100\nS12000 M3\nG0 X0 Y0\nG1 Z-1 F200\nM852\nM400\nM30\n";
    let program = parse_program_with(source, &makera()).expect("makera program");
    assert_eq!(program.stats.tools, vec![1]);
    assert_eq!(program.motions.len(), 1);
    assert!(program.skipped_blocks.is_empty());
    assert_eq!(program.geometry_uncertain_after(), None);
}

#[test]
fn m851_s_is_not_spindle_rpm() {
    let source = "M3 S8000\nM851 S100\nG1 X10 F200\n";
    let program = parse_program_with(source, &makera()).expect("makera program");
    assert_eq!(program.motions[0].spindle_rpm, Some(8000.0));
}

#[test]
fn generic_dialect_rejects_the_makera_codes() {
    let source = "G21 G90\nM851 S100\nG1 X10 F200\n";
    let errors = parse_program_with(source, &ParserConfig::default()).expect_err("generic");
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ParseErrorKind::UnknownCode)
    );

    let lossy = parse_program_lossy(source, &ParserConfig::default());
    assert_eq!(lossy.skipped_blocks.len(), 1);
    assert_eq!(lossy.skipped_blocks[0].line, 2);
    // A skipped auxiliary output does not make the motion geometry uncertain.
    assert_eq!(lossy.program.geometry_uncertain_after(), None);
}

#[test]
fn makera_g28_is_a_clearance_move_with_no_invented_target() {
    let source = "G21 G90\nG0 X10 Y10 Z5\nG28\nM30\n";
    let program = parse_program_with(source, &makera()).expect("makera program");
    assert_eq!(program.motions.len(), 1);
    assert_eq!(program.motions[0].end_mm, [10.0, 10.0, 5.0]);
    assert!(
        !program
            .warnings
            .iter()
            .any(|warning| { matches!(warning.kind, nc_gcode::WarningKind::AssumedHomePosition) })
    );
}

#[test]
fn makera_g28_and_g30_consume_axis_words_without_motion() {
    let source = "G21 G90\nG0 X10 Y10 Z5\nG28 X0 Y0 Z0\nG30 X20 Y20 Z20\nM30\n";
    let program = parse_program_with(source, &makera()).expect("makera clearance moves");
    assert_eq!(program.motions.len(), 1);
    assert_eq!(program.motions[0].end_mm, [10.0, 10.0, 5.0]);
    assert!(program.skipped_blocks.is_empty());
    assert!(
        !program
            .warnings
            .iter()
            .any(|warning| { matches!(warning.kind, nc_gcode::WarningKind::AssumedHomePosition) })
    );
}
