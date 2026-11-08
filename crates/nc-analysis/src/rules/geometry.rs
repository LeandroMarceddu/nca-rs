//! `geometry.*` rules: holes, tiny segments, arc geometry, feed/spindle ranges.
//!
//! Every rule here is [`Basis::Exact`](crate::Basis): it depends only on the
//! motion records and the configured tolerance or machine limits. Hole
//! geometry comes from canned-cycle records and from explicit `Drill`/`Peck`/
//! `Bore` motions; neither rule approximates an arc with chords.

use std::collections::{BTreeMap, BTreeSet};

use nc_core::ToolField;
use nc_gcode::{ArcRecord, CutterComp, MotionRecord};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

/// Tool field shared by the two tool-size rules.
const DIAMETER: &[ToolField] = &[ToolField::Diameter];

/// Every rule in this module, in stable id order.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(ArcRadiusMismatch),
        Box::new(ArcSmallerThanTool),
        Box::new(DuplicateHole),
        Box::new(FeedOutOfRange),
        Box::new(OverlappingHoles),
        Box::new(SpindleOutOfRange),
        Box::new(TinySegment),
    ]
}

/// One drilled hole extracted from the motion records.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hole {
    /// Source line of the motion that drilled it.
    line: usize,
    /// Motion index, used to break ties on the same source line.
    index: usize,
    /// Active tool.
    tool: u32,
    /// Hole centre in XY, machine millimetres.
    xy: [f64; 2],
    /// Lower end of the drilled Z interval.
    low_z: f64,
    /// Upper end of the drilled Z interval.
    high_z: f64,
}

impl Hole {
    /// Position used in evidence: centre XY plus the bottom of the hole.
    fn point_mm(self) -> [f64; 3] {
        [self.xy[0], self.xy[1], self.low_z]
    }

    /// `true` when the two holes share at least one Z value.
    fn z_overlaps(self, other: Self) -> bool {
        self.low_z.max(other.low_z) <= self.high_z.min(other.high_z)
    }

    /// XY distance in millimetres.
    fn distance_mm(self, other: Self) -> f64 {
        nc_core::math::hypot(self.xy[0] - other.xy[0], self.xy[1] - other.xy[1])
    }
}

/// Collects holes from canned records and explicit drilling motions, sorted by
/// `(tool, line, index)` so pair reporting is deterministic.
fn holes(model: &AnalysisModel<'_>) -> Vec<Hole> {
    let mut holes = Vec::new();
    for motion in &model.program.motions {
        if let Some(hole) = hole_from(motion) {
            holes.push(hole);
        }
    }
    holes.sort_by_key(|hole| (hole.tool, hole.line, hole.index));
    holes
}

/// Extracts one hole, or `None` when the record is not a hole or has
/// non-finite geometry.
fn hole_from(motion: &MotionRecord) -> Option<Hole> {
    let canned = motion.canned.as_deref();
    if canned.is_none() && !motion.kind.is_hole() {
        return None;
    }

    let xy = [motion.end_mm[0], motion.end_mm[1]];
    if !xy.iter().all(|value| value.is_finite()) {
        return None;
    }

    let (low_z, high_z) = match canned {
        // Canned cycles drill from the R plane down to the bottom.
        Some(record) => (record.bottom_z_mm, record.r_plane_mm),
        // A stored drilling record runs from its start Z to its end Z.
        None => (motion.end_mm[2], motion.start_mm[2]),
    };
    if !low_z.is_finite() || !high_z.is_finite() {
        return None;
    }

    Some(Hole {
        line: motion.line,
        index: motion.index,
        tool: motion.tool,
        xy,
        low_z: low_z.min(high_z),
        high_z: low_z.max(high_z),
    })
}

/// Returns `true` when the two tools are the same and the records differ.
fn same_tool_different_record(first: &Hole, second: &Hole) -> bool {
    first.tool == second.tool && (first.line != second.line || first.index != second.index)
}

/// Builds one diagnostic with a stable rule id and exact basis.
fn diagnostic(
    rule: &'static str,
    severity: Severity,
    line: usize,
    message: String,
    evidence: BTreeMap<String, EvidenceValue>,
) -> Diagnostic {
    Diagnostic {
        rule: RuleId::new(rule),
        severity,
        basis: Basis::Exact,
        line,
        column: 0,
        line_end: line,
        message,
        evidence,
    }
}

/// Converts a source line to the integer evidence variant.
fn line_value(line: usize) -> EvidenceValue {
    EvidenceValue::Int(i64::try_from(line).unwrap_or(i64::MAX))
}

/// Adds `tool_source` when a tool field came from a program comment.
fn insert_tool_source(
    evidence: &mut BTreeMap<String, EvidenceValue>,
    model: &AnalysisModel<'_>,
    tool: u32,
    field: ToolField,
) {
    if let Some(source) = model.tools.source_of(tool, field)
        && source.is_comment()
    {
        evidence.insert(
            "tool_source".to_string(),
            EvidenceValue::Text(source.label()),
        );
    }
}

/// Returns a known, positive tool diameter.
fn tool_diameter(model: &AnalysisModel<'_>, tool: u32) -> Option<f64> {
    model
        .tools
        .get(tool)
        .and_then(|tool| tool.diameter_mm)
        .filter(|diameter| diameter.is_finite() && *diameter > 0.0)
}

/// Builds the skip outcome for a tool whose diameter is unknown.
fn missing_diameter(tool: u32) -> RuleOutcome {
    RuleOutcome::Skipped {
        missing_input: format!("tool.{}:T{tool}", ToolField::Diameter.key()),
        reason: format!("tool T{tool} has no diameter"),
    }
}

// ---------------------------------------------------------------------------
// geometry.duplicate_hole
// ---------------------------------------------------------------------------

/// Stable rule id.
const DUPLICATE_HOLE_ID: &str = "geometry.duplicate_hole";

/// Two holes with the same tool, the same XY, and overlapping Z.
struct DuplicateHole;

impl Rule for DuplicateHole {
    fn id(&self) -> RuleId {
        RuleId::new(DUPLICATE_HOLE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let tolerance_mm = model.tolerances.hole_xy_mm;
        let holes = holes(model);
        let mut findings = Vec::new();

        for (index, first) in holes.iter().enumerate() {
            for second in holes.iter().skip(index + 1) {
                if second.tool != first.tool {
                    // Holes are sorted by tool, so no later hole can match.
                    break;
                }
                if !same_tool_different_record(first, second) {
                    continue;
                }
                let distance_mm = first.distance_mm(*second);
                if distance_mm > tolerance_mm {
                    continue;
                }
                if !first.z_overlaps(*second) {
                    continue;
                }
                findings.push(duplicate_finding(
                    *first,
                    *second,
                    distance_mm,
                    tolerance_mm,
                ));
            }
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Drilling the same hole twice at an overlapping depth doubles the cut, \
         overheats the drill, and can oversize or break through the part where \
         the program expected one hole."
    }
}

/// Builds the finding for the later hole of one duplicate pair.
fn duplicate_finding(first: Hole, second: Hole, distance_mm: f64, tolerance_mm: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(second.tool)),
    );
    evidence.insert("first_line".to_string(), line_value(first.line));
    evidence.insert("second_line".to_string(), line_value(second.line));
    evidence.insert(
        "first_mm".to_string(),
        EvidenceValue::Point3(first.point_mm()),
    );
    evidence.insert(
        "second_mm".to_string(),
        EvidenceValue::Point3(second.point_mm()),
    );
    evidence.insert("distance_mm".to_string(), EvidenceValue::Float(distance_mm));
    evidence.insert(
        "tolerance_mm".to_string(),
        EvidenceValue::Float(tolerance_mm),
    );

    diagnostic(
        DUPLICATE_HOLE_ID,
        Severity::Warning,
        second.line,
        format!(
            "hole at line {} repeats the hole at line {} with tool T{}",
            second.line, first.line, second.tool
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.overlapping_holes
// ---------------------------------------------------------------------------

/// Stable rule id.
const OVERLAPPING_HOLES_ID: &str = "geometry.overlapping_holes";

/// Two holes from the same tool whose centres are closer than its diameter.
struct OverlappingHoles;

impl Rule for OverlappingHoles {
    fn id(&self) -> RuleId {
        RuleId::new(OVERLAPPING_HOLES_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_tools(DIAMETER)
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let holes = holes(model);
        if holes.is_empty() {
            return RuleOutcome::Findings(Vec::new());
        }

        // Collect the diameters that are known. The engine gates this rule on
        // every used tool having `Diameter`, so a missing value here means the
        // rule is called directly and should still report the gap.
        let mut diameters: BTreeMap<u32, f64> = BTreeMap::new();
        let mut missing: BTreeSet<u32> = BTreeSet::new();
        for hole in &holes {
            if let Some(diameter) = tool_diameter(model, hole.tool) {
                diameters.insert(hole.tool, diameter);
            } else {
                missing.insert(hole.tool);
            }
        }
        if diameters.is_empty() {
            if let Some(tool) = missing.iter().next().copied() {
                return missing_diameter(tool);
            }
            return RuleOutcome::Findings(Vec::new());
        }

        let mut findings = Vec::new();
        for (index, first) in holes.iter().enumerate() {
            for second in holes.iter().skip(index + 1) {
                if second.tool != first.tool {
                    break;
                }
                if !same_tool_different_record(first, second) {
                    continue;
                }
                let Some(diameter_mm) = diameters.get(&first.tool).copied() else {
                    continue;
                };
                let distance_mm = first.distance_mm(*second);
                // Equal centres belong to `geometry.duplicate_hole`, and a
                // distance equal to the diameter leaves the holes tangent.
                if distance_mm <= 0.0 || distance_mm >= diameter_mm {
                    continue;
                }
                findings.push(overlap_finding(
                    model,
                    *first,
                    *second,
                    distance_mm,
                    diameter_mm,
                ));
            }
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Holes whose centres are closer than the cutter diameter break into \
         each other, so the second hole loses its wall and the finished part \
         no longer matches the drawing."
    }
}

/// Builds the finding for the later hole of one overlapping pair.
fn overlap_finding(
    model: &AnalysisModel<'_>,
    first: Hole,
    second: Hole,
    distance_mm: f64,
    diameter_mm: f64,
) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(second.tool)),
    );
    evidence.insert("first_line".to_string(), line_value(first.line));
    evidence.insert("second_line".to_string(), line_value(second.line));
    evidence.insert(
        "first_mm".to_string(),
        EvidenceValue::Point3(first.point_mm()),
    );
    evidence.insert(
        "second_mm".to_string(),
        EvidenceValue::Point3(second.point_mm()),
    );
    evidence.insert("distance_mm".to_string(), EvidenceValue::Float(distance_mm));
    evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
    insert_tool_source(&mut evidence, model, second.tool, ToolField::Diameter);

    diagnostic(
        OVERLAPPING_HOLES_ID,
        Severity::Warning,
        second.line,
        format!(
            "hole at line {} is {distance_mm:.3} mm from the hole at line {} \
             with tool T{}, closer than its {diameter_mm:.3} mm diameter",
            second.line, first.line, second.tool
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.tiny_segment
// ---------------------------------------------------------------------------

/// Stable rule id.
const TINY_SEGMENT_ID: &str = "geometry.tiny_segment";

/// A cutting motion shorter than the configured minimum.
struct TinySegment;

impl Rule for TinySegment {
    fn id(&self) -> RuleId {
        RuleId::new(TINY_SEGMENT_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let threshold_mm = model.tolerances.min_segment_mm;
        let mut findings = Vec::new();

        for motion in &model.program.motions {
            if !motion.kind.is_cutting() || motion.canned.is_some() {
                // Canned cycles are holes, not segments; their stored record
                // describes a drill, not the zero-length XY step it replaced.
                continue;
            }
            let length_mm = motion.path_length_mm();
            if !length_mm.is_finite() || length_mm >= threshold_mm {
                continue;
            }
            findings.push(tiny_finding(motion, length_mm, threshold_mm));
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Very short cutting segments make the controller hesitate or dwell, \
         leaving burn marks and a poor surface where a smooth pass was \
         intended."
    }
}

/// Builds the finding for one short cutting motion.
fn tiny_finding(motion: &MotionRecord, length_mm: f64, threshold_mm: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );
    evidence.insert("length_mm".to_string(), EvidenceValue::Float(length_mm));
    evidence.insert("threshold".to_string(), EvidenceValue::Float(threshold_mm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3(motion.start_mm),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));

    diagnostic(
        TINY_SEGMENT_ID,
        Severity::Info,
        motion.line,
        format!(
            "{} cutting segment is {length_mm:.4} mm long, below the \
             {threshold_mm:.3} mm minimum",
            motion.kind.label()
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.arc_radius_mismatch
// ---------------------------------------------------------------------------

/// Stable rule id.
const ARC_RADIUS_MISMATCH_ID: &str = "geometry.arc_radius_mismatch";

/// An arc whose start and end radii disagree beyond the tolerance.
struct ArcRadiusMismatch;

impl Rule for ArcRadiusMismatch {
    fn id(&self) -> RuleId {
        RuleId::new(ARC_RADIUS_MISMATCH_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Error
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let tolerance_mm = model.tolerances.arc_radius_mm;
        let mut findings = Vec::new();

        for motion in &model.program.motions {
            let Some(arc) = motion.arc.as_deref() else {
                continue;
            };
            if !arc.radius_mm.is_finite() || !arc.end_radius_mm.is_finite() {
                continue;
            }
            let delta_mm = nc_core::math::abs(arc.radius_mm - arc.end_radius_mm);
            if delta_mm <= tolerance_mm {
                continue;
            }
            findings.push(radius_mismatch_finding(motion, arc, delta_mm, tolerance_mm));
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "An arc whose start and end radius disagree is not a true circle; the \
         controller must distort the path to close it, leaving a witness mark \
         or an undersized feature."
    }
}

/// Builds the finding for one inconsistent arc.
fn radius_mismatch_finding(
    motion: &MotionRecord,
    arc: &ArcRecord,
    delta_mm: f64,
    tolerance_mm: f64,
) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );
    evidence.insert("radius_mm".to_string(), EvidenceValue::Float(arc.radius_mm));
    evidence.insert(
        "end_radius_mm".to_string(),
        EvidenceValue::Float(arc.end_radius_mm),
    );
    evidence.insert("delta_mm".to_string(), EvidenceValue::Float(delta_mm));
    evidence.insert("threshold".to_string(), EvidenceValue::Float(tolerance_mm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert(
        "start_mm".to_string(),
        EvidenceValue::Point3(motion.start_mm),
    );
    evidence.insert("end_mm".to_string(), EvidenceValue::Point3(motion.end_mm));

    diagnostic(
        ARC_RADIUS_MISMATCH_ID,
        Severity::Error,
        motion.line,
        format!(
            "arc start radius {:.3} mm and end radius {:.3} mm differ by \
             {delta_mm:.3} mm",
            arc.radius_mm, arc.end_radius_mm
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.arc_smaller_than_tool
// ---------------------------------------------------------------------------

/// Stable rule id.
const ARC_SMALLER_THAN_TOOL_ID: &str = "geometry.arc_smaller_than_tool";

/// An arc tighter than the cutter radius.
struct ArcSmallerThanTool;

impl Rule for ArcSmallerThanTool {
    fn id(&self) -> RuleId {
        RuleId::new(ARC_SMALLER_THAN_TOOL_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements::with_tools(DIAMETER)
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let tolerance_mm = model.tolerances.arc_radius_mm;
        let mut findings = Vec::new();
        let mut missing: BTreeSet<u32> = BTreeSet::new();
        let mut eligible = 0usize;

        for motion in &model.program.motions {
            let Some(arc) = motion.arc.as_deref() else {
                continue;
            };
            // With cutter compensation active the programmed centerline is not
            // the finished radius, so the tool-radius comparison is invalid.
            if model.program.modal(motion.modal).cutter_comp != CutterComp::Off {
                continue;
            }
            eligible += 1;
            if !arc.radius_mm.is_finite() {
                continue;
            }
            let Some(diameter_mm) = tool_diameter(model, motion.tool) else {
                missing.insert(motion.tool);
                continue;
            };
            let tool_radius_mm = diameter_mm / 2.0;
            let shortfall_mm = tool_radius_mm - arc.radius_mm;
            if shortfall_mm <= tolerance_mm {
                continue;
            }
            findings.push(smaller_than_tool_finding(
                model,
                motion,
                arc,
                diameter_mm,
                shortfall_mm,
                tolerance_mm,
            ));
        }

        if eligible > 0
            && findings.is_empty()
            && let Some(tool) = missing.iter().next().copied()
        {
            return missing_diameter(tool);
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A cutter cannot follow an internal arc tighter than its own radius; \
         the controller cuts a larger corner or leaves stock where the drawing \
         expects a sharp feature."
    }
}

/// Builds the finding for one arc that is too tight for its tool.
fn smaller_than_tool_finding(
    model: &AnalysisModel<'_>,
    motion: &MotionRecord,
    arc: &ArcRecord,
    diameter_mm: f64,
    shortfall_mm: f64,
    tolerance_mm: f64,
) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert("radius_mm".to_string(), EvidenceValue::Float(arc.radius_mm));
    evidence.insert(
        "tool_diameter_mm".to_string(),
        EvidenceValue::Float(diameter_mm),
    );
    evidence.insert(
        "shortfall_mm".to_string(),
        EvidenceValue::Float(shortfall_mm),
    );
    evidence.insert("threshold".to_string(), EvidenceValue::Float(tolerance_mm));
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );
    insert_tool_source(&mut evidence, model, motion.tool, ToolField::Diameter);

    diagnostic(
        ARC_SMALLER_THAN_TOOL_ID,
        Severity::Warning,
        motion.line,
        format!(
            "arc radius {:.3} mm is smaller than the {:.3} mm radius of tool T{}",
            arc.radius_mm,
            diameter_mm / 2.0,
            motion.tool
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.feed_out_of_range
// ---------------------------------------------------------------------------

/// Stable rule id.
const FEED_OUT_OF_RANGE_ID: &str = "geometry.feed_out_of_range";

/// A cutting feed above the configured machine maximum.
struct FeedOutOfRange;

impl Rule for FeedOutOfRange {
    fn id(&self) -> RuleId {
        RuleId::new(FEED_OUT_OF_RANGE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            machine: true,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let Some(machine) = model.machine else {
            return RuleOutcome::Skipped {
                missing_input: "machine".to_string(),
                reason: "machine limits are required".to_string(),
            };
        };
        let Some(max_feed_mm_min) = machine.max_feed_mm_min.filter(|value| value.is_finite())
        else {
            return RuleOutcome::Skipped {
                missing_input: "machine.max_feed_mm_min".to_string(),
                reason: "machine has no maximum feed rate configured".to_string(),
            };
        };

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            if !motion.kind.is_cutting() {
                continue;
            }
            let Some(feed_mm_min) = motion.feed_mm_min else {
                continue;
            };
            if !feed_mm_min.is_finite() || feed_mm_min <= max_feed_mm_min {
                continue;
            }
            findings.push(feed_finding(motion, feed_mm_min, max_feed_mm_min));
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Feeding faster than the machine or setup allows overloads the cutter \
         and the drives; the controller cannot follow the programmed path and \
         the tool may break."
    }
}

/// Builds the finding for one feed above the machine maximum.
fn feed_finding(motion: &MotionRecord, feed_mm_min: f64, limit_mm_min: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert("feed_mm_min".to_string(), EvidenceValue::Float(feed_mm_min));
    evidence.insert("threshold".to_string(), EvidenceValue::Float(limit_mm_min));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );

    diagnostic(
        FEED_OUT_OF_RANGE_ID,
        Severity::Warning,
        motion.line,
        format!(
            "feed {feed_mm_min:.1} mm/min exceeds the machine maximum of \
             {limit_mm_min:.1} mm/min"
        ),
        evidence,
    )
}

// ---------------------------------------------------------------------------
// geometry.spindle_out_of_range
// ---------------------------------------------------------------------------

/// Stable rule id.
const SPINDLE_OUT_OF_RANGE_ID: &str = "geometry.spindle_out_of_range";

/// A spindle speed above the configured machine maximum.
struct SpindleOutOfRange;

impl Rule for SpindleOutOfRange {
    fn id(&self) -> RuleId {
        RuleId::new(SPINDLE_OUT_OF_RANGE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            machine: true,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        let Some(machine) = model.machine else {
            return RuleOutcome::Skipped {
                missing_input: "machine".to_string(),
                reason: "machine limits are required".to_string(),
            };
        };
        let Some(max_spindle_rpm) = machine.max_spindle_rpm.filter(|value| value.is_finite())
        else {
            return RuleOutcome::Skipped {
                missing_input: "machine.max_spindle_rpm".to_string(),
                reason: "machine has no maximum spindle speed configured".to_string(),
            };
        };

        let mut findings = Vec::new();
        for motion in &model.program.motions {
            let Some(spindle_rpm) = motion.spindle_rpm else {
                continue;
            };
            if !spindle_rpm.is_finite() || spindle_rpm <= max_spindle_rpm {
                continue;
            }
            findings.push(spindle_finding(motion, spindle_rpm, max_spindle_rpm));
        }

        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Running the spindle above its rated speed overheats the bearings and \
         can exceed the tool's safe surface speed, shortening tool life or \
         damaging the spindle."
    }
}

/// Builds the finding for one spindle speed above the machine maximum.
fn spindle_finding(motion: &MotionRecord, spindle_rpm: f64, limit_rpm: f64) -> Diagnostic {
    let mut evidence = BTreeMap::new();
    evidence.insert("spindle_rpm".to_string(), EvidenceValue::Float(spindle_rpm));
    evidence.insert("threshold".to_string(), EvidenceValue::Float(limit_rpm));
    evidence.insert(
        "tool".to_string(),
        EvidenceValue::Int(i64::from(motion.tool)),
    );
    evidence.insert(
        "kind".to_string(),
        EvidenceValue::Text(motion.kind.label().to_string()),
    );

    diagnostic(
        SPINDLE_OUT_OF_RANGE_ID,
        Severity::Warning,
        motion.line,
        format!(
            "spindle speed {spindle_rpm:.0} rpm exceeds the machine maximum of \
             {limit_rpm:.0} rpm"
        ),
        evidence,
    )
}
