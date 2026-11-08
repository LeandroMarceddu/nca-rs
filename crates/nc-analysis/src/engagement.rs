//! The shared, lazily built material-engagement pass for `process.*` rules.
//!
//! One deterministic 2.5D z-map pass walks [`Program::expanded_motions`] in
//! program order. Before each cut it samples the stock around the cutter
//! circumference at the cut Z to estimate radial engagement, then lowers the
//! covered cells with the shared `nc-stock` sweep. Canned cycles are consumed
//! through `expanded_motions`, so pecks and retracts are explicit sub-moves.
//!
//! The pass is headless: no ticks, no rendering state, and no time. It is
//! computed at most once per [`AnalysisModel`] and shared by every rule that
//! needs it.

use nc_core::math;
use nc_gcode::MotionRecord;
use nc_stock::{Cutter, Heightmap, StockError, SweepParams, arc_point};

use crate::rule::{AnalysisModel, EngagementData, PassEngagement};

/// Nominal engagement grid cell size in millimetres.
pub(crate) const ENGAGEMENT_CELL_MM: f64 = 0.75;

/// Hard cap on engagement grid cells.
pub(crate) const ENGAGEMENT_MAX_CELLS: usize = 4_000_000;

/// Number of points sampled around the cutter circumference.
///
/// Fixed so the contact-angle estimate and every report derived from it are
/// byte-identical on repeat runs.
const CIRCUMFERENCE_SAMPLES: usize = 64;

/// Motion fractions tried, in order, to find a probe point inside the grid.
///
/// The midpoint is tried first; the quarter points and endpoints let a cut that
/// crosses the stock boundary still contribute an engagement estimate.
const PROBE_FRACTIONS: [f64; 5] = [0.5, 0.25, 0.75, 0.0, 1.0];

/// Why the shared engagement pass could not be built.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum EngagementError {
    /// The stock extents are non-finite or degenerate.
    #[error("stock extents are degenerate")]
    Stock,
    /// The heightmap grid could not be constructed.
    #[error(transparent)]
    Grid(#[from] StockError),
    /// The shared cell could not be populated.
    #[error("engagement pass is unavailable")]
    Unavailable,
}

/// Returns the one shared engagement pass for this analysis run.
///
/// The first call builds the pass and stores it in
/// [`AnalysisModel::engagement`]; later calls return the same data. A build
/// failure is not cached, so a rule that skips for a transient reason cannot
/// poison the cell for the rest of the run.
pub(crate) fn engagement<'a>(
    model: &'a AnalysisModel<'_>,
) -> Result<&'a EngagementData, EngagementError> {
    if let Some(data) = model.engagement.get() {
        return Ok(data);
    }
    let data = build(model)?;
    // `OnceCell::set` only fails when a value is already present. Analysis is
    // single-threaded, so this cannot race; never panic if it somehow does.
    let _ = model.engagement.set(data);
    model.engagement.get().ok_or(EngagementError::Unavailable)
}

/// Builds the heightmap and the per-pass engagement estimates.
fn build(model: &AnalysisModel<'_>) -> Result<EngagementData, EngagementError> {
    let stock = &model.stock;
    if !has_valid_stock(stock) {
        return Err(EngagementError::Stock);
    }
    let size_xy = [
        stock.max_mm[0] - stock.min_mm[0],
        stock.max_mm[1] - stock.min_mm[1],
    ];
    let (mut heightmap, fitted_coarse) = Heightmap::new_fitted(
        [stock.min_mm[0], stock.min_mm[1]],
        size_xy,
        ENGAGEMENT_CELL_MM,
        stock.top_z_mm,
        ENGAGEMENT_MAX_CELLS,
    )?;
    let coarse = fitted_coarse || heightmap.cell_mm() > ENGAGEMENT_CELL_MM;

    let mut passes = Vec::new();
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

        let cutter = match model.tools.get(motion.tool) {
            Some(tool) => Cutter::from_tool(tool, None),
            None => Cutter::centerline(),
        };

        // The estimate is taken before the tool lowers any cell, so it sees the
        // stock this pass actually bites into.
        let probe_xy = probe_xy(&heightmap, &motion);
        let cut_z_mm = motion.start_mm[2].min(motion.end_mm[2]);
        let pre_cut_height_mm = probe_xy.and_then(|xy| sample_height(&heightmap, xy));
        let axial_doc_mm = pre_cut_height_mm.map_or(0.0, |height| (height - cut_z_mm).max(0.0));
        let (contact_angle_rad, radial_doc_mm) = match (probe_xy, cutter.radius_mm()) {
            (Some(xy), Some(radius)) if radius.is_finite() && radius > 0.0 => {
                circumference_contact(&heightmap, xy, radius, cut_z_mm)
            }
            _ => (0.0, 0.0),
        };
        let feed_mm_min = motion
            .feed_mm_min
            .filter(|feed| feed.is_finite() && *feed > 0.0)
            .unwrap_or(0.0);

        passes.push(PassEngagement {
            motion_index: motion.index,
            line: motion.line,
            kind: motion.kind,
            axial_doc_mm,
            radial_doc_mm,
            contact_angle_rad,
            mrr_mm3_min: axial_doc_mm * radial_doc_mm * feed_mm_min,
        });

        let _ =
            heightmap.cut_motion_with(&cutter, &motion, SweepParams::for_cell(heightmap.cell_mm()));
    }

    Ok(EngagementData {
        final_heightmap: heightmap,
        passes,
        coarse,
    })
}

/// Returns `true` when the stock box can host a heightmap.
fn has_valid_stock(stock: &nc_stock::StockModel) -> bool {
    let finite = stock
        .min_mm
        .iter()
        .chain(stock.max_mm.iter())
        .chain(std::iter::once(&stock.top_z_mm))
        .all(|value| value.is_finite());
    let ordered = (0..3).all(|axis| stock.min_mm[axis] < stock.max_mm[axis]);
    finite && ordered
}

/// Returns the cell height at a point, when the point is on the grid.
fn sample_height(map: &Heightmap, point_xy_mm: [f64; 2]) -> Option<f64> {
    let (col, row) = map.cell_at(point_xy_mm)?;
    map.get(col, row).map(f64::from)
}

/// Picks a probe point on the motion that lies inside the heightmap.
///
/// `None` means the whole motion is outside the stock footprint, in which case
/// no material is engaged and the pass records zero depth and zero contact.
fn probe_xy(map: &Heightmap, motion: &MotionRecord) -> Option<[f64; 2]> {
    for fraction in PROBE_FRACTIONS {
        let point = point_at(motion, fraction);
        if map.cell_at([point[0], point[1]]).is_some() {
            return Some([point[0], point[1]]);
        }
    }
    None
}

/// Exact point on a straight, arc, or helix motion at `fraction`.
fn point_at(motion: &MotionRecord, fraction: f64) -> [f64; 3] {
    match &motion.arc {
        Some(arc) => arc_point(arc, motion.plane, motion.start_mm, motion.end_mm, fraction),
        None => [
            motion.start_mm[0] + (motion.end_mm[0] - motion.start_mm[0]) * fraction,
            motion.start_mm[1] + (motion.end_mm[1] - motion.start_mm[1]) * fraction,
            motion.start_mm[2] + (motion.end_mm[2] - motion.start_mm[2]) * fraction,
        ],
    }
}

/// Estimates contact angle and radial depth from the circumference.
///
/// The fraction of the sampled circumference whose stock is above the cut Z
/// becomes `theta`; `radial_DOC = R * (1 - cos(theta / 2))`, capped at the
/// cutter diameter. A full circle of stock gives a full slot (`D`); half a
/// circle gives `R`.
fn circumference_contact(
    map: &Heightmap,
    center_xy_mm: [f64; 2],
    radius_mm: f64,
    cut_z_mm: f64,
) -> (f64, f64) {
    let samples = CIRCUMFERENCE_SAMPLES as f64;
    let mut contacts = 0usize;
    for index in 0..CIRCUMFERENCE_SAMPLES {
        let angle = math::TAU * (index as f64) / samples;
        let point = [
            center_xy_mm[0] + radius_mm * math::cos(angle),
            center_xy_mm[1] + radius_mm * math::sin(angle),
        ];
        if let Some(height) = sample_height(map, point)
            && height > cut_z_mm
        {
            contacts += 1;
        }
    }
    let contact_angle_rad = math::TAU * (contacts as f64) / samples;
    let radial_doc_mm =
        (radius_mm * (1.0 - math::cos(contact_angle_rad * 0.5))).min(radius_mm * 2.0);
    (contact_angle_rad, radial_doc_mm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_gcode::{MotionRecord, MoveKind, Plane};
    use nc_stock::StockModel;

    fn motion(start_mm: [f64; 3], end_mm: [f64; 3]) -> MotionRecord {
        MotionRecord::linear(1, 0, MoveKind::Feed, start_mm, end_mm)
    }

    #[test]
    fn straight_midpoint_is_used_for_the_probe() {
        let map = Heightmap::new([0.0, 0.0], [20.0, 20.0], 1.0, 0.0, 10_000).expect("heightmap");
        let point = probe_xy(&map, &motion([2.0, 4.0, 0.0], [8.0, 4.0, 0.0]));
        assert_eq!(point, Some([5.0, 4.0]));
    }

    #[test]
    fn a_motion_entirely_outside_has_no_probe() {
        let map = Heightmap::new([0.0, 0.0], [20.0, 20.0], 1.0, 0.0, 10_000).expect("heightmap");
        assert_eq!(
            probe_xy(&map, &motion([30.0, 4.0, 0.0], [40.0, 4.0, 0.0])),
            None
        );
    }

    #[test]
    fn arc_midpoint_follows_the_recorded_curve() {
        let map = Heightmap::new([0.0; 2], [100.0; 2], 1.0, 0.0, 100_000).expect("heightmap");
        let mut record = motion([10.0, 0.0, 0.0], [0.0, 10.0, 0.0]);
        record.plane = Plane::Xy;
        record.arc = Some(Box::new(nc_gcode::ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: math::FRAC_PI_2,
            axial_delta_mm: 0.0,
            format: nc_gcode::ArcFormat::Center,
        }));
        let point = probe_xy(&map, &record).expect("inside");
        assert!(math::abs(point[0] - 7.071_067_811_865_475) < 1.0e-9);
        assert!(math::abs(point[1] - 7.071_067_811_865_475) < 1.0e-9);
    }

    #[test]
    fn full_circumference_contact_is_a_full_diameter() {
        let map = Heightmap::new([0.0; 2], [20.0; 2], 0.5, 0.0, 10_000).expect("heightmap");
        let (angle, radial) = circumference_contact(&map, [10.0, 10.0], 3.0, -1.0);
        assert!(math::abs(angle - math::TAU) < 1.0e-12);
        assert!(math::abs(radial - 6.0) < 1.0e-12);
    }

    #[test]
    fn no_stock_above_the_cut_is_zero_engagement() {
        let map = Heightmap::new([0.0; 2], [20.0; 2], 0.5, 0.0, 10_000).expect("heightmap");
        let (angle, radial) = circumference_contact(&map, [10.0, 10.0], 3.0, 1.0);
        assert_eq!(angle, 0.0);
        assert_eq!(radial, 0.0);
    }

    #[test]
    fn degenerate_stock_is_rejected() {
        let stock = StockModel {
            min_mm: [0.0, 0.0, -1.0],
            max_mm: [0.0, 10.0, 0.0],
            top_z_mm: 0.0,
            source: nc_stock::StockSource::Explicit,
            inference: None,
        };
        assert!(!has_valid_stock(&stock));
    }
}
