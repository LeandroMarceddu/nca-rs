//! Parameter assignments and bracketed arithmetic expressions (VIEW-02 part 1).

use nc_gcode::{
    MAX_EXPRESSION_BYTES, MAX_EXPRESSION_DEPTH, MAX_NAMED_PARAMETERS, MAX_NUMERIC_PARAMETERS,
    Parameters, ParseErrorKind, Parser, lex_line_with_parameters, parse_program,
};
use nc_viewer::Point3;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0e-4
}

fn first_kind(text: &str) -> ParseErrorKind {
    parse_program(text)
        .expect_err("expected an error")
        .first()
        .expect("error")
        .kind
}

#[test]
fn assignment_updates_the_table_and_reads_back() {
    let program = parse_program("#1=5\nG0 X#1\nG0 Y[#1+2]\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].end, Point3::new(5.0, 0.0, 0.0));
    assert_eq!(segments[1].end, Point3::new(5.0, 7.0, 0.0));
}

#[test]
fn arithmetic_precedence_is_applied() {
    let program = parse_program("#1=2\nG0 X[#1+3*4]\n").expect("valid");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(14.0, 0.0, 0.0)
    );
}

#[test]
fn parentheses_and_nested_groups_are_applied() {
    let program = parse_program("#1=2\nG0 X[(#1+3)*2]\nG0 Y[[#1+1]*3]\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments[0].end, Point3::new(10.0, 0.0, 0.0));
    assert_eq!(segments[1].end, Point3::new(10.0, 9.0, 0.0));
}

#[test]
fn unary_signs_are_applied() {
    let program = parse_program("#1=4\nG0 X[-#1+10]\nG0 Y[+5]\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments[0].end, Point3::new(6.0, 0.0, 0.0));
    assert_eq!(segments[1].end, Point3::new(6.0, 5.0, 0.0));
}

#[test]
fn nested_expressions_use_earlier_assignments() {
    let program = parse_program("#1=2\n#2=[#1*3]\nG0 X[#1+#2*2]\n").expect("valid");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(14.0, 0.0, 0.0)
    );
}

#[test]
fn axis_feed_and_spindle_words_accept_expressions() {
    let program =
        parse_program("#<feed>=250\n#100=12000\n#1=2\nG1 X[#1] Y[#1+1] F[#<feed>*2]\nS[#100]\n")
            .expect("valid");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(2.0, 3.0, 0.0)
    );
    assert_eq!(program.state.feed, Some(500.0));
    assert_eq!(program.stats.max_feed_mm_per_min, Some(500.0));
    assert_eq!(program.state.spindle_rpm, 12000.0);
    assert_eq!(program.stats.max_spindle_rpm, 12000.0);
}

#[test]
fn negative_and_decimal_results_are_kept() {
    let program = parse_program("#1=1.5\n#2=-0.25\nG0 X[#1*-2.5]\nG0 Y[#2]\n").expect("valid");
    let segments = program.toolpath.segments();
    assert!(close(segments[0].end.x, -3.75));
    assert!(close(segments[1].end.y, -0.25));
}

#[test]
fn an_unset_numbered_parameter_is_an_error() {
    let errors = parse_program("G0 X[#1]\n").expect_err("unresolved");
    assert_eq!(errors[0].kind, ParseErrorKind::UnresolvedParameter);
    assert_eq!(errors[0].line, 1);
    assert_eq!(errors[0].column, 6);
}

#[test]
fn an_unknown_named_parameter_is_an_error() {
    assert_eq!(
        first_kind("G0 X[#<feed>]\n"),
        ParseErrorKind::UnknownNamedParameter
    );
}

#[test]
fn division_by_zero_is_rejected() {
    assert_eq!(
        first_kind("G0 X[1/0]\n"),
        ParseErrorKind::InvalidExpressionValue
    );
    assert_eq!(
        first_kind("#1=1\nG0 X[#1/0]\n"),
        ParseErrorKind::InvalidExpressionValue
    );
}

#[test]
fn non_finite_results_are_rejected() {
    let literal = "9".repeat(250);
    let source = format!("G0 X[{literal}*{literal}]\n");
    let errors = parse_program(&source).expect_err("non-finite");
    assert_eq!(errors[0].kind, ParseErrorKind::InvalidExpressionValue);
}

#[test]
fn an_assignment_without_equals_is_rejected() {
    assert_eq!(first_kind("#1 5\n"), ParseErrorKind::MissingAssignment);
    assert_eq!(first_kind("#1\n"), ParseErrorKind::MissingAssignment);
    assert_eq!(
        first_kind("#<feed> 100\n"),
        ParseErrorKind::MissingAssignment
    );
}

#[test]
fn compound_assignment_values_must_be_bracketed() {
    for source in [
        "#1=2+3\n",
        "#1=2-3\n",
        "#1=2/3\n",
        "#1=2 + 3\n",
        "#1=2 / 3\n",
    ] {
        let errors = parse_program(source).expect_err(source);
        assert_eq!(errors[0].kind, ParseErrorKind::MalformedExpression);
        assert!(
            errors[0].message.contains("bracketed"),
            "unexpected message: {}",
            errors[0].message
        );
    }

    let errors = parse_program("#2=1\n#1=#2+3\n").expect_err("parameter compound");
    assert_eq!(errors[0].kind, ParseErrorKind::MalformedExpression);
    assert_eq!(errors[0].line, 2);

    let program = parse_program("#1=[2+3*4]\nG0 X#1\n").expect("bracketed");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(14.0, 0.0, 0.0)
    );

    // An unbracketed `*` is the line checksum, not multiplication.
    let errors = parse_program("#1=5*2\n").expect_err("checksum");
    assert_eq!(errors[0].kind, ParseErrorKind::ChecksumMismatch);

    // A top-level '(' is a comment, not an assignment expression.
    let errors = parse_program("#1=(2+3)\n").expect_err("comment parenthesis");
    assert_eq!(errors[0].kind, ParseErrorKind::MalformedExpression);
    assert!(errors[0].message.contains("brackets"));
}

#[test]
fn malformed_brackets_are_rejected() {
    assert_eq!(
        first_kind("G0 X[1+2\n"),
        ParseErrorKind::MalformedExpression
    );
    assert_eq!(first_kind("G0 X[]\n"), ParseErrorKind::MalformedExpression);
    assert_eq!(
        first_kind("G0 X[1+]\n"),
        ParseErrorKind::MalformedExpression
    );
    assert_eq!(first_kind("[1+2]\n"), ParseErrorKind::ExpressionNotAllowed);
}

#[test]
fn expressions_are_rejected_in_g_m_n_and_o_words() {
    assert_eq!(first_kind("G[1+2]\n"), ParseErrorKind::ExpressionNotAllowed);
    assert_eq!(first_kind("M[3]\n"), ParseErrorKind::ExpressionNotAllowed);
    assert_eq!(
        first_kind("N[1] G0\n"),
        ParseErrorKind::ExpressionNotAllowed
    );
    assert_eq!(first_kind("O[1]\n"), ParseErrorKind::ExpressionNotAllowed);
}

#[test]
fn the_parameter_table_is_bounded() {
    assert_eq!(first_kind("#0=1\n"), ParseErrorKind::ParameterOutOfRange);
    assert_eq!(
        first_kind(&format!("#{}=1\n", MAX_NUMERIC_PARAMETERS + 1)),
        ParseErrorKind::ParameterOutOfRange
    );

    let mut source = String::new();
    for index in 0..=MAX_NAMED_PARAMETERS {
        source.push_str(&format!("#<P{index}>=1\n"));
    }
    let errors = parse_program(&source).expect_err("named table full");
    assert_eq!(errors[0].kind, ParseErrorKind::TooManyParameters);
    assert_eq!(errors[0].line, MAX_NAMED_PARAMETERS + 1);
}

#[test]
fn expressions_are_bounded_by_depth_and_length() {
    let deep = format!(
        "G0 X{}1{}\n",
        "[".repeat(MAX_EXPRESSION_DEPTH + 1),
        "]".repeat(MAX_EXPRESSION_DEPTH + 1)
    );
    assert_eq!(first_kind(&deep), ParseErrorKind::ExpressionTooDeep);

    let long = format!("G0 X[{}1]\n", "1+".repeat(MAX_EXPRESSION_BYTES / 2 + 20));
    assert_eq!(first_kind(&long), ParseErrorKind::ExpressionTooLong);
}

#[test]
fn planner_evaluates_expressions_and_assignments_emit_no_motion() {
    let program = parse_program("#1=3\nG0 X[#1+2]\n#2=[#1*2]\nG0 Y[#2]\n").expect("valid");
    let segments = program.toolpath.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].end, Point3::new(5.0, 0.0, 0.0));
    assert_eq!(segments[1].end, Point3::new(5.0, 6.0, 0.0));

    assert_eq!(program.lines[0].segment_count(), 0);
    assert_eq!(program.lines[0].segments, 0..0);
    assert_eq!(program.lines[2].segment_count(), 0);
    assert_eq!(program.lines[2].segments, 1..1);
    assert_eq!(program.stats.blocks, 2);
    assert_eq!(program.stats.words, 4);
    assert_eq!(program.parameters.numeric(1), Some(3.0));
    assert_eq!(program.parameters.numeric(2), Some(6.0));
}

#[test]
fn a_failed_block_does_not_commit_its_assignments() {
    let mut parser = Parser::new();
    parser.parse_line_at(1, "#1=5").expect("assignment");
    assert_eq!(parser.parameters().numeric(1), Some(5.0));

    parser
        .parse_line_at(2, "#2=7 G123")
        .expect_err("unknown code");
    assert_eq!(parser.parameters().numeric(2), None);
    assert_eq!(parser.parameters().numeric(1), Some(5.0));
}

#[test]
fn a_lexing_failure_does_not_commit_assignments() {
    let mut parser = Parser::new();
    parser
        .parse_line_at(1, "#1=5 #2=6 X[#3]")
        .expect_err("unresolved parameter");
    assert_eq!(parser.parameters().numeric(1), None);
    assert_eq!(parser.parameters().numeric(2), None);
}

#[test]
fn a_planning_failure_does_not_commit_assignments() {
    let mut parser = Parser::new();
    parser
        .parse_line_at(1, "#1=5 G1 X1")
        .expect_err("feed rate undefined");
    assert_eq!(parser.parameters().numeric(1), None);
}

#[test]
fn lex_line_with_parameters_is_line_transactional() {
    let mut parameters = Parameters::new();
    lex_line_with_parameters(1, "#1=5 #<", 64, &mut parameters).expect_err("malformed");
    assert_eq!(parameters.numeric(1), None);

    lex_line_with_parameters(1, "#1=5 #2=6", 64, &mut parameters).expect("valid");
    assert_eq!(parameters.numeric(1), Some(5.0));
    assert_eq!(parameters.numeric(2), Some(6.0));
}

#[test]
fn assignments_are_applied_in_source_order() {
    let program = parse_program("#1=1 #2=[#1+1] #1=10 G0 X[#2]\n").expect("valid");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(2.0, 0.0, 0.0)
    );
}

#[test]
fn named_parameters_are_case_insensitive() {
    let program = parse_program("#<Feed>=100\nG1 X0 F[#<FEED>]\n").expect("valid");
    assert_eq!(program.state.feed, Some(100.0));
}

#[test]
fn assignments_allow_spaces_around_equals_and_inside_brackets() {
    let program = parse_program("#1 = 5\nG0 X[ #1 + 1 ]\n").expect("valid");
    assert_eq!(
        program.toolpath.segments()[0].end,
        Point3::new(6.0, 0.0, 0.0)
    );
}

#[test]
fn expressions_use_the_active_unit_mode() {
    let program = parse_program("#1=1\nG20\nG0 X[#1*2]\n").expect("valid");
    assert!(close(program.toolpath.segments()[0].end.x, 50.8));
}

#[test]
fn expressions_are_only_allowed_in_numeric_word_values() {
    // A bare expression is not a word value.
    assert_eq!(first_kind("[#1+2]\n"), ParseErrorKind::ExpressionNotAllowed);
    // A word value that is an expression is accepted.
    parse_program("#1=1\nG0 X[#1+2]\n").expect("word value");
}
