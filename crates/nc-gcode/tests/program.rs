//! Whole-program behavior: statistics, bounds, and line mapping.

use nc_gcode::{MotionMode, SpindleMode, parse_program};

const SAMPLE: &str = "\
%
(6 mm stock, 6 mm end mill)
N10 G21 G90 G54
N20 G0 Z10
T1 M6
M3 S12000
N30 G0 X5 Y5
N40 G1 Z-2 F300
N50 G1 X45 Y5 F800
N60 G2 X45 Y15 I0 J5
N70 G1 X5 Y15
N90 G3 X5 Y5 I0 J-5
N100 G1 Z-4 F300
N110 G81 X10 Y10 Z-6 R-3 F200
N120 X40
N130 Y20
N140 G80
N150 G0 Z10
M5
M30
%
";

#[test]
fn realistic_sample_produces_a_sane_path_and_statistics() {
    let program = parse_program(SAMPLE).expect("valid sample");

    assert!(!program.toolpath.is_empty());
    assert_eq!(program.stats.segments, program.toolpath.len());
    assert_eq!(program.stats.lines, program.lines.len());
    assert!(program.stats.blocks > 10);
    assert!(program.stats.words > 50);

    let bounds = program.bounds;
    assert!(bounds.min().x >= 0.0 && bounds.max().x <= 45.0);
    assert!(bounds.min().y >= 0.0 && bounds.max().y <= 20.0);
    assert!((bounds.min().z - -6.0).abs() < 1.0e-4);
    assert!((bounds.max().z - 10.0).abs() < 1.0e-4);

    assert_eq!(program.stats.tools, vec![0, 1]);
    assert_eq!(program.stats.tool_changes, 1);
    assert_eq!(program.stats.arc_moves, 2);
    assert_eq!(program.stats.canned_cycles, 3);
    assert_eq!(program.stats.max_spindle_rpm, 12000.0);
    assert_eq!(program.stats.min_feed_mm_per_min, Some(200.0));
    assert_eq!(program.stats.max_feed_mm_per_min, Some(800.0));
    assert!(program.stats.cutting_length_mm > 80.0);
    assert!(program.stats.rapid_length_mm > 10.0);
    assert!(
        (program.stats.path_length_mm
            - (program.stats.cutting_length_mm + program.stats.rapid_length_mm))
            .abs()
            < 1.0e-6
    );

    assert_eq!(program.state.motion, MotionMode::Rapid);
    assert_eq!(program.state.spindle, SpindleMode::Off);
    assert_eq!(program.state.tool, 1);
    assert!(program.warnings.is_empty());
}

#[test]
fn line_lookup_maps_segments_back_to_source() {
    let program = parse_program(SAMPLE).expect("valid");
    let arc_line = program
        .lines
        .iter()
        .find(|line| line.source.contains("G2 X45"))
        .expect("arc line");
    assert!(arc_line.segment_count() > 1);
    let segment = arc_line.segments.start;
    assert_eq!(
        program.line_for_segment(segment).map(|line| line.line),
        Some(arc_line.line)
    );
    assert_eq!(program.line_for_segment(program.toolpath.len()), None);
}

#[test]
fn line_numbers_from_n_words_are_preserved() {
    let program = parse_program(SAMPLE).expect("valid");
    let line = program
        .lines
        .iter()
        .find(|line| line.source.starts_with("N60"))
        .expect("n60");
    assert_eq!(line.number, Some(60));
    assert_eq!(line.line, 10);
}

#[test]
fn comment_only_and_empty_programs_are_valid() {
    let empty = parse_program("").expect("empty");
    assert!(empty.toolpath.is_empty());
    assert_eq!(empty.lines.len(), 0);
    assert_eq!(empty.stats.segments, 0);

    let comments = parse_program("(header)\n; nothing here\n").expect("comments");
    assert!(comments.toolpath.is_empty());
    assert_eq!(comments.lines.len(), 2);
    assert_eq!(comments.stats.blocks, 0);
    assert_eq!(comments.stats.words, 0);
}

#[test]
fn stats_are_deterministic() {
    let first = parse_program(SAMPLE).expect("first");
    let second = parse_program(SAMPLE).expect("second");
    assert_eq!(first.stats, second.stats);
    assert_eq!(first.toolpath, second.toolpath);
    assert_eq!(first.bounds, second.bounds);
}

#[test]
fn block_delete_lines_remain_in_the_program() {
    let program = parse_program("/N10 G0 X1\nN20 G0 X2\n").expect("valid");
    assert!(program.lines[0].block_delete);
    assert_eq!(program.lines[0].number, Some(10));
    assert_eq!(program.stats.rapid_segments, 2);
}
