//! Dialect detection: vendor evidence from program text.

use std::time::Instant;

use nc_core::Dialect;
use nc_gcode::{DialectDetection, detect_dialect};

fn detection(source: &str) -> DialectDetection {
    detect_dialect(source).expect("expected dialect evidence")
}

#[test]
fn the_latchbase_post_is_makera_on_its_first_m851() {
    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/real/makera-latchbase/source.nc"
    );
    let text = std::fs::read_to_string(FIXTURE).expect("read latchbase fixture");
    let detected = detection(&text);
    assert_eq!(detected.dialect, Dialect::Makera);
    // The code marker wins over the header text, and M851 first appears here.
    assert_eq!(detected.evidence.line, 33);
    assert_eq!(detected.evidence.marker, "M851");
    assert_eq!(detected.evidence.text, "M851 S100");
}

#[test]
fn makera_vendor_header_reports_the_first_header_line() {
    let source = "\
(LatchBase)
(Machine)
(  vendor: Makera)
(  model: Z1 3 axis)

G90 G94
G17
G21

T5M6
M400
G54
G0 X16.7 Y16.5
";
    let detected = detection(source);
    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 3);
    assert_eq!(detected.evidence.marker, "makera-header");
    assert_eq!(detected.evidence.text, "(  vendor: Makera)");
}

#[test]
fn generated_with_makera_utility_is_makera() {
    let source = "\
(Generated with Makera Community Post Process Utility version 0.9.1-RC1)
G21
G0 X0 Y0
";
    let detected = detection(source);
    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 1);
    assert_eq!(detected.evidence.marker, "makera-header");
    assert_eq!(
        detected.evidence.text,
        "(Generated with Makera Community Post Process Utility version 0.9.1-RC1)"
    );
}

#[test]
fn makera_model_and_carvera_headers_are_recognized() {
    for source in [
        "(  model: Z1 3 axis)\nG21\nG0 X0\n",
        "(Carvera Air)\nG21\nG0 X0\n",
        "(Generated with Community Post Process Utility)\nG21\n",
    ] {
        assert_eq!(detection(source).dialect, Dialect::Makera, "{source}");
    }
}

#[test]
fn an_m851_word_is_makera_even_without_a_header() {
    let source = "G21 G90\nM851 S100\nG1 X10 F200\n";
    let detected = detection(source);
    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 2);
    assert_eq!(detected.evidence.marker, "M851");
    assert_eq!(detected.evidence.text, "M851 S100");
}

#[test]
fn an_m852_word_is_makera_and_compact_words_work() {
    for (source, marker, line) in [
        ("m852\n", "M852", 1),
        ("G0 X0\nm851s100\n", "M851", 2),
        ("M851.0 S100\n", "M851", 1),
    ] {
        let detected = detection(source);
        assert_eq!(detected.dialect, Dialect::Makera, "{source}");
        assert_eq!(detected.evidence.marker, marker, "{source}");
        assert_eq!(detected.evidence.line, line, "{source}");
    }
}

#[test]
fn commented_codes_and_m400_are_not_makera() {
    let source = "\
(M851 S100)
; M852
G21 G90
M400
M8510
#<m851>=1
M30
";
    assert!(detect_dialect(source).is_none());
}

#[test]
fn header_comments_map_to_each_dialect() {
    for (source, dialect, marker) in [
        ("(grbl)\nG21\nG0 X0\n", Dialect::Grbl, "grbl-header"),
        ("(grblHAL 1.1)\nG21\n", Dialect::Grbl, "grbl-header"),
        ("(linuxcnc 2.9)\n", Dialect::LinuxCnc, "linuxcnc-header"),
        ("(Linux CNC)\n", Dialect::LinuxCnc, "linuxcnc-header"),
        ("(Mach3)\n", Dialect::Mach3, "mach3-header"),
        ("; mach4 post\n", Dialect::Mach3, "mach3-header"),
        ("(Fanuc 0i)\n", Dialect::Fanuc, "fanuc-header"),
    ] {
        let detected = detection(source);
        assert_eq!(detected.dialect, dialect, "{source}");
        assert_eq!(detected.evidence.marker, marker, "{source}");
        assert_eq!(detected.evidence.line, 1, "{source}");
    }
}

#[test]
fn conflicting_header_dialects_are_ambiguous() {
    let source = "(grbl)\n(mach3)\nG21\nG0 X0\n";
    assert!(detect_dialect(source).is_none());

    let same_line = "(grbl and mach3)\nG21\n";
    assert!(detect_dialect(same_line).is_none());
}

#[test]
fn repeating_the_same_header_dialect_keeps_the_first_hit() {
    let source = "(grbl)\n(grblHAL)\nG21\n";
    let detected = detection(source);
    assert_eq!(detected.dialect, Dialect::Grbl);
    assert_eq!(detected.evidence.line, 1);
    assert_eq!(detected.evidence.text, "(grbl)");
}

#[test]
fn a_clean_generic_program_has_no_vendor_evidence() {
    let source = "\
(Fusion 360)
G21 G90
G0 X0 Y0 Z5
G1 Z-1 F100
G1 X10
M30
";
    assert!(detect_dialect(source).is_none());
}

#[test]
fn header_text_after_the_first_motion_is_ignored() {
    let source = "G21 G90\nG0 X0 Y0\n(vendor: Makera)\nG1 X10 F100\n";
    assert!(detect_dialect(source).is_none());
}

#[test]
fn comments_before_a_motion_on_the_same_line_still_count() {
    let detected = detection("(grbl) G0 X0\n");
    assert_eq!(detected.dialect, Dialect::Grbl);
    assert_eq!(detected.evidence.line, 1);

    // A comment after the motion word is not header evidence.
    assert!(detect_dialect("G0 X0 (grbl)\n").is_none());
}

#[test]
fn a_code_marker_wins_over_header_text_and_header_conflicts() {
    let source = "(grbl)\n(mach3)\nG0 X0\nM851 S100\n";
    let detected = detection(source);
    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 4);
    assert_eq!(detected.evidence.marker, "M851");
}

#[test]
fn evidence_line_text_and_determinism_are_stable() {
    let source = "(LatchBase)\r\n(  vendor: Makera)\r\nG21\r\nG0 X0\r\n";
    let first = detection(source);
    let second = detection(source);
    assert_eq!(first, second);
    assert_eq!(first.dialect, Dialect::Makera);
    assert_eq!(first.evidence.line, 2);
    assert_eq!(first.evidence.text, "(  vendor: Makera)");
    assert!(!first.evidence.text.contains('\r'));

    let long = format!("(vendor: Makera {})\nG0 X0\n", "\u{4e2d}".repeat(200));
    let detected = detection(&long);
    assert!(detected.evidence.text.len() <= 120);
    assert!(detected.evidence.text.starts_with("(vendor: Makera"));
}

#[test]
fn a_million_lines_with_an_early_marker_stay_fast() {
    let mut source = String::with_capacity(8 * 1024 * 1024);
    source.push_str("(vendor: Makera)\nG0 X0\n");
    for _ in 0..1_000_000 {
        source.push_str("G1 X1 Y1\n");
    }

    let started = Instant::now();
    let detected = detection(&source);
    let elapsed = started.elapsed();

    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 1);
    assert!(elapsed.as_secs_f64() < 2.0, "detection took {elapsed:?}");
}

#[test]
fn a_strong_code_marker_stops_the_scan_early() {
    let mut source = String::with_capacity(8 * 1024 * 1024);
    source.push_str("M851 S100\n");
    for _ in 0..1_000_000 {
        source.push_str("(grbl) G1 X1 Y1\n");
    }

    let started = Instant::now();
    let detected = detection(&source);
    let elapsed = started.elapsed();

    assert_eq!(detected.dialect, Dialect::Makera);
    assert_eq!(detected.evidence.line, 1);
    assert!(elapsed.as_secs_f64() < 2.0, "detection took {elapsed:?}");
}
