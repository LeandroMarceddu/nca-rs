//! Lazy canned-cycle expansion: peck increments, retract modes, and boring.

use nc_gcode::{CannedCycle, MoveKind, RetractMode, parse_program};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

#[test]
fn g81_expands_to_clearance_feed_and_retract() {
    let program = parse_program("G0 Z5\nG81 Z-5 R2 F100").expect("g81");
    let moves: Vec<_> = program.expanded_motions().skip(1).collect();
    assert_eq!(moves.len(), 3);
    assert_eq!(moves[0].kind, MoveKind::Rapid);
    assert_eq!(moves[0].start_mm, [0.0, 0.0, 5.0]);
    assert_eq!(moves[0].end_mm, [0.0, 0.0, 2.0]);
    assert_eq!(moves[1].kind, MoveKind::Drill);
    assert_eq!(moves[1].start_mm, [0.0, 0.0, 2.0]);
    assert_eq!(moves[1].end_mm, [0.0, 0.0, -5.0]);
    assert_eq!(moves[2].kind, MoveKind::Retract);
    assert_eq!(moves[2].end_mm, [0.0, 0.0, 5.0]);
    assert!(moves.iter().all(|motion| motion.canned.is_none()));
}

#[test]
fn g99_retracts_to_the_r_plane() {
    let program = parse_program("G0 Z5\nG99 G81 Z-5 R2 F100").expect("g99");
    let moves: Vec<_> = program.expanded_motions().skip(1).collect();
    assert_eq!(moves.len(), 3);
    assert_eq!(moves[0].kind, MoveKind::Rapid);
    assert_eq!(moves[0].end_mm, [0.0, 0.0, 2.0]);
    assert_eq!(moves[1].kind, MoveKind::Drill);
    assert_eq!(moves[1].end_mm, [0.0, 0.0, -5.0]);
    assert_eq!(moves[2].kind, MoveKind::Retract);
    assert_eq!(
        program.motions[1]
            .canned
            .as_ref()
            .expect("canned")
            .retract_mode,
        RetractMode::RPlane
    );
}

#[test]
fn repeated_holes_travel_at_the_initial_plane_for_g98() {
    let program = parse_program("G0 Z5\nG81 Z-5 R2 F100\nX10").expect("repeat");
    let moves: Vec<_> = program.expanded_motions().skip(1).collect();
    // First hole: down to R, drill, retract. Second hole: travel at initial Z,
    // down to R, drill, retract.
    assert_eq!(moves.len(), 7);
    assert_eq!(moves[1].kind, MoveKind::Drill);
    assert_eq!(moves[2].kind, MoveKind::Retract);
    assert_eq!(moves[2].end_mm, [0.0, 0.0, 5.0]);
    assert_eq!(moves[3].kind, MoveKind::Rapid);
    assert_eq!(moves[3].start_mm, [0.0, 0.0, 5.0]);
    assert_eq!(moves[3].end_mm, [10.0, 0.0, 5.0]);
    assert_eq!(moves[4].kind, MoveKind::Rapid);
    assert_eq!(moves[4].end_mm, [10.0, 0.0, 2.0]);
    assert_eq!(moves[5].kind, MoveKind::Drill);
    assert_eq!(moves[5].end_mm, [10.0, 0.0, -5.0]);
    assert_eq!(moves[6].kind, MoveKind::Retract);
}

#[test]
fn g83_pecks_in_q_increments_with_full_retracts() {
    let program = parse_program("G0 Z0\nG83 Z-3 R-1 Q1 F100").expect("g83");
    let moves: Vec<_> = program.expanded_motions().collect();
    let feeds: Vec<_> = moves
        .iter()
        .filter(|motion| motion.kind == MoveKind::Peck)
        .collect();
    assert_eq!(feeds.len(), 2);
    assert!(close(feeds[0].start_mm[2], -1.0));
    assert!(close(feeds[0].end_mm[2], -2.0));
    assert!(close(feeds[1].start_mm[2], -1.0));
    assert!(close(feeds[1].end_mm[2], -3.0));

    let rapids: Vec<_> = moves
        .iter()
        .filter(|motion| motion.kind == MoveKind::Rapid)
        .collect();
    // Only the positioning rapid to R is a rapid; the full retract is a
    // Retract and the next peck feeds back down from R.
    assert_eq!(rapids.len(), 1);
    assert!(close(rapids[0].end_mm[2], -1.0));
    assert_eq!(moves.last().expect("last").kind, MoveKind::Retract);
}

#[test]
fn g83_upward_returns_are_retracts_and_rapids_stay_above_r() {
    let program = parse_program("G0 Z5\nG83 Z-8 R2 Q2 F100").expect("g83");
    let moves: Vec<_> = program.expanded_motions().skip(1).collect();
    assert!(!moves.is_empty());
    for motion in &moves {
        if motion.end_mm[2] > motion.start_mm[2] {
            assert_eq!(motion.kind, MoveKind::Retract, "{motion:?}");
        }
        if motion.kind == MoveKind::Rapid {
            assert!(
                motion.end_mm[2] >= 2.0 - 1e-9,
                "rapid ends below R: {motion:?}"
            );
        }
    }
    assert!(moves.iter().any(|motion| motion.kind == MoveKind::Peck));
    assert_eq!(moves.last().expect("last").kind, MoveKind::Retract);
    assert!(close(moves.last().expect("last").end_mm[2], 5.0));
}

#[test]
fn g73_pecks_with_a_chip_break_retract() {
    let program = parse_program("G0 Z0\nG73 Z-5 R-2.5 Q1 F100").expect("g73");
    let moves: Vec<_> = program.expanded_motions().collect();
    let feeds: Vec<_> = moves
        .iter()
        .filter(|motion| motion.kind == MoveKind::Peck)
        .collect();
    assert_eq!(feeds.len(), 3);
    for (feed, expected_end) in feeds.iter().zip([-3.5, -4.5, -5.0]) {
        assert!(close(feed.end_mm[2], expected_end));
    }
    let retract = moves.last().expect("last");
    assert_eq!(retract.kind, MoveKind::Retract);
    assert!(close(retract.end_mm[2], 0.0));
    // Chip-break returns are Retract moves, never rapids at depth.
    let upward: Vec<_> = moves
        .iter()
        .filter(|motion| motion.end_mm[2] > motion.start_mm[2])
        .collect();
    assert_eq!(upward.len(), 3);
    assert!(upward.iter().all(|motion| motion.kind == MoveKind::Retract));
}

#[test]
fn boring_cycles_feed_or_rapid_out() {
    let g85 = parse_program("G0 Z5\nG85 Z-2 R1 F100").expect("g85");
    let moves: Vec<_> = g85.expanded_motions().skip(1).collect();
    assert_eq!(moves.len(), 4);
    assert_eq!(moves[1].kind, MoveKind::Bore);
    assert_eq!(moves[2].kind, MoveKind::Bore);
    assert_eq!(moves[2].start_mm[2], -2.0);
    assert_eq!(moves[2].end_mm[2], 1.0);
    assert_eq!(moves[3].kind, MoveKind::Retract);
    assert_eq!(moves[3].end_mm[2], 5.0);

    let g86 = parse_program("G0 Z5\nG86 Z-2 R1 P0.2 F100").expect("g86");
    let moves: Vec<_> = g86.expanded_motions().skip(1).collect();
    assert_eq!(moves[1].kind, MoveKind::Bore);
    assert_eq!(moves[2].kind, MoveKind::Retract);
    assert_eq!(moves[2].end_mm[2], 1.0);
    let canned = g86.motions[1].canned.as_ref().expect("canned");
    assert_eq!(canned.cycle, CannedCycle::BoreStop);
    assert_eq!(canned.dwell_s, Some(0.2));

    let g89 = parse_program("G0 Z5\nG89 Z-2 R1 P0.2 F100").expect("g89");
    let moves: Vec<_> = g89.expanded_motions().skip(1).collect();
    assert_eq!(moves[1].kind, MoveKind::Bore);
    assert_eq!(moves[2].kind, MoveKind::Bore);
    let canned = g89.motions[1].canned.as_ref().expect("canned");
    assert_eq!(canned.cycle, CannedCycle::BoreDwell);
}

#[test]
fn g82_expands_like_g81_and_keeps_the_dwell() {
    let program = parse_program("G0 Z5\nG82 Z-5 R2 P0.5 F100").expect("g82");
    let moves: Vec<_> = program.expanded_motions().skip(1).collect();
    assert_eq!(moves.len(), 3);
    assert_eq!(moves[1].kind, MoveKind::Drill);
    let canned = program.motions[1].canned.as_ref().expect("canned");
    assert_eq!(canned.cycle, CannedCycle::DrillDwell);
    assert_eq!(canned.dwell_s, Some(0.5));
}

#[test]
fn expansion_does_not_allocate_stored_sub_moves() {
    let program = parse_program("G0 Z5\nG81 Z-5 R2 F100\nX10\nX20").expect("repeat");
    assert_eq!(program.motions.len(), 4);
    assert!(
        program
            .motions
            .iter()
            .all(|motion| motion.canned.is_some() || motion.kind == MoveKind::Retract)
    );
    assert_eq!(program.expanded_motions().count(), 12);
}
