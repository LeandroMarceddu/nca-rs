//! Stock inference and the stock box conversion.

use nc_gcode::{MotionRecord, Program};
use nc_viewer::{Bounds3, Point3, StockBox};
use serde::{Deserialize, Serialize};

use crate::StockError;
use crate::arc::arc_bounds;

/// Where the stock dimensions came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockSource {
    /// Fitted to the program's cutting bounds.
    Inferred,
    /// Supplied by the user.
    Explicit,
}

/// Why an inferred stock got its dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceBasis {
    /// Some cutting move went below Z0, so Z0 is treated as the stock top.
    ZeroAtTop,
    /// Every cut stayed above Z0; the stock top is above the highest cut.
    AllCutsAboveZero,
    /// There were no cutting moves; all motion extents were used instead.
    ///
    /// With no motion records at all, the `f32` display bounds are the
    /// fallback.
    NoCuttingMoves,
}

/// A rectangular stock block in machine millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StockModel {
    /// Minimum corner in millimetres.
    pub min_mm: [f64; 3],
    /// Maximum corner in millimetres.
    pub max_mm: [f64; 3],
    /// Z of the top face in millimetres.
    pub top_z_mm: f64,
    /// Where the dimensions came from.
    pub source: StockSource,
    /// Inference basis; `Some` only when `source == Inferred`.
    pub inference: Option<InferenceBasis>,
}

impl StockModel {
    /// Infers stock from the program's exact motion extents.
    ///
    /// Cutting bounds exclude safe-Z rapids, so a tall retract does not inflate
    /// the block. The XY margin is `max(2% of the larger cutting XY extent,
    /// 1 mm)`. If any cut is below Z0 the top is Z0; otherwise the top sits
    /// 2 mm above the highest cut and the block is 10 mm thick. With no cutting
    /// moves, all motion extents are used and the basis is
    /// [`InferenceBasis::NoCuttingMoves`].
    ///
    /// Arcs contribute their exact extents, not just their endpoints. When the
    /// program has motion records, only their exact `f64` extents are used; the
    /// display bounds are `f32` and are consulted only for a legacy program
    /// with no motion records. An empty program gets a small finite block
    /// around the origin rather than an infinite one.
    #[must_use]
    pub fn infer(program: &Program) -> Self {
        let mut cutting = Extents::empty();
        let mut all = Extents::empty();
        for motion in program.expanded_motions() {
            expand_motion(&mut all, &motion);
            if motion.kind.is_cutting() {
                expand_motion(&mut cutting, &motion);
            }
        }
        if program.motions.is_empty() {
            // Legacy/display-only fallback. `Bounds3` stores `f32`, so this
            // path is less precise; it must never override exact motion
            // extents when those exist.
            if let Some(bounds) = Extents::from_viewer(program.bounds) {
                all.union(bounds);
            }
            if let Some(bounds) = Extents::from_viewer(program.cutting_bounds()) {
                cutting.union(bounds);
            }
        }
        if !cutting.is_empty() {
            let basis = if cutting.min[2] < 0.0 {
                InferenceBasis::ZeroAtTop
            } else {
                InferenceBasis::AllCutsAboveZero
            };
            Self::infer_from_bounds(cutting, basis)
        } else if !all.is_empty() {
            Self::infer_from_bounds(all, InferenceBasis::NoCuttingMoves)
        } else {
            Self::infer_from_bounds(Self::empty_program_bounds(), InferenceBasis::NoCuttingMoves)
        }
    }

    fn infer_from_bounds(bounds: Extents, basis: InferenceBasis) -> Self {
        let extent_x = (bounds.max[0] - bounds.min[0]).max(0.0);
        let extent_y = (bounds.max[1] - bounds.min[1]).max(0.0);
        let margin = (extent_x.max(extent_y) * 0.02).max(1.0);
        let top_z = match basis {
            InferenceBasis::ZeroAtTop => 0.0,
            InferenceBasis::AllCutsAboveZero => bounds.max[2] + 2.0,
            InferenceBasis::NoCuttingMoves => bounds.max[2].max(0.0),
        };
        let bottom_z = match basis {
            InferenceBasis::AllCutsAboveZero => top_z - 10.0,
            InferenceBasis::ZeroAtTop | InferenceBasis::NoCuttingMoves => bounds.min[2] - 2.0,
        };
        Self {
            min_mm: [
                bounds.min[0] - margin,
                bounds.min[1] - margin,
                bottom_z.min(top_z - 0.001),
            ],
            max_mm: [bounds.max[0] + margin, bounds.max[1] + margin, top_z],
            top_z_mm: top_z,
            source: StockSource::Inferred,
            inference: Some(basis),
        }
    }

    fn empty_program_bounds() -> Extents {
        Extents {
            min: [-10.0, -10.0, -10.0],
            max: [10.0, 10.0, 0.0],
        }
    }

    /// Builds an explicit stock block from its corners.
    ///
    /// Returns an error when an extent is non-finite or degenerate.
    pub fn explicit(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, StockError> {
        let finite = min_mm
            .iter()
            .chain(max_mm.iter())
            .all(|value| value.is_finite());
        let ordered = (0..3).all(|axis| min_mm[axis] < max_mm[axis]);
        if !finite || !ordered {
            return Err(StockError::InvalidExtent { min_mm, max_mm });
        }
        Ok(Self {
            min_mm,
            max_mm,
            top_z_mm: max_mm[2],
            source: StockSource::Explicit,
            inference: None,
        })
    }

    /// The stock top used by clearance rules.
    #[must_use]
    pub fn top_z(&self) -> f64 {
        self.top_z_mm
    }

    /// XY extent in millimetres.
    #[must_use]
    pub fn xy_extent_mm(&self) -> [f64; 2] {
        [
            self.max_mm[0] - self.min_mm[0],
            self.max_mm[1] - self.min_mm[1],
        ]
    }

    /// Returns `true` when `point_mm` lies inside the block.
    #[must_use]
    pub fn contains(&self, point_mm: [f64; 3]) -> bool {
        (0..3)
            .all(|axis| point_mm[axis] >= self.min_mm[axis] && point_mm[axis] <= self.max_mm[axis])
    }

    /// Converts to the display-only viewer box.
    #[must_use]
    pub fn to_stock_box(&self) -> Option<StockBox> {
        let min = Point3::new(
            self.min_mm[0] as f32,
            self.min_mm[1] as f32,
            self.min_mm[2] as f32,
        );
        let max = Point3::new(
            self.max_mm[0] as f32,
            self.max_mm[1] as f32,
            self.max_mm[2] as f32,
        );
        StockBox::new(min, max)
    }
}

#[derive(Debug, Clone, Copy)]
struct Extents {
    min: [f64; 3],
    max: [f64; 3],
}

impl Extents {
    fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }

    fn is_empty(self) -> bool {
        (0..3).any(|axis| self.min[axis] > self.max[axis])
    }

    fn expand_point(&mut self, point: [f64; 3]) {
        if !point.iter().all(|value| value.is_finite()) {
            return;
        }
        for (axis, value) in point.iter().enumerate() {
            self.min[axis] = self.min[axis].min(*value);
            self.max[axis] = self.max[axis].max(*value);
        }
    }

    fn expand_min_max(&mut self, min: [f64; 3], max: [f64; 3]) {
        self.expand_point(min);
        self.expand_point(max);
    }

    fn union(&mut self, other: Self) {
        if other.is_empty() {
            return;
        }
        self.expand_min_max(other.min, other.max);
    }

    fn from_viewer(bounds: Bounds3) -> Option<Self> {
        if bounds.is_empty() {
            return None;
        }
        let min = bounds.min();
        let max = bounds.max();
        let min = [f64::from(min.x), f64::from(min.y), f64::from(min.z)];
        let max = [f64::from(max.x), f64::from(max.y), f64::from(max.z)];
        if !min.iter().chain(max.iter()).all(|value| value.is_finite()) {
            return None;
        }
        Some(Self { min, max })
    }
}

fn expand_motion(extents: &mut Extents, motion: &MotionRecord) {
    extents.expand_point(motion.start_mm);
    extents.expand_point(motion.end_mm);
    if let Some(arc) = &motion.arc {
        let (min, max) = arc_bounds(arc, motion.plane, motion.start_mm, motion.end_mm);
        extents.expand_min_max(min, max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_gcode::ProgramBuilder;
    use nc_viewer::MoveKind;

    #[test]
    fn explicit_stock_validates() {
        let stock = StockModel::explicit([0.0, 0.0, -20.0], [100.0, 80.0, 0.0]).expect("valid");
        assert_eq!(stock.source, StockSource::Explicit);
        assert_eq!(stock.top_z(), 0.0);
        assert!(stock.contains([50.0, 40.0, -10.0]));
        assert!(!stock.contains([101.0, 40.0, -10.0]));
        assert!(stock.to_stock_box().is_some());
        assert_eq!(stock.inference, None);
    }

    #[test]
    fn degenerate_explicit_stock_is_rejected() {
        assert!(StockModel::explicit([0.0, 0.0, 0.0], [0.0, 1.0, 1.0]).is_err());
    }

    #[test]
    fn inference_ignores_safe_z_rapids() {
        let program = ProgramBuilder::new()
            .motion(
                1,
                MoveKind::Rapid,
                [0.0, 0.0, 50.0],
                [50.0, 50.0, 50.0],
                None,
            )
            .motion(
                2,
                MoveKind::Feed,
                [10.0, 10.0, -1.0],
                [20.0, 20.0, -1.0],
                Some(500.0),
            )
            .build();
        let stock = StockModel::infer(&program);
        assert_eq!(stock.source, StockSource::Inferred);
        assert_eq!(stock.inference, Some(InferenceBasis::ZeroAtTop));
        assert_eq!(stock.top_z(), 0.0);
        assert_eq!(stock.max_mm[2], 0.0);
        assert_eq!(stock.min_mm[2], -3.0);
        assert_eq!(stock.min_mm[0], 9.0);
        assert_eq!(stock.max_mm[0], 21.0);
        assert_eq!(stock.min_mm[1], 9.0);
        assert_eq!(stock.max_mm[1], 21.0);
    }

    #[test]
    fn above_zero_cuts_get_a_ten_mm_block() {
        let program = ProgramBuilder::new()
            .motion(
                1,
                MoveKind::Feed,
                [0.0, 0.0, 5.0],
                [10.0, 10.0, 5.0],
                Some(500.0),
            )
            .build();
        let stock = StockModel::infer(&program);
        assert_eq!(stock.inference, Some(InferenceBasis::AllCutsAboveZero));
        assert_eq!(stock.top_z(), 7.0);
        assert_eq!(stock.max_mm[2], 7.0);
        assert_eq!(stock.min_mm[2], -3.0);
        assert_eq!(stock.min_mm[0], -1.0);
        assert_eq!(stock.max_mm[0], 11.0);
    }

    #[test]
    fn no_cutting_moves_use_all_path_bounds() {
        let program = ProgramBuilder::new()
            .motion(1, MoveKind::Rapid, [-5.0, -5.0, 5.0], [5.0, 5.0, 5.0], None)
            .build();
        let stock = StockModel::infer(&program);
        assert_eq!(stock.inference, Some(InferenceBasis::NoCuttingMoves));
        assert_eq!(stock.top_z(), 5.0);
        assert_eq!(stock.min_mm[2], 3.0);
        assert_eq!(stock.min_mm[0], -6.0);
        assert_eq!(stock.max_mm[0], 6.0);
    }

    #[test]
    fn empty_program_stays_finite() {
        let program = ProgramBuilder::new().build();
        let stock = StockModel::infer(&program);
        assert!(stock.min_mm.iter().all(|value| value.is_finite()));
        assert!(stock.max_mm.iter().all(|value| value.is_finite()));
        assert!(stock.to_stock_box().is_some());
        assert_eq!(stock.inference, Some(InferenceBasis::NoCuttingMoves));
    }

    #[test]
    fn arc_extents_are_included_in_inference() {
        let arc = nc_gcode::ArcRecord {
            center_mm: [50.0, 0.0, 0.0],
            radius_mm: 50.0,
            end_radius_mm: 50.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: -2.0,
            format: nc_gcode::ArcFormat::Center,
        };
        let mut motion = MotionRecord::linear(
            1,
            0,
            MoveKind::ArcCcw,
            [100.0, 0.0, 0.0],
            [100.0, 0.0, -2.0],
        );
        motion.arc = Some(Box::new(arc));
        let program = ProgramBuilder::new().record(motion).build();
        let stock = StockModel::infer(&program);
        // The half circle reaches y = -50; the display chord would not.
        assert!(stock.min_mm[1] <= -51.0);
        assert!(stock.max_mm[1] >= 51.0);
        assert_eq!(stock.inference, Some(InferenceBasis::ZeroAtTop));
    }

    #[test]
    fn arc_extents_are_exact_without_display_rounding() {
        // Center and radius are deliberately not representable in `f32`. The
        // exact extents are X 0.1..100.1 and Y -49.9..50.1, so a 2 mm margin
        // gives Y -51.9..52.1. Unioning the f32 display bounds would move both
        // limits by about 1.5e-6 mm.
        let arc = nc_gcode::ArcRecord {
            center_mm: [50.1, 0.1, -1.0],
            radius_mm: 50.0,
            end_radius_mm: 50.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: 0.0,
            format: nc_gcode::ArcFormat::Center,
        };
        let mut motion = MotionRecord::linear(
            1,
            0,
            MoveKind::ArcCcw,
            [100.1, 0.1, -1.0],
            [100.1, 0.1, -1.0],
        );
        motion.arc = Some(Box::new(arc));
        let program = ProgramBuilder::new().record(motion).build();
        let stock = StockModel::infer(&program);

        assert!(
            (stock.min_mm[1] - (-51.9)).abs() < 1e-9,
            "{:?}",
            stock.min_mm
        );
        assert!((stock.max_mm[1] - 52.1).abs() < 1e-9, "{:?}", stock.max_mm);
        assert_eq!(stock.inference, Some(InferenceBasis::ZeroAtTop));
    }

    #[test]
    fn motion_extents_do_not_round_through_f32_display_bounds() {
        // -10.0001 and 10.0001 are not representable in `f32`. With motion
        // records present, inference must use the exact f64 endpoints only.
        let program = ProgramBuilder::new()
            .motion(
                1,
                MoveKind::Feed,
                [-10.0001, 0.0, -1.0],
                [0.0, 10.0001, -1.0],
                Some(500.0),
            )
            .build();
        let stock = StockModel::infer(&program);

        assert!(
            (stock.min_mm[0] - (-11.0001)).abs() < 1e-12,
            "{:?}",
            stock.min_mm
        );
        assert!(
            (stock.max_mm[1] - 11.0001).abs() < 1e-12,
            "{:?}",
            stock.max_mm
        );
        assert_eq!(stock.min_mm[2], -3.0);
        assert_eq!(stock.max_mm[2], 0.0);
        assert_eq!(stock.inference, Some(InferenceBasis::ZeroAtTop));
    }
}
