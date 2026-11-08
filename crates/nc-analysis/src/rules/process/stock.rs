//! Final-state stock rules: shank/holder collision, thin walls, rest material.
//!
//! The shank rule reads the shared engagement pass. The wall and rest rules
//! inspect the final heightmap after the whole program has run and are skipped
//! when the stock was inferred, because inferred stock is fitted to the cuts
//! and would manufacture or erase the feature being checked.

use std::collections::{BTreeSet, VecDeque};

use nc_core::ToolField;
use nc_core::math;
use nc_gcode::MotionRecord;
use nc_stock::{Heightmap, arc_point};

use crate::config::RuleSettings;
use crate::report::Diagnostic;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome, StockRequirement};
use crate::types::{Basis, EvidenceValue, RuleId, Severity};

use super::{
    diagnostic, engagement_data, insert_tool_source, invalid_tool_skip, positive, skip,
    stock_evidence, stock_skip, threshold,
};

/// Stable rule id for the shank/holder collision rule.
const SHANK_COLLISION_ID: &str = "process.shank_collision";
/// Stable rule id for the thin-wall rule.
const THIN_WALL_ID: &str = "process.thin_wall";
/// Stable rule id for the rest-material rule.
const REST_MATERIAL_ID: &str = "process.rest_material";

/// Rules in catalog order.
#[must_use]
pub(crate) fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(ShankCollision),
        Box::new(ThinWall),
        Box::new(RestMaterial),
    ]
}

/// Flags a cut deeper than the fluted length (shank) or stickout (holder).
struct ShankCollision;

impl Rule for ShankCollision {
    fn id(&self) -> RuleId {
        RuleId::new(SHANK_COLLISION_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: &[ToolField::FluteLength],
            stock: StockRequirement::InferredOk,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::FluteLength, "flute length") {
            return outcome;
        }
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };

        let mut findings = Vec::new();
        let mut seen: BTreeSet<(u32, &'static str)> = BTreeSet::new();
        for pass in &data.passes {
            let Some(tool_number) = super::pass_tool_number(model, pass) else {
                continue;
            };
            let Some(tool) = model.tools.get(tool_number) else {
                continue;
            };
            let Some(flute_length_mm) = positive(tool.flute_length_mm) else {
                continue;
            };
            if pass.axial_doc_mm <= 0.0 {
                continue;
            }
            // Conservative: use the deepest point below the stock top, not just
            // the pre-cut height at the tool centre, so an uncleared wall around
            // the pass cannot hide the shank or holder.
            let cut_z_mm = model
                .program
                .motions
                .get(pass.motion_index)
                .map(|motion| motion.start_mm[2].min(motion.end_mm[2]))
                .unwrap_or(f64::NAN);
            let depth_mm = if cut_z_mm.is_finite() {
                (model.stock.top_z_mm - cut_z_mm).max(pass.axial_doc_mm)
            } else {
                pass.axial_doc_mm
            };

            if depth_mm > flute_length_mm
                && seen.insert((tool_number, "shank"))
                && let Some(finding) = collision_finding(
                    model,
                    pass.line,
                    tool_number,
                    "shank",
                    depth_mm,
                    flute_length_mm,
                    tool.effective_shank_diameter_mm(),
                )
            {
                findings.push(finding);
            }

            let holder_diameter_mm = tool
                .holder_diameter_mm
                .filter(|value| value.is_finite() && *value > 0.0);
            let stickout_mm = tool
                .stickout_mm
                .filter(|value| value.is_finite() && *value > 0.0);
            if let (Some(holder_diameter_mm), Some(stickout_mm)) = (holder_diameter_mm, stickout_mm)
                && depth_mm > stickout_mm
                && seen.insert((tool_number, "holder"))
                && let Some(finding) = collision_finding(
                    model,
                    pass.line,
                    tool_number,
                    "holder",
                    depth_mm,
                    stickout_mm,
                    Some(holder_diameter_mm),
                )
            {
                findings.push(finding);
            }
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "If the shank or holder reaches the stock, the flutes cannot clear chips \
         and the unfluted body grinds against the part, burning the cutter and \
         ruining the wall."
    }
}

/// Builds one shank or holder collision finding.
fn collision_finding(
    model: &AnalysisModel<'_>,
    line: usize,
    tool: u32,
    part: &'static str,
    depth_mm: f64,
    limit_mm: f64,
    diameter_mm: Option<f64>,
) -> Option<Diagnostic> {
    let mut evidence = stock_evidence(&model.stock);
    evidence.insert("tool".to_string(), EvidenceValue::Int(i64::from(tool)));
    insert_tool_source(
        &mut evidence,
        model,
        tool,
        if part == "shank" {
            &[ToolField::FluteLength]
        } else {
            &[ToolField::Stickout, ToolField::HolderDiameter]
        },
    );
    evidence.insert("part".to_string(), EvidenceValue::Text(part.to_string()));
    evidence.insert("depth_mm".to_string(), EvidenceValue::Float(depth_mm));
    evidence.insert("limit_mm".to_string(), EvidenceValue::Float(limit_mm));
    evidence.insert("measured".to_string(), EvidenceValue::Float(depth_mm));
    evidence.insert("threshold".to_string(), EvidenceValue::Float(limit_mm));
    evidence.insert(
        "penetration_mm".to_string(),
        EvidenceValue::Float((depth_mm - limit_mm).max(0.0)),
    );
    if let Some(diameter_mm) = diameter_mm
        && diameter_mm.is_finite()
        && diameter_mm > 0.0
    {
        evidence.insert("diameter_mm".to_string(), EvidenceValue::Float(diameter_mm));
    }

    Some(diagnostic(
        SHANK_COLLISION_ID,
        Severity::Warning,
        line,
        format!(
            "the {part} reaches {:.3} mm into the stock: cut depth {:.3} mm below \
             the stock top exceeds the {:.3} mm limit",
            (depth_mm - limit_mm).max(0.0),
            depth_mm,
            limit_mm
        ),
        evidence,
    ))
}

/// Flags a standing wall narrower than `min_wall_mm` in the final heightmap.
struct ThinWall;

impl Rule for ThinWall {
    fn id(&self) -> RuleId {
        RuleId::new(THIN_WALL_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            tool_fields: &[ToolField::Diameter],
            stock: StockRequirement::Explicit,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        if let Some(outcome) = invalid_tool_skip(model, ToolField::Diameter, "diameter") {
            return outcome;
        }
        let min_wall_mm = threshold(cfg, "min_wall_mm", 0.8);
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };
        let map = &data.final_heightmap;
        if map.cell_mm() > min_wall_mm {
            return skip(
                "stock",
                "the engagement grid is coarser than the wall threshold",
            );
        }
        let cols = map.cols();
        let rows = map.rows();
        let len = map.len();
        if cols < 3 || rows < 3 || len == 0 {
            return RuleOutcome::Findings(Vec::new());
        }
        let top_mm = map.top_z_mm();
        let mut cut = vec![false; len];
        for (index, cell) in map.cells().iter().enumerate() {
            if f64::from(*cell) < top_mm - 1.0e-9
                && let Some(slot) = cut.get_mut(index)
            {
                *slot = true;
            }
        }
        let (labels, region_count) = label_regions(cols, rows, &cut);
        if region_count == 0 {
            return RuleOutcome::Findings(Vec::new());
        }
        let stats = region_stats(map, &labels, region_count);
        let walls = scan_walls(map, &labels, region_count, min_wall_mm);

        let mut findings = Vec::new();
        for (index, wall) in walls.iter().enumerate() {
            let Some(wall) = wall else {
                continue;
            };
            let Some(stat) = stats.get(index) else {
                continue;
            };
            if stat.cells == 0 {
                continue;
            }
            let nearest = nearest_cut(model, wall.midpoint_xy);
            let line = nearest.as_ref().map_or(1, |reference| reference.line);
            let mut evidence = stock_evidence(&model.stock);
            evidence.insert(
                "axis".to_string(),
                EvidenceValue::Text(wall.axis.to_string()),
            );
            evidence.insert("wall_mm".to_string(), EvidenceValue::Float(wall.width_mm));
            evidence.insert("measured".to_string(), EvidenceValue::Float(wall.width_mm));
            evidence.insert("threshold".to_string(), EvidenceValue::Float(min_wall_mm));
            evidence.insert(
                "height_mm".to_string(),
                EvidenceValue::Float(wall.height_mm),
            );
            evidence.insert("cell_mm".to_string(), EvidenceValue::Float(map.cell_mm()));
            evidence.insert(
                "region_area_mm2".to_string(),
                EvidenceValue::Float(stat.area_mm2(map.cell_mm())),
            );
            evidence.insert(
                "region_cells".to_string(),
                EvidenceValue::Int(i64::try_from(stat.cells).unwrap_or(i64::MAX)),
            );
            if let Some(reference) = &nearest {
                evidence.insert(
                    "tool".to_string(),
                    EvidenceValue::Int(i64::from(reference.tool)),
                );
            }
            findings.push(diagnostic(
                THIN_WALL_ID,
                Severity::Warning,
                line,
                format!(
                    "a standing wall {:.3} mm wide is below the {:.3} mm minimum \
                     in the final stock state",
                    wall.width_mm, min_wall_mm
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "A thin wall left between two cuts can deflect, chatter, or break during \
         the finishing pass; the program should leave more material or support it."
    }
}

/// Flags enclosed rest material left above the finished floor.
struct RestMaterial;

impl Rule for RestMaterial {
    fn id(&self) -> RuleId {
        RuleId::new(REST_MATERIAL_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn basis(&self) -> Basis {
        Basis::Model
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            stock: StockRequirement::Explicit,
            ..Requirements::NONE
        }
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome {
        if let Some(outcome) = stock_skip(&model.stock) {
            return outcome;
        }
        let rest_material_mm = threshold(cfg, "rest_material_mm", 0.5);
        let data = match engagement_data(model) {
            Ok(data) => data,
            Err(outcome) => return outcome,
        };
        let map = &data.final_heightmap;
        let cols = map.cols();
        let rows = map.rows();
        let len = map.len();
        if cols == 0 || rows == 0 || len == 0 {
            return skip("stock", "final stock state has no cells");
        }

        // A cell is cut when the pass lowered it below the stock top. The
        // footprint is the cut mask, not its bounding rectangle: square-corner
        // pockets cut with a round tool leave stock outside the actual sweep.
        let top_mm = map.top_z_mm();
        let mut cut = vec![false; len];
        let mut lowest_mm = f64::INFINITY;
        for (index, cell) in map.cells().iter().enumerate() {
            let height_mm = f64::from(*cell);
            if height_mm < top_mm - 1.0e-9 {
                if let Some(slot) = cut.get_mut(index) {
                    *slot = true;
                }
                lowest_mm = lowest_mm.min(height_mm);
            }
        }
        if !lowest_mm.is_finite() {
            return RuleOutcome::Findings(Vec::new());
        }
        let limit_mm = lowest_mm + rest_material_mm;

        // Flood-fill uncut stock from the grid border. Uncut cells that are
        // unreachable are enclosed by cuts; only those can be rest material.
        let mut exterior = vec![false; len];
        let mut stack: Vec<usize> = Vec::new();
        let seed = |index: usize, exterior: &mut Vec<bool>, stack: &mut Vec<usize>| {
            if !cut[index] && !exterior[index] {
                exterior[index] = true;
                stack.push(index);
            }
        };
        for col in 0..cols {
            seed(col, &mut exterior, &mut stack);
            seed((rows - 1) * cols + col, &mut exterior, &mut stack);
        }
        for row in 0..rows {
            seed(row * cols, &mut exterior, &mut stack);
            seed(row * cols + (cols - 1), &mut exterior, &mut stack);
        }
        while let Some(index) = stack.pop() {
            let col = index % cols;
            let row = index / cols;
            if col > 0 {
                seed(index - 1, &mut exterior, &mut stack);
            }
            if col + 1 < cols {
                seed(index + 1, &mut exterior, &mut stack);
            }
            if row > 0 {
                seed(index - cols, &mut exterior, &mut stack);
            }
            if row + 1 < rows {
                seed(index + cols, &mut exterior, &mut stack);
            }
        }

        let mut mask = vec![false; len];
        for (index, cell) in map.cells().iter().enumerate() {
            let height_mm = f64::from(*cell);
            if cut[index] || exterior[index] || height_mm <= limit_mm {
                continue;
            }
            if let Some(slot) = mask.get_mut(index) {
                *slot = true;
            }
        }

        let (labels, region_count) = label_regions(cols, rows, &mask);
        let stats = region_stats(map, &labels, region_count);
        let mut findings = Vec::new();
        for stat in &stats {
            if stat.cells == 0 {
                continue;
            }
            let centroid = stat.centroid_xy();
            let nearest = nearest_cut(model, centroid);
            let line = nearest.as_ref().map_or(1, |reference| reference.line);

            let mut evidence = stock_evidence(&model.stock);
            evidence.insert(
                "area_mm2".to_string(),
                EvidenceValue::Float(stat.area_mm2(map.cell_mm())),
            );
            evidence.insert(
                "region_cells".to_string(),
                EvidenceValue::Int(i64::try_from(stat.cells).unwrap_or(i64::MAX)),
            );
            evidence.insert(
                "highest_mm".to_string(),
                EvidenceValue::Float(stat.highest_mm),
            );
            evidence.insert(
                "height_above_lowest_mm".to_string(),
                EvidenceValue::Float(stat.highest_mm - lowest_mm),
            );
            evidence.insert(
                "measured".to_string(),
                EvidenceValue::Float(stat.highest_mm - lowest_mm),
            );
            evidence.insert(
                "lowest_final_height_mm".to_string(),
                EvidenceValue::Float(lowest_mm),
            );
            evidence.insert(
                "threshold".to_string(),
                EvidenceValue::Float(rest_material_mm),
            );
            evidence.insert("cell_mm".to_string(), EvidenceValue::Float(map.cell_mm()));
            evidence.insert(
                "centroid_mm".to_string(),
                EvidenceValue::Point3([centroid[0], centroid[1], stat.highest_mm]),
            );
            if let Some(reference) = &nearest {
                evidence.insert(
                    "tool".to_string(),
                    EvidenceValue::Int(i64::from(reference.tool)),
                );
            }

            findings.push(diagnostic(
                REST_MATERIAL_ID,
                Severity::Info,
                line,
                format!(
                    "material up to {:.3} mm above the lowest final height remains \
                     over {:.1} mm2 inside the cleared footprint",
                    stat.highest_mm - lowest_mm,
                    stat.area_mm2(map.cell_mm())
                ),
                evidence,
            ));
        }
        RuleOutcome::Findings(findings)
    }

    fn why_it_matters(&self) -> &'static str {
        "Material left above the finished floor means the program did not clear \
         the pocket; a finishing pass can rub, overload, or miss the feature."
    }
}

/// One thin-wall candidate attached to a connected cut region.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WallCandidate {
    /// Narrowest width between the two cut sides, in millimetres.
    width_mm: f64,
    /// Lowest height of the standing ridge, in machine Z.
    height_mm: f64,
    /// Scan axis: `"X"` or `"Y"`.
    axis: &'static str,
    /// Midpoint of the ridge, used only for source-line attribution.
    midpoint_xy: [f64; 2],
}

/// A connected-region statistic.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RegionStat {
    cells: usize,
    sum_x: f64,
    sum_y: f64,
    highest_mm: f64,
}

impl RegionStat {
    fn empty() -> Self {
        Self {
            cells: 0,
            sum_x: 0.0,
            sum_y: 0.0,
            highest_mm: f64::NEG_INFINITY,
        }
    }

    fn centroid_xy(&self) -> [f64; 2] {
        if self.cells == 0 {
            return [0.0, 0.0];
        }
        let count = self.cells as f64;
        [self.sum_x / count, self.sum_y / count]
    }

    fn area_mm2(&self, cell_mm: f64) -> f64 {
        self.cells as f64 * cell_mm * cell_mm
    }
}

/// Labels 4-connected regions of `mask`, returning labels and the region count.
///
/// Labels are assigned in row-major discovery order, so the result is stable
/// for a fixed grid and mask.
fn label_regions(cols: usize, rows: usize, mask: &[bool]) -> (Vec<u32>, usize) {
    let mut labels = vec![0u32; mask.len()];
    let mut queue = VecDeque::new();
    let mut count = 0usize;

    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col;
            if index >= mask.len() || !mask[index] || labels[index] != 0 {
                continue;
            }
            count += 1;
            let label = u32::try_from(count).unwrap_or(u32::MAX);
            labels[index] = label;
            queue.push_back(index);

            while let Some(current) = queue.pop_front() {
                let current_row = current / cols.max(1);
                let current_col = current % cols.max(1);
                if current_col > 0 {
                    visit(current - 1, label, mask, &mut labels, &mut queue);
                }
                if current_col + 1 < cols {
                    visit(current + 1, label, mask, &mut labels, &mut queue);
                }
                if current_row > 0 {
                    visit(current - cols, label, mask, &mut labels, &mut queue);
                }
                if current_row + 1 < rows {
                    visit(current + cols, label, mask, &mut labels, &mut queue);
                }
            }
        }
    }
    (labels, count)
}

/// Adds one neighbour to a region when it is masked, unlabelled, and in range.
fn visit(index: usize, label: u32, mask: &[bool], labels: &mut [u32], queue: &mut VecDeque<usize>) {
    if index >= mask.len() {
        return;
    }
    if !mask[index] || labels[index] != 0 {
        return;
    }
    labels[index] = label;
    queue.push_back(index);
}

/// Collects size, centroid, and highest height for every labelled region.
fn region_stats(map: &Heightmap, labels: &[u32], count: usize) -> Vec<RegionStat> {
    let mut stats = vec![RegionStat::empty(); count];
    let cols = map.cols().max(1);
    for (index, label) in labels.iter().enumerate() {
        if *label == 0 {
            continue;
        }
        let Some(stat) = stats.get_mut((*label as usize).saturating_sub(1)) else {
            continue;
        };
        let Some(cell) = map.cells().get(index) else {
            continue;
        };
        let row = index / cols;
        let col = index % cols;
        let centre = map.cell_center_mm(col, row);
        stat.cells += 1;
        stat.sum_x += centre[0];
        stat.sum_y += centre[1];
        stat.highest_mm = stat.highest_mm.max(f64::from(*cell));
    }
    stats
}

/// Scans rows and columns for standing ridges narrower than `min_wall_mm`.
///
/// A ridge is a maximal run of cells higher than the cut cells on both sides.
/// The run must be bounded on both sides inside the grid, so the stock edge is
/// never mistaken for a wall. The narrowest ridge per connected cut region is
/// kept; ties keep the first ridge in row-major X-then-Y scan order.
fn scan_walls(
    map: &Heightmap,
    labels: &[u32],
    region_count: usize,
    min_wall_mm: f64,
) -> Vec<Option<WallCandidate>> {
    let mut walls: Vec<Option<WallCandidate>> = vec![None; region_count];
    let cols = map.cols();
    let rows = map.rows();
    if cols < 3 || rows < 3 {
        return walls;
    }

    for row in 0..rows {
        let mut col = 1;
        while col + 1 < cols {
            let Some(left) = height_at(map, col - 1, row) else {
                break;
            };
            let Some(here) = height_at(map, col, row) else {
                break;
            };
            if here <= left {
                col += 1;
                continue;
            }
            let mut end = col;
            while end + 1 < cols {
                let Some(next) = height_at(map, end + 1, row) else {
                    break;
                };
                if next <= left {
                    break;
                }
                end += 1;
            }
            if end + 1 < cols {
                record_wall(
                    map,
                    labels,
                    &mut walls,
                    col,
                    row,
                    end,
                    row,
                    min_wall_mm,
                    "X",
                );
            }
            col = end + 1;
        }
    }

    for col in 0..cols {
        let mut row = 1;
        while row + 1 < rows {
            let Some(low) = height_at(map, col, row - 1) else {
                break;
            };
            let Some(here) = height_at(map, col, row) else {
                break;
            };
            if here <= low {
                row += 1;
                continue;
            }
            let mut end = row;
            while end + 1 < rows {
                let Some(next) = height_at(map, col, end + 1) else {
                    break;
                };
                if next <= low {
                    break;
                }
                end += 1;
            }
            if end + 1 < rows {
                record_wall(
                    map,
                    labels,
                    &mut walls,
                    col,
                    row,
                    col,
                    end,
                    min_wall_mm,
                    "Y",
                );
            }
            row = end + 1;
        }
    }

    walls
}

/// Records one ridge run as a candidate for the region it borders.
///
/// `first` and `last` are `(col, row)` of the run's first and last cell.
#[allow(clippy::too_many_arguments)]
fn record_wall(
    map: &Heightmap,
    labels: &[u32],
    walls: &mut [Option<WallCandidate>],
    first_col: usize,
    first_row: usize,
    last_col: usize,
    last_row: usize,
    min_wall_mm: f64,
    axis: &'static str,
) {
    let run_cells = if axis == "X" {
        last_col - first_col + 1
    } else {
        last_row - first_row + 1
    };
    let width_mm = run_cells as f64 * map.cell_mm();
    if width_mm >= min_wall_mm {
        return;
    }

    let (left_index, right_index) = if axis == "X" {
        (
            first_row * map.cols() + first_col - 1,
            first_row * map.cols() + last_col + 1,
        )
    } else {
        (
            (first_row - 1) * map.cols() + first_col,
            (last_row + 1) * map.cols() + first_col,
        )
    };
    let left_label = labels.get(left_index).copied().unwrap_or(0);
    let right_label = labels.get(right_index).copied().unwrap_or(0);
    let label = match (left_label, right_label) {
        // A wall separates two cut regions. A cut-to-stock boundary is the
        // part edge, not a standing wall.
        (0, _) | (_, 0) => return,
        (left, right) => left.min(right),
    };
    let Some(slot) = walls.get_mut((label as usize).saturating_sub(1)) else {
        return;
    };

    let mut min_height = f64::INFINITY;
    if axis == "X" {
        for col in first_col..=last_col {
            if let Some(height) = height_at(map, col, first_row) {
                min_height = min_height.min(height);
            }
        }
    } else {
        for row in first_row..=last_row {
            if let Some(height) = height_at(map, first_col, row) {
                min_height = min_height.min(height);
            }
        }
    }
    if !min_height.is_finite() {
        return;
    }
    let first_centre = map.cell_center_mm(first_col, first_row);
    let last_centre = map.cell_center_mm(last_col, last_row);
    let midpoint_xy = [
        (first_centre[0] + last_centre[0]) * 0.5,
        (first_centre[1] + last_centre[1]) * 0.5,
    ];
    let candidate = WallCandidate {
        width_mm,
        height_mm: min_height,
        axis,
        midpoint_xy,
    };
    if slot.is_none_or(|current| candidate.width_mm < current.width_mm) {
        *slot = Some(candidate);
    }
}

/// Reads one cell height, or `None` when out of range.
fn height_at(map: &Heightmap, col: usize, row: usize) -> Option<f64> {
    map.get(col, row).map(f64::from)
}

/// A cutting motion near a final-state region, used for source attribution.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CutReference {
    line: usize,
    tool: u32,
}

/// Returns the cutting motion whose XY path is closest to `point_xy_mm`.
///
/// Ties keep the earlier motion in program order. Arcs are sampled on their
/// exact recorded curve; this is line attribution, never a reported distance.
fn nearest_cut(model: &AnalysisModel<'_>, point_xy_mm: [f64; 2]) -> Option<CutReference> {
    let mut best: Option<(f64, CutReference)> = None;
    for motion in model.program.expanded_motions() {
        if !motion.kind.is_cutting() {
            continue;
        }
        if !motion
            .start_mm
            .iter()
            .chain(motion.end_mm.iter())
            .all(|value| value.is_finite())
        {
            continue;
        }
        let distance_mm = distance_to_motion(&motion, point_xy_mm);
        if !distance_mm.is_finite() {
            continue;
        }
        if best.is_none_or(|(best_distance, _)| distance_mm < best_distance) {
            best = Some((
                distance_mm,
                CutReference {
                    line: motion.line,
                    tool: motion.tool,
                },
            ));
        }
    }
    best.map(|(_, reference)| reference)
}

/// Minimum XY distance from a point to a motion path.
fn distance_to_motion(motion: &MotionRecord, point_xy_mm: [f64; 2]) -> f64 {
    if let Some(arc) = &motion.arc {
        let mut best = f64::INFINITY;
        for step in 0..=16 {
            let fraction = step as f64 / 16.0;
            let point = arc_point(arc, motion.plane, motion.start_mm, motion.end_mm, fraction);
            best = best.min(math::hypot(
                point_xy_mm[0] - point[0],
                point_xy_mm[1] - point[1],
            ));
        }
        return best;
    }
    point_segment_distance(
        point_xy_mm,
        [motion.start_mm[0], motion.start_mm[1]],
        [motion.end_mm[0], motion.end_mm[1]],
    )
}

/// XY distance from a point to a line segment.
fn point_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length_squared = dx * dx + dy * dy;
    if length_squared <= 1.0e-24 {
        return math::hypot(point[0] - start[0], point[1] - start[1]);
    }
    let projection = ((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / length_squared;
    let fraction = math::clamp(projection, 0.0, 1.0);
    math::hypot(
        point[0] - (start[0] + dx * fraction),
        point[1] - (start[1] + dy * fraction),
    )
}
