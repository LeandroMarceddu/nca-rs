//! The display toolpath and the motion records must agree on every semantic
//! kind, including arc/helix chords and canned-cycle cutting sub-moves.

use std::collections::BTreeSet;

use nc_gcode::{MoveKind, parse_program};

/// One program that exercises every kind in the taxonomy.
const REPRESENTATIVE: &str = "\
G21 G90
G0 Z5
G0 X10 Y0
G1 Z0 F100
G1 X20
G1 X30 Z-2
G1 Z5
G0 X0 Y0
G2 X10 Y0 I5 J0 F100
G3 X0 Y0 I-5 J0
G2 X10 Y0 Z-1 I5 J0
G3 X0 Y0 Z0 I-5 J0
G0 Z5
G81 X20 Y20 Z-5 R2 F100
G83 X30 Y20 Z-5 R2 Q1
G85 X40 Y20 Z-5 R2
G0 Z5
G38.2 Z0 F50
";

#[test]
fn every_kind_appears_in_both_streams() {
    let program = parse_program(REPRESENTATIVE).expect("valid program");
    let display: BTreeSet<MoveKind> = program
        .toolpath
        .segments()
        .iter()
        .map(|segment| segment.kind)
        .collect();
    let records: BTreeSet<MoveKind> = program.motions.iter().map(|motion| motion.kind).collect();

    assert_eq!(display.len(), MoveKind::ALL.len(), "display: {display:?}");
    assert_eq!(records.len(), MoveKind::ALL.len(), "records: {records:?}");
    assert_eq!(display, records);
    for kind in MoveKind::ALL {
        assert!(display.contains(&kind), "missing display kind {kind:?}");
        assert!(records.contains(&kind), "missing record kind {kind:?}");
    }
}

#[test]
fn each_line_display_kind_matches_its_record() {
    let program = parse_program(REPRESENTATIVE).expect("valid program");
    for motion in &program.motions {
        let line = program
            .lines
            .iter()
            .find(|line| line.line == motion.line)
            .expect("record line");
        let kinds: BTreeSet<MoveKind> = program.toolpath.segments()[line.segments.clone()]
            .iter()
            .map(|segment| segment.kind)
            .collect();
        assert!(
            kinds.contains(&motion.kind),
            "line {} display {kinds:?} is missing record kind {:?}",
            motion.line,
            motion.kind
        );
        if motion.canned.is_some() {
            assert!(
                kinds
                    .iter()
                    .all(|kind| *kind == motion.kind || kind.is_rapid()),
                "line {} canned display {kinds:?} has an unrelated cutting kind",
                motion.line
            );
        } else {
            assert_eq!(
                kinds,
                BTreeSet::from([motion.kind]),
                "line {} has one record but several display kinds",
                motion.line
            );
        }
    }
}

#[test]
fn arc_chords_keep_the_arc_or_helix_kind() {
    let program = parse_program("G0 X0 Y0\nG2 X10 Y0 Z-1 I5 J0 F100\n").expect("helix");
    let segments = program.toolpath.segments();
    assert!(segments.len() > 4);
    assert!(
        segments
            .iter()
            .all(|segment| segment.kind == MoveKind::HelixCw)
    );
    assert_eq!(program.motions[0].kind, MoveKind::HelixCw);

    let program = parse_program("G0 X0 Y0\nG3 X10 Y0 I5 J0 F100\n").expect("arc");
    assert!(
        program
            .toolpath
            .segments()
            .iter()
            .all(|segment| segment.kind == MoveKind::ArcCcw)
    );
    assert_eq!(program.motions[0].kind, MoveKind::ArcCcw);
}

#[test]
fn canned_cutting_sub_moves_keep_the_cycle_kind() {
    for (line, expected) in [
        ("G81 X0 Y0 Z-2 R1 F100", MoveKind::Drill),
        ("G73 X0 Y0 Z-2 R1 Q1 F100", MoveKind::Peck),
        ("G83 X0 Y0 Z-2 R1 Q1 F100", MoveKind::Peck),
        ("G85 X0 Y0 Z-2 R1 F100", MoveKind::Bore),
        ("G86 X0 Y0 Z-2 R1 P0.1 F100", MoveKind::Bore),
        ("G89 X0 Y0 Z-2 R1 P0.1 F100", MoveKind::Bore),
    ] {
        let program = parse_program(&format!("G0 Z5\n{line}\n")).expect(line);
        let kinds: Vec<MoveKind> = program
            .toolpath
            .segments()
            .iter()
            .map(|segment| segment.kind)
            .collect();
        assert!(
            kinds.contains(&expected),
            "{line} cutting sub-moves are {kinds:?}, expected {expected:?}"
        );
        assert!(
            kinds
                .iter()
                .all(|kind| *kind == expected || kind.is_rapid()),
            "{line} non-cutting sub-moves are not rapid/retract: {kinds:?}"
        );
        assert_eq!(program.motions[1].kind, expected, "{line}");
    }
}
