//! The deterministic simulation state and tick loop.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use nc_core::Tool;
use nc_core::ToolTable;
use nc_core::math;
use nc_gcode::{ArcRecord, Plane, Program};
use nc_stock::{Cutter, Heightmap, StockError, StockModel};
use nc_viewer::MoveKind;

use crate::TICK_S;
use crate::config::SimConfig;
use crate::error::SimError;
use crate::timeline::{MotionSpan, Timeline};

/// A read-only view of the simulation at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct SimSnapshot {
    /// Exact simulated tick count.
    pub tick: u64,
    /// Exact pose at `tick`; the determinism oracle.
    pub tick_pose_mm: [f64; 3],
    /// Display pose, interpolated between ticks.
    pub pose_mm: [f64; 3],
    /// Active tool number.
    pub tool: u32,
    /// `true` when the spindle is on.
    pub spindle_on: bool,
    /// Simulated elapsed time in seconds.
    pub time_s: f64,
    /// Current source line.
    pub current_line: usize,
    /// Current motion kind.
    pub current_kind: Option<MoveKind>,
    /// Heightmap content version.
    pub stock_version: u64,
    /// Progress in `0.0..=1.0`.
    pub progress: f64,
}

/// Aggregate statistics for the simulation UI.
///
/// Volume and depth are computed from the current heightmap. The time fields
/// are timeline totals for the whole program. `volume_assumed` is set when the
/// material was removed by a centerline trace or by a cutter whose diameter is
/// not real tool data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimStats {
    /// Estimated removed volume in cubic millimetres.
    pub cut_volume_mm3: f64,
    /// Deepest cut below the stock top in millimetres.
    pub max_depth_mm: f64,
    /// Total cutting time in simulated seconds.
    pub cut_time_s: f64,
    /// Total rapid and retract time in simulated seconds.
    pub rapid_time_s: f64,
    /// Total non-cutting, non-rapid motion time in simulated seconds.
    pub air_time_s: f64,
    /// Total dwell time in simulated seconds.
    pub dwell_time_s: f64,
    /// Total tool-change pause time in simulated seconds.
    pub tool_change_time_s: f64,
    /// `true` when the removed volume depends on an assumed or centerline
    /// cutter width and is therefore an estimate.
    pub volume_assumed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct StatsCache {
    version: u64,
    cut_volume_mm3: f64,
    max_depth_mm: f64,
}

/// The deterministic stock-removal engine.
pub struct Simulation {
    timeline: Timeline,
    config: SimConfig,
    stock: StockModel,
    heightmap: Heightmap,
    cutters: BTreeMap<u32, Cutter>,
    tick: u64,
    accumulator_s: f64,
    tick_pose_mm: [f64; 3],
    next_pose_mm: [f64; 3],
    coarse: bool,
    stats_cache: Mutex<StatsCache>,
    assumed_cut_used: bool,
}

impl Simulation {
    /// Builds a simulation for a program, stock, and tool table.
    pub fn new(
        program: &Program,
        stock: StockModel,
        tools: &ToolTable,
        cfg: &SimConfig,
    ) -> Result<Self, SimError> {
        let config = cfg.sanitized();
        let timeline = Timeline::from_program(program, &config);
        if timeline.is_empty() {
            return Err(SimError::EmptyProgram);
        }
        let extent = [
            stock.max_mm[0] - stock.min_mm[0],
            stock.max_mm[1] - stock.min_mm[1],
        ];
        let valid = stock
            .min_mm
            .iter()
            .chain(stock.max_mm.iter())
            .all(|value| value.is_finite())
            && stock.top_z_mm.is_finite()
            && extent.iter().all(|value| value.is_finite() && *value > 0.0);
        if !valid {
            return Err(SimError::InvalidStock(format!("{stock:?}")));
        }
        let (heightmap, coarse) = Heightmap::new_fitted(
            [stock.min_mm[0], stock.min_mm[1]],
            extent,
            config.cell_mm,
            stock.top_z_mm,
            config.max_cells,
        )
        .map_err(|error| match error {
            StockError::GridTooLarge { cells, max_cells } => {
                SimError::GridTooLarge { cells, max_cells }
            }
            StockError::InvalidExtent { .. } => SimError::InvalidStock(format!("{stock:?}")),
        })?;
        let mut cutters = BTreeMap::new();
        for tool in tools.iter().map(|(_, tool)| tool) {
            cutters.insert(
                tool.number,
                Cutter::from_tool(tool, config.assumed_tool_diameter_mm),
            );
        }
        for motion in program.expanded_motions() {
            cutters.entry(motion.tool).or_insert_with(|| {
                Cutter::from_tool(&Tool::new(motion.tool), config.assumed_tool_diameter_mm)
            });
        }
        cutters
            .entry(0)
            .or_insert_with(|| Cutter::from_tool(&Tool::new(0), config.assumed_tool_diameter_mm));
        let tick_pose = timeline.pose_at_time(0.0).unwrap_or([0.0; 3]);
        let next_pose = timeline.pose_at_time(TICK_S).unwrap_or(tick_pose);
        Ok(Self {
            timeline,
            config,
            stock,
            heightmap,
            cutters,
            tick: 0,
            accumulator_s: 0.0,
            tick_pose_mm: tick_pose,
            next_pose_mm: next_pose,
            coarse,
            stats_cache: Mutex::new(StatsCache::default()),
            assumed_cut_used: false,
        })
    }

    /// Adds simulated time and runs whole ticks.
    ///
    /// The remainder stays in the accumulator. At most
    /// [`SimConfig::max_ticks_per_advance`] ticks run per call; queued ticks
    /// from an earlier call are not discarded. A zero step adds no simulated
    /// time but still drains queued ticks.
    pub fn advance(&mut self, dt_s: f64) {
        if !dt_s.is_finite() || dt_s < 0.0 {
            return;
        }
        self.accumulator_s += dt_s;
        let requested = tick_for_elapsed(self.accumulator_s);
        let ticks = requested.min(self.config.max_ticks_per_advance as u64);
        if ticks == 0 {
            return;
        }
        self.accumulator_s -= ticks as f64 * TICK_S;
        if self.accumulator_s < 0.0 {
            self.accumulator_s = 0.0;
        }
        for _ in 0..ticks {
            self.tick_once();
        }
    }

    /// Seeks to the tick containing `time_s`.
    ///
    /// The target tick is `floor(time / TICK_S)`. A backward seek resets and
    /// replays so the heightmap is identical to a fresh run. A forward seek is
    /// incremental; time beyond the last motion is reached without replaying
    /// ticks that cannot change the stock.
    pub fn seek_to_time(&mut self, time_s: f64) {
        if !time_s.is_finite() || time_s < 0.0 {
            return;
        }
        let target = tick_at_time(time_s);
        self.replay_to_tick(target);
        self.accumulator_s = (time_s - target as f64 * TICK_S).max(0.0);
        if self.accumulator_s >= TICK_S {
            self.accumulator_s = 0.0;
        }
    }

    /// Seeks to the first indexed line at or after `line`.
    pub fn seek_to_line(&mut self, line: usize) {
        let Some(time_s) = self.timeline.time_for_line_at_or_after(line) else {
            return;
        };
        let target = tick_at_or_after(time_s);
        self.replay_to_tick(target);
        self.accumulator_s = 0.0;
    }

    /// Advances to the end of the current motion.
    pub fn step_motion(&mut self) {
        let now = self.tick as f64 * TICK_S;
        let Some(index) = self.timeline.span_at_time(now) else {
            return;
        };
        let Some(span) = self.timeline.spans().get(index) else {
            return;
        };
        let target = tick_at_or_after(span.end_time_s);
        self.replay_to_tick(target);
        self.accumulator_s = 0.0;
    }

    /// Advances to the next line with motion.
    pub fn step_line(&mut self) {
        let now = self.tick as f64 * TICK_S;
        let Some(index) = self.timeline.span_at_time(now) else {
            return;
        };
        let Some(span) = self.timeline.spans().get(index) else {
            return;
        };
        let time_s = self
            .timeline
            .time_for_motion_line_after(span.line)
            .unwrap_or_else(|| self.timeline.last_motion_end_s());
        let target = tick_at_or_after(time_s);
        self.replay_to_tick(target);
        self.accumulator_s = 0.0;
    }

    /// Resets to tick zero with the uncut heightmap.
    pub fn reset(&mut self) {
        self.tick = 0;
        self.accumulator_s = 0.0;
        self.heightmap.reset();
        *self.stats_cache_lock() = StatsCache::default();
        self.assumed_cut_used = false;
        self.tick_pose_mm = self.timeline.pose_at_time(0.0).unwrap_or([0.0; 3]);
        self.next_pose_mm = self
            .timeline
            .pose_at_time(TICK_S)
            .unwrap_or(self.tick_pose_mm);
    }

    /// Current snapshot.
    #[must_use]
    pub fn snapshot(&self) -> SimSnapshot {
        let total = self.timeline.total_time_s();
        let time_s = self.tick as f64 * TICK_S;
        let span = self
            .timeline
            .span_at_time(time_s)
            .and_then(|index| self.timeline.spans().get(index));
        let alpha = math::clamp(self.accumulator_s / TICK_S, 0.0, 1.0);
        let pose = [
            self.tick_pose_mm[0] + (self.next_pose_mm[0] - self.tick_pose_mm[0]) * alpha,
            self.tick_pose_mm[1] + (self.next_pose_mm[1] - self.tick_pose_mm[1]) * alpha,
            self.tick_pose_mm[2] + (self.next_pose_mm[2] - self.tick_pose_mm[2]) * alpha,
        ];
        SimSnapshot {
            tick: self.tick,
            tick_pose_mm: self.tick_pose_mm,
            pose_mm: pose,
            tool: span.map_or(0, |span| span.tool),
            spindle_on: span.is_some_and(|span| span.spindle_on),
            time_s,
            current_line: span.map_or(0, |span| span.line),
            current_kind: span.map(|span| span.kind),
            stock_version: self.heightmap.version(),
            progress: if total > 0.0 {
                math::clamp(time_s / total, 0.0, 1.0)
            } else {
                0.0
            },
        }
    }

    /// UI-facing statistics for the current heightmap and the program timeline.
    ///
    /// The volume and depth scan is cached by heightmap version, so calling
    /// this every frame does not rescan the grid unless material changed. Time
    /// fields are timeline totals for the whole program.
    #[must_use]
    pub fn stats(&self) -> SimStats {
        let version = self.heightmap.version();
        let (cut_volume_mm3, max_depth_mm) = {
            let mut cache = self.stats_cache_lock();
            if cache.version != version {
                let top_z_mm = self.stock.top_z_mm;
                let area_mm2 = self.heightmap.cell_mm() * self.heightmap.cell_mm();
                let mut volume_mm3 = 0.0_f64;
                let mut lowest_mm = top_z_mm;
                for cell in self.heightmap.cells() {
                    let height_mm = f64::from(*cell);
                    volume_mm3 += (top_z_mm - height_mm).max(0.0) * area_mm2;
                    if height_mm < lowest_mm {
                        lowest_mm = height_mm;
                    }
                }
                *cache = StatsCache {
                    version,
                    cut_volume_mm3: volume_mm3,
                    max_depth_mm: (top_z_mm - lowest_mm).max(0.0),
                };
            }
            (cache.cut_volume_mm3, cache.max_depth_mm)
        };
        SimStats {
            cut_volume_mm3,
            max_depth_mm,
            cut_time_s: self.timeline.cutting_time_s(),
            rapid_time_s: self.timeline.rapid_time_s(),
            air_time_s: self.timeline.air_time_s(),
            dwell_time_s: self.timeline.dwell_time_s(),
            tool_change_time_s: self.timeline.tool_change_time_s(),
            volume_assumed: self.assumed_cut_used
                || self
                    .current_cutter()
                    .is_some_and(|cutter| cutter.assumed || cutter.is_centerline()),
        }
    }

    /// The heightmap after the current tick.
    #[must_use]
    pub fn heightmap(&self) -> &Heightmap {
        &self.heightmap
    }

    /// The timeline in use.
    #[must_use]
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// The sanitized configuration in use.
    #[must_use]
    pub fn config(&self) -> &SimConfig {
        &self.config
    }

    /// The stock model in use.
    #[must_use]
    pub fn stock(&self) -> &StockModel {
        &self.stock
    }

    /// The cutter for a tool number.
    #[must_use]
    pub fn cutter_for_tool(&self, tool: u32) -> Option<&Cutter> {
        self.cutters.get(&tool)
    }

    /// The cutter active at the current tick.
    #[must_use]
    pub fn current_cutter(&self) -> Option<&Cutter> {
        let time_s = self.tick as f64 * TICK_S;
        let tool = self
            .timeline
            .span_at_time(time_s)
            .and_then(|index| self.timeline.spans().get(index))
            .map_or(0, |span| span.tool);
        self.cutters.get(&tool)
    }

    /// `true` while the active cutter diameter is an assumption, not tool data.
    #[must_use]
    pub fn is_tool_size_assumed(&self) -> bool {
        self.current_cutter().is_some_and(|cutter| cutter.assumed)
    }

    /// `true` when crash preview is enabled and rapid travel can gouge.
    #[must_use]
    pub fn crash_preview(&self) -> bool {
        self.config.crash_preview
    }

    /// `true` when the grid is coarser than 2 mm.
    #[must_use]
    pub fn is_coarse(&self) -> bool {
        self.coarse || self.heightmap.is_coarse()
    }

    fn stats_cache_lock(&self) -> MutexGuard<'_, StatsCache> {
        match self.stats_cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn replay_to_tick(&mut self, target: u64) {
        if target < self.tick {
            self.reset();
        }
        if target <= self.tick {
            return;
        }
        let end_tick = tick_at_or_after(self.timeline.last_motion_end_s());
        let run_to = target.min(end_tick);
        while self.tick < run_to {
            self.tick_once();
        }
        if target > end_tick {
            self.tick = target;
            let time_s = target as f64 * TICK_S;
            self.tick_pose_mm = self
                .timeline
                .pose_at_time(time_s)
                .unwrap_or(self.tick_pose_mm);
            let next_time = target.saturating_add(1) as f64 * TICK_S;
            self.next_pose_mm = self
                .timeline
                .pose_at_time(next_time)
                .unwrap_or(self.tick_pose_mm);
        }
    }

    fn tick_once(&mut self) {
        if self.tick == u64::MAX {
            return;
        }
        let previous_time = self.tick as f64 * TICK_S;
        let next_time = (self.tick + 1) as f64 * TICK_S;
        self.apply_interval(previous_time, next_time);
        self.tick += 1;
        self.tick_pose_mm = self
            .timeline
            .pose_at_time(next_time)
            .unwrap_or(self.tick_pose_mm);
        let following_time = (self.tick + 1) as f64 * TICK_S;
        self.next_pose_mm = self
            .timeline
            .pose_at_time(following_time)
            .unwrap_or(self.tick_pose_mm);
    }

    fn apply_interval(&mut self, start_time_s: f64, end_time_s: f64) {
        let range = self.timeline.overlapping_spans(start_time_s, end_time_s);
        for index in range {
            let Some(span) = self.timeline.spans().get(index).copied() else {
                continue;
            };
            let start = start_time_s.max(span.start_time_s);
            let end = end_time_s.min(span.end_time_s);
            if end <= start {
                continue;
            }
            let cuts =
                span.kind.is_cutting() || (self.config.crash_preview && span.kind.is_rapid());
            if !cuts {
                continue;
            }
            let cutter = match self.cutters.get(&span.tool) {
                Some(cutter) => cutter.clone(),
                None => Cutter::centerline(),
            };
            if cutter.assumed || cutter.is_centerline() {
                self.assumed_cut_used = true;
            }
            let from = self
                .timeline
                .pose_in_span(index, start)
                .unwrap_or(span.start_pose_mm);
            let to = self
                .timeline
                .pose_in_span(index, end)
                .unwrap_or(span.end_pose_mm);
            let arc = self.timeline.arc_for_span(index).cloned();
            let tolerance_mm = self.heightmap.cell_mm() * 0.25;
            match arc {
                Some(arc) => {
                    let plane = self.timeline.plane_for_span(index).unwrap_or(Plane::Xy);
                    let fraction_start = fraction_in_span(span, start);
                    let fraction_end = fraction_in_span(span, end);
                    let sub_arc = ArcRecord {
                        center_mm: arc.center_mm,
                        radius_mm: arc.radius_mm,
                        end_radius_mm: arc.end_radius_mm,
                        start_angle_rad: arc.start_angle_rad + arc.sweep_rad * fraction_start,
                        sweep_rad: arc.sweep_rad * (fraction_end - fraction_start),
                        axial_delta_mm: arc.axial_delta_mm * (fraction_end - fraction_start),
                        format: arc.format,
                    };
                    self.heightmap
                        .cut_arc(&cutter, &sub_arc, from, to, plane, tolerance_mm);
                }
                None => {
                    self.heightmap.cut_segment(&cutter, from, to);
                }
            }
        }
    }
}

fn fraction_in_span(span: MotionSpan, time_s: f64) -> f64 {
    let duration = span.duration_s();
    if duration > 0.0 {
        math::clamp((time_s - span.start_time_s) / duration, 0.0, 1.0)
    } else {
        0.0
    }
}

fn tick_for_elapsed(elapsed_s: f64) -> u64 {
    const GUARD_S: f64 = 1.0e-9;
    if !elapsed_s.is_finite() || elapsed_s <= 0.0 {
        return 0;
    }
    let ticks = math::floor(elapsed_s / TICK_S + GUARD_S);
    if ticks <= 0.0 { 0 } else { ticks as u64 }
}

fn tick_at_time(time_s: f64) -> u64 {
    tick_for_elapsed(time_s)
}

fn tick_at_or_after(time_s: f64) -> u64 {
    const GUARD_S: f64 = 1.0e-9;
    if !time_s.is_finite() || time_s <= 0.0 {
        return 0;
    }
    let ticks = math::ceil((time_s - GUARD_S) / TICK_S);
    if ticks <= 0.0 { 0 } else { ticks as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::{ToolShape, ToolTable};
    use nc_gcode::{
        Annotation, ArcFormat, ArcRecord, CannedCycle, CannedRecord, ProgramBuilder, RetractMode,
    };
    use nc_viewer::AnnotationKind;

    fn config() -> SimConfig {
        SimConfig {
            cell_mm: 1.0,
            max_cells: 1_000_000,
            rapid_rate_mm_min: 5000.0,
            tool_change_pause_s: 2.0,
            crash_preview: false,
            render_resolution: 64,
            max_ticks_per_advance: 1000,
            assumed_tool_diameter_mm: None,
        }
    }

    fn block() -> StockModel {
        StockModel::explicit([-10.5, -10.5, -10.0], [10.5, 10.5, 0.0]).expect("stock")
    }

    fn flat_tools(diameter: f64) -> ToolTable {
        let mut tool = Tool::new(1);
        tool.shape = ToolShape::Flat;
        tool.diameter_mm = Some(diameter);
        let mut table = ToolTable::new();
        table.insert(tool);
        table
    }

    fn drill_tools(diameter: f64, tip_angle_deg: f64) -> ToolTable {
        let mut tool = Tool::new(2);
        tool.shape = ToolShape::Drill;
        tool.diameter_mm = Some(diameter);
        tool.tip_angle_deg = Some(tip_angle_deg);
        let mut table = ToolTable::new();
        table.insert(tool);
        table
    }

    fn record(
        line: usize,
        kind: MoveKind,
        start: [f64; 3],
        end: [f64; 3],
        feed: f64,
        tool: u32,
    ) -> nc_gcode::MotionRecord {
        let mut motion = nc_gcode::MotionRecord::linear(line, line, kind, start, end);
        motion.feed_mm_min = Some(feed);
        motion.tool = tool;
        motion.spindle_rpm = Some(12000.0);
        motion.spindle_cw = Some(true);
        motion
    }

    fn linear_program() -> Program {
        ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Feed,
                [-5.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .build()
    }

    fn mixed_program() -> Program {
        let arc = ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 4.0,
            end_radius_mm: 4.0,
            start_angle_rad: 0.0,
            sweep_rad: math::PI,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let mut arc_motion = record(
            2,
            MoveKind::ArcCw,
            [4.0, 0.0, -1.0],
            [-4.0, 0.0, -1.0],
            800.0,
            1,
        );
        arc_motion.arc = Some(Box::new(arc));
        ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Feed,
                [-5.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .record(arc_motion)
            .build()
    }

    fn run_to_tick(sim: &mut Simulation, target: u64) {
        let mut guard = 0_u32;
        while sim.snapshot().tick < target {
            sim.advance(TICK_S);
            guard += 1;
            assert!(guard < 1_000_000, "simulation did not progress");
        }
    }

    #[test]
    fn flat_feed_removes_material() {
        let mut sim = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        run_to_tick(&mut sim, 120);
        assert_eq!(sim.snapshot().tick, 120);
        assert_eq!(sim.heightmap().get(10, 10), Some(-1.0));
        assert_eq!(sim.heightmap().get(5, 10), Some(-1.0));
        assert_eq!(sim.heightmap().get(15, 10), Some(-1.0));
        assert_eq!(sim.heightmap().get(10, 15), Some(0.0));
        assert!(sim.snapshot().stock_version > 0);
    }

    #[test]
    fn stats_volume_and_depth_grow_after_a_cut() {
        let mut sim = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        let before = sim.stats();
        assert_eq!(before.cut_volume_mm3, 0.0);
        assert_eq!(before.max_depth_mm, 0.0);
        assert!((before.cut_time_s - 1.0).abs() < 1e-12);
        assert_eq!(before.rapid_time_s, 0.0);
        assert!(!before.volume_assumed);
        run_to_tick(&mut sim, 120);
        let after = sim.stats();
        assert!(after.cut_volume_mm3 > 0.0);
        assert!((after.max_depth_mm - 1.0).abs() < 1e-12);
        assert!(!after.volume_assumed);
        // The second call is served from the version-keyed cache.
        assert_eq!(after, sim.stats());
    }

    #[test]
    fn centerline_volume_is_marked_assumed() {
        let mut sim = Simulation::new(&linear_program(), block(), &ToolTable::new(), &config())
            .expect("simulation");
        sim.seek_to_time(1.5);
        let stats = sim.stats();
        assert!(stats.cut_volume_mm3 > 0.0);
        assert!(stats.volume_assumed);

        let mut known = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        known.seek_to_time(1.5);
        assert!(!known.stats().volume_assumed);
    }

    #[test]
    fn stats_match_at_the_same_tick_for_different_dt_sequences() {
        let mut a = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        for _ in 0..240 {
            a.advance(TICK_S);
        }
        let mut b = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        for _ in 0..200 {
            b.advance(0.01);
        }
        assert_eq!(a.snapshot().tick, 240);
        assert_eq!(b.snapshot().tick, 240);
        assert_eq!(a.stats(), b.stats());
    }

    #[test]
    fn max_depth_matches_the_deepest_cut() {
        let program = ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Feed,
                [0.0, 5.0, -3.5],
                [10.0, 5.0, -3.5],
                600.0,
                1,
            ))
            .build();
        let mut sim =
            Simulation::new(&program, block(), &flat_tools(2.0), &config()).expect("simulation");
        sim.seek_to_time(2.0);
        let stats = sim.stats();
        assert!((stats.max_depth_mm - 3.5).abs() < 1e-12);
        assert!(stats.cut_volume_mm3 > 0.0);
    }

    #[test]
    fn rapids_do_not_cut_by_default() {
        let program = ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Rapid,
                [-5.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .build();
        let mut sim =
            Simulation::new(&program, block(), &flat_tools(2.0), &config()).expect("simulation");
        let initial = sim.heightmap().content_hash();
        sim.advance(0.5);
        assert_eq!(sim.snapshot().tick, 60);
        assert_eq!(sim.heightmap().content_hash(), initial);
        assert_eq!(sim.snapshot().stock_version, 0);
    }

    #[test]
    fn crash_preview_cuts_rapids_and_marks_the_version() {
        let program = ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Rapid,
                [-5.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .build();
        let mut normal =
            Simulation::new(&program, block(), &flat_tools(2.0), &config()).expect("simulation");
        let mut crash = Simulation::new(
            &program,
            block(),
            &flat_tools(2.0),
            &SimConfig {
                crash_preview: true,
                ..config()
            },
        )
        .expect("simulation");
        normal.advance(0.5);
        crash.advance(0.5);
        assert_ne!(
            normal.heightmap().content_hash(),
            crash.heightmap().content_hash()
        );
        assert_eq!(crash.heightmap().get(10, 10), Some(-1.0));
        assert_eq!(normal.heightmap().get(10, 10), Some(0.0));
        assert!(crash.snapshot().stock_version > 0);
    }

    #[test]
    fn default_budget_supports_four_times_speed_at_fifteen_fps() {
        // 1x at 15 fps needs 8 ticks; 4x needs 32. The old default of 8 made
        // both speeds hit the same cap, so 4x looked identical to 1x.
        let config = SimConfig::default();
        let mut one = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config)
            .expect("simulation");
        one.advance(1.0 / 15.0);
        assert_eq!(one.snapshot().tick, 8);

        let mut four = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config)
            .expect("simulation");
        four.advance((1.0 / 15.0) * 4.0);
        assert_eq!(four.snapshot().tick, 32);
    }

    #[test]
    fn different_dt_sequences_reach_the_same_hash() {
        let target = 240_u64;
        let sequences: [(&[f64], usize); 5] = [
            (&[TICK_S], 240),
            (&[2.0 * TICK_S], 120),
            (&[5.0 * TICK_S], 48),
            (&[0.01], 200),
            (&[0.1, 0.1, 0.05], 8),
        ];
        let mut expected: Option<(u64, [f64; 3])> = None;
        for (pattern, cycles) in sequences {
            let mut sim = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
                .expect("simulation");
            for _ in 0..cycles {
                for dt in pattern {
                    sim.advance(*dt);
                }
            }
            assert_eq!(sim.snapshot().tick, target, "sequence {pattern:?}");
            let observed = (sim.heightmap().content_hash(), sim.snapshot().tick_pose_mm);
            match expected {
                Some(expected) => {
                    assert_eq!(expected.0, observed.0, "hash for sequence {pattern:?}");
                    assert_eq!(expected.1, observed.1, "pose for sequence {pattern:?}");
                }
                None => expected = Some(observed),
            }
        }
    }

    #[test]
    fn max_ticks_per_advance_splitting_keeps_the_hash() {
        let target = 240_u64;
        let mut split = Simulation::new(
            &mixed_program(),
            block(),
            &flat_tools(2.0),
            &SimConfig {
                max_ticks_per_advance: 3,
                ..config()
            },
        )
        .expect("simulation");
        split.advance(240.0 * TICK_S);
        assert_eq!(split.snapshot().tick, 3);
        for _ in 0..79 {
            split.advance(0.0);
        }
        assert_eq!(split.snapshot().tick, target);

        let mut whole = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        whole.advance(240.0 * TICK_S);
        assert_eq!(whole.snapshot().tick, target);
        assert_eq!(
            split.heightmap().content_hash(),
            whole.heightmap().content_hash()
        );
        assert_eq!(split.snapshot().tick_pose_mm, whole.snapshot().tick_pose_mm);
    }

    #[test]
    fn advance_keeps_queued_ticks() {
        let mut sim = Simulation::new(
            &linear_program(),
            block(),
            &flat_tools(2.0),
            &SimConfig {
                max_ticks_per_advance: 1,
                ..config()
            },
        )
        .expect("simulation");
        sim.advance(5.0 * TICK_S);
        assert_eq!(sim.snapshot().tick, 1);
        for _ in 0..20 {
            sim.advance(0.25 * TICK_S);
        }
        assert_eq!(sim.snapshot().tick, 10);
    }

    #[test]
    fn backward_seek_matches_a_fresh_replay() {
        let target = 120_u64;
        let mut fresh = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        fresh.seek_to_time(target as f64 * TICK_S);
        let mut travelled = Simulation::new(&mixed_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        travelled.seek_to_time(1.8);
        travelled.seek_to_time(target as f64 * TICK_S);
        assert_eq!(travelled.snapshot().tick, target);
        assert_eq!(fresh.snapshot().tick, travelled.snapshot().tick);
        assert_eq!(
            fresh.heightmap().content_hash(),
            travelled.heightmap().content_hash()
        );
        assert_eq!(
            fresh.snapshot().tick_pose_mm,
            travelled.snapshot().tick_pose_mm
        );
        assert_eq!(
            fresh.snapshot().stock_version,
            travelled.snapshot().stock_version
        );
    }

    #[test]
    fn seek_targets_the_floor_tick_and_keeps_the_fraction() {
        let mut sim = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        sim.seek_to_time(10.5 * TICK_S);
        let snapshot = sim.snapshot();
        assert_eq!(snapshot.tick, 10);
        assert!(snapshot.pose_mm[0] > snapshot.tick_pose_mm[0]);
        let exact = sim.timeline().pose_at_time(10.0 * TICK_S).expect("pose");
        assert_eq!(snapshot.tick_pose_mm, exact);
    }

    #[test]
    fn step_motion_and_step_line_reach_the_next_motion() {
        let program = ProgramBuilder::new()
            .record(record(
                10,
                MoveKind::Feed,
                [-5.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .record(record(
                20,
                MoveKind::Feed,
                [0.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .build();
        let mut sim =
            Simulation::new(&program, block(), &flat_tools(2.0), &config()).expect("simulation");
        sim.step_motion();
        assert_eq!(sim.snapshot().tick, 60);
        sim.step_line();
        assert_eq!(sim.snapshot().tick, 120);
        assert_eq!(sim.snapshot().current_line, 20);
        sim.reset();
        sim.seek_to_line(20);
        assert_eq!(sim.snapshot().tick, 60);
        assert_eq!(sim.snapshot().current_line, 20);
    }

    #[test]
    fn arc_helix_and_drill_are_repeatable() {
        let arc = ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 3.0,
            end_radius_mm: 3.0,
            start_angle_rad: 0.0,
            sweep_rad: math::PI,
            axial_delta_mm: 0.0,
            format: ArcFormat::Center,
        };
        let helix = ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            end_radius_mm: 2.0,
            start_angle_rad: 0.0,
            sweep_rad: math::TAU,
            axial_delta_mm: -2.0,
            format: ArcFormat::Center,
        };
        let mut arc_motion = record(
            2,
            MoveKind::ArcCw,
            [3.0, 0.0, -1.0],
            [-3.0, 0.0, -1.0],
            600.0,
            1,
        );
        arc_motion.arc = Some(Box::new(arc));
        let mut helix_motion = record(
            3,
            MoveKind::HelixCcw,
            [2.0, 0.0, -1.0],
            [2.0, 0.0, -3.0],
            600.0,
            1,
        );
        helix_motion.arc = Some(Box::new(helix));
        let drill = record(
            4,
            MoveKind::Drill,
            [5.0, 5.0, 0.0],
            [5.0, 5.0, -4.0],
            200.0,
            2,
        );
        let program = ProgramBuilder::new()
            .record(arc_motion)
            .record(helix_motion)
            .record(drill)
            .build();
        let mut tools = flat_tools(2.0);
        let mut drill_tool = Tool::new(2);
        drill_tool.shape = ToolShape::Drill;
        drill_tool.diameter_mm = Some(4.0);
        drill_tool.tip_angle_deg = Some(118.0);
        tools.insert(drill_tool);
        let mut first = Simulation::new(&program, block(), &tools, &config()).expect("simulation");
        let mut second = Simulation::new(&program, block(), &tools, &config()).expect("simulation");
        first.seek_to_time(10.0);
        second.seek_to_time(10.0);
        assert!(first.snapshot().stock_version > 0);
        let helix_cell = first.heightmap().get(8, 10).unwrap_or(0.0);
        assert!(
            (-2.4..=-1.8).contains(&helix_cell),
            "the full helix circle must cut away from its endpoints: {helix_cell}"
        );
        assert_eq!(
            first.heightmap().get(7, 10),
            Some(-1.0),
            "the half arc must cut away from its endpoints"
        );
        assert_eq!(
            first.heightmap().content_hash(),
            second.heightmap().content_hash()
        );
        assert_eq!(
            first.snapshot().tick_pose_mm,
            second.snapshot().tick_pose_mm
        );
    }

    #[test]
    fn drill_reaches_tip_depth_and_the_cone_walls() {
        let program = ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Drill,
                [0.0, 0.0, 0.5],
                [0.0, 0.0, -5.0],
                200.0,
                2,
            ))
            .build();
        let mut sim = Simulation::new(&program, block(), &drill_tools(6.0, 118.0), &config())
            .expect("simulation");
        sim.seek_to_time(2.0);
        let tip = sim.heightmap().get(10, 10).expect("cell");
        assert!((tip - (-5.0)).abs() < 1.0e-6, "tip height {tip}");
        let wall = sim.heightmap().get(11, 10).expect("cell");
        let expected = -5.0 + 1.0 / math::tan(math::to_radians(59.0));
        assert!(
            (wall - expected as f32).abs() < 1.0e-5,
            "{wall} vs {expected}"
        );
        assert_eq!(sim.heightmap().get(14, 10), Some(0.0));
    }

    #[test]
    fn peck_retract_does_not_fill_the_hole() {
        let mut motion = record(
            1,
            MoveKind::Drill,
            [0.0, 0.0, 0.5],
            [0.0, 0.0, -5.0],
            200.0,
            2,
        );
        motion.canned = Some(Box::new(CannedRecord {
            cycle: CannedCycle::PeckDrill,
            hole_index: 0,
            r_plane_mm: 0.5,
            bottom_z_mm: -5.0,
            peck_q_mm: Some(2.0),
            dwell_s: None,
            retract_mode: RetractMode::RPlane,
        }));
        let program = ProgramBuilder::new().record(motion).build();
        let expanded = program.expanded_motions().count();
        assert!(expanded > 2, "peck should expand: {expanded}");
        let mut sim = Simulation::new(&program, block(), &drill_tools(6.0, 118.0), &config())
            .expect("simulation");
        sim.seek_to_time(5.0);
        assert_eq!(sim.heightmap().get(10, 10), Some(-5.0));
        assert!(sim.snapshot().stock_version > 0);
    }

    #[test]
    fn unknown_tool_is_a_centerline_trace_unless_a_size_is_assumed() {
        let program = linear_program();
        let tools = ToolTable::new();
        let mut centerline =
            Simulation::new(&program, block(), &tools, &config()).expect("simulation");
        centerline.seek_to_time(1.5);
        assert!(centerline.is_tool_size_assumed());
        assert_eq!(centerline.heightmap().get(10, 9), Some(0.0));
        assert_eq!(centerline.heightmap().get(10, 10), Some(-1.0));

        let mut assumed = Simulation::new(
            &program,
            block(),
            &tools,
            &SimConfig {
                assumed_tool_diameter_mm: Some(2.0),
                ..config()
            },
        )
        .expect("simulation");
        assumed.seek_to_time(1.5);
        assert!(assumed.is_tool_size_assumed());
        assert_eq!(assumed.heightmap().get(10, 9), Some(-1.0));
        let cutter = assumed.current_cutter().expect("cutter");
        assert_eq!(cutter.diameter_mm, Some(2.0));
        assert!(cutter.assumed);
    }

    #[test]
    fn a_grid_above_the_cap_coarsens_and_is_labeled() {
        let stock = StockModel::explicit([-50.0, -50.0, -10.0], [50.0, 50.0, 0.0]).expect("stock");
        let mut sim = Simulation::new(
            &linear_program(),
            stock,
            &flat_tools(2.0),
            &SimConfig {
                cell_mm: 0.75,
                max_cells: 1000,
                ..config()
            },
        )
        .expect("simulation");
        assert!(sim.is_coarse());
        assert!(sim.heightmap().cell_mm() > 2.0);
        assert!(sim.heightmap().len() <= 1000);
        sim.seek_to_time(1.5);
        assert!(sim.snapshot().stock_version > 0);
    }

    #[test]
    fn tick_pose_is_exact_and_display_pose_interpolates() {
        let mut sim = Simulation::new(&linear_program(), block(), &flat_tools(2.0), &config())
            .expect("simulation");
        let start = sim.snapshot();
        sim.advance(0.5 * TICK_S);
        let half = sim.snapshot();
        assert_eq!(half.tick, 0);
        assert_eq!(half.tick_pose_mm, start.tick_pose_mm);
        assert!(half.pose_mm[0] > start.tick_pose_mm[0]);
        sim.advance(0.5 * TICK_S);
        let one = sim.snapshot();
        assert_eq!(one.tick, 1);
        let expected = sim.timeline().pose_at_time(TICK_S).expect("pose");
        assert_eq!(one.tick_pose_mm, expected);
    }

    #[test]
    fn spindle_state_comes_from_the_motion_and_annotations() {
        let program = ProgramBuilder::new()
            .record(record(
                1,
                MoveKind::Feed,
                [-5.0, 0.0, -1.0],
                [5.0, 0.0, -1.0],
                600.0,
                1,
            ))
            .annotation(Annotation {
                line: 2,
                next_motion_index: 1,
                pos_mm: [5.0, 0.0, -1.0],
                kind: AnnotationKind::Spindle {
                    on: false,
                    rpm: 0.0,
                    clockwise: true,
                },
            })
            .build();
        let mut sim =
            Simulation::new(&program, block(), &flat_tools(2.0), &config()).expect("simulation");
        assert!(sim.snapshot().spindle_on);
        sim.seek_to_time(2.0);
        // The trailing spindle-off has no motion after it, so the last span
        // still reports the state it ran with.
        assert!(sim.snapshot().spindle_on);
    }
}
