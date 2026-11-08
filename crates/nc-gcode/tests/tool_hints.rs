//! Comment retention and conservative tool-hint extraction.

use nc_core::{FieldSource, Tool, ToolField, ToolShape, ToolTable};
use nc_gcode::{ParserConfig, WarningKind, parse_program, parse_program_lossy, parse_program_with};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

#[test]
fn comments_are_retained_with_their_line() {
    let program = parse_program("(header)\nG0 X1 (side note)\n; trailing\n").expect("valid");
    let texts: Vec<(&usize, &str)> = program
        .comments
        .iter()
        .map(|comment| (&comment.line, comment.text.as_str()))
        .collect();
    assert_eq!(
        texts,
        vec![(&1, "header"), (&2, "side note"), (&3, "trailing")]
    );
}

#[test]
fn header_comment_extracts_a_complete_hint() {
    let program = parse_program("T1 (D=6mm 2F flat endmill)\nG0 X0").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    let hint = &program.tool_hints[0];
    assert_eq!(hint.tool, 1);
    assert_eq!(hint.line, 1);
    assert!(close(hint.diameter_mm.expect("diameter"), 6.0));
    assert_eq!(hint.flutes, Some(2));
    assert_eq!(hint.shape, Some(ToolShape::Flat));
    assert!(
        hint.name
            .as_deref()
            .is_some_and(|name| name.contains("endmill"))
    );
}

#[test]
fn declared_units_disambiguate_a_bare_diameter() {
    let program = parse_program("G21\nT2 (D=6 3FL ball)\nG0 X0").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    let hint = &program.tool_hints[0];
    assert_eq!(hint.tool, 2);
    assert!(close(hint.diameter_mm.expect("diameter"), 6.0));
    assert_eq!(hint.flutes, Some(3));
    assert_eq!(hint.shape, Some(ToolShape::Ball));
    assert!(
        program
            .warnings
            .iter()
            .all(|warning| warning.kind != WarningKind::AmbiguousToolHint)
    );

    let inch = parse_program("G20\nT2 (D=0.25 2F flat)\nG0 X0").expect("valid");
    assert!(close(
        inch.tool_hints[0].diameter_mm.expect("diameter"),
        6.35
    ));
}

#[test]
fn bare_diameter_without_units_is_ambiguous_and_dropped() {
    let program = parse_program("T1 (D=6 2F flat)\nG0 X0").expect("valid");
    assert!(program.tool_hints.is_empty());
    assert!(
        program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::AmbiguousToolHint)
    );
}

#[test]
fn self_declared_units_are_recognized() {
    let metric = parse_program("T1 (DIA 3.175mm 2F)\nG0 X0").expect("metric");
    assert!(close(
        metric.tool_hints[0].diameter_mm.expect("diameter"),
        3.175
    ));

    let inch = parse_program("T1 (1/4\" 2F flat)\nG0 X0").expect("fraction");
    assert!(close(
        inch.tool_hints[0].diameter_mm.expect("diameter"),
        6.35
    ));

    let thousandths = parse_program("T1 (0.125 in 2F flat)\nG0 X0").expect("inches");
    assert!(close(
        thousandths.tool_hints[0].diameter_mm.expect("diameter"),
        3.175
    ));

    let symbol = parse_program("T1 (Ø6mm 2F flat)\nG0 X0").expect("symbol");
    assert!(close(
        symbol.tool_hints[0].diameter_mm.expect("diameter"),
        6.0
    ));
}

#[test]
fn a_comment_can_name_its_tool() {
    let program = parse_program("(tool 3 D=8mm flat endmill)\nG0 X0\n").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    assert_eq!(program.tool_hints[0].tool, 3);
    assert!(close(
        program.tool_hints[0].diameter_mm.expect("diameter"),
        8.0
    ));
}

#[test]
fn header_units_may_be_declared_after_the_comment() {
    // D017: a Fusion-style header lists the tool table before G21.
    let program =
        parse_program("(T1  D=3.1 CR=0 - flat end mill)\nG90 G94\nG17\nG21\nG0 X0").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    let hint = &program.tool_hints[0];
    assert_eq!(hint.tool, 1);
    assert_eq!(hint.line, 1);
    assert!(close(hint.diameter_mm.expect("diameter"), 3.1));
    assert_eq!(hint.shape, Some(ToolShape::Flat));
}

#[test]
fn a_unit_after_the_first_motion_never_validates_a_header_hint() {
    let program = parse_program("(T1  D=3.1 flat end mill)\nG0 X5\nG21\nG0 X6").expect("valid");
    assert!(program.tool_hints.is_empty());
    assert!(
        program
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::AmbiguousToolHint)
    );

    // A commanded zero-length move is still the first motion.
    let zero = parse_program("(T1  D=3.1 flat end mill)\nG0 X0\nG21\nG0 X1").expect("valid");
    assert!(zero.tool_hints.is_empty());
}

#[test]
fn comments_outside_the_window_are_not_scanned() {
    let text = "T1\nG0 X0\nG0 X1\n(D=4mm 2F flat)\n";
    let program = parse_program(text).expect("default window");
    assert_eq!(program.tool_hints.len(), 1);

    let config = ParserConfig {
        tool_hint_window: 0,
        ..ParserConfig::default()
    };
    let strict = parse_program_with(text, &config).expect("zero window");
    assert!(strict.tool_hints.is_empty());
}

#[test]
fn header_comments_without_a_tool_are_ignored() {
    let program = parse_program("(6 mm stock, 6 mm end mill)\nG0 X0\n").expect("valid");
    assert!(program.tool_hints.is_empty());
}

#[test]
fn unanchored_header_comment_attaches_to_the_first_tool_selected_later() {
    let program = parse_program("(clean pocket, 6 mm 2F flat)\nT1 M6\nG0 X0\n").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    let hint = &program.tool_hints[0];
    assert_eq!(hint.tool, 1);
    assert_eq!(hint.line, 1);
    assert!(close(hint.diameter_mm.expect("diameter"), 6.0));
    assert_eq!(hint.flutes, Some(2));
    assert_eq!(hint.shape, Some(ToolShape::Flat));

    let resolved = ToolTable::resolve(&ToolTable::new(), &program.tool_hints);
    let tool = resolved.get(1).expect("T1");
    assert!(close(tool.diameter_mm.expect("diameter"), 6.0));
    assert_eq!(tool.flutes, Some(2));
    assert_eq!(tool.shape, ToolShape::Flat);
}

#[test]
fn unanchored_header_comment_without_a_tool_is_dropped() {
    let program = parse_program("(clean pocket, 6 mm 2F flat)\nG0 X0\n").expect("valid");
    assert!(program.tool_hints.is_empty());
}

#[test]
fn unanchored_header_comment_attaches_to_the_tool_actually_selected() {
    let program = parse_program("(clean pocket, 6 mm 2F flat)\nT2 M6\nG0 X0\n").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    assert_eq!(program.tool_hints[0].tool, 2);
}

#[test]
fn a_tool_selected_after_the_first_motion_is_not_a_header_target() {
    let program =
        parse_program("(clean pocket, 6 mm 2F flat)\nG0 X0\nT1 M6\nG0 X1\n").expect("valid");
    assert!(program.tool_hints.is_empty());
}

#[test]
fn a_specific_tool_comment_wins_over_an_earlier_unanchored_header_hint() {
    let program =
        parse_program("(clean pocket, 6 mm 2F flat)\n(T1 D=3.1mm 4F ball)\nT1 M6\nG0 X0\n")
            .expect("valid");
    let resolved = ToolTable::resolve(&ToolTable::new(), &program.tool_hints);
    let tool = resolved.get(1).expect("T1");
    assert!(close(tool.diameter_mm.expect("diameter"), 3.1));
    assert_eq!(tool.flutes, Some(4));
    assert_eq!(tool.shape, ToolShape::Ball);
    assert_eq!(
        resolved.source_of(1, ToolField::Diameter),
        Some(FieldSource::Comment { line: 2 })
    );
}

#[test]
fn earlier_unanchored_header_comments_win_per_field() {
    let program =
        parse_program("(roughing, 6 mm 2F flat)\n(finishing, 3 mm 4F ball)\nT1 M6\nG0 X0\n")
            .expect("valid");
    let resolved = ToolTable::resolve(&ToolTable::new(), &program.tool_hints);
    let tool = resolved.get(1).expect("T1");
    assert!(close(tool.diameter_mm.expect("diameter"), 6.0));
    assert_eq!(tool.flutes, Some(2));
    assert_eq!(tool.shape, ToolShape::Flat);
    assert_eq!(
        resolved.source_of(1, ToolField::Diameter),
        Some(FieldSource::Comment { line: 1 })
    );
}

#[test]
fn unanchored_header_hint_uses_a_unit_declared_later_in_the_header() {
    // D017 applies to unanchored header hints too.
    let program = parse_program("(D=6 2F flat)\nT1 M6\nG21\nG0 X0\n").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    assert!(close(
        program.tool_hints[0].diameter_mm.expect("diameter"),
        6.0
    ));

    // A unit after the first motion never validates the hint.
    let late = parse_program("(D=6 2F flat)\nT1 M6\nG0 X0\nG21\nG0 X1\n").expect("valid");
    assert!(late.tool_hints.is_empty());
    assert!(
        late.warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::AmbiguousToolHint)
    );
}

#[test]
fn unanchored_header_hint_line_is_the_comment_line() {
    let program = parse_program("(clean pocket, 6 mm 2F flat)\n\nT1 M6\nG0 X0\n").expect("valid");
    assert_eq!(program.tool_hints.len(), 1);
    assert_eq!(program.tool_hints[0].line, 1);
}

#[test]
fn unanchored_header_hint_does_not_replace_user_configuration() {
    let program = parse_program("(clean pocket, 6 mm 2F flat)\nT1 M6\nG0 X0\n").expect("valid");
    let mut config = ToolTable::new();
    let mut tool = Tool::new(1);
    tool.diameter_mm = Some(10.0);
    tool.sources
        .insert(ToolField::Diameter, FieldSource::Config);
    config.insert(tool);

    let resolved = ToolTable::resolve(&config, &program.tool_hints);
    let tool = resolved.get(1).expect("T1");
    assert_eq!(tool.diameter_mm, Some(10.0));
    assert_eq!(
        resolved.source_of(1, ToolField::Diameter),
        Some(FieldSource::Config)
    );
    assert_eq!(tool.flutes, Some(2));
    assert_eq!(tool.shape, ToolShape::Flat);
}

#[test]
fn a_name_only_header_comment_creates_no_hint() {
    let program = parse_program("(clean pocket)\nT1 M6\nG0 X0\n").expect("valid");
    assert!(program.tool_hints.is_empty());
}

#[test]
fn user_configuration_wins_over_a_comment_hint() {
    let program = parse_program("T1 (D=6mm 2F flat)\nG0 X0").expect("valid");
    let mut config = ToolTable::new();
    let mut tool = Tool::new(1);
    tool.diameter_mm = Some(10.0);
    tool.sources
        .insert(ToolField::Diameter, FieldSource::Config);
    config.insert(tool);

    let resolved = ToolTable::resolve(&config, &program.tool_hints);
    let tool = resolved.get(1).expect("tool");
    assert_eq!(tool.diameter_mm, Some(10.0));
    assert_eq!(
        resolved.source_of(1, ToolField::Diameter),
        Some(FieldSource::Config)
    );
    // The comment still fills fields the configuration leaves unknown.
    assert_eq!(tool.flutes, Some(2));
    assert_eq!(tool.shape, ToolShape::Flat);
}

#[test]
fn comments_on_skipped_lines_are_retained_and_hint_extraction_still_runs() {
    let result = parse_program_lossy(
        "T1 (D=6mm 2F flat)\nG1 X1 Q2 (skipped note)\nG0 X2\n",
        &ParserConfig::default(),
    );
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.skipped_blocks.len(), 1);
    assert_eq!(result.program.comments[0].text, "D=6mm 2F flat");
    assert!(
        result
            .program
            .comments
            .iter()
            .any(|comment| comment.text == "skipped note")
    );
    assert_eq!(result.program.tool_hints.len(), 1);
    assert!(close(
        result.program.tool_hints[0].diameter_mm.expect("diameter"),
        6.0
    ));
}
