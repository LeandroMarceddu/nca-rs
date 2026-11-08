//! Pure SIMULATE-mode models: transport, scrub math, tool-chip text, path
//! phases, and the render-heightfield sampler.
//!
//! Nothing here opens a window or touches GPUI. The shell owns the
//! `Simulation` and paints; this module owns the decisions that can be tested
//! without a display.

use std::sync::Arc;
use std::time::Instant;

use nc_core::{FieldSource, Tool, ToolField, ToolShape};
use nc_gcode::Program;
use nc_sim::SimSnapshot;
use nc_stock::{Cutter, Heightmap, StockModel};
use nc_viewer::StockBox;

use crate::program::ProgramIndex;

/// A seek longer than this many simulated seconds runs on the worker thread.
pub const SHORT_SEEK_S: f64 = 0.6;

/// Upper bound on one wall-clock frame delta, in seconds.
pub const MAX_FRAME_DT_S: f64 = 0.25;

/// Playback transport state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    /// At the start, not advancing.
    Stopped,
    /// Position held, not advancing.
    #[default]
    Paused,
    /// The frame clock advances the simulation.
    Playing,
}

impl PlaybackState {
    /// Short uppercase label for a status chip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stopped => "STOPPED",
            Self::Paused => "PAUSED",
            Self::Playing => "PLAYING",
        }
    }
}

/// Playback speed multiplier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackSpeed {
    /// Half real time.
    Half,
    /// Real time.
    #[default]
    One,
    /// Twice real time.
    Two,
    /// Four times real time.
    Four,
}

impl PlaybackSpeed {
    /// Every selectable speed, in button order.
    pub const ALL: [Self; 4] = [Self::Half, Self::One, Self::Two, Self::Four];

    /// Multiplier applied to the frame delta.
    #[must_use]
    pub const fn factor(self) -> f64 {
        match self {
            Self::Half => 0.5,
            Self::One => 1.0,
            Self::Two => 2.0,
            Self::Four => 4.0,
        }
    }

    /// Short button label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Half => "0.5x",
            Self::One => "1x",
            Self::Two => "2x",
            Self::Four => "4x",
        }
    }
}

/// Play/pause state plus the selected speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transport {
    state: PlaybackState,
    speed: PlaybackSpeed,
}

impl Transport {
    /// Starts paused at the beginning.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current state.
    #[must_use]
    pub const fn state(self) -> PlaybackState {
        self.state
    }

    /// Current speed.
    #[must_use]
    pub const fn speed(self) -> PlaybackSpeed {
        self.speed
    }

    /// Returns `true` while the frame clock should advance the simulation.
    #[must_use]
    pub const fn is_playing(self) -> bool {
        matches!(self.state, PlaybackState::Playing)
    }

    /// Starts playback. Returns `true` when the state changed.
    pub fn play(&mut self) -> bool {
        if self.is_playing() {
            return false;
        }
        self.state = PlaybackState::Playing;
        true
    }

    /// Pauses playback. Returns `true` when the state changed.
    pub fn pause(&mut self) -> bool {
        if !self.is_playing() {
            return false;
        }
        self.state = PlaybackState::Paused;
        true
    }

    /// Toggles play and pause. Returns the new state.
    #[cfg(test)]
    pub fn toggle(&mut self) -> PlaybackState {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
        self.state
    }

    /// Stops at the beginning.
    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
    }

    /// Marks a manual step as paused.
    pub fn step(&mut self) {
        self.state = PlaybackState::Paused;
    }

    /// Selects a speed. Returns `true` when it changed.
    pub fn set_speed(&mut self, speed: PlaybackSpeed) -> bool {
        if self.speed == speed {
            return false;
        }
        self.speed = speed;
        true
    }
}

/// Wall-clock frame delta accumulator.
///
/// The simulator's own accumulator is the source of determinism; this clock
/// only turns wall time into a bounded `dt`. Resetting it after a pause or a
/// seek prevents a stale delta from jumping the tool.
#[derive(Clone, Debug)]
pub struct FrameClock {
    last: Option<Instant>,
    max_dt_s: f64,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self {
            last: None,
            max_dt_s: MAX_FRAME_DT_S,
        }
    }
}

impl FrameClock {
    /// Creates a clock with the given per-frame cap.
    #[must_use]
    #[cfg(test)]
    pub fn with_cap(max_dt_s: f64) -> Self {
        let max_dt_s = if max_dt_s.is_finite() && max_dt_s > 0.0 {
            max_dt_s
        } else {
            MAX_FRAME_DT_S
        };
        Self {
            last: None,
            max_dt_s,
        }
    }

    /// Forgets the previous timestamp; the next frame contributes zero.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Returns the bounded delta since the previous frame and records `now`.
    ///
    /// The first frame after [`Self::reset`] returns `0.0` rather than the time
    /// spent setting up or seeking.
    pub fn step(&mut self, now: Instant) -> f64 {
        let Some(previous) = self.last.replace(now) else {
            return 0.0;
        };
        let dt = now.duration_since(previous).as_secs_f64();
        if !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        dt.min(self.max_dt_s)
    }
}

/// Maps a scrub fraction in `0.0..=1.0` to a timeline time in seconds.
#[must_use]
pub fn time_for_fraction(fraction: f64, total_s: f64) -> f64 {
    if !fraction.is_finite() || !total_s.is_finite() || total_s <= 0.0 {
        return 0.0;
    }
    fraction.clamp(0.0, 1.0) * total_s
}

/// Maps a timeline time to a scrub fraction in `0.0..=1.0`.
#[must_use]
pub fn fraction_for_time(time_s: f64, total_s: f64) -> f64 {
    if !time_s.is_finite() || !total_s.is_finite() || total_s <= 0.0 {
        return 0.0;
    }
    (time_s / total_s).clamp(0.0, 1.0)
}

/// Whether a seek is long enough to leave the UI thread.
#[must_use]
pub fn is_long_seek(current_s: f64, target_s: f64) -> bool {
    if !current_s.is_finite() || !target_s.is_finite() {
        return false;
    }
    (target_s - current_s).abs() > SHORT_SEEK_S
}

/// Formats simulated seconds as `m:ss.s`.
///
/// Invalid and negative values render as `0:00.0`; huge values saturate.
#[must_use]
pub fn format_time(time_s: f64) -> String {
    if !time_s.is_finite() || time_s <= 0.0 {
        return "0:00.0".to_string();
    }
    let tenths = (time_s * 10.0).round().min(u64::MAX as f64) as u64;
    let minutes = tenths / 600;
    let seconds = (tenths / 10) % 60;
    let fraction = tenths % 10;
    format!("{minutes}:{seconds:02}.{fraction}")
}

/// One side of the current source line in the display toolpath.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPhase {
    /// A source line before the current line.
    Traversed,
    /// The current source line, or one of its canned-cycle segments.
    Current,
    /// A source line after the current line.
    Future,
}

/// Maps every display toolpath segment to the 1-based source line that made it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SegmentLines {
    lines: Vec<usize>,
}

impl SegmentLines {
    /// Builds the mapping from a parsed program's line ranges.
    #[must_use]
    pub fn from_program(program: &Program) -> Self {
        let mut lines = vec![0_usize; program.toolpath.len()];
        for line in &program.lines {
            for segment in line.segments.clone() {
                if let Some(slot) = lines.get_mut(segment) {
                    *slot = line.line;
                }
            }
        }
        Self { lines }
    }

    /// Number of mapped segments.
    #[must_use]
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Source line for a segment, when known.
    #[must_use]
    pub fn line_for(&self, segment: usize) -> Option<usize> {
        self.lines.get(segment).copied().filter(|line| *line > 0)
    }

    /// Classifies one segment against the current source line.
    #[must_use]
    pub fn phase(&self, segment: usize, current_line: usize) -> PathPhase {
        match self.line_for(segment) {
            Some(line) if current_line > 0 && line < current_line => PathPhase::Traversed,
            Some(line) if current_line > 0 && line == current_line => PathPhase::Current,
            _ => PathPhase::Future,
        }
    }

    /// Counts segments in each phase, for tests and diagnostics.
    #[must_use]
    #[cfg(test)]
    pub fn phase_counts(&self, current_line: usize) -> [usize; 3] {
        let mut counts = [0_usize; 3];
        for segment in 0..self.lines.len() {
            let index = match self.phase(segment, current_line) {
                PathPhase::Traversed => 0,
                PathPhase::Current => 1,
                PathPhase::Future => 2,
            };
            counts[index] += 1;
        }
        counts
    }
}

/// Visible program-list row for a follow line, when the filtered list contains
/// it.
#[must_use]
pub fn follow_visible_row(index: &ProgramIndex, line: usize) -> Option<usize> {
    let source = line.checked_sub(1)?;
    index.visible_index(source)
}

/// Which overlays and stock layers SIMULATE draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimToggles {
    /// Show the cut stock in 2D and 3D.
    pub stock: bool,
    /// Show rapid and retract moves.
    pub rapids: bool,
    /// Show cutting moves above the stock top.
    pub air_cuts: bool,
    /// Show the not-yet-traversed path dimmed.
    pub future_path: bool,
    /// Show the tool cylinder and crosshair.
    pub tool: bool,
    /// Show diagnostic warning overlays. No diagnostics are loaded in this
    /// milestone, so the toggle is present but has no segments to gate yet.
    pub warnings: bool,
}

impl Default for SimToggles {
    fn default() -> Self {
        Self {
            stock: true,
            rapids: true,
            air_cuts: true,
            future_path: true,
            tool: true,
            warnings: true,
        }
    }
}

/// Display text for the tool chip, derived from resolved data only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolChip {
    /// Tool number.
    pub tool: u32,
    /// `T3 dia 6 comment L4`-style label, using the product diameter glyph.
    pub text: String,
    /// `true` while the active diameter is assumed or a centerline trace.
    pub assumed: bool,
}

/// Builds the tool-chip label and assumption flag.
///
/// The comment suffix appears only when the diameter came from a program
/// comment. A missing or assumed diameter renders as a question mark.
#[must_use]
pub fn tool_chip(tool: u32, entry: Option<&Tool>, cutter: Option<&Cutter>) -> ToolChip {
    let assumed = cutter.is_some_and(|cutter| cutter.assumed || cutter.is_centerline());
    let diameter = cutter
        .and_then(|cutter| cutter.diameter_mm)
        .or_else(|| entry.and_then(|entry| entry.diameter_mm));
    let comment_line = entry
        .and_then(|entry| entry.sources.get(&ToolField::Diameter).copied())
        .and_then(|source| match source {
            FieldSource::Comment { line } => Some(line),
            FieldSource::Config => None,
        });
    let diameter_text = if assumed {
        "?".to_string()
    } else {
        diameter.map_or_else(|| "?".to_string(), format_diameter)
    };
    let mut text = format!("T{tool} \u{00b7} \u{00d8}{diameter_text}");
    if let Some(line) = comment_line
        && !assumed
    {
        text.push_str(&format!(" \u{00b7} comment L{line}"));
    }
    ToolChip {
        tool,
        text,
        assumed,
    }
}

/// Parses the quick-edit fields and returns the tool entry to persist.
///
/// Empty diameter or flute text means unknown; invalid text is rejected with a
/// plain-language message instead of storing a zero or a panic.
pub fn parse_tool_edit(
    number: u32,
    diameter_text: &str,
    flutes_text: &str,
    shape: ToolShape,
    existing: Option<&Tool>,
) -> Result<Tool, String> {
    let diameter_text = diameter_text.trim();
    let diameter = if diameter_text.is_empty() {
        None
    } else {
        match diameter_text.parse::<f64>() {
            Ok(value) if value.is_finite() && value > 0.0 => Some(value),
            _ => {
                return Err("Diameter must be a positive number, or empty for unknown.".to_string());
            }
        }
    };
    let flutes_text = flutes_text.trim();
    let flutes = if flutes_text.is_empty() {
        None
    } else {
        match flutes_text.parse::<u32>() {
            Ok(value) if value > 0 => Some(value),
            _ => return Err("Flutes must be a positive whole number, or empty.".to_string()),
        }
    };
    let mut tool = existing.cloned().unwrap_or_else(|| Tool::new(number));
    tool.number = number;
    tool.shape = shape;
    tool.diameter_mm = diameter;
    tool.flutes = flutes;
    if diameter.is_some() {
        tool.sources
            .insert(ToolField::Diameter, FieldSource::Config);
    } else {
        tool.sources.remove(&ToolField::Diameter);
    }
    if flutes.is_some() {
        tool.sources.insert(ToolField::Flutes, FieldSource::Config);
    } else {
        tool.sources.remove(&ToolField::Flutes);
    }
    if shape != ToolShape::Unknown {
        tool.sources.insert(ToolField::Shape, FieldSource::Config);
    } else {
        tool.sources.remove(&ToolField::Shape);
    }
    Ok(tool)
}

/// Formats a diameter with up to two decimals, trimming trailing zeros.
#[must_use]
pub fn format_diameter(diameter_mm: f64) -> String {
    if !diameter_mm.is_finite() || diameter_mm <= 0.0 {
        return "?".to_string();
    }
    let text = format!("{diameter_mm:.2}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

/// A sampled heightfield over the stock box, ready for a painter.
///
/// Heights are sampled bilinearly from the simulator heightmap and stored as
/// `f32` only at the render boundary. The grid has `resolution + 1` vertices
/// per axis; vertex `(col, row)` is row-major with row 0 at minimum Y.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightfieldMesh {
    /// Cells per axis in the render grid.
    pub resolution: usize,
    /// Heightmap version this mesh was sampled from.
    pub stock_version: u64,
    /// Stock minimum XY in millimetres.
    pub origin_xy_mm: [f64; 2],
    /// Stock size XY in millimetres.
    pub size_xy_mm: [f64; 2],
    /// Stock top Z in millimetres.
    pub top_z_mm: f64,
    /// Deepest sampled cut below the top, in millimetres.
    pub deepest_mm: f64,
    /// `(resolution + 1)^2` vertex heights in millimetres.
    pub heights_mm: Vec<f32>,
}

impl HeightfieldMesh {
    /// Samples the simulator heightmap over the stock box.
    #[must_use]
    pub fn sample(heightmap: &Heightmap, stock: &StockModel, resolution: usize) -> Self {
        let resolution = resolution.clamp(1, 512);
        let width = stock.max_mm[0] - stock.min_mm[0];
        let depth = stock.max_mm[1] - stock.min_mm[1];
        let vertices = resolution + 1;
        let mut heights_mm = Vec::with_capacity(vertices * vertices);
        let mut deepest_mm = 0.0_f64;
        for row in 0..vertices {
            #[allow(clippy::cast_precision_loss)]
            let v = row as f64 / resolution as f64;
            let y = stock.min_mm[1] + depth * v;
            for col in 0..vertices {
                #[allow(clippy::cast_precision_loss)]
                let u = col as f64 / resolution as f64;
                let x = stock.min_mm[0] + width * u;
                let height = sample_height_bilinear(heightmap, x, y);
                deepest_mm = deepest_mm.max(stock.top_z_mm - height);
                heights_mm.push(height as f32);
            }
        }
        Self {
            resolution,
            stock_version: heightmap.version(),
            origin_xy_mm: [stock.min_mm[0], stock.min_mm[1]],
            size_xy_mm: [width, depth],
            top_z_mm: stock.top_z_mm,
            deepest_mm: deepest_mm.max(0.0),
            heights_mm,
        }
    }

    /// Vertex count per axis.
    #[must_use]
    #[cfg(test)]
    pub fn vertices(&self) -> usize {
        self.resolution + 1
    }

    /// Height at a grid vertex, when in range.
    #[must_use]
    pub fn height_at(&self, col: usize, row: usize) -> Option<f32> {
        let vertices = self.resolution + 1;
        if col >= vertices || row >= vertices {
            return None;
        }
        self.heights_mm.get(row * vertices + col).copied()
    }

    /// World XY of a grid vertex.
    #[must_use]
    pub fn vertex_xy_mm(&self, col: usize, row: usize) -> [f64; 2] {
        let resolution = self.resolution.max(1);
        #[allow(clippy::cast_precision_loss)]
        let u = col as f64 / resolution as f64;
        #[allow(clippy::cast_precision_loss)]
        let v = row as f64 / resolution as f64;
        [
            self.origin_xy_mm[0] + self.size_xy_mm[0] * u,
            self.origin_xy_mm[1] + self.size_xy_mm[1] * v,
        ]
    }

    /// Cut fraction for a sampled height: `0.0` uncut, `1.0` at the deepest
    /// sampled cut.
    #[must_use]
    pub fn depth_fraction(&self, height_mm: f32) -> f32 {
        if self.deepest_mm <= 1.0e-9 {
            return 0.0;
        }
        let depth = self.top_z_mm - f64::from(height_mm);
        #[allow(clippy::cast_possible_truncation)]
        let fraction = (depth / self.deepest_mm) as f32;
        if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Number of quads in the mesh.
    #[must_use]
    #[cfg(test)]
    pub fn quad_count(&self) -> usize {
        self.resolution * self.resolution
    }
}

/// Bilinear sample of the cell-centred heightmap at a world XY point.
fn sample_height_bilinear(heightmap: &Heightmap, x: f64, y: f64) -> f64 {
    let cell = heightmap.cell_mm();
    let origin = heightmap.origin_xy_mm();
    let cols = heightmap.cols();
    let rows = heightmap.rows();
    if cols == 0 || rows == 0 || !cell.is_finite() || cell <= 0.0 {
        return heightmap.top_z_mm();
    }
    let raw_col = (x - origin[0]) / cell - 0.5;
    let raw_row = (y - origin[1]) / cell - 0.5;
    let col0 = raw_col.floor();
    let row0 = raw_row.floor();
    let tx = (raw_col - col0).clamp(0.0, 1.0);
    let ty = (raw_row - row0).clamp(0.0, 1.0);
    let col0 = clamp_index(col0, cols);
    let row0 = clamp_index(row0, rows);
    let col1 = (col0 + 1).min(cols - 1);
    let row1 = (row0 + 1).min(rows - 1);
    let h00 = heightmap
        .get(col0, row0)
        .map_or(heightmap.top_z_mm(), f64::from);
    let h10 = heightmap
        .get(col1, row0)
        .map_or(heightmap.top_z_mm(), f64::from);
    let h01 = heightmap
        .get(col0, row1)
        .map_or(heightmap.top_z_mm(), f64::from);
    let h11 = heightmap
        .get(col1, row1)
        .map_or(heightmap.top_z_mm(), f64::from);
    (h00 * (1.0 - tx) + h10 * tx) * (1.0 - ty) + (h01 * (1.0 - tx) + h11 * tx) * ty
}

fn clamp_index(value: f64, count: usize) -> usize {
    if !value.is_finite() || count == 0 {
        return 0;
    }
    let max = count.saturating_sub(1);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = value.clamp(0.0, max as f64) as usize;
    clamped.min(max)
}

/// Version- and resolution-keyed heightfield cache.
#[derive(Default)]
pub struct HeightfieldCache {
    version: Option<u64>,
    resolution: Option<usize>,
    mesh: Option<Arc<HeightfieldMesh>>,
    #[cfg(test)]
    builds: u32,
}

impl HeightfieldCache {
    /// Returns the cached mesh, rebuilding only when the heightmap version or
    /// render resolution changed.
    pub fn ensure(
        &mut self,
        heightmap: &Heightmap,
        stock: &StockModel,
        resolution: usize,
    ) -> Arc<HeightfieldMesh> {
        let resolution = resolution.clamp(1, 512);
        if self.version == Some(heightmap.version())
            && self.resolution == Some(resolution)
            && let Some(mesh) = &self.mesh
        {
            return Arc::clone(mesh);
        }
        let mesh = Arc::new(HeightfieldMesh::sample(heightmap, stock, resolution));
        self.version = Some(heightmap.version());
        self.resolution = Some(resolution);
        self.mesh = Some(Arc::clone(&mesh));
        #[cfg(test)]
        {
            self.builds += 1;
        }
        mesh
    }

    /// Drops the cached mesh, used when the program or stock changes.
    pub fn clear(&mut self) {
        self.version = None;
        self.resolution = None;
        self.mesh = None;
    }

    /// The currently cached mesh, when one has been sampled.
    #[must_use]
    pub fn current(&self) -> Option<Arc<HeightfieldMesh>> {
        self.mesh.as_ref().map(Arc::clone)
    }

    /// Build counter for tests.
    #[cfg(test)]
    pub fn builds(&self) -> u32 {
        self.builds
    }
}

/// Everything the canvas needs to paint one SIMULATE frame.
///
/// The heightmap itself stays in the simulation; only the sampled render mesh
/// is cloned into the canvas closure.
#[derive(Clone, Debug)]
pub struct SimCanvas {
    /// Current simulation snapshot.
    pub snapshot: SimSnapshot,
    /// Sampled stock surface.
    pub mesh: Arc<HeightfieldMesh>,
    /// Stock box for the outline and 2D footprint.
    pub stock: StockBox,
    /// Active cutter, when the timeline names one.
    pub cutter: Option<Cutter>,
    /// Display-toolpath segment to source-line mapping.
    pub segment_lines: Arc<SegmentLines>,
    /// Overlay toggles.
    pub toggles: SimToggles,
    /// `true` while the active diameter is assumed.
    pub tool_assumed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use nc_core::Tool;

    #[test]
    fn transport_plays_pauses_and_stops() {
        let mut transport = Transport::new();
        assert_eq!(transport.state(), PlaybackState::Paused);
        assert!(transport.play());
        assert!(!transport.play());
        assert!(transport.is_playing());
        assert!(transport.pause());
        assert_eq!(transport.toggle(), PlaybackState::Playing);
        transport.stop();
        assert_eq!(transport.state(), PlaybackState::Stopped);
        assert!(!transport.is_playing());
        transport.step();
        assert_eq!(transport.state(), PlaybackState::Paused);
    }

    #[test]
    fn transport_speed_buttons_are_stable() {
        let mut transport = Transport::new();
        assert_eq!(transport.speed(), PlaybackSpeed::One);
        assert_eq!(
            PlaybackSpeed::ALL.map(PlaybackSpeed::factor),
            [0.5, 1.0, 2.0, 4.0]
        );
        assert!(transport.set_speed(PlaybackSpeed::Four));
        assert!(!transport.set_speed(PlaybackSpeed::Four));
        assert_eq!(transport.speed().label(), "4x");
    }

    #[test]
    fn frame_clock_accumulates_and_resets() {
        let start = Instant::now();
        let mut clock = FrameClock::with_cap(1.0);
        assert_eq!(clock.step(start), 0.0);
        assert!((clock.step(start + Duration::from_millis(16)) - 0.016).abs() < 1.0e-9);
        assert!((clock.step(start + Duration::from_millis(32)) - 0.016).abs() < 1.0e-9);
        clock.reset();
        assert_eq!(clock.step(start + Duration::from_secs(5)), 0.0);
    }

    #[test]
    fn frame_clock_caps_a_stall() {
        let start = Instant::now();
        let mut clock = FrameClock::with_cap(0.05);
        let _ = clock.step(start);
        let dt = clock.step(start + Duration::from_secs(2));
        assert!((dt - 0.05).abs() < 1.0e-12);
    }

    #[test]
    fn scrub_maps_fraction_and_time_both_ways() {
        assert!((time_for_fraction(0.25, 20.0) - 5.0).abs() < 1.0e-12);
        assert!((fraction_for_time(5.0, 20.0) - 0.25).abs() < 1.0e-12);
        assert_eq!(time_for_fraction(-1.0, 20.0), 0.0);
        assert_eq!(time_for_fraction(2.0, 20.0), 20.0);
        assert_eq!(fraction_for_time(30.0, 20.0), 1.0);
        assert_eq!(time_for_fraction(0.5, 0.0), 0.0);
        assert_eq!(fraction_for_time(f64::NAN, 20.0), 0.0);
    }

    #[test]
    fn long_seek_threshold_is_absolute() {
        assert!(!is_long_seek(1.0, 1.2));
        assert!(is_long_seek(1.0, 2.0));
        assert!(is_long_seek(2.0, 1.0));
        assert!(!is_long_seek(f64::NAN, 1.0));
    }

    #[test]
    fn time_format_is_stable() {
        assert_eq!(format_time(0.0), "0:00.0");
        assert_eq!(format_time(-2.0), "0:00.0");
        assert_eq!(format_time(f64::NAN), "0:00.0");
        assert_eq!(format_time(5.04), "0:05.0");
        assert_eq!(format_time(65.25), "1:05.3");
        assert_eq!(format_time(3600.0), "60:00.0");
    }

    #[test]
    fn tool_edit_parses_known_and_unknown_values() {
        let mut existing = Tool::new(2);
        existing.name = Some("rough".to_string());
        let tool =
            parse_tool_edit(2, "6.35", "3", ToolShape::Ball, Some(&existing)).expect("valid edit");
        assert_eq!(tool.number, 2);
        assert_eq!(tool.diameter_mm, Some(6.35));
        assert_eq!(tool.flutes, Some(3));
        assert_eq!(tool.shape, ToolShape::Ball);
        assert_eq!(tool.name.as_deref(), Some("rough"));
        assert_eq!(
            tool.sources.get(&ToolField::Diameter),
            Some(&FieldSource::Config)
        );
        assert_eq!(
            tool.sources.get(&ToolField::Flutes),
            Some(&FieldSource::Config)
        );

        let cleared = parse_tool_edit(4, "", "", ToolShape::Unknown, None).expect("clear");
        assert_eq!(cleared.diameter_mm, None);
        assert_eq!(cleared.flutes, None);
        assert_eq!(cleared.shape, ToolShape::Unknown);

        assert!(parse_tool_edit(1, "0", "", ToolShape::Flat, None).is_err());
        assert!(parse_tool_edit(1, "-2", "", ToolShape::Flat, None).is_err());
        assert!(parse_tool_edit(1, "abc", "", ToolShape::Flat, None).is_err());
        assert!(parse_tool_edit(1, "6", "0", ToolShape::Flat, None).is_err());
        assert!(parse_tool_edit(1, "6", "many", ToolShape::Flat, None).is_err());
    }

    #[test]
    fn tool_chip_uses_resolved_diameter_and_comment_source() {
        let mut tool = Tool::new(3);
        tool.diameter_mm = Some(6.0);
        tool.sources
            .insert(ToolField::Diameter, FieldSource::Comment { line: 4 });
        let cutter = Cutter::from_tool(&tool, None);
        let chip = tool_chip(3, Some(&tool), Some(&cutter));
        assert_eq!(chip.text, "T3 \u{00b7} \u{00d8}6 \u{00b7} comment L4");
        assert!(!chip.assumed);
    }

    #[test]
    fn tool_chip_marks_unknown_size_as_assumed() {
        let tool = Tool::new(7);
        let cutter = Cutter::from_tool(&tool, None);
        let chip = tool_chip(7, Some(&tool), Some(&cutter));
        assert_eq!(chip.text, "T7 \u{00b7} \u{00d8}?");
        assert!(chip.assumed);
        let centerline = tool_chip(7, None, Some(&Cutter::centerline()));
        assert!(centerline.assumed);
        assert_eq!(format_diameter(6.0), "6");
        assert_eq!(format_diameter(6.35), "6.35");
        assert_eq!(format_diameter(0.0), "?");
    }

    #[test]
    fn shape_vocabulary_is_available_to_the_editor() {
        assert_eq!(ToolShape::Flat.label(), "flat");
        assert_eq!(ToolShape::parse("ball-nose"), Some(ToolShape::Ball));
        assert_eq!(ToolShape::parse("nonsense"), None);
    }

    #[test]
    fn toggles_default_to_everything_on() {
        let toggles = SimToggles::default();
        assert!(toggles.stock);
        assert!(toggles.rapids);
        assert!(toggles.air_cuts);
        assert!(toggles.future_path);
        assert!(toggles.tool);
        assert!(toggles.warnings);
    }

    #[test]
    fn segment_lines_split_past_current_and_future() {
        let source = "G21 G90\n\
                      G0 X0 Y0\n\
                      G1 X10 F100\n\
                      G81 X20 Y0 Z-2 R2 F50\n\
                      G1 X30\n";
        let program = nc_gcode::parse_program(source).expect("parse");
        let mapping = SegmentLines::from_program(&program);
        assert_eq!(mapping.len(), program.toolpath.len());
        assert!(mapping.len() >= 4);

        let counts = mapping.phase_counts(4);
        assert!(counts[0] > 0, "past segments");
        assert!(counts[1] > 0, "current line segments");
        assert!(counts[2] > 0, "future segments");

        // Every canned-cycle sub-move carries the same drill line.
        let drill_line = program
            .lines
            .iter()
            .find(|line| line.segment_count() > 1)
            .map(|line| line.line)
            .expect("a canned-cycle line with several segments");
        assert!(drill_line >= 1);
        let counts = mapping.phase_counts(drill_line);
        assert!(counts[1] >= 2, "canned-cycle segments are all current");

        let all_future = mapping.phase_counts(0);
        assert_eq!(all_future[2], mapping.len());
        assert_eq!(all_future[0], 0);
        let all_past = mapping.phase_counts(usize::MAX);
        assert_eq!(all_past[0], mapping.len());
    }

    #[test]
    fn follow_row_maps_a_line_through_the_search_filter() {
        let source = "G0 X0\nG1 X1 F100\nG1 X2\n";
        let program = nc_gcode::parse_program(source).expect("parse");
        let lines = crate::program::build_source_lines(source, Some(&program));
        let index = ProgramIndex::new(&lines, "X2");
        assert_eq!(index.visible_count(), 1);
        assert_eq!(follow_visible_row(&index, 3), Some(0));
        assert_eq!(follow_visible_row(&index, 2), None);
        assert_eq!(follow_visible_row(&index, 0), None);
    }

    fn stock_and_heightmap() -> (StockModel, Heightmap) {
        let stock = StockModel::explicit([0.0, 0.0, -5.0], [10.0, 10.0, 0.0]).expect("stock");
        let heightmap =
            Heightmap::new([0.0, 0.0], [10.0, 10.0], 1.0, 0.0, 1000).expect("heightmap");
        (stock, heightmap)
    }

    #[test]
    fn heightfield_samples_the_requested_resolution() {
        let (stock, heightmap) = stock_and_heightmap();
        let mesh = HeightfieldMesh::sample(&heightmap, &stock, 8);
        assert_eq!(mesh.resolution, 8);
        assert_eq!(mesh.vertices(), 9);
        assert_eq!(mesh.heights_mm.len(), 81);
        assert_eq!(mesh.quad_count(), 64);
        assert!(mesh.heights_mm.iter().all(|height| *height == 0.0));
        assert_eq!(mesh.deepest_mm, 0.0);
        assert_eq!(mesh.depth_fraction(0.0), 0.0);
        assert_eq!(mesh.height_at(8, 8), Some(0.0));
        assert_eq!(mesh.height_at(9, 0), None);
    }

    #[test]
    fn heightfield_samples_a_cut_cell() {
        let (stock, mut heightmap) = stock_and_heightmap();
        // Cell (5, 5) covers world X/Y 5..6 and is lowered to -1 mm.
        assert!(heightmap.lower_cell(5, 5, -1.0));
        let mesh = HeightfieldMesh::sample(&heightmap, &stock, 20);
        // Resolution 20 puts a vertex at the 5.5 mm cell centre.
        let centre = mesh.height_at(11, 11).expect("vertex");
        assert!((centre - (-1.0)).abs() < 0.01, "centre was {centre}");
        assert!(mesh.deepest_mm > 0.5);
        assert!(mesh.depth_fraction(centre) > 0.0);
    }

    #[test]
    fn heightfield_cache_rebuilds_on_version_and_resolution() {
        let (stock, mut heightmap) = stock_and_heightmap();
        let mut cache = HeightfieldCache::default();
        let first = cache.ensure(&heightmap, &stock, 8);
        let again = cache.ensure(&heightmap, &stock, 8);
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(cache.builds(), 1);

        heightmap.lower_cell(2, 2, -1.0);
        let versioned = cache.ensure(&heightmap, &stock, 8);
        assert!(!Arc::ptr_eq(&first, &versioned));
        assert_eq!(cache.builds(), 2);

        let resized = cache.ensure(&heightmap, &stock, 16);
        assert_eq!(resized.resolution, 16);
        assert_eq!(cache.builds(), 3);

        cache.clear();
        let cleared = cache.ensure(&heightmap, &stock, 16);
        assert_eq!(cache.builds(), 4);
        assert!(!Arc::ptr_eq(&resized, &cleared));
    }
}
