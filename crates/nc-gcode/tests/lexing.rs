//! Lexer-facing behavior: comments, line numbers, checksums, block delete.

use nc_gcode::{ParseErrorKind, checksum_xor, lex_line, parse_program};

#[test]
fn comments_line_numbers_and_checksums_are_preserved() {
    let body = "N10 G0 X1 (start)";
    let sum = checksum_xor(body.as_bytes());
    let text = format!("{body}*{sum:02}\n");
    let program = parse_program(&text).expect("valid program");

    assert_eq!(program.lines.len(), 1);
    let line = &program.lines[0];
    assert_eq!(line.number, Some(10));
    assert_eq!(line.comments, vec!["start"]);
    assert_eq!(line.checksum, Some(sum));
    assert!(!line.block_delete);
    assert_eq!(line.segment_count(), 1);
    assert_eq!(line.words.len(), 3);
}

#[test]
fn semicolon_comment_keeps_the_rest_of_the_line() {
    let program = parse_program("G0 X5 ; rapid to the corner\n").expect("valid");
    let line = &program.lines[0];
    assert_eq!(line.comments, vec!["rapid to the corner"]);
    assert_eq!(program.stats.rapid_segments, 1);
}

#[test]
fn block_delete_is_flagged_and_geometry_is_kept() {
    let program = parse_program("/G0 X5\nG0 X10\n").expect("valid");
    assert!(program.lines[0].block_delete);
    assert!(!program.lines[1].block_delete);
    assert_eq!(program.stats.rapid_segments, 2);
    assert_eq!(program.bounds.max().x, 10.0);
}

#[test]
fn percent_marker_is_a_no_op_line() {
    let program = parse_program("%\nG0 X1\n%\n").expect("valid");
    assert_eq!(program.lines.len(), 3);
    assert!(program.lines[0].words.is_empty());
    assert!(program.lines[2].words.is_empty());
    assert_eq!(program.stats.segments, 1);
}

#[test]
fn a_bad_checksum_is_an_error() {
    let errors = parse_program("G0 X1*99\n").expect_err("checksum");
    assert_eq!(errors[0].kind, ParseErrorKind::ChecksumMismatch);
    assert_eq!(errors[0].line, 1);
}

#[test]
fn unterminated_comment_is_an_error() {
    let errors = parse_program("(never closed\nG0 X0\n").expect_err("comment");
    assert_eq!(errors[0].kind, ParseErrorKind::UnterminatedComment);
    assert_eq!(errors[0].line, 1);
}

#[test]
fn crlf_and_bom_are_accepted() {
    let program = parse_program("\u{feff}G0 X1\r\nG0 X2\r\n").expect("valid");
    assert_eq!(program.lines.len(), 2);
    assert_eq!(program.lines[0].source, "G0 X1");
    assert_eq!(program.bounds.max().x, 2.0);
}

#[test]
fn cr_only_line_endings_are_split() {
    let program = parse_program("G0 X1\rG0 X2\r").expect("valid");
    assert_eq!(program.lines.len(), 2);
    assert_eq!(program.lines[1].source, "G0 X2");
}

#[test]
fn lexer_reports_columns() {
    let errors = lex_line(1, "G0 X", 64).expect_err("missing value");
    assert_eq!(errors[0].column, 4);
    assert_eq!(errors[0].kind, ParseErrorKind::MissingValue);
}

#[test]
fn lexed_line_can_be_fed_back_through_the_extension_point() {
    let mut lexed = lex_line(1, "G0 X5", 64).expect("lex");
    // A future expression evaluator would rewrite values here.
    lexed.words[1].value = 7.0;
    let mut parser = nc_gcode::Parser::new();
    parser.parse_lexed(1, "G0 X5", lexed).expect("parsed");
    let program = parser.finish();
    assert_eq!(program.bounds.max().x, 7.0);
}
