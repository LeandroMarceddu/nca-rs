//! Real-file regression: a 119k-line, CRLF, Fusion-style program.
//!
//! The fixture is read-only and lives in the repository. This test is part of
//! the default suite unless the debug parse becomes slow; the printed duration
//! makes a regression visible.

use std::time::Instant;

use nc_core::ToolShape;
use nc_gcode::{MoveKind, ParserConfig, parse_program_lossy};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/real/box54-lid-50cm/source.nc"
);

fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .expect("fixture line")
}

#[test]
fn crlf_diagnostics_use_one_based_source_lines() {
    let text = "G21\r\nG0 X1\r\nG1 X2\r\nG1 X3 F100\r\n";
    let result = parse_program_lossy(text, &ParserConfig::default());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].line, 3);
    assert_eq!(
        result.errors[0].kind,
        nc_gcode::ParseErrorKind::FeedRateUndefined
    );
    assert_eq!(result.skipped_blocks[0].line, 3);
    assert_eq!(result.skipped_blocks[0].source, "G1 X2");
    assert_eq!(result.program.motions.len(), 2);
    assert!(
        result
            .program
            .lines
            .iter()
            .all(|line| !line.source.contains('\r'))
    );
}

#[test]
fn box54_lid_parses_cleanly_with_crlf_and_header_hints() {
    let text = std::fs::read_to_string(FIXTURE).expect("read fixture");
    assert!(text.contains("\r\n"), "fixture must keep CRLF line endings");

    let started = Instant::now();
    let result = parse_program_lossy(&text, &ParserConfig::default());
    let elapsed = started.elapsed();

    println!(
        "real fixture: {} bytes, {} lines, {} records, {} expanded sub-moves, {} errors, {} skipped, trusted={:?}, {:?}",
        text.len(),
        result.program.lines.len(),
        result.program.motions.len(),
        result.program.expanded_motions().count(),
        result.errors.len(),
        result.skipped_blocks.len(),
        result.trusted_geometry_until,
        elapsed
    );
    for hint in &result.program.tool_hints {
        println!(
            "  T{} line {} diameter {:?} mm shape {:?} flutes {:?}",
            hint.tool, hint.line, hint.diameter_mm, hint.shape, hint.flutes
        );
    }
    let mut counts = [0usize; 13];
    for motion in &result.program.motions {
        counts[motion.kind as usize] += 1;
    }
    for kind in MoveKind::ALL {
        let count = counts[kind as usize];
        if count > 0 {
            println!("  {:<9} {count}", kind.label());
        }
    }
    println!("  bounds {:?}", result.program.bounds);

    assert!(
        result.errors.is_empty(),
        "unexpected diagnostics: {:?}",
        &result.errors[..result.errors.len().min(5)]
    );
    assert!(result.skipped_blocks.is_empty());
    assert_eq!(result.trusted_geometry_until, None);
    assert!(result.program.lines.len() > 100_000);
    assert!(!result.program.motions.is_empty());
    assert!(!result.program.toolpath.is_empty());
    assert!(counts[MoveKind::ArcCcw as usize] > 0);
    assert!(counts[MoveKind::Drill as usize] > 0);

    // Arc bulges can exceed the explicit coordinate words slightly; this is a
    // broad sanity envelope around the fixture's documented extremes.
    let bounds = result.program.bounds;
    assert!(bounds.min().x >= -10.0 && bounds.min().x <= -3.0);
    assert!(bounds.max().x >= 503.0 && bounds.max().x <= 510.0);
    assert!(bounds.min().y >= -10.0 && bounds.min().y <= -6.0);
    assert!(bounds.max().y >= 202.0 && bounds.max().y <= 210.0);
    assert!(bounds.min().z <= -17.0 && bounds.min().z >= -19.0);
    assert!(bounds.max().z >= 14.0 && bounds.max().z <= 16.0);

    // CRLF must never leak into stored source text or comments.
    assert!(
        result
            .program
            .lines
            .iter()
            .all(|line| !line.source.contains('\r'))
    );
    assert!(
        result
            .program
            .comments
            .iter()
            .all(|comment| !comment.text.contains('\r'))
    );

    assert_eq!(result.program.tool_hints.len(), 2);
    for tool in [1_u32, 2] {
        let hint = result
            .program
            .tool_hints
            .iter()
            .find(|hint| hint.tool == tool)
            .unwrap_or_else(|| panic!("T{tool} hint"));
        let diameter = hint
            .diameter_mm
            .unwrap_or_else(|| panic!("T{tool} diameter"));
        let expected = if tool == 1 { 3.1 } else { 6.0 };
        assert!(
            (diameter - expected).abs() < 1e-9,
            "T{tool} diameter {diameter}"
        );
        assert_eq!(hint.shape, Some(ToolShape::Flat));
        let expected_line = line_of(&text, &format!("(T{tool}  D="));
        assert_eq!(hint.line, expected_line, "T{tool} hint line");
    }

    // The 1-based line numbers stay within the source line count.
    let line_count = text.lines().count();
    assert!(
        result
            .program
            .motions
            .iter()
            .all(|motion| { motion.line >= 1 && motion.line <= line_count })
    );

    // Keep the default suite fast: this parse is expected to stay far below a
    // second in debug builds.
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "real fixture parse took {elapsed:?}"
    );
}
