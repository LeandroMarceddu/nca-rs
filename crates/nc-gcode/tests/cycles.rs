//! Canned cycles: simple drilling, pecks, repeats, retract modes, cancel.

use nc_gcode::{ParseErrorKind, SpindleMode, parse_program};
use nc_viewer::SegmentKind;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0e-4
}

#[test]
fn g81_drills_and_retracts_to_the_initial_level() {
    let program = parse_program("G0 X0 Y0 Z5\nG81 X10 Y10 Z-5 R2 F100\nG80\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0].kind, SegmentKind::Rapid);
    assert_eq!(segments[1].kind, SegmentKind::Rapid);
    assert_eq!(segments[1].end, nc_viewer::Point3::new(10.0, 10.0, 5.0));
    assert_eq!(segments[2].kind, SegmentKind::Rapid);
    assert_eq!(segments[2].end, nc_viewer::Point3::new(10.0, 10.0, 2.0));
    assert_eq!(segments[3].kind, SegmentKind::Plunge);
    assert_eq!(segments[3].end, nc_viewer::Point3::new(10.0, 10.0, -5.0));
    assert_eq!(segments[4].kind, SegmentKind::Rapid);
    assert_eq!(segments[4].end, nc_viewer::Point3::new(10.0, 10.0, 5.0));
    assert_eq!(program.stats.canned_cycles, 1);
    assert_eq!(program.state.motion, nc_gcode::MotionMode::None);
    assert!(close(program.bounds.min().z, -5.0));
}

#[test]
fn g99_retracts_to_the_r_plane() {
    let program = parse_program("G0 Z5\nG99 G81 Z-5 R2 F100\n").expect("valid");
    let last = program.toolpath.segments().last().expect("last");
    assert_eq!(last.end, nc_viewer::Point3::new(0.0, 0.0, 2.0));
    assert_eq!(program.state.position, [0.0, 0.0, 2.0]);
}

#[test]
fn g82_dwell_adds_no_geometry() {
    let plain = parse_program("G0 Z5\nG81 Z-5 R2 F100\n").expect("g81");
    let dwell = parse_program("G0 Z5\nG82 Z-5 R2 P1 F100\n").expect("g82");
    assert_eq!(
        plain.toolpath.segments().len(),
        dwell.toolpath.segments().len()
    );
}

#[test]
fn g83_pecks_with_full_retract() {
    let program = parse_program("G0 X0 Y0 Z0\nG83 Z-3 R-1 Q1 F100\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 6);
    let kinds: Vec<SegmentKind> = segments.iter().map(|segment| segment.kind).collect();
    assert_eq!(
        kinds,
        vec![
            SegmentKind::Rapid,
            SegmentKind::Plunge,
            SegmentKind::Rapid,
            SegmentKind::Rapid,
            SegmentKind::Plunge,
            SegmentKind::Rapid,
        ]
    );
    assert!(close(segments[1].end.z, -2.0));
    assert!(close(segments[2].end.z, -1.0));
    assert!(close(segments[3].end.z, -1.5));
    assert!(close(segments[4].end.z, -3.0));
    assert!(close(segments[5].end.z, 0.0));
    assert_eq!(program.stats.plunge_segments, 2);
}

#[test]
fn g73_pecks_with_chip_break_retract() {
    let program = parse_program("G0 X0 Y0 Z0\nG73 Z-3 R-1 Q1 F100\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 5);
    assert!(close(segments[1].end.z, -2.0));
    assert!(close(segments[2].end.z, -1.5));
    assert!(close(segments[3].end.z, -3.0));
    assert!(close(segments[4].end.z, 0.0));
    assert_eq!(program.stats.plunge_segments, 2);
}

#[test]
fn a_cycle_repeats_on_xy_words() {
    let program = parse_program("G0 X0 Y0 Z5\nG81 Z-5 R2 F100\nX10\nY10\n").expect("valid");
    assert_eq!(program.stats.canned_cycles, 3);
    assert_eq!(program.stats.plunge_segments, 3);
    let segments = program.toolpath.segments();
    let holes: Vec<f32> = segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Plunge)
        .map(|segment| segment.end.x)
        .collect();
    assert_eq!(holes, vec![0.0, 10.0, 10.0]);
    assert_eq!(program.toolpath.segments().len(), 12);
    assert_eq!(program.state.position, [10.0, 10.0, 5.0]);
}

#[test]
fn g80_cancels_the_cycle() {
    let errors = parse_program("G0 Z5\nG81 Z-5 R2 F100\nG80\nX10\n").expect_err("no motion");
    assert_eq!(errors[0].kind, ParseErrorKind::NoMotionMode);
    assert_eq!(errors[0].line, 4);

    let program = parse_program("G0 Z5\nG81 Z-5 R2 F100\nG80\nG1 X10 F100\n").expect("valid");
    assert_eq!(program.state.motion, nc_gcode::MotionMode::Linear);
}

#[test]
fn selecting_a_cycle_requires_r_and_z() {
    let missing_r = parse_program("G81 Z-5 F100\n").expect_err("no r");
    assert_eq!(missing_r[0].kind, ParseErrorKind::MissingParameter);
    assert!(missing_r[0].message.contains('R'));

    let missing_z = parse_program("G81 R2 F100\n").expect_err("no z");
    assert_eq!(missing_z[0].kind, ParseErrorKind::MissingParameter);
    assert!(missing_z[0].message.contains('Z'));
}

#[test]
fn peck_cycles_require_a_positive_q() {
    let missing = parse_program("G83 Z-3 R-1 F100\n").expect_err("no q");
    assert_eq!(missing[0].kind, ParseErrorKind::MissingParameter);

    let zero = parse_program("G83 Z-3 R-1 Q0 F100\n").expect_err("zero q");
    assert_eq!(zero[0].kind, ParseErrorKind::InvalidParameter);

    let negative = parse_program("G83 Z-3 R-1 Q-1 F100\n").expect_err("negative q");
    assert_eq!(negative[0].kind, ParseErrorKind::InvalidParameter);
}

#[test]
fn cycles_require_the_xy_plane() {
    let errors = parse_program("G18\nG81 Z-5 R2 F100\n").expect_err("plane");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
}

#[test]
fn an_r_plane_below_the_depth_is_an_error() {
    let errors = parse_program("G0 Z5\nG81 Z3 R2 F100\n").expect_err("r below depth");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidParameter);
}

#[test]
fn a_cycle_that_removes_nothing_warns() {
    let program = parse_program("G0 Z5\nG81 Z2 R2 F100\n").expect("valid");
    assert!(
        program
            .warnings
            .iter()
            .any(|warning| warning.kind == nc_gcode::WarningKind::EmptyCannedCycle)
    );
    assert_eq!(program.stats.plunge_segments, 0);
}

#[test]
fn dwell_cycles_require_p_on_their_first_block() {
    for line in ["G82 Z-2 R1 F100", "G86 Z-2 R1 F100", "G89 Z-2 R1 F100"] {
        let errors = parse_program(line).expect_err("missing dwell");
        assert_eq!(errors[0].kind, ParseErrorKind::MissingParameter);
        assert!(errors[0].message.contains('P'));
    }
}

#[test]
fn g86_stops_the_spindle_at_the_bottom() {
    let program = parse_program("M3\nG0 Z5\nG86 Z-2 R1 P0.5 F100\n").expect("valid");
    assert_eq!(program.state.spindle, SpindleMode::Off);
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 5);
    assert!(close(segments[2].end.z, -2.0));
    assert!(close(segments[3].end.z, 1.0));
    assert!(close(segments[4].end.z, 5.0));
}

#[test]
fn g85_feeds_back_out() {
    let program = parse_program("G0 Z5\nG85 Z-2 R1 F100\n").expect("valid");
    let kinds: Vec<SegmentKind> = program
        .toolpath
        .segments()
        .iter()
        .map(|segment| segment.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            SegmentKind::Rapid,
            SegmentKind::Rapid,
            SegmentKind::Plunge,
            SegmentKind::Feed,
            SegmentKind::Rapid,
        ]
    );
}

#[test]
fn incremental_cycle_parameters_are_relative_to_the_cycle_planes() {
    // R is relative to the initial Z (5) and Z is relative to R (3).
    let program = parse_program("G0 Z5\nG91\nG81 Z-5 R-2 F100\n").expect("valid");
    let segments = program.toolpath.segments();
    let plunge = segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::Plunge)
        .expect("plunge");
    assert!(close(plunge.end.z, -2.0));
}

#[test]
fn canned_cycles_are_rejected_with_inverse_time_feed() {
    let errors = parse_program("G93\nG81 Z-5 R2 F0.5\n").expect_err("g93 cycle");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
    assert!(errors[0].message.contains("G93"));
}

#[test]
fn canned_cycle_l_repeat_counts_are_rejected() {
    let errors = parse_program("G81 Z-5 R2 L2 F100\n").expect_err("l");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
    assert!(errors[0].message.contains('L'));
}
