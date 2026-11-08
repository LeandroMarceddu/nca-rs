//! Motion-record emission: semantic kinds, exact arcs, canned cycles, modal
//! interning, annotations, and feed-mode flags.

use nc_core::math;
use nc_gcode::{
    AnnotationKind, ArcFormat, CannedCycle, CutterComp, Dialect, FeedMode, MoveKind, ParserConfig,
    Plane, RetractMode, WarningKind, parse_program, parse_program_with,
};

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn straight_motion_kinds_follow_the_taxonomy() {
    let program = parse_program(
        "G21 G90\n\
         G0 Z5\n\
         G0 X10\n\
         G1 Z-1 F100\n\
         G1 Z2\n\
         G1 X20 Z-3\n\
         G1 X30\n\
         G1 Z5\n\
         G0 Z-2\n",
    )
    .expect("valid");

    let kinds: Vec<MoveKind> = program.motions.iter().map(|motion| motion.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MoveKind::Retract,
            MoveKind::Rapid,
            MoveKind::Plunge,
            MoveKind::Retract,
            MoveKind::Ramp,
            MoveKind::Feed,
            MoveKind::Retract,
            MoveKind::Rapid,
        ]
    );

    let first = &program.motions[0];
    assert_eq!(first.line, 2);
    assert_eq!(first.index, 0);
    assert_eq!(first.start_mm, [0.0, 0.0, 0.0]);
    assert_eq!(first.end_mm, [0.0, 0.0, 5.0]);
    assert_eq!(first.plane, Plane::Xy);
    assert_eq!(first.modal, program.motions[1].modal);
    assert_eq!(program.modal(first.modal).units, Some(nc_core::Units::Mm));
    assert_eq!(
        program.modal(first.modal).distance,
        Some(nc_gcode::DistanceMode::Absolute)
    );

    let plunge = &program.motions[3];
    assert_eq!(plunge.feed_mm_min, Some(100.0));
    let ramp = &program.motions[4];
    assert_eq!(ramp.kind, MoveKind::Ramp);
    assert_eq!(ramp.start_mm, [10.0, 0.0, 2.0]);
    assert_eq!(ramp.end_mm, [20.0, 0.0, -3.0]);
}

#[test]
fn g17_arcs_have_the_documented_sweep_convention() {
    let clockwise = parse_program("G2 X10 Y0 I5 J0 F100").expect("arc");
    let record = &clockwise.motions[0];
    assert_eq!(record.kind, MoveKind::ArcCw);
    let arc = record.arc.as_ref().expect("arc record");
    assert_eq!(arc.center_mm, [5.0, 0.0, 0.0]);
    assert!(close(arc.radius_mm, 5.0, 1e-12));
    assert!(close(arc.end_radius_mm, 5.0, 1e-12));
    assert!(close(arc.start_angle_rad, math::PI, 1e-12));
    assert!(close(arc.sweep_rad, -math::PI, 1e-12));
    assert_eq!(arc.axial_delta_mm, 0.0);
    assert_eq!(arc.format, ArcFormat::Center);

    let counter = parse_program("G3 X10 Y0 I5 J0 F100").expect("arc");
    let arc = counter.motions[0].arc.as_ref().expect("arc record");
    assert!(close(arc.start_angle_rad, math::PI, 1e-12));
    assert!(close(arc.sweep_rad, math::PI, 1e-12));
    assert_eq!(counter.motions[0].kind, MoveKind::ArcCcw);
}

#[test]
fn g18_and_g19_arcs_use_their_plane_basis() {
    let zx = parse_program("G18\nG3 X0 Z10 I0 K5 F100").expect("g18");
    let record = &zx.motions[0];
    let arc = record.arc.as_ref().expect("arc");
    assert_eq!(record.plane, Plane::Zx);
    assert_eq!(arc.center_mm, [0.0, 0.0, 5.0]);
    assert!(close(arc.start_angle_rad, math::PI, 1e-12));
    assert!(close(arc.sweep_rad, math::PI, 1e-12));
    assert_eq!(arc.axial_delta_mm, 0.0);
    assert_eq!(record.kind, MoveKind::ArcCcw);

    let yz = parse_program("G19\nG2 Y10 Z0 J5 K0 F100").expect("g19");
    let record = &yz.motions[0];
    let arc = record.arc.as_ref().expect("arc");
    assert_eq!(record.plane, Plane::Yz);
    assert_eq!(arc.center_mm, [0.0, 5.0, 0.0]);
    assert!(close(arc.start_angle_rad, math::PI, 1e-12));
    assert!(close(arc.sweep_rad, -math::PI, 1e-12));
    assert_eq!(record.kind, MoveKind::ArcCw);
}

#[test]
fn full_circles_are_exactly_one_turn() {
    let program = parse_program("G2 I5 J0 F100").expect("full circle");
    let record = &program.motions[0];
    assert_eq!(record.kind, MoveKind::ArcCw);
    assert_eq!(record.start_mm, record.end_mm);
    let arc = record.arc.as_ref().expect("arc");
    assert!(close(arc.sweep_rad, -math::TAU, 1e-12));
    assert!(close(arc.radius_mm, 5.0, 1e-12));

    let counter = parse_program("G3 I5 J0 F100").expect("full circle");
    let arc = counter.motions[0].arc.as_ref().expect("arc");
    assert!(close(arc.sweep_rad, math::TAU, 1e-12));
}

#[test]
fn r_format_arcs_are_recorded_as_radius_programming() {
    let program = parse_program("G2 X6 Y0 R5 F100").expect("r arc");
    let record = &program.motions[0];
    let arc = record.arc.as_ref().expect("arc");
    assert_eq!(arc.format, ArcFormat::Radius);
    assert!(close(arc.radius_mm, 5.0, 1e-12));
    assert!(close(arc.end_radius_mm, 5.0, 1e-12));
    assert!(arc.sweep_rad < 0.0);
    assert!(math::abs(arc.sweep_rad) < math::PI);
    assert_eq!(record.kind, MoveKind::ArcCw);

    let counter = parse_program("G3 X6 Y0 R5 F100").expect("ccw r arc");
    let arc = counter.motions[0].arc.as_ref().expect("arc");
    assert!(arc.sweep_rad > 0.0);
    assert!(math::abs(arc.sweep_rad) < math::PI);

    let major = parse_program("G2 X6 Y0 R-5 F100").expect("major r arc");
    let arc = major.motions[0].arc.as_ref().expect("arc");
    assert!(arc.sweep_rad < 0.0);
    assert!(math::abs(arc.sweep_rad) > math::PI);
    assert!(close(arc.center_mm[1], 4.0, 1e-12));
}

#[test]
fn helical_arcs_keep_the_axial_delta() {
    let program = parse_program("G2 X10 Y0 Z-2 I5 J0 F100").expect("helix");
    let record = &program.motions[0];
    assert_eq!(record.kind, MoveKind::HelixCw);
    let arc = record.arc.as_ref().expect("arc");
    assert_eq!(arc.axial_delta_mm, -2.0);
    assert!(close(arc.sweep_rad, -math::PI, 1e-12));

    let g18 = parse_program("G18\nG3 X0 Z10 Y2 I0 K5 F100").expect("g18 helix");
    let record = &g18.motions[0];
    assert_eq!(record.kind, MoveKind::HelixCcw);
    let arc = record.arc.as_ref().expect("arc");
    assert_eq!(arc.axial_delta_mm, 2.0);
}

#[test]
fn canned_cycles_store_one_record_per_hole() {
    let program =
        parse_program("G0 Z5\nG81 Z-5 R2 F100\nX10\nG82 Z-5 R2 P0.25 F100\n").expect("cycles");
    assert_eq!(program.motions.len(), 4);
    assert_eq!(program.motions[0].kind, MoveKind::Retract);
    assert_eq!(program.motions[1].kind, MoveKind::Drill);
    assert_eq!(program.motions[2].kind, MoveKind::Drill);
    assert_eq!(program.motions[3].kind, MoveKind::Drill);

    let first = program.motions[1].canned.as_ref().expect("canned");
    assert_eq!(first.cycle, CannedCycle::Drill);
    assert_eq!(first.hole_index, 0);
    assert_eq!(first.r_plane_mm, 2.0);
    assert_eq!(first.bottom_z_mm, -5.0);
    assert_eq!(first.retract_mode, RetractMode::Initial);
    assert_eq!(first.dwell_s, None);

    let second = program.motions[2].canned.as_ref().expect("canned");
    assert_eq!(second.hole_index, 1);
    assert_eq!(second.cycle, CannedCycle::Drill);

    let dwell = program.motions[3].canned.as_ref().expect("canned");
    assert_eq!(dwell.cycle, CannedCycle::DrillDwell);
    assert_eq!(dwell.dwell_s, Some(0.25));
}

#[test]
fn cycle_families_map_to_drill_peck_and_bore() {
    for (line, expected) in [
        ("G81 Z-2 R1 F100", MoveKind::Drill),
        ("G82 Z-2 R1 P0.1 F100", MoveKind::Drill),
        ("G73 Z-2 R1 Q0.5 F100", MoveKind::Peck),
        ("G83 Z-2 R1 Q0.5 F100", MoveKind::Peck),
        ("G85 Z-2 R1 F100", MoveKind::Bore),
        ("G86 Z-2 R1 P0.1 F100", MoveKind::Bore),
        ("G89 Z-2 R1 P0.1 F100", MoveKind::Bore),
    ] {
        let program = parse_program(&format!("G0 Z5\n{line}\n")).expect(line);
        assert_eq!(program.motions[1].kind, expected, "{line}");
        assert_eq!(
            program.motions[1]
                .canned
                .as_ref()
                .expect("canned")
                .cycle
                .g_code(),
            line.split_whitespace().next().expect("code")
        );
    }
}

#[test]
fn g38_probe_records_are_tagged() {
    let program = parse_program("G0 Z0\nG38.2 Z-5 F50").expect("probe");
    assert_eq!(program.motions[0].kind, MoveKind::Probe);
    assert_eq!(program.motions[0].start_mm, [0.0, 0.0, 0.0]);
    assert_eq!(program.motions[0].end_mm, [0.0, 0.0, -5.0]);
    assert!(program.motions[0].arc.is_none());
    assert!(program.motions[0].canned.is_none());
}

#[test]
fn annotations_capture_dwell_tool_spindle_and_coolant() {
    let program = parse_program("G4 P1.5\nT2 M6\nS12000 M3\nM8\nM5 M9\n").expect("events");
    let kinds: Vec<&AnnotationKind> = program
        .annotations
        .iter()
        .map(|annotation| &annotation.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            &AnnotationKind::Dwell { seconds: 1.5 },
            &AnnotationKind::ToolChange { tool: 2 },
            &AnnotationKind::Spindle {
                on: true,
                rpm: 12000.0,
                clockwise: true,
            },
            &AnnotationKind::Coolant { on: true },
            &AnnotationKind::Spindle {
                on: false,
                rpm: 12000.0,
                clockwise: false,
            },
            &AnnotationKind::Coolant { on: false },
        ]
    );
    assert_eq!(program.annotations[0].line, 1);
    assert_eq!(program.annotations[0].next_motion_index, 0);
    assert_eq!(program.annotations[1].pos_mm, [0.0; 3]);
}

#[test]
fn fanuc_dwell_p_is_milliseconds() {
    let generic = parse_program("G4 P1000").expect("generic");
    assert_eq!(
        generic.annotations[0].kind,
        AnnotationKind::Dwell { seconds: 1000.0 }
    );

    let config = ParserConfig::default().with_dialect(Dialect::Fanuc);
    let fanuc = parse_program_with("G4 P1000", &config).expect("fanuc");
    assert_eq!(
        fanuc.annotations[0].kind,
        AnnotationKind::Dwell { seconds: 1.0 }
    );
}

#[test]
fn modal_states_are_interned_and_records_stay_small() {
    let program = parse_program("G21 G90\nG0 X1\nG0 X2\nG91\nG0 X1\nG90\nG0 X4\n").expect("modal");
    assert_eq!(program.modal_table.len(), 2);
    assert_eq!(program.motions[0].modal, program.motions[1].modal);
    assert_ne!(program.motions[1].modal, program.motions[2].modal);
    assert_eq!(program.motions[3].modal, program.motions[0].modal);
    assert_eq!(
        program.modal(program.motions[2].modal).distance,
        Some(nc_gcode::DistanceMode::Incremental)
    );
    assert!(std::mem::size_of::<nc_gcode::MotionRecord>() <= 128);
}

#[test]
fn g93_feed_mode_is_recorded_and_flagged() {
    let program = parse_program("G93\nG1 X10 F0.5").expect("g93");
    let record = &program.motions[0];
    assert_eq!(record.feed_mm_min, None);
    assert_eq!(program.modal(record.modal).feed_mode, FeedMode::InverseTime);
}

#[test]
fn wide_tool_numbers_survive_in_records() {
    let program = parse_program("T4294967295\nG0 X1").expect("tool");
    assert_eq!(program.motions[0].tool, u32::MAX);
    assert_eq!(program.stats.tools, vec![u32::MAX]);
}

#[test]
fn cutter_compensation_is_recorded_but_geometry_continues_on_the_centerline() {
    let program = parse_program("G0 X1\nG41 D1 G1 X2 F100\nG0 X3\nG42 D1 G1 X4 F100\nG40\nG0 X5")
        .expect("comp");
    assert_eq!(program.geometry_uncertain_after(), None);
    assert_eq!(program.motions.len(), 5);
    let compensation: Vec<CutterComp> = program
        .motions
        .iter()
        .map(|motion| program.modal(motion.modal).cutter_comp)
        .collect();
    assert_eq!(
        compensation,
        vec![
            CutterComp::Off,
            CutterComp::Left,
            CutterComp::Left,
            CutterComp::Right,
            CutterComp::Off,
        ]
    );
    assert_eq!(program.motions[1].end_mm, [2.0, 0.0, 0.0]);
    assert_eq!(program.motions[2].end_mm, [3.0, 0.0, 0.0]);
    assert_eq!(program.motions[3].end_mm, [4.0, 0.0, 0.0]);
    assert_eq!(program.motions[4].end_mm, [5.0, 0.0, 0.0]);
    assert_eq!(program.state.cutter_comp, CutterComp::Off);
    assert!(
        program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::CutterCompNotApplied)
    );
}

#[test]
fn records_carry_feed_spindle_tool_and_plane_state() {
    let program =
        parse_program("G21 G90\nT2 M6\nS12000 M3\nG1 X10 F300\nM5\nG0 X20").expect("state");
    let cutting = &program.motions[0];
    assert_eq!(cutting.feed_mm_min, Some(300.0));
    assert_eq!(cutting.spindle_rpm, Some(12000.0));
    assert_eq!(cutting.spindle_cw, Some(true));
    assert_eq!(cutting.tool, 2);
    assert_eq!(cutting.plane, Plane::Xy);

    let stopped = &program.motions[1];
    assert_eq!(stopped.kind, MoveKind::Rapid);
    assert_eq!(stopped.spindle_rpm, Some(12000.0));
    assert_eq!(stopped.spindle_cw, None);
    assert_eq!(stopped.tool, 2);
    assert_eq!(stopped.feed_mm_min, Some(300.0));
}

#[test]
fn straight_kinds_are_plane_independent_and_records_carry_the_plane() {
    for (plane_code, plane) in [("G17", Plane::Xy), ("G18", Plane::Zx), ("G19", Plane::Yz)] {
        let program = parse_program(&format!("{plane_code}\nG0 Z5\nG0 X1\nG1 Z-1 F100\nG1 X2\n"))
            .expect("valid");
        let kinds: Vec<MoveKind> = program.motions.iter().map(|motion| motion.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MoveKind::Retract,
                MoveKind::Rapid,
                MoveKind::Plunge,
                MoveKind::Feed,
            ],
            "{plane_code}"
        );
        assert!(
            program.motions.iter().all(|motion| motion.plane == plane),
            "{plane_code}"
        );
    }
}

#[test]
fn parsing_twice_produces_identical_records() {
    let text = "G21 G90\nG0 Z5\nG1 Z-1 F300\nG2 X10 Y0 Z-3 I5 J0\nG83 X20 Y0 Z-5 R2 Q1 F100\nT2 M6\nM3 S12000\n(6mm flat)\n";
    let first = parse_program(text).expect("first");
    let second = parse_program(text).expect("second");
    assert_eq!(first.motions, second.motions);
    assert_eq!(first.annotations, second.annotations);
    assert_eq!(first.comments, second.comments);
    assert_eq!(first.tool_hints, second.tool_hints);
    assert_eq!(first.modal_table, second.modal_table);
    assert_eq!(
        first.expanded_motions().collect::<Vec<_>>(),
        second.expanded_motions().collect::<Vec<_>>()
    );
}

#[test]
fn cutting_bounds_use_semantic_records_and_exact_arcs() {
    let arc = parse_program("G0 X0 Y0 Z1\nG1 Z0 F100\nG2 X10 Y0 I5 J0").expect("arc");
    let bounds = arc.cutting_bounds();
    assert!(close(f64::from(bounds.min().z), 0.0, 1e-6));
    assert!(close(f64::from(bounds.max().y), 5.0, 1e-6));
    assert!(close(f64::from(bounds.max().x), 10.0, 1e-6));

    let drill = parse_program("G0 Z5\nG81 Z-5 R2 F100").expect("drill");
    let bounds = drill.cutting_bounds();
    assert!(close(f64::from(bounds.min().z), -5.0, 1e-6));
    // The safe-Z rapid and the initial retract level do not inflate cutting
    // bounds; the drill feed starts at the R plane.
    assert!(close(f64::from(bounds.max().z), 2.0, 1e-6));
}
