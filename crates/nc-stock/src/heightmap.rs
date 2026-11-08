//! The 2.5D material heightmap and the deterministic cutter sweep.
//!
//! Cells are stored as `f32`; every computation that produces a height is
//! `f64`, and the store is the only rounding boundary. Iteration is row-major
//! with a fixed order, so the content hash is reproducible.
//!
//! The sweep is shared with the analysis engagement pass: it walks exact
//! motions and lowers cells, with no tick loop and no rendering state.

use nc_core::ToolShape;
use nc_core::math;
use nc_gcode::{ArcRecord, MotionRecord, Plane};

use crate::arc::arc_point;
use crate::{Cutter, StockError};

/// Default cap on sub-segments for one straight motion in [`Heightmap::cut_motion`].
const MAX_MOTION_STEPS: usize = 100_000;

/// Tuning for one sweep operation.
///
/// `None` means "derive from the grid": half a cell for the centreline step and
/// a quarter cell for arc chord tolerance. The step cap prevents a malformed or
/// extreme motion from producing an unbounded loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepParams {
    /// Maximum distance between centreline samples, in millimetres.
    pub max_step_mm: Option<f64>,
    /// Default arc chord tolerance, in millimetres.
    pub chord_tolerance_mm: Option<f64>,
    /// Maximum subdivision steps for one centreline or one arc.
    pub max_steps: usize,
}

impl Default for SweepParams {
    fn default() -> Self {
        Self {
            max_step_mm: None,
            chord_tolerance_mm: None,
            max_steps: Self::DEFAULT_MAX_STEPS,
        }
    }
}

impl SweepParams {
    /// Default upper bound on subdivision steps.
    pub const DEFAULT_MAX_STEPS: usize = 1_000_000;

    /// Params that follow the grid: half-cell centreline steps and a
    /// quarter-cell arc chord tolerance.
    #[must_use]
    pub fn for_cell(cell_mm: f64) -> Self {
        let cell = if cell_mm.is_finite() && cell_mm > 0.0 {
            cell_mm
        } else {
            1.0
        };
        Self {
            max_step_mm: Some(cell * 0.5),
            chord_tolerance_mm: Some(cell * 0.25),
            max_steps: Self::DEFAULT_MAX_STEPS,
        }
    }

    fn step_mm(self, cell_mm: f64) -> f64 {
        match self.max_step_mm {
            Some(step) if step.is_finite() && step > 0.0 => step,
            _ => {
                if cell_mm.is_finite() && cell_mm > 0.0 {
                    cell_mm * 0.5
                } else {
                    0.5
                }
            }
        }
    }

    fn chord_tolerance_mm(self, cell_mm: f64) -> f64 {
        match self.chord_tolerance_mm {
            Some(tolerance) if tolerance.is_finite() && tolerance > 0.0 => tolerance,
            _ => {
                if cell_mm.is_finite() && cell_mm > 0.0 {
                    cell_mm * 0.25
                } else {
                    0.25
                }
            }
        }
    }

    fn step_cap(self) -> usize {
        self.max_steps.max(1)
    }
}

/// A rectangular height grid over the stock footprint.
///
/// `cells` holds the material top for every cell, row-major from the minimum
/// Y row upward. Received heights never exceed [`Heightmap::top_z_mm`].
#[derive(Debug, Clone, PartialEq)]
pub struct Heightmap {
    origin_xy_mm: [f64; 2],
    cell_mm: f64,
    cols: usize,
    rows: usize,
    top_z_mm: f64,
    cells: Vec<f32>,
    version: u64,
}

impl Heightmap {
    /// Builds a grid at the requested cell size.
    ///
    /// Returns [`StockError::GridTooLarge`] when the grid would exceed
    /// `max_cells`; callers that must simulate anyway coarsen the cell size and
    /// retry.
    pub fn new(
        origin_xy_mm: [f64; 2],
        size_xy_mm: [f64; 2],
        cell_mm: f64,
        top_z_mm: f64,
        max_cells: usize,
    ) -> Result<Self, StockError> {
        if !origin_xy_mm.iter().all(|value| value.is_finite())
            || !size_xy_mm
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            || !top_z_mm.is_finite()
        {
            return Err(StockError::InvalidExtent {
                min_mm: [origin_xy_mm[0], origin_xy_mm[1], 0.0],
                max_mm: [
                    origin_xy_mm[0] + size_xy_mm[0],
                    origin_xy_mm[1] + size_xy_mm[1],
                    top_z_mm,
                ],
            });
        }
        let cell = if cell_mm.is_finite() && cell_mm > 0.0 {
            cell_mm
        } else {
            return Err(StockError::GridTooLarge {
                cells: usize::MAX,
                max_cells,
            });
        };
        let cols = if size_xy_mm[0] > 0.0 {
            (math::ceil(size_xy_mm[0] / cell) as usize).saturating_add(1)
        } else {
            1
        };
        let rows = if size_xy_mm[1] > 0.0 {
            (math::ceil(size_xy_mm[1] / cell) as usize).saturating_add(1)
        } else {
            1
        };
        let cells = cols.saturating_mul(rows);
        if cells > max_cells {
            return Err(StockError::GridTooLarge { cells, max_cells });
        }
        Ok(Self {
            origin_xy_mm,
            cell_mm: cell,
            cols,
            rows,
            top_z_mm,
            cells: vec![top_z_mm as f32; cells],
            version: 0,
        })
    }

    /// Builds a grid, doubling the cell size until it fits the cell cap.
    ///
    /// Returns the map and whether the final cell size is above 2 mm, which is
    /// the label the UI shows as **coarse**. The loop is bounded; a cell size
    /// that cannot be doubled any further returns the last sizing error rather
    /// than looping forever.
    pub fn new_fitted(
        origin_xy_mm: [f64; 2],
        size_xy_mm: [f64; 2],
        cell_mm: f64,
        top_z_mm: f64,
        max_cells: usize,
    ) -> Result<(Self, bool), StockError> {
        let mut cell = if cell_mm.is_finite() && cell_mm > 0.0 {
            cell_mm
        } else {
            return Err(StockError::GridTooLarge {
                cells: usize::MAX,
                max_cells,
            });
        };
        let mut last_error: Option<StockError> = None;
        for _ in 0..64 {
            match Self::new(origin_xy_mm, size_xy_mm, cell, top_z_mm, max_cells) {
                Ok(map) => return Ok((map, cell > 2.0)),
                Err(error) => {
                    last_error = Some(error);
                }
            }
            let doubled = cell * 2.0;
            if !doubled.is_finite() || doubled <= cell {
                break;
            }
            cell = doubled;
        }
        match last_error {
            Some(error) => Err(error),
            None => Err(StockError::GridTooLarge {
                cells: usize::MAX,
                max_cells,
            }),
        }
    }

    /// Returns `true` when the grid cell is coarser than 2 mm.
    #[must_use]
    pub fn is_coarse(&self) -> bool {
        self.cell_mm > 2.0
    }

    /// Cell size in millimetres.
    #[must_use]
    pub fn cell_mm(&self) -> f64 {
        self.cell_mm
    }

    /// Column count along X.
    #[must_use]
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Row count along Y.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Total cell count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns `true` when the grid has no cells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// XY origin of the grid (the centre of cell `(0, 0)` is half a cell in).
    #[must_use]
    pub fn origin_xy_mm(&self) -> [f64; 2] {
        self.origin_xy_mm
    }

    /// Height of the uncut top face in millimetres.
    #[must_use]
    pub fn top_z_mm(&self) -> f64 {
        self.top_z_mm
    }

    /// Monotonic content version, incremented by every write that changes a
    /// cell.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Marks the grid changed even when no cell value moved.
    pub fn bump_version(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Restores every cell to the uncut top and resets the version to zero.
    ///
    /// This is the deterministic reset used by replay: a freshly constructed
    /// map for the same parameters also starts at version zero.
    pub fn reset(&mut self) {
        self.cells.fill(self.top_z_mm as f32);
        self.version = 0;
    }

    /// Reads one cell, if the index is in range.
    #[must_use]
    pub fn get(&self, col: usize, row: usize) -> Option<f32> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells.get(row * self.cols + col).copied()
    }

    /// Iterates cell heights in row-major order.
    pub fn cells(&self) -> &[f32] {
        &self.cells
    }

    /// Lowers one cell to `height_mm` when that is lower than its current
    /// value. Returns `true` when the grid changed.
    pub fn lower_cell(&mut self, col: usize, row: usize, height_mm: f64) -> bool {
        if col >= self.cols || row >= self.rows || !height_mm.is_finite() {
            return false;
        }
        let index = row * self.cols + col;
        let Some(cell) = self.cells.get_mut(index) else {
            return false;
        };
        let lowered = height_mm as f32;
        if lowered < *cell {
            *cell = lowered;
            self.version = self.version.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// Centre of cell `(col, row)` in world millimetres.
    #[must_use]
    pub fn cell_center_mm(&self, col: usize, row: usize) -> [f64; 2] {
        [
            self.origin_xy_mm[0] + (col as f64 + 0.5) * self.cell_mm,
            self.origin_xy_mm[1] + (row as f64 + 0.5) * self.cell_mm,
        ]
    }

    /// The cell containing `point_xy_mm`, when inside the grid.
    #[must_use]
    pub fn cell_at(&self, point_xy_mm: [f64; 2]) -> Option<(usize, usize)> {
        let fx = (point_xy_mm[0] - self.origin_xy_mm[0]) / self.cell_mm;
        let fy = (point_xy_mm[1] - self.origin_xy_mm[1]) / self.cell_mm;
        if !fx.is_finite() || !fy.is_finite() || fx < 0.0 || fy < 0.0 {
            return None;
        }
        let col = fx as usize;
        let row = fy as usize;
        if col >= self.cols || row >= self.rows {
            return None;
        }
        Some((col, row))
    }

    /// Sweeps a cutter along a straight segment and lowers every covered cell.
    ///
    /// The swept volume is a capsule in XY: the cutter profile is applied at
    /// the closest point of the segment for a flat cutter and over the covered
    /// parameter interval for shaped cutters. A centerline cutter lowers the
    /// containing cell at half-cell steps, never a zero-diameter disk.
    ///
    /// Returns `true` when at least one cell changed.
    pub fn cut_segment(&mut self, cutter: &Cutter, from_mm: [f64; 3], to_mm: [f64; 3]) -> bool {
        self.cut_segment_with(cutter, from_mm, to_mm, SweepParams::default())
    }

    /// [`Heightmap::cut_segment`] with explicit sweep tuning.
    pub fn cut_segment_with(
        &mut self,
        cutter: &Cutter,
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        params: SweepParams,
    ) -> bool {
        if !finite_point(from_mm) || !finite_point(to_mm) {
            return false;
        }
        match cutter.radius_mm() {
            Some(radius) if radius.is_finite() && radius > 0.0 => {
                self.cut_shaped_segment(cutter, from_mm, to_mm, radius, params)
            }
            _ => self.cut_centerline_segment(from_mm, to_mm, params),
        }
    }

    /// Sweeps a cutter along the exact arc or helix and lowers covered cells.
    ///
    /// The curve is subdivided so the chord error is at most `tolerance_mm`
    /// (or a quarter cell when the tolerance is invalid). Each chord is swept
    /// as a capsule, so a fast arc leaves no gaps.
    ///
    /// Returns `true` when at least one cell changed.
    pub fn cut_arc(
        &mut self,
        cutter: &Cutter,
        arc: &ArcRecord,
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        plane: Plane,
        tolerance_mm: f64,
    ) -> bool {
        self.cut_arc_with(
            cutter,
            arc,
            from_mm,
            to_mm,
            plane,
            tolerance_mm,
            SweepParams::default(),
        )
    }

    /// [`Heightmap::cut_arc`] with explicit sweep tuning.
    #[allow(clippy::too_many_arguments)]
    pub fn cut_arc_with(
        &mut self,
        cutter: &Cutter,
        arc: &ArcRecord,
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        plane: Plane,
        tolerance_mm: f64,
        params: SweepParams,
    ) -> bool {
        if !finite_point(from_mm) || !finite_point(to_mm) {
            return false;
        }
        let radius = if arc.radius_mm.is_finite() && arc.radius_mm > 0.0 {
            arc.radius_mm
        } else {
            return self.cut_segment_with(cutter, from_mm, to_mm, params);
        };
        if !arc.sweep_rad.is_finite() || math::abs(arc.sweep_rad) <= 1.0e-12 {
            return self.cut_segment_with(cutter, from_mm, to_mm, params);
        }
        let tolerance = if tolerance_mm.is_finite() && tolerance_mm > 0.0 {
            tolerance_mm
        } else {
            params.chord_tolerance_mm(self.cell_mm)
        };
        let width = tolerance * (2.0 * radius - tolerance);
        let chord_max = if width > 0.0 {
            2.0 * math::sqrt(width)
        } else {
            2.0 * radius
        };
        let planar_length = radius * math::abs(arc.sweep_rad);
        let raw_steps = if chord_max > 1.0e-12 {
            math::ceil(planar_length / chord_max)
        } else {
            1.0
        };
        let steps = clamp_steps(raw_steps, params.step_cap()).max(1);
        let mut changed = false;
        let mut previous = arc_point(arc, plane, from_mm, to_mm, 0.0);
        for step in 1..=steps {
            let fraction = step as f64 / steps as f64;
            let point = arc_point(arc, plane, from_mm, to_mm, fraction);
            changed |= self.cut_segment_with(cutter, previous, point, params);
            previous = point;
        }
        changed
    }

    /// Cuts one expanded motion record.
    ///
    /// Arcs and helixes use the exact record geometry. A straight motion longer
    /// than half a cell is cut in equal sub-segments so a ramp or 3D feed
    /// follows the tool Z instead of lowering every covered cell to the lowest
    /// endpoint. The subdivision is capped so an extreme record cannot loop
    /// forever. A raw canned-cycle record is not expanded here; callers should
    /// walk `Program::expanded_motions()` so pecks and retracts are explicit.
    pub fn cut_motion(&mut self, cutter: &Cutter, motion: &MotionRecord) -> bool {
        let params = SweepParams {
            max_steps: MAX_MOTION_STEPS,
            ..SweepParams::default()
        };
        self.cut_motion_with(cutter, motion, params)
    }

    /// [`Heightmap::cut_motion`] with explicit motion subdivision tuning.
    ///
    /// `params.max_step_mm` overrides the half-cell chord step. `params.max_steps`
    /// bounds both the straight-motion subdivision and arc curve subdivision.
    pub fn cut_motion_with(
        &mut self,
        cutter: &Cutter,
        motion: &MotionRecord,
        params: SweepParams,
    ) -> bool {
        if motion.canned.is_some() {
            return false;
        }
        if let Some(arc) = &motion.arc {
            return self.cut_arc_with(
                cutter,
                arc,
                motion.start_mm,
                motion.end_mm,
                motion.plane,
                self.cell_mm * 0.25,
                params,
            );
        }
        let chord = motion.chord_length_mm();
        let step = params.step_mm(self.cell_mm).max(self.cell_mm * 1.0e-3);
        let steps = if chord.is_finite() && chord > step {
            clamp_steps(math::ceil(chord / step), params.step_cap()).max(1)
        } else {
            1
        };
        let mut changed = false;
        let mut previous = motion.start_mm;
        for index in 1..=steps {
            let fraction = index as f64 / steps as f64;
            let point = [
                motion.start_mm[0] + (motion.end_mm[0] - motion.start_mm[0]) * fraction,
                motion.start_mm[1] + (motion.end_mm[1] - motion.start_mm[1]) * fraction,
                motion.start_mm[2] + (motion.end_mm[2] - motion.start_mm[2]) * fraction,
            ];
            changed |= self.cut_segment_with(cutter, previous, point, params);
            previous = point;
        }
        changed
    }

    fn cut_centerline_segment(
        &mut self,
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        params: SweepParams,
    ) -> bool {
        let dx = to_mm[0] - from_mm[0];
        let dy = to_mm[1] - from_mm[1];
        let dz = to_mm[2] - from_mm[2];
        let planar = math::hypot(dx, dy);
        if planar <= 0.0 {
            let Some((col, row)) = self.cell_at([from_mm[0], from_mm[1]]) else {
                return false;
            };
            return self.lower_cell(col, row, from_mm[2].min(to_mm[2]));
        }
        let step = params.step_mm(self.cell_mm).max(self.cell_mm * 1.0e-3);
        let steps = clamp_steps(math::ceil(planar / step), params.step_cap()).max(1);
        let mut changed = false;
        for index in 0..=steps {
            let fraction = index as f64 / steps as f64;
            let x = from_mm[0] + dx * fraction;
            let y = from_mm[1] + dy * fraction;
            let z = from_mm[2] + dz * fraction;
            let Some((col, row)) = self.cell_at([x, y]) else {
                continue;
            };
            changed |= self.lower_cell(col, row, z);
        }
        changed
    }

    fn cut_shaped_segment(
        &mut self,
        cutter: &Cutter,
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        radius: f64,
        _params: SweepParams,
    ) -> bool {
        let dx = to_mm[0] - from_mm[0];
        let dy = to_mm[1] - from_mm[1];
        let dz = to_mm[2] - from_mm[2];
        let planar_squared = dx * dx + dy * dy;
        let Some((col_min, col_max, row_min, row_max)) = self.cell_range(
            from_mm[0].min(to_mm[0]) - radius,
            from_mm[0].max(to_mm[0]) + radius,
            from_mm[1].min(to_mm[1]) - radius,
            from_mm[1].max(to_mm[1]) + radius,
        ) else {
            return false;
        };
        let shape = cutter.effective_shape();
        let mut changed = false;
        for row in row_min..=row_max {
            for col in col_min..=col_max {
                let centre = self.cell_center_mm(col, row);
                let fx = centre[0] - from_mm[0];
                let fy = centre[1] - from_mm[1];
                let interval = if planar_squared <= 1.0e-24 {
                    if fx * fx + fy * fy > radius * radius {
                        None
                    } else {
                        Some((0.0, 1.0))
                    }
                } else {
                    let b = fx * dx + fy * dy;
                    let c = fx * fx + fy * fy - radius * radius;
                    let discriminant = b * b - planar_squared * c;
                    if discriminant < 0.0 {
                        None
                    } else {
                        let root = math::sqrt(discriminant);
                        let start = ((b - root) / planar_squared).max(0.0);
                        let end = ((b + root) / planar_squared).min(1.0);
                        if end < start {
                            None
                        } else {
                            Some((start, end))
                        }
                    }
                };
                let Some((start, end)) = interval else {
                    continue;
                };
                let height = if matches!(shape, ToolShape::Flat) {
                    let t = if dz >= 0.0 { start } else { end };
                    from_mm[2] + dz * t
                } else {
                    let evaluate = |t: f64| {
                        // The tool centre at `t` is `from + d * t`, so the
                        // radial vector is `centre - from - d * t`.
                        let px = fx - dx * t;
                        let py = fy - dy * t;
                        let r = math::sqrt(px * px + py * py).min(radius);
                        from_mm[2] + dz * t + profile_offset(cutter, radius, r)
                    };
                    let mut low = start;
                    let mut high = end;
                    for _ in 0..24 {
                        let third = (high - low) / 3.0;
                        let first_third = low + third;
                        let second_third = high - third;
                        if evaluate(first_third) <= evaluate(second_third) {
                            high = second_third;
                        } else {
                            low = first_third;
                        }
                    }
                    let middle = (low + high) * 0.5;
                    evaluate(start).min(evaluate(end)).min(evaluate(middle))
                };
                changed |= self.lower_cell(col, row, height);
            }
        }
        changed
    }

    fn cell_range(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
    ) -> Option<(usize, usize, usize, usize)> {
        if !min_x.is_finite() || !max_x.is_finite() || !min_y.is_finite() || !max_y.is_finite() {
            return None;
        }
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        let last_col = (self.cols - 1) as f64;
        let last_row = (self.rows - 1) as f64;
        let first_col = math::floor((min_x - self.origin_xy_mm[0]) / self.cell_mm);
        let last_col_index = math::ceil((max_x - self.origin_xy_mm[0]) / self.cell_mm);
        let first_row = math::floor((min_y - self.origin_xy_mm[1]) / self.cell_mm);
        let last_row_index = math::ceil((max_y - self.origin_xy_mm[1]) / self.cell_mm);
        if last_col_index < 0.0
            || last_row_index < 0.0
            || first_col > last_col
            || first_row > last_row
        {
            return None;
        }
        Some((
            math::clamp(first_col, 0.0, last_col) as usize,
            math::clamp(last_col_index, 0.0, last_col) as usize,
            math::clamp(first_row, 0.0, last_row) as usize,
            math::clamp(last_row_index, 0.0, last_row) as usize,
        ))
    }

    /// FNV-1a hash of the stored cell bits in row-major order.
    ///
    /// This is the determinism oracle for the simulator: two runs that reach
    /// the same tick must produce the same hash, regardless of frame rate.
    #[must_use]
    pub fn content_hash(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET;
        for cell in &self.cells {
            for byte in cell.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
        hash
    }
}

fn profile_offset(cutter: &Cutter, radius: f64, r: f64) -> f64 {
    let r = math::clamp(r, 0.0, radius);
    match cutter.effective_shape() {
        ToolShape::Ball => radius - math::sqrt((radius * radius - r * r).max(0.0)),
        ToolShape::Bull => {
            let corner = match cutter.corner_radius_mm {
                Some(value) if value.is_finite() && value > 0.0 => value.min(radius),
                _ => 0.0,
            };
            if corner <= 0.0 {
                0.0
            } else {
                let flat_radius = radius - corner;
                if r <= flat_radius {
                    0.0
                } else {
                    let offset = r - flat_radius;
                    corner - math::sqrt((corner * corner - offset * offset).max(0.0))
                }
            }
        }
        ToolShape::Drill => {
            let half_angle = tip_half_angle_radians(cutter.tip_angle_deg, 118.0);
            let tangent = math::tan(half_angle);
            if tangent > 1.0e-9 { r / tangent } else { 0.0 }
        }
        ToolShape::VBit => {
            let half_angle = tip_half_angle_radians(cutter.tip_angle_deg, 90.0);
            let tangent = math::tan(half_angle);
            if tangent > 1.0e-9 { r / tangent } else { 0.0 }
        }
        ToolShape::Flat | ToolShape::Unknown => 0.0,
    }
}

fn tip_half_angle_radians(tip_angle_deg: Option<f64>, fallback_deg: f64) -> f64 {
    let included = match tip_angle_deg {
        Some(value) if value.is_finite() && value > 0.0 && value < 180.0 => value,
        _ => fallback_deg,
    };
    math::to_radians(included) * 0.5
}

fn clamp_steps(raw: f64, cap: usize) -> usize {
    if !raw.is_finite() || raw <= 1.0 {
        return 1;
    }
    if raw >= cap as f64 {
        cap.max(1)
    } else {
        raw as usize
    }
}

fn finite_point(point: [f64; 3]) -> bool {
    point.iter().all(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::ToolShape;
    use nc_gcode::ArcFormat;

    fn flat(diameter: f64) -> Cutter {
        Cutter {
            shape: ToolShape::Flat,
            diameter_mm: Some(diameter),
            corner_radius_mm: None,
            tip_angle_deg: None,
            stickout_mm: None,
            flute_length_mm: None,
            shank_diameter_mm: None,
            holder_diameter_mm: None,
            assumed: false,
        }
    }

    fn ball(diameter: f64) -> Cutter {
        Cutter {
            shape: ToolShape::Ball,
            diameter_mm: Some(diameter),
            ..flat(diameter)
        }
    }

    fn bull(diameter: f64, corner: f64) -> Cutter {
        Cutter {
            shape: ToolShape::Bull,
            diameter_mm: Some(diameter),
            corner_radius_mm: Some(corner),
            ..flat(diameter)
        }
    }

    fn shaped(shape: ToolShape, diameter: f64, tip: f64) -> Cutter {
        Cutter {
            shape,
            diameter_mm: Some(diameter),
            tip_angle_deg: Some(tip),
            ..flat(diameter)
        }
    }

    fn test_map(cols: usize, rows: usize, cell: f64) -> Heightmap {
        Heightmap::new(
            [0.0, 0.0],
            [cols as f64 * cell, rows as f64 * cell],
            cell,
            0.0,
            10_000_000,
        )
        .expect("grid")
    }

    #[test]
    fn new_grid_is_full_height_and_hashes_stably() {
        let map = Heightmap::new([0.0, 0.0], [10.0, 10.0], 1.0, 0.0, 1000).expect("grid");
        assert_eq!(map.cols(), 11);
        assert_eq!(map.rows(), 11);
        assert_eq!(map.get(0, 0), Some(0.0));
        let first = map.content_hash();
        let again = Heightmap::new([0.0, 0.0], [10.0, 10.0], 1.0, 0.0, 1000).expect("grid");
        assert_eq!(first, again.content_hash());
    }

    #[test]
    fn lowering_changes_hash_and_version() {
        let mut map = Heightmap::new([0.0, 0.0], [4.0, 4.0], 1.0, 0.0, 1000).expect("grid");
        let hash = map.content_hash();
        assert!(map.lower_cell(1, 1, -2.5));
        assert_ne!(map.content_hash(), hash);
        assert!(map.version() > 0);
        assert_eq!(map.get(1, 1), Some(-2.5));
        assert!(!map.lower_cell(1, 1, 0.0));
    }

    #[test]
    fn out_of_range_is_a_no_op() {
        let mut map = Heightmap::new([0.0, 0.0], [2.0, 2.0], 1.0, 0.0, 1000).expect("grid");
        assert!(!map.lower_cell(99, 0, -1.0));
        assert!(!map.lower_cell(0, 99, -1.0));
        assert_eq!(map.get(99, 0), None);
    }

    #[test]
    fn cell_cap_is_enforced() {
        assert!(Heightmap::new([0.0, 0.0], [100.0, 100.0], 0.1, 0.0, 1000).is_err());
    }

    #[test]
    fn reset_restores_the_initial_hash() {
        let mut map = Heightmap::new([0.0, 0.0], [4.0, 4.0], 1.0, 1.0, 1000).expect("grid");
        let hash = map.content_hash();
        assert!(map.cut_segment(&flat(2.0), [2.0, 2.0, 1.0], [2.0, 2.0, -1.0]));
        assert_ne!(map.content_hash(), hash);
        map.reset();
        assert_eq!(map.content_hash(), hash);
        assert_eq!(map.version(), 0);
    }

    #[test]
    fn grid_coarsens_until_it_fits_and_labels_above_two_mm() {
        let (map, coarse) =
            Heightmap::new_fitted([0.0, 0.0], [100.0, 100.0], 0.75, 0.0, 1000).expect("grid");
        assert!(map.cell_mm() > 0.75);
        assert!(coarse);
        assert!(map.cell_mm() > 2.0);
        assert!(map.len() <= 1000);
        let (fine, coarse) =
            Heightmap::new_fitted([0.0, 0.0], [10.0, 10.0], 0.75, 0.0, 100_000).expect("grid");
        assert_eq!(fine.cell_mm(), 0.75);
        assert!(!coarse);
        assert!(!fine.is_coarse());
    }

    #[test]
    fn flat_capsule_has_no_gaps_at_high_feed() {
        let mut map = test_map(100, 20, 0.75);
        let changed = map.cut_segment(&flat(1.5), [1.0, 10.125, -1.0], [99.0, 10.125, -1.0]);
        assert!(changed);
        for col in 0..100 {
            assert_eq!(
                map.get(col, 13),
                Some(-1.0),
                "gap at column {col}: {:?}",
                map.get(col, 13)
            );
        }
        assert_eq!(map.get(50, 2), Some(0.0));
    }

    #[test]
    fn long_single_tick_style_segment_is_fully_covered() {
        let mut map = test_map(200, 10, 1.0);
        assert!(map.cut_segment(&flat(2.0), [0.5, 5.5, -1.0], [199.5, 5.5, -1.0]));
        for col in 0..200 {
            assert_eq!(map.get(col, 5), Some(-1.0), "gap at {col}");
        }
    }

    #[test]
    fn centerline_trace_is_one_cell_wide() {
        let mut map = test_map(60, 20, 1.0);
        let cutter = Cutter::centerline();
        assert!(cutter.is_centerline());
        assert_eq!(cutter.display_radius_mm(), 0.0);
        assert!(map.cut_segment(&cutter, [0.5, 10.5, -1.0], [59.5, 10.5, -1.0]));
        for col in 0..60 {
            assert_eq!(map.get(col, 10), Some(-1.0), "gap at {col}");
        }
        for col in 0..60 {
            assert_eq!(map.get(col, 9), Some(0.0), "trace too wide at {col}");
            assert_eq!(map.get(col, 11), Some(0.0), "trace too wide at {col}");
        }
    }

    #[test]
    fn centerline_plunge_reaches_the_tip_depth() {
        let mut map = test_map(20, 20, 1.0);
        let cutter = Cutter::centerline();
        assert!(map.cut_segment(&cutter, [10.5, 10.5, 0.0], [10.5, 10.5, -5.0]));
        assert_eq!(map.get(10, 10), Some(-5.0));
    }

    #[test]
    fn flat_full_circle_cuts_only_the_ring() {
        let mut map = test_map(60, 60, 1.0);
        let arc = ArcRecord {
            center_mm: [30.0, 30.0, 0.0],
            radius_mm: 20.0,
            end_radius_mm: 20.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let changed = map.cut_arc(
            &flat(2.0),
            &arc,
            [50.0, 30.0, -2.0],
            [50.0, 30.0, -2.0],
            Plane::Xy,
            0.25,
        );
        assert!(changed);
        assert_eq!(map.get(30, 30), Some(0.0));
        assert_eq!(map.get(50, 30), Some(-2.0));
        assert_eq!(map.get(10, 30), Some(-2.0));
        assert_eq!(map.get(5, 5), Some(0.0));
    }

    #[test]
    fn arc_has_no_gaps_at_high_feed() {
        let mut map = test_map(80, 80, 0.5);
        let arc = ArcRecord {
            center_mm: [20.0, 20.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        assert!(map.cut_arc(
            &flat(2.0),
            &arc,
            [30.0, 20.0, -1.0],
            [30.0, 20.0, -1.0],
            Plane::Xy,
            0.125,
        ));
        let mut cut_cells = 0;
        for row in 0..80 {
            for col in 0..80 {
                if map.get(col, row) == Some(-1.0) {
                    cut_cells += 1;
                }
            }
        }
        assert!(cut_cells > 300, "ring is not continuous: {cut_cells} cells");
    }

    #[test]
    fn descending_helix_is_deeper_at_the_end() {
        let mut map = test_map(60, 60, 1.0);
        let arc = ArcRecord {
            center_mm: [30.0, 30.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::PI,
            axial_delta_mm: -5.0,
            format: ArcFormat::Center,
        };
        assert!(map.cut_arc(
            &flat(2.0),
            &arc,
            [40.0, 30.0, 0.0],
            [20.0, 30.0, -5.0],
            Plane::Xy,
            0.25,
        ));
        let end = map.get(20, 30).unwrap_or(0.0);
        let start = map.get(40, 30).unwrap_or(0.0);
        assert!(end <= -4.5, "end height {end}");
        assert!(start >= -1.0, "start height {start}");
    }

    #[test]
    fn ball_profile_matches_the_sphere() {
        let mut map = Heightmap::new([0.5, 0.5], [20.0, 20.0], 1.0, 0.0, 10_000).expect("grid");
        assert!(map.cut_segment(&ball(6.0), [10.0, 10.0, -1.0], [10.0, 10.0, -1.0]));
        let centre = map.get(9, 9).expect("cell");
        assert!((centre - (-1.0)).abs() < 1.0e-6, "{centre}");
        let two_mm = map.get(11, 9).expect("cell");
        let expected = -1.0 + 3.0 - nc_core::math::sqrt(5.0);
        assert!(
            (two_mm - expected as f32).abs() < 1.0e-5,
            "{two_mm} vs {expected}"
        );
        assert_eq!(map.get(12, 9), Some(0.0));
    }

    #[test]
    fn bull_profile_has_a_flat_tip_and_torus() {
        let mut map = Heightmap::new([0.5, 0.5], [20.0, 20.0], 1.0, 0.0, 10_000).expect("grid");
        assert!(map.cut_segment(&bull(6.0, 2.0), [10.0, 10.0, -1.0], [10.0, 10.0, -1.0]));
        assert_eq!(map.get(9, 9), Some(-1.0));
        assert_eq!(map.get(10, 9), Some(-1.0));
        let expected = -1.0 + 2.0 - nc_core::math::sqrt(3.0);
        let torus = map.get(11, 9).expect("cell");
        assert!(
            (torus - expected as f32).abs() < 1.0e-5,
            "{torus} vs {expected}"
        );
    }

    #[test]
    fn drill_and_vbit_profiles_are_cones() {
        let mut map = Heightmap::new([0.5, 0.5], [20.0, 20.0], 1.0, 0.0, 10_000).expect("grid");
        assert!(map.cut_segment(
            &shaped(ToolShape::Drill, 6.0, 118.0),
            [10.0, 10.0, -2.0],
            [10.0, 10.0, -2.0],
        ));
        assert!((map.get(9, 9).unwrap_or(0.0) - (-2.0)).abs() < 1.0e-6);
        let expected = -2.0 + 1.0 / nc_core::math::tan(nc_core::math::to_radians(59.0));
        assert!((map.get(10, 9).unwrap_or(0.0) - expected as f32).abs() < 1.0e-5);
        let edge = -2.0 + 3.0 / nc_core::math::tan(nc_core::math::to_radians(59.0));
        assert!((map.get(12, 9).unwrap_or(0.0) - edge as f32).abs() < 1.0e-5);
        assert_eq!(map.get(13, 9), Some(0.0));
    }

    #[test]
    fn horizontal_drill_sweep_matches_the_cone() {
        // d2.5, 118 deg, tip at Z=-1 sweeping along X at Y=10.5. The 59 deg
        // cone rises 1/tan(59 deg) per mm of radial distance.
        let mut map = test_map(20, 20, 1.0);
        let cutter = shaped(ToolShape::Drill, 2.5, 118.0);
        assert!(map.cut_segment(&cutter, [0.5, 10.5, -1.0], [19.5, 10.5, -1.0]));
        let centre = map.get(10, 10).expect("cell");
        assert!((centre - (-1.0)).abs() < 1.0e-4, "centreline {centre}");
        let expected = -1.0 + 1.0 / nc_core::math::tan(nc_core::math::to_radians(59.0));
        let off_axis = f64::from(map.get(10, 11).expect("cell"));
        assert!(
            (off_axis - expected).abs() < 1.0e-4,
            "1 mm off-axis {off_axis} vs {expected}"
        );
        assert_eq!(map.get(10, 12), Some(0.0));
    }

    #[test]
    fn horizontal_vbit_sweep_matches_the_cone() {
        // d3.175, 90 deg, tip at Z=-2: the cone rises 1 mm per mm.
        let mut map = test_map(20, 20, 1.0);
        let cutter = shaped(ToolShape::VBit, 3.175, 90.0);
        assert!(map.cut_segment(&cutter, [0.5, 10.5, -2.0], [19.5, 10.5, -2.0]));
        let centre = map.get(10, 10).expect("cell");
        assert!((centre - (-2.0)).abs() < 1.0e-4, "centreline {centre}");
        let off_axis = map.get(10, 11).expect("cell");
        assert!(
            (off_axis - (-1.0)).abs() < 1.0e-4,
            "1 mm off-axis {off_axis}"
        );
        assert_eq!(map.get(10, 12), Some(0.0));
    }

    #[test]
    fn angled_vbit_sweep_cuts_symmetrically_about_the_centreline() {
        // A 45 deg sweep must not favour either side of the path. Cells
        // (11, 9) and (9, 11) are both sqrt(2) mm from the line y = x.
        let mut map = test_map(20, 20, 1.0);
        let cutter = shaped(ToolShape::VBit, 3.175, 90.0);
        assert!(map.cut_segment(&cutter, [0.5, 0.5, -2.0], [19.5, 19.5, -2.0]));
        let centre = map.get(10, 10).expect("cell");
        assert!((centre - (-2.0)).abs() < 1.0e-4, "centreline {centre}");
        let left = f64::from(map.get(11, 9).expect("cell"));
        let right = f64::from(map.get(9, 11).expect("cell"));
        assert!((left - right).abs() < 1.0e-6, "{left} vs {right}");
        let expected = -2.0 + nc_core::math::sqrt(2.0);
        assert!(
            (left - expected).abs() < 1.0e-4,
            "off-axis {left} vs {expected}"
        );
    }

    #[test]
    fn horizontal_ball_sweep_keeps_the_sphere_profile() {
        let mut map = test_map(20, 20, 1.0);
        assert!(map.cut_segment(&ball(6.0), [0.5, 10.5, -1.0], [19.5, 10.5, -1.0]));
        let centre = map.get(10, 10).expect("cell");
        assert!((centre - (-1.0)).abs() < 1.0e-4, "centreline {centre}");
        let expected = -1.0 + 3.0 - nc_core::math::sqrt(8.0);
        let off_axis = f64::from(map.get(10, 11).expect("cell"));
        assert!(
            (off_axis - expected).abs() < 1.0e-4,
            "1 mm off-axis {off_axis} vs {expected}"
        );
        assert_eq!(map.get(10, 13), Some(0.0));
    }

    #[test]
    fn horizontal_bull_sweep_keeps_the_flat_tip() {
        let mut map = test_map(20, 20, 1.0);
        assert!(map.cut_segment(&bull(6.0, 2.0), [0.5, 10.5, -1.0], [19.5, 10.5, -1.0]));
        assert_eq!(map.get(10, 10), Some(-1.0));
        assert_eq!(map.get(10, 11), Some(-1.0));
        let expected = -1.0 + 2.0 - nc_core::math::sqrt(3.0);
        let torus = f64::from(map.get(10, 12).expect("cell"));
        assert!(
            (torus - expected).abs() < 1.0e-4,
            "2 mm off-axis {torus} vs {expected}"
        );
        assert_eq!(map.get(10, 13), Some(0.0));
    }

    #[test]
    fn shaped_arc_sweep_cuts_the_centrepath() {
        // Full circle of radius 10 around (20.5, 20.5). Cell (26, 28) has an
        // offset of (6, 8) from the centre, so it sits exactly on the path at
        // 53.13 deg, between chord samples. A 90 deg V-bit at Z=-2 must cut it
        // to roughly the tip depth (only the chord sagitta remains).
        let mut map = test_map(45, 45, 1.0);
        let arc = ArcRecord {
            center_mm: [20.5, 20.5, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let cutter = shaped(ToolShape::VBit, 3.175, 90.0);
        assert!(map.cut_arc(
            &cutter,
            &arc,
            [30.5, 20.5, -2.0],
            [30.5, 20.5, -2.0],
            Plane::Xy,
            0.25,
        ));
        let on_path = map.get(26, 28).expect("cell");
        assert!(on_path <= -1.7, "centreline cell {on_path}");
        assert_eq!(map.get(20, 20), Some(0.0));
    }

    #[test]
    fn repeated_shaped_sweeps_hash_identically() {
        let run = || {
            let mut map = test_map(20, 20, 1.0);
            assert!(map.cut_segment(
                &shaped(ToolShape::Drill, 2.5, 118.0),
                [0.5, 10.5, -1.0],
                [19.5, 10.5, -1.0],
            ));
            assert!(map.cut_segment(
                &shaped(ToolShape::VBit, 3.175, 90.0),
                [10.5, 0.5, -2.0],
                [10.5, 19.5, -2.0],
            ));
            map.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn unknown_shape_cuts_as_a_flat_cylinder() {
        let mut map = Heightmap::new([0.5, 0.5], [20.0, 20.0], 1.0, 0.0, 10_000).expect("grid");
        let cutter = Cutter {
            shape: ToolShape::Unknown,
            diameter_mm: Some(4.0),
            ..Cutter::centerline()
        };
        assert!(map.cut_segment(&cutter, [10.0, 10.0, -1.0], [10.0, 10.0, -1.0]));
        assert_eq!(map.get(9, 9), Some(-1.0));
        assert_eq!(map.get(11, 9), Some(-1.0));
        assert_eq!(map.get(12, 9), Some(0.0));
    }

    #[test]
    fn cut_motion_accepts_an_arc_record() {
        let mut map = test_map(40, 40, 1.0);
        let arc = ArcRecord {
            center_mm: [20.0, 20.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad: 0.0,
            sweep_rad: nc_core::math::TAU,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let mut motion = MotionRecord::linear(
            1,
            0,
            nc_viewer::MoveKind::ArcCw,
            [30.0, 20.0, -1.0],
            [30.0, 20.0, -1.0],
        );
        motion.arc = Some(Box::new(arc));
        assert!(map.cut_motion(&flat(2.0), &motion));
        assert_eq!(map.get(20, 20), Some(0.0));
        assert_eq!(map.get(30, 20), Some(-1.0));
    }

    #[test]
    fn long_ramp_follows_the_tool_z_instead_of_the_lowest_endpoint() {
        let mut map = test_map(30, 10, 1.0);
        let motion = MotionRecord::linear(
            1,
            0,
            nc_viewer::MoveKind::Ramp,
            [2.5, 5.5, 0.0],
            [22.5, 5.5, -2.0],
        );
        assert!(map.cut_motion(&flat(2.0), &motion));
        let start = map.get(2, 5).unwrap_or(0.0);
        let middle = map.get(12, 5).unwrap_or(0.0);
        let end = map.get(22, 5).unwrap_or(0.0);
        assert!(start > -0.5, "start cell fell to {start}");
        assert!((-1.3..=-0.7).contains(&middle), "middle cell {middle}");
        assert!(end < -1.9, "end cell {end}");
    }

    #[test]
    fn non_finite_input_is_a_no_op() {
        let mut map = test_map(10, 10, 1.0);
        assert!(!map.cut_segment(&flat(2.0), [f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert!(!map.cut_segment(&flat(2.0), [0.0, 0.0, f64::INFINITY], [1.0, 0.0, 0.0]));
    }
}
