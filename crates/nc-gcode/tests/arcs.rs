//! Arc geometry: planes, directions, I/J/K, R, full circles, and bounds.

use nc_gcode::{ParseErrorKind, parse_program};
use nc_viewer::MoveKind;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0e-3
}

/// Asserts that the bounds of `program` contain the expected extents.
fn assert_bounds(program: &nc_gcode::Program, min: [f32; 3], max: [f32; 3]) {
    let bounds = program.bounds;
    assert!(
        close(bounds.min().x, min[0])
            && close(bounds.min().y, min[1])
            && close(bounds.min().z, min[2]),
        "min {:?}, expected {min:?}",
        bounds.min()
    );
    assert!(
        close(bounds.max().x, max[0])
            && close(bounds.max().y, max[1])
            && close(bounds.max().z, max[2]),
        "max {:?}, expected {max:?}",
        bounds.max()
    );
}

#[test]
fn xy_clockwise_arc_uses_the_top_half() {
    let program = parse_program("G0 X0 Y0\nG2 X10 Y0 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, 0.0, 0.0], [10.0, 5.0, 0.0]);
    let segments = program.toolpath.segments();
    assert!(segments.len() > 2);
    assert!(
        segments
            .iter()
            .all(|segment| segment.kind == MoveKind::ArcCw)
    );
    assert!(close(segments[0].start.x, 0.0) && close(segments[0].start.y, 0.0));
    assert!(close(segments[segments.len() - 1].end.x, 10.0));
    assert!(close(segments[segments.len() - 1].end.y, 0.0));
    assert_eq!(program.stats.arc_moves, 1);
    assert!((program.stats.cutting_length_mm - 5.0 * std::f64::consts::PI).abs() < 1.0e-3);
}

#[test]
fn xy_counter_clockwise_arc_uses_the_bottom_half() {
    let program = parse_program("G0 X0 Y0\nG3 X10 Y0 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, -5.0, 0.0], [10.0, 0.0, 0.0]);
}

#[test]
fn full_circle_with_ijk_closes_and_has_exact_bounds() {
    let program = parse_program("G0 X0 Y0\nG2 X0 Y0 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, -5.0, 0.0], [10.0, 5.0, 0.0]);
    let segments = program.toolpath.segments();
    let first = segments.first().expect("first");
    let last = segments.last().expect("last");
    assert!(close(first.start.x, last.end.x) && close(first.start.y, last.end.y));
    assert!((program.stats.cutting_length_mm - 10.0 * std::f64::consts::PI).abs() < 1.0e-3);
}

#[test]
fn full_circle_without_axis_words_is_accepted() {
    let program = parse_program("G0 X0 Y0\nG2 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, -5.0, 0.0], [10.0, 5.0, 0.0]);
    assert_eq!(program.stats.arc_moves, 1);
}

#[test]
fn g18_arc_lies_in_the_zx_plane() {
    let program = parse_program("G18\nG0 X0 Y0 Z0\nG3 X0 Z10 I0 K5 F100\n").expect("valid");
    assert_bounds(&program, [-5.0, 0.0, 0.0], [0.0, 0.0, 10.0]);
    assert!(
        program
            .toolpath
            .segments()
            .iter()
            .all(|segment| close(segment.start.y, 0.0))
    );
}

#[test]
fn g19_arc_lies_in_the_yz_plane() {
    let program = parse_program("G19\nG0 X0 Y0 Z0\nG2 Y10 Z0 J5 K0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, 0.0, 0.0], [0.0, 10.0, 5.0]);
    assert!(
        program
            .toolpath
            .segments()
            .iter()
            .all(|segment| close(segment.start.x, 0.0))
    );
}

#[test]
fn r_form_selects_minor_and_major_arcs() {
    let minor = parse_program("G0 X0 Y0\nG2 X6 Y0 R5 F100\n").expect("valid");
    assert_bounds(&minor, [0.0, 0.0, 0.0], [6.0, 1.0, 0.0]);

    let major = parse_program("G0 X0 Y0\nG2 X6 Y0 R-5 F100\n").expect("valid");
    assert_bounds(&major, [-2.0, 0.0, 0.0], [8.0, 9.0, 0.0]);

    let ccw_minor = parse_program("G0 X0 Y0\nG3 X6 Y0 R5 F100\n").expect("valid");
    assert_bounds(&ccw_minor, [0.0, -1.0, 0.0], [6.0, 0.0, 0.0]);
}

#[test]
fn helical_arc_splits_into_planar_slices_with_exact_bounds() {
    let program = parse_program("G0 X0 Y0 Z0\nG2 X10 Y0 Z-2 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, 0.0, -2.0], [10.0, 5.0, 0.0]);
    let segments = program.toolpath.segments();
    assert!(segments.len() > 4);
    assert!(
        segments
            .iter()
            .all(|segment| segment.kind == MoveKind::HelixCw)
    );
    let last = segments.last().expect("last");
    assert!(close(last.end.x, 10.0) && close(last.end.z, -2.0));
    let length = (5.0 * std::f64::consts::PI).hypot(2.0);
    assert!((program.stats.cutting_length_mm - length).abs() < 1.0e-3);
}

#[test]
fn arc_endpoint_radius_mismatch_is_rejected() {
    let errors = parse_program("G0 X0 Y0\nG2 X10 Y1 I5 J0 F100\n").expect_err("radius");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidArc);
    assert_eq!(errors[0].line, 2);
    assert!(errors[0].message.contains("radius"));
}

#[test]
fn arc_requires_a_center_or_radius() {
    let errors = parse_program("G2 X10 Y0 F100\n").expect_err("no center");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidArc);
}

#[test]
fn full_circle_with_r_is_rejected() {
    let errors = parse_program("G0 X0 Y0\nG2 X0 Y0 R5 F100\n").expect_err("full circle");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidArc);
}

#[test]
fn r_radius_too_small_for_the_chord_is_rejected() {
    let errors = parse_program("G0 X0 Y0\nG2 X10 Y0 R4 F100\n").expect_err("radius");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidArc);
}

#[test]
fn arc_without_feed_is_rejected() {
    let errors = parse_program("G2 X10 Y0 I5 J0\n").expect_err("feed");
    assert_eq!(errors[0].kind, ParseErrorKind::FeedRateUndefined);
}

#[test]
fn mixed_ijk_and_r_is_rejected() {
    let errors = parse_program("G2 X10 Y0 I5 J0 R5 F100\n").expect_err("both");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidParameter);
}

#[test]
fn seeded_rotation_with_a_g18_arc_is_rejected() {
    use nc_gcode::{Parser, ParserState, Rotation};
    let mut parser = Parser::new();
    parser.set_state(ParserState {
        rotation: Some(Rotation {
            angle_rad: 0.5,
            pivot_mm: [0.0, 0.0],
        }),
        ..ParserState::default()
    });
    let errors = parser
        .parse_line("G18 G3 X0 Z10 I0 K5 F100")
        .expect_err("rotation");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
}

#[test]
fn arc_bounds_are_exact_even_with_a_coarse_tolerance() {
    let config = nc_gcode::ParserConfig::default().with_chord_tolerance_mm(1.0);
    let program =
        nc_gcode::parse_program_with("G0 X0 Y0\nG3 X10 Y0 I5 J0 F100\n", &config).expect("valid");
    assert_bounds(&program, [0.0, -5.0, 0.0], [10.0, 0.0, 0.0]);
}

#[test]
fn seeded_rotation_turns_xy_arcs() {
    use nc_gcode::{Parser, ParserState, Rotation};
    let mut parser = Parser::new();
    parser.set_state(ParserState {
        rotation: Some(Rotation {
            angle_rad: std::f64::consts::FRAC_PI_2,
            pivot_mm: [0.0, 0.0],
        }),
        ..ParserState::default()
    });
    parser.parse_line("G0 X1 Y0").expect("start");
    parser.parse_line("G3 X0 Y1 I-1 J0 F100").expect("arc");
    let program = parser.finish();
    assert_bounds(&program, [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
}

#[test]
fn g51_scaling_scales_arc_radius() {
    let program = parse_program("G0 X0 Y0\nG51 P2\nG2 X10 Y0 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, 0.0, 0.0], [20.0, 10.0, 0.0]);
}

#[test]
fn helical_full_circle_closes_and_has_exact_bounds() {
    let program = parse_program("G0 X0 Y0 Z0\nG2 X0 Y0 Z-2 I5 J0 F100\n").expect("valid");
    assert_bounds(&program, [0.0, -5.0, -2.0], [10.0, 5.0, 0.0]);
    let segments = program.toolpath.segments();
    let first = segments.first().expect("first");
    let last = segments.last().expect("last");
    assert!(close(first.start.x, last.end.x) && close(first.start.y, last.end.y));
    assert!(close(last.end.z, -2.0));
}

#[test]
fn semicircle_second_half_is_tessellated() {
    let program = parse_program("G0 X0 Y0\nG1 X40 F800\nG2 X40 Y40 I0 J20\n").expect("valid");
    let arcs: Vec<_> = program
        .toolpath
        .segments()
        .iter()
        .filter(|segment| segment.kind == MoveKind::ArcCw)
        .collect();

    assert!(
        arcs.len() > 20,
        "the second half collapsed to a chord: {} arc segments",
        arcs.len()
    );
    assert!(
        arcs.iter()
            .all(|segment| segment.start.distance(segment.end) < 4.0),
        "an arc chord is longer than the tessellation allows"
    );
    assert!(
        arcs.iter()
            .any(|segment| close(segment.end.x, 25.858) && close(segment.end.y, 34.142)),
        "second-half midpoint is missing"
    );
    let last = arcs.last().expect("arc segments");
    assert!(close(last.end.x, 40.0) && close(last.end.y, 40.0));
}
