//! Straight-line motion, modal state, units, offsets, and transforms.

use nc_core::{Position, Wcs};
use nc_gcode::{
    CoolantMode, ParseErrorKind, Parser, ParserConfig, SpindleMode, ToolLength, WarningKind,
    parse_program, parse_program_with,
};
use nc_viewer::MoveKind;

fn offsets_with(wcs: Wcs, x: f64, y: f64, z: f64) -> [Position; 9] {
    let mut offsets = [Position::zero(); 9];
    offsets[wcs.index()] = Position::xyz(x, y, z);
    offsets
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0e-4
}

#[test]
fn simple_line_rapid_plunge_path() {
    let program = parse_program("G21\nG90\nG0 X0 Y0 Z5\nG1 Z-1 F100\nG1 X10 Y0\n").expect("valid");
    let segments = program.toolpath.segments();

    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].kind, MoveKind::Retract);
    assert_eq!(segments[1].kind, MoveKind::Plunge);
    assert_eq!(segments[2].kind, MoveKind::Feed);

    assert_eq!(segments[0].start, nc_viewer::Point3::ZERO);
    assert_eq!(segments[0].end, nc_viewer::Point3::new(0.0, 0.0, 5.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(0.0, 0.0, -1.0));
    assert_eq!(segments[2].end, nc_viewer::Point3::new(10.0, 0.0, -1.0));

    assert_eq!(program.stats.rapid_segments, 1);
    assert_eq!(program.stats.plunge_segments, 1);
    assert_eq!(program.stats.feed_segments, 1);
    assert_eq!(program.stats.segments, 3);
    assert!(close(program.bounds.min().z, -1.0));
    assert!(close(program.bounds.max().x, 10.0));
    assert!(close(program.bounds.max().z, 5.0));
    assert!((program.stats.cutting_length_mm - 16.0).abs() < 1.0e-6);
    assert!((program.stats.rapid_length_mm - 5.0).abs() < 1.0e-6);

    assert_eq!(program.lines[0].segment_count(), 0);
    assert_eq!(program.lines[2].segments, 0..1);
    assert_eq!(program.lines[3].segments, 1..2);
    assert_eq!(program.lines[4].segments, 2..3);
    assert_eq!(program.line_for_segment(2).map(|line| line.line), Some(5));
}

#[test]
fn modal_absolute_and_incremental() {
    let program = parse_program("G0 X10 Y0\nG1 X5 Y0 F100\nG91\nG1 X10 Y10\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[1].end, nc_viewer::Point3::new(5.0, 0.0, 0.0));
    assert_eq!(segments[2].end, nc_viewer::Point3::new(15.0, 10.0, 0.0));
    assert_eq!(program.state.position, [15.0, 10.0, 0.0]);
}

#[test]
fn inch_mode_converts_coordinates_and_feed() {
    let program = parse_program("G20\nG0 X1 Y0\nG1 X2 Y0 F10\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments[0].end, nc_viewer::Point3::new(25.4, 0.0, 0.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(50.8, 0.0, 0.0));
    assert!((program.stats.max_feed_mm_per_min.expect("feed") - 254.0).abs() < 1.0e-9);
}

#[test]
fn work_coordinate_offsets_are_applied() {
    let config = ParserConfig::default().with_offsets(offsets_with(Wcs::G55, 10.0, 20.0, 30.0));
    let program = parse_program_with("G55 G0 X0 Y0 Z0\nG54 G0 X1 Y1 Z1\n", &config).expect("valid");
    let segments = program.toolpath.segments();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].start, nc_viewer::Point3::ZERO);
    assert_eq!(segments[0].end, nc_viewer::Point3::new(10.0, 20.0, 30.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(1.0, 1.0, 1.0));
    assert_eq!(program.state.wcs, Wcs::G54);
    assert!(close(program.bounds.min().x, 0.0));
    assert!(close(program.bounds.max().x, 10.0));
}

#[test]
fn g92_shifts_the_frame_without_moving() {
    let program = parse_program("G0 X10 Y0\nG92 X0 Y0\nG0 X5 Y0\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].end, nc_viewer::Point3::new(10.0, 0.0, 0.0));
    assert_eq!(segments[1].start, nc_viewer::Point3::new(10.0, 0.0, 0.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(15.0, 0.0, 0.0));
    assert_eq!(program.state.g92, [10.0, 0.0, 0.0]);
    assert_eq!(program.state.position, [5.0, 0.0, 0.0]);

    let cleared = parse_program("G92 X10\nG92.1\n").expect("valid");
    assert_eq!(cleared.state.g92, [0.0, 0.0, 0.0]);
}

#[test]
fn g53_targets_machine_coordinates() {
    let config = ParserConfig::default().with_offsets(offsets_with(Wcs::G54, 10.0, 0.0, 0.0));
    let program = parse_program_with("G0 X1 Y0\nG53 G0 X0 Y0\nG0 X1 Y0\n", &config).expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].end, nc_viewer::Point3::new(11.0, 0.0, 0.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::ZERO);
    assert_eq!(segments[2].end, nc_viewer::Point3::new(11.0, 0.0, 0.0));
    assert_eq!(program.state.position, [1.0, 0.0, 0.0]);
}

#[test]
fn g28_uses_the_configured_home() {
    let config = ParserConfig::default().with_home_position([0.0, 0.0, 20.0]);
    let program = parse_program_with("G0 X5 Y5 Z5\nG28\n", &config).expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].start, nc_viewer::Point3::new(5.0, 5.0, 5.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(0.0, 0.0, 20.0));
    assert!(program.warnings.is_empty());
}

#[test]
fn g28_without_a_stored_position_warns_and_assumes_the_origin() {
    let program = parse_program("G0 X5 Y5 Z5\nG28\n").expect("valid");
    assert_eq!(program.warnings.len(), 1);
    assert_eq!(program.warnings[0].kind, WarningKind::AssumedHomePosition);
    assert_eq!(program.toolpath.segments()[1].end, nc_viewer::Point3::ZERO);
}

#[test]
fn g28_1_stores_the_current_machine_position() {
    let program = parse_program("G0 X5 Y5 Z5\nG28.1\nG0 X0 Y0 Z0\nG28\n").expect("valid");
    let segments = program.toolpath.segments();
    let home = *segments.last().expect("home move");
    assert_eq!(home.end, nc_viewer::Point3::new(5.0, 5.0, 5.0));
}

#[test]
fn g10_l2_sets_work_offsets() {
    let program =
        parse_program("G0 X1 Y1 Z1\nG10 L2 P2 X10 Y20 Z30\nG55 G0 X0 Y0 Z0\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].end, nc_viewer::Point3::new(10.0, 20.0, 30.0));
    assert_eq!(
        program.offsets[Wcs::G55.index()],
        Position::xyz(10.0, 20.0, 30.0)
    );
}

#[test]
fn g10_l20_sets_the_offset_from_the_current_position() {
    let program = parse_program("G0 X5 Y5 Z5\nG10 L20 P1 X0 Y0 Z0\nG0 X1 Y1 Z1\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments[1].start, nc_viewer::Point3::new(5.0, 5.0, 5.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(6.0, 6.0, 6.0));
    assert_eq!(
        program.offsets[Wcs::G54.index()],
        Position::xyz(5.0, 5.0, 5.0)
    );
}

#[test]
fn g43_1_applies_a_tool_length_offset() {
    let program = parse_program("G0 X0 Y0 Z0\nG43.1 Z10\nG0 Z0\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].end, nc_viewer::Point3::new(0.0, 0.0, 10.0));
    assert_eq!(
        program.state.tool_length,
        ToolLength::Dynamic {
            offset_mm: [0.0, 0.0, 10.0]
        }
    );

    let cancelled = parse_program("G43.1 Z10\nG49\n").expect("valid");
    assert_eq!(cancelled.state.tool_length, ToolLength::Cancel);
}

#[test]
fn g51_scales_about_its_pivot() {
    let program = parse_program("G0 X1 Y0 Z0\nG51 X0 Y0 Z0 P2\nG0 X2 Y0 Z0\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments[1].start, nc_viewer::Point3::new(1.0, 0.0, 0.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(4.0, 0.0, 0.0));
    assert!(program.state.scaling.is_some());

    let cancelled = parse_program("G51 P2\nG50\n").expect("valid");
    assert!(cancelled.state.scaling.is_none());
}

#[test]
fn g68_is_rejected_because_grblhal_has_no_rotation_gcode() {
    let errors = parse_program("G68 X0 Y0 R90\n").expect_err("no g68");
    assert_eq!(errors[0].kind, ParseErrorKind::UnsupportedCode);
    assert!(errors[0].message.contains("per-WCS"));
}

#[test]
fn rotation_seeded_through_set_state_is_applied() {
    let mut parser = Parser::new();
    let state = nc_gcode::ParserState {
        rotation: Some(nc_gcode::Rotation {
            angle_rad: std::f64::consts::FRAC_PI_2,
            pivot_mm: [0.0, 0.0],
        }),
        ..nc_gcode::ParserState::default()
    };
    parser.set_state(state);
    parser.parse_line("G0 X1 Y0").expect("move");
    let program = parser.finish();
    let end = program.toolpath.segments()[0].end;
    assert!(close(end.x, 0.0));
    assert!(close(end.y, 1.0));
}

#[test]
fn spindle_coolant_and_tool_state() {
    let program = parse_program("S1000 M3\nM8\nT2\nM6\nM5 M9\n").expect("valid");
    assert_eq!(program.state.spindle, SpindleMode::Off);
    assert_eq!(program.state.coolant, CoolantMode::Off);
    assert_eq!(program.state.tool, 2);
    assert_eq!(program.state.spindle_rpm, 1000.0);
    assert_eq!(program.stats.max_spindle_rpm, 1000.0);
    assert_eq!(program.stats.tool_changes, 1);
    assert_eq!(program.stats.tools, vec![2]);
    assert!(program.toolpath.is_empty());
}

#[test]
fn program_end_is_recorded_and_motion_after_it_warns() {
    let program = parse_program("M30\nG0 X1\n").expect("valid");
    assert_eq!(program.warnings.len(), 1);
    assert_eq!(program.warnings[0].kind, WarningKind::MotionAfterProgramEnd);
    // The display still shows the unreachable move.
    assert_eq!(program.stats.rapid_segments, 1);
}

#[test]
fn incremental_parser_api_keeps_state_and_lines() {
    let mut parser = Parser::new();
    parser.parse_line("G0 X1").expect("first");
    parser.parse_line("G1 X2 F100").expect("second");
    assert_eq!(parser.state().position, [2.0, 0.0, 0.0]);
    assert_eq!(parser.machine_position(), [2.0, 0.0, 0.0]);
    let program = parser.finish();
    assert_eq!(program.lines.len(), 2);
    assert_eq!(program.lines[1].line, 2);
    assert_eq!(program.stats.segments, 2);
}

#[test]
fn a_failed_block_does_not_change_state() {
    let mut parser = Parser::new();
    parser.parse_line("G0 X1").expect("first");
    let errors = parser.parse_line("G1 X2").expect_err("missing feed");
    assert_eq!(errors[0].kind, ParseErrorKind::FeedRateUndefined);
    assert_eq!(parser.state().position, [1.0, 0.0, 0.0]);
    assert_eq!(parser.toolpath().len(), 1);
    parser.parse_line("G1 X3 F100").expect("third");
    assert_eq!(parser.finish().stats.segments, 2);
}

#[test]
fn g93_inverse_time_feed_is_tracked_without_mm_per_min() {
    let program = parse_program("G93\nG1 X10 F0.5\n").expect("valid");
    assert!(program.state.feed_mm_per_min().is_none());
    assert_eq!(program.stats.min_feed_mm_per_min, None);
}

#[test]
fn probing_moves_are_marked_and_not_cutting() {
    let program = parse_program("G0 Z0\nG38.2 Z-5 F50\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].kind, MoveKind::Probe);
    assert_eq!(program.stats.probe_segments, 1);
    assert_eq!(program.stats.cutting_length_mm, 0.0);
    assert!((program.stats.path_length_mm - 5.0).abs() < 1.0e-6);
}

#[test]
fn a_modal_word_after_a_probe_move_is_not_a_probe() {
    // `G91` changes the distance mode; it must not be treated as a probe move
    // just because `G38.2` is the modal motion.
    let program = parse_program("G0 Z0\nG38.2 Z-5 F50\nG91\nG0 Z2\nG90\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].kind, MoveKind::Retract);
    assert!((segments[1].end.z - segments[1].start.z - 2.0).abs() < 1.0e-6);

    // A probe code without an axis is still rejected.
    let errors = parse_program("G38.2 F50\n").expect_err("probe without axes");
    assert_eq!(errors[0].kind, ParseErrorKind::MissingParameter);
}

#[test]
fn g40_g61_and_g64_are_accepted() {
    let program = parse_program("G40\nG61\nG64\n").expect("valid");
    assert_eq!(
        program.state.path_control,
        nc_gcode::PathControl::Continuous
    );
}

#[test]
fn changing_feed_mode_without_a_new_feed_clears_the_stale_value() {
    let errors = parse_program("G94 F100\nG93\nG1 X1\n").expect_err("no inverse feed");
    assert_eq!(errors[0].kind, ParseErrorKind::FeedRateUndefined);

    // G93 requires an F word on every block with a feed motion.
    let errors = parse_program("G93 F0.5\nG1 X1\n").expect_err("stale inverse feed");
    assert_eq!(errors[0].kind, ParseErrorKind::FeedRateUndefined);

    let program = parse_program("G93 F0.5\nG1 X1 F0.5\n").expect("valid");
    assert_eq!(program.state.feed, Some(0.5));
    assert!(program.state.feed_mm_per_min().is_none());
}

#[test]
fn inch_arc_radius_is_converted() {
    let program = parse_program("G20\nG2 X1 Y0 R0.5 F10\n").expect("valid");
    let bounds = program.bounds;
    assert!(close(bounds.min().x, 0.0));
    assert!(close(bounds.max().x, 25.4));
    assert!(close(bounds.min().y, 0.0));
    assert!(close(bounds.max().y, 12.7));
}

#[test]
fn g53_is_absolute_even_in_incremental_mode() {
    let program = parse_program("G91 G0 X10\nG53 G0 X0\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].end, nc_viewer::Point3::ZERO);
    assert_eq!(program.state.position, [0.0, 0.0, 0.0]);
}

#[test]
fn g10_l2_is_absolute_even_in_incremental_mode() {
    // grblHAL sets L2 offsets absolutely; it does not add under G91.
    let program = parse_program("G10 L2 P1 X10 Y5\nG91\nG10 L2 P1 X5 Y-2\n").expect("valid");
    assert_eq!(
        program.offsets[Wcs::G54.index()],
        Position::xyz(5.0, -2.0, 0.0)
    );
}

#[test]
fn g92_2_suspends_and_g92_3_restores() {
    let program = parse_program("G0 X10\nG92 X0\nG92.2\nG0 X1\nG92.3\n").expect("valid");
    assert_eq!(program.state.g92, [10.0, 0.0, 0.0]);
    assert_eq!(program.state.suspended_g92, [10.0, 0.0, 0.0]);
    assert_eq!(program.state.position, [-9.0, 0.0, 0.0]);
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].start, nc_viewer::Point3::new(10.0, 0.0, 0.0));
    assert_eq!(segments[1].end, nc_viewer::Point3::new(1.0, 0.0, 0.0));
}

#[test]
fn mist_and_flood_are_independent() {
    let program = parse_program("M7\nM8\n").expect("valid");
    assert_eq!(program.state.coolant, CoolantMode::MistAndFlood);
    assert_eq!(program.state.coolant.m_code(), "M7 M8");

    let errors = parse_program("M7 M8\n").expect_err("same group");
    assert_eq!(errors[0].kind, ParseErrorKind::ConflictingCodes);

    let program = parse_program("M7\nM8\nM9\n").expect("valid");
    assert_eq!(program.state.coolant, CoolantMode::Off);
}

#[test]
fn transform_parameters_are_validated() {
    assert_eq!(
        parse_program("G51 P0\n").expect_err("zero scale")[0].kind,
        ParseErrorKind::ValueOutOfRange
    );
    assert_eq!(
        parse_program("G68 X0 Y0\n").expect_err("no g68")[0].kind,
        ParseErrorKind::UnsupportedCode
    );
    assert_eq!(
        parse_program("G43.1\n").expect_err("no axis")[0].kind,
        ParseErrorKind::MissingParameter
    );
    assert_eq!(
        parse_program("G51 P2 G43.1 Z1\n").expect_err("two transforms")[0].kind,
        ParseErrorKind::ConflictingCommands
    );
}
