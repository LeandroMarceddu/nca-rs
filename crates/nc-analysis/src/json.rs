//! Deterministic JSON rendering for [`Report`].
//!
//! Report numbers are rounded at this boundary and written with a fixed
//! decimal count, so the bytes do not depend on the shortest-round-trip float
//! algorithm. The compact and pretty forms never end with a newline; the
//! fixture layer adds one when it writes a golden file.

use nc_core::Dialect;
use nc_stock::StockSource;

use crate::report::Report;
use crate::types::EvidenceValue;

const INDENT: &str = "  ";

/// Number of fraction digits for an evidence/inventory float key.
fn decimals_for_key(key: &str) -> u32 {
    if key.ends_with("_mm_min") {
        1
    } else if key.ends_with("_mm") {
        3
    } else if key.ends_with("_rpm") {
        0
    } else if key.ends_with("_deg") {
        2
    } else if key.ends_with("_percent") {
        1
    } else {
        3
    }
}

/// Rounds a float to `decimals` places. Non-finite values become zero so the
/// report stays valid JSON; rule authors should never emit them.
fn quantize(value: f64, decimals: u32) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let factor = match decimals {
        0 => 1.0,
        1 => 10.0,
        2 => 100.0,
        _ => 1000.0,
    };
    let scaled = value * factor;
    if !scaled.is_finite() {
        return 0.0;
    }
    let rounded = nc_core::math::round(scaled) / factor;
    if !rounded.is_finite() || rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// Renders a report as compact or pretty JSON without a trailing newline.
pub(crate) fn render(report: &Report, pretty: bool) -> String {
    let mut writer = Writer::new(pretty);
    writer.begin_object();

    writer.key("schema");
    writer.string(&report.schema);
    writer.key("ruleset_version");
    writer.string(&report.ruleset_version);

    writer.key("input");
    writer.begin_object();
    writer.key("file_name");
    writer.string(&report.input.file_name);
    writer.key("sha256");
    writer.string(&report.input.sha256);
    writer.key("line_count");
    writer.integer(report.input.line_count);
    writer.key("dialect");
    writer.string(dialect_label(report.input.dialect));
    writer.key("geometry_uncertain_after");
    match report.input.geometry_uncertain_after {
        Some(line) => writer.integer(line),
        None => writer.null(),
    }
    writer.end_object();

    writer.key("config_digest");
    writer.string(&report.config_digest);
    writer.key("stock_source");
    writer.string(stock_source_label(report.stock_source));

    writer.key("tool_inventory");
    writer.begin_array();
    for entry in &report.tool_inventory {
        writer.begin_object();
        writer.key("tool");
        writer.integer(entry.tool);
        writer.key("first_line");
        writer.integer(entry.first_line);
        writer.key("motion_count");
        writer.integer(entry.motion_count);
        writer.key("fields");
        writer.begin_object();
        for (key, field) in &entry.fields {
            writer.key(key);
            writer.begin_object();
            writer.key("value");
            write_evidence(&mut writer, key, &field.value);
            writer.key("source");
            writer.string(&field.source);
            writer.end_object();
        }
        writer.end_object();
        writer.end_object();
    }
    writer.end_array();

    writer.key("diagnostics");
    writer.begin_array();
    for diagnostic in &report.diagnostics {
        writer.begin_object();
        writer.key("rule");
        writer.string(diagnostic.rule.as_str());
        writer.key("severity");
        writer.string(diagnostic.severity.label());
        writer.key("basis");
        writer.string(diagnostic.basis.label());
        writer.key("line");
        writer.integer(diagnostic.line);
        writer.key("column");
        writer.integer(diagnostic.column);
        writer.key("line_end");
        writer.integer(diagnostic.line_end);
        writer.key("message");
        writer.string(&diagnostic.message);
        writer.key("evidence");
        writer.begin_object();
        for (key, value) in &diagnostic.evidence {
            writer.key(key);
            write_evidence(&mut writer, key, value);
        }
        writer.end_object();
        writer.end_object();
    }
    writer.end_array();

    writer.key("truncated");
    writer.begin_object();
    for (rule, count) in &report.truncated {
        writer.key(rule);
        writer.integer(*count);
    }
    writer.end_object();

    writer.key("coverage");
    writer.begin_object();
    writer.key("evaluated");
    writer.integer(report.coverage.evaluated);
    writer.key("total");
    writer.integer(report.coverage.total);
    writer.key("skipped");
    writer.begin_array();
    for skip in &report.coverage.skipped {
        writer.begin_object();
        writer.key("rule");
        writer.string(skip.rule.as_str());
        writer.key("missing_input");
        writer.string(&skip.missing_input);
        writer.key("reason");
        writer.string(&skip.reason);
        writer.end_object();
    }
    writer.end_array();
    writer.end_object();

    writer.end_object();
    writer.finish()
}

fn write_evidence(writer: &mut Writer, key: &str, value: &EvidenceValue) {
    match value {
        EvidenceValue::Int(value) => writer.integer(*value),
        EvidenceValue::Float(value) => writer.fixed(*value, decimals_for_key(key)),
        EvidenceValue::Text(value) => writer.string(value),
        EvidenceValue::Bool(value) => writer.boolean(*value),
        EvidenceValue::Point3(point) => {
            writer.begin_array();
            for coordinate in point {
                // Coordinates are always reported in millimetres at 0.001.
                writer.fixed(*coordinate, 3);
            }
            writer.end_array();
        }
    }
}

fn dialect_label(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Generic => "generic",
        Dialect::Grbl => "grbl",
        Dialect::LinuxCnc => "linux_cnc",
        Dialect::Mach3 => "mach3",
        Dialect::Fanuc => "fanuc",
        Dialect::Makera => "makera",
    }
}

fn stock_source_label(source: StockSource) -> &'static str {
    match source {
        StockSource::Inferred => "inferred",
        StockSource::Explicit => "explicit",
    }
}

/// Minimal ordered JSON writer with fixed-decimal numbers.
struct Writer {
    out: String,
    pretty: bool,
    stack: Vec<bool>,
    after_key: bool,
}

impl Writer {
    fn new(pretty: bool) -> Self {
        Self {
            out: String::new(),
            pretty,
            stack: Vec::new(),
            after_key: false,
        }
    }

    fn finish(self) -> String {
        self.out
    }

    fn indent(&mut self, depth: usize) {
        if !self.pretty {
            return;
        }
        self.out.push('\n');
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
    }

    fn before_value(&mut self) {
        if self.after_key {
            self.after_key = false;
            return;
        }
        let Some(first) = self.stack.last_mut() else {
            // A top-level value has no surrounding container to indent for.
            return;
        };
        if *first {
            *first = false;
        } else {
            self.out.push(',');
        }
        let depth = self.stack.len();
        self.indent(depth);
    }

    fn begin_object(&mut self) {
        self.before_value();
        self.out.push('{');
        self.stack.push(true);
    }

    fn end_object(&mut self) {
        let empty = self.stack.pop().unwrap_or(true);
        if !empty {
            let depth = self.stack.len();
            self.indent(depth);
        }
        self.out.push('}');
    }

    fn begin_array(&mut self) {
        self.before_value();
        self.out.push('[');
        self.stack.push(true);
    }

    fn end_array(&mut self) {
        let empty = self.stack.pop().unwrap_or(true);
        if !empty {
            let depth = self.stack.len();
            self.indent(depth);
        }
        self.out.push(']');
    }

    fn key(&mut self, key: &str) {
        self.before_value();
        write_string(&mut self.out, key);
        self.out.push(':');
        if self.pretty {
            self.out.push(' ');
        }
        self.after_key = true;
    }

    fn string(&mut self, value: &str) {
        self.before_value();
        write_string(&mut self.out, value);
    }

    fn integer<T: std::fmt::Display>(&mut self, value: T) {
        self.before_value();
        self.out.push_str(&value.to_string());
    }

    fn fixed(&mut self, value: f64, decimals: u32) {
        self.before_value();
        let precision = decimals as usize;
        let rounded = quantize(value, decimals);
        self.out.push_str(&format!("{rounded:.precision$}"));
    }

    fn boolean(&mut self, value: bool) {
        self.before_value();
        self.out.push_str(if value { "true" } else { "false" });
    }

    fn null(&mut self) {
        self.before_value();
        self.out.push_str("null");
    }
}

/// Writes a JSON string with the same escaping rules as `serde_json`.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0000}'..='\u{001f}' => {
                out.push_str("\\u00");
                let code = character as u32;
                out.push(char::from_digit(code >> 4, 16).unwrap_or('0'));
                out.push(char::from_digit(code & 0x0f, 16).unwrap_or('0'));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_follow_evidence_key_convention() {
        assert_eq!(decimals_for_key("depth_mm"), 3);
        assert_eq!(decimals_for_key("feed_mm_min"), 1);
        assert_eq!(decimals_for_key("spindle_rpm"), 0);
        assert_eq!(decimals_for_key("angle_deg"), 2);
        assert_eq!(decimals_for_key("width_percent"), 1);
        assert_eq!(decimals_for_key("threshold"), 3);
    }

    #[test]
    fn quantization_is_by_key_and_normalizes_zero() {
        assert_eq!(quantize(1.23456, 3), 1.235);
        assert_eq!(quantize(800.04, 1), 800.0);
        assert_eq!(quantize(11999.6, 0), 12000.0);
        assert_eq!(quantize(89.996, 2), 90.0);
        assert_eq!(quantize(-0.0001, 3), 0.0);
        assert_eq!(quantize(f64::NAN, 3), 0.0);
    }

    #[test]
    fn strings_escape_like_json() {
        let mut out = String::new();
        write_string(&mut out, "a\"b\\c\nd\u{0001}");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001\"");
        let parsed: String = serde_json::from_str(&out).expect("valid json string");
        assert_eq!(parsed, "a\"b\\c\nd\u{0001}");
    }
}
