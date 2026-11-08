# Interfaces

This file is the frozen contract for shared types. It wins over `SPEC.md` if
they disagree; every deliberate deviation is recorded in `DECISIONS.md`.

Rules:

1. Every type named in `SPEC.md` is defined here, either in Rust or as an
   explicit standard-library alias.
2. Implementers code against this file, not against another crate's internals.
3. Only the orchestrator edits this file. A subagent that needs a change reports
   it; the change is recorded here and in `DECISIONS.md` before other agents
   depend on it.
4. Units are millimetres and degrees unless a field name says otherwise.
5. No type in this contract uses `f32` for computation, ambient time, a locale,
   or an absolute path.

## Crate ownership and dependency direction

| Crate | Owns | Depends on |
|---|---|---|
| `nc-core` | coordinates, units, WCS, `Tool`/`ToolTable`, `MachineConfig`, `MaterialPreset`, `Dialect`, `math` | no internal crates |
| `nc-viewer` | `MoveKind`, styles, `Segment`, `Toolpath`, `Bounds3`, `StockBox` | no crates |
| `nc-gcode` | `Program`, `ProgramLine`, `MotionRecord`, `ArcRecord`, `CannedRecord`, `ModalState`, parse errors, tool hints | `nc-core`, `nc-viewer` |
| `nc-stock` | `StockModel`, `StockSource`, `InferenceBasis`, `Heightmap`, `Cutter`, sweep | `nc-core`, `nc-viewer`, `nc-gcode` |
| `nc-sim` | `Simulation`, `SimSnapshot`, `Timeline`, `MotionSpan`, `SimConfig` | `nc-core`, `nc-gcode`, `nc-stock`, `nc-viewer` |
| `nc-analysis` | `AnalysisConfig`, `Report`, `Diagnostic`, `Rule`, `Coverage`, `EngagementData` | `nc-core`, `nc-gcode`, `nc-stock` |
| `nc-app` | GPUI shell, dialogs, persistence, worker threads, playback clock | all |
| `nc-cli` | `nca` binary | `nc-analysis`, `nc-sim` (never GPUI or `nc-app`) |
| `nc-fixtures` | fixture loading and golden-report comparison | `nc-core`, `nc-gcode`, `nc-stock`, `nc-analysis` |

`nc-gcode` re-exports `MoveKind`, `AnnotationKind`, and `Bounds3` from
`nc-viewer`, so downstream crates need no direct `nc-viewer` dependency.

## `nc-core`

```rust
pub enum ToolShape { Flat, Ball, Bull, Drill, VBit, Unknown } // Default: Unknown
pub enum Dialect { Generic, Grbl, LinuxCnc, Mach3, Fanuc }   // Default: Generic
pub enum ToolField {
    Name, Shape, Diameter, Flutes, Stickout, FluteLength, TipAngle,
    CornerRadius, ShankDiameter, HolderDiameter, RightHand,
}
pub enum FieldSource { Config, Comment { line: usize } }

pub struct Tool {
    pub number: u32,
    pub name: Option<String>,
    pub shape: ToolShape,
    pub diameter_mm: Option<f64>,
    pub flutes: Option<u32>,
    pub stickout_mm: Option<f64>,
    pub flute_length_mm: Option<f64>,
    pub tip_angle_deg: Option<f64>,
    pub corner_radius_mm: Option<f64>,
    pub shank_diameter_mm: Option<f64>,
    pub holder_diameter_mm: Option<f64>,
    pub right_hand: Option<bool>,
    pub sources: BTreeMap<ToolField, FieldSource>,
}

pub struct ToolTable(BTreeMap<u32, Tool>); // sorted, may be empty

pub struct ToolHint {
    pub tool: u32,
    pub line: usize,
    pub name: Option<String>,
    pub shape: Option<ToolShape>,
    pub diameter_mm: Option<f64>,
    pub flutes: Option<u32>,
}

pub struct MaterialPreset {
    pub name: String,
    pub chipload_min_mm: Option<f64>,
    pub chipload_max_mm: Option<f64>,
    pub surface_speed_min_mpm: Option<f64>,
    pub surface_speed_max_mpm: Option<f64>,
    pub max_mrr_mm3_min: Option<f64>,
    pub requires_coolant: bool,
    pub notes: String,
}

pub struct MachineConfig {
    pub travel_min_mm: [f64; 3],
    pub travel_max_mm: [f64; 3],
    pub max_feed_mm_min: Option<f64>,
    pub max_spindle_rpm: Option<f64>,
    pub rapid_rate_mm_min: Option<f64>,
    pub spindle_manual: bool,
}
```

Required methods:

```rust
impl ToolTable {
    pub fn resolve(config: &ToolTable, hints: &[ToolHint]) -> ToolTable;
    pub fn get(&self, number: u32) -> Option<&Tool>;
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &Tool)>;
    pub fn source_of(&self, number: u32, field: ToolField) -> Option<FieldSource>;
}
impl Tool {
    pub fn effective_shank_diameter_mm(&self) -> Option<f64>; // shank else diameter
}
impl MachineConfig {
    pub fn validate(&self) -> Result<(), CoreError>;
    pub fn contains(&self, point_mm: [f64; 3]) -> bool;
    pub fn rapid_rate_mm_min(&self) -> f64; // documented default 5000
}
impl MaterialPreset {
    pub fn builtin(name: &str) -> Option<Self>;
    pub fn builtin_names() -> &'static [&'static str];
}
```

### `nc_core::math`

All trigonometry in the workspace goes through this module. It wraps `libm`
and exposes `f64` functions only:

```text
PI, FRAC_PI_2, TAU, to_radians, to_degrees,
sin, cos, tan, asin, acos, atan, atan2,
sqrt, hypot, pow, floor, ceil, round, trunc, abs, clamp
```

`clamp` is total: NaN returns the minimum.

## `nc-viewer`

```rust
pub enum MoveKind {
    Rapid, Retract, Feed, Plunge, Ramp, ArcCw, ArcCcw,
    HelixCw, HelixCcw, Drill, Peck, Bore, Probe,
}
pub enum AnnotationKind {
    Dwell { seconds: f64 },
    ToolChange { tool: u32 },
    Spindle { on: bool, rpm: f64, clockwise: bool },
    Coolant { on: bool },
}
pub enum MoveFamily { Travel, Cutting, Arcs, Holes, Special }
pub enum HoleMarker { Ring, DashedRing, DoubleRing }
pub struct MoveStyle {
    pub label: &'static str,
    pub color_token: &'static str,
    pub width: f32,
    pub dash: Option<[f32; 2]>,
    pub marker: Option<HoleMarker>,
}
```

Required methods:

```rust
impl MoveKind {
    pub const ALL: [Self; 13];
    pub const fn is_cutting(self) -> bool;
    pub const fn is_rapid(self) -> bool;
    pub const fn is_retract(self) -> bool;
    pub const fn is_arc(self) -> bool;
    pub const fn is_helix(self) -> bool;
    pub const fn is_hole(self) -> bool;
    pub const fn family(self) -> MoveFamily;
    pub const fn label(self) -> &'static str;
    pub const fn short_label(self) -> &'static str;
    pub const fn style(self) -> MoveStyle;
}
impl AnnotationKind { pub const fn label(&self) -> &'static str; }
```

`color_token` names a theme token and never a literal color. The app owns the
dark, light, and high-contrast palettes.

The display types carry the same semantic kinds:

```rust
pub struct Segment {
    pub start: Point3,
    pub end: Point3,
    pub layer: u16,
    pub tool: u16,
    pub kind: MoveKind,
}
pub struct Polyline {
    pub points: Vec<Point2>,
    pub depth: f32,
    pub kind: MoveKind,
    pub layer: u16,
    pub tool: u16,
}
impl Toolpath {
    pub fn append_line(&mut self, start: Point3, end: Point3,
                       kind: MoveKind, layer: u16, tool: u16) -> bool;
}
```

`SegmentKind` is removed. `MoveKind` also derives `Ord`/`PartialOrd` for
per-kind visibility arrays. `Segment.tool` remains `u16` for display; all
analysis and simulation tool ids are `u32`.

`ProgramStats` folds semantic kinds into its existing counters: `Retract`
counts as rapid, `Ramp` as feed, and `Drill`/`Peck`/`Bore` as plunge. The
counters are aggregates, not the taxonomy.

## `nc-gcode`

### Modal and motion records

```rust
pub enum Plane { Xy, Zx, Yz }
pub struct ModalId(pub u32);
pub enum CutterComp { Off, Left, Right }
pub enum ArcFormat { Center, Radius }
pub type CannedCycle = CannedCycleKind;   // G73/G81/G82/G83/G85/G86/G89
pub type CoolantState = CoolantMode;      // Off/Mist/Flood

pub struct ModalState {
    pub units: Option<Units>,
    pub distance: Option<DistanceMode>,
    pub feed_mode: FeedMode,              // UnitsPerMinute | InverseTime
    pub wcs: Wcs,
    pub coolant: CoolantState,
    pub cutter_comp: CutterComp,
    pub g92_active: bool,
    pub retract_mode: RetractMode,
}

pub struct MotionRecord {
    pub line: usize,
    pub index: usize,
    pub kind: MoveKind,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub feed_mm_min: Option<f64>,
    pub spindle_rpm: Option<f64>,
    pub spindle_cw: Option<bool>,
    pub tool: u32,
    pub plane: Plane,
    pub arc: Option<Box<ArcRecord>>,
    pub canned: Option<Box<CannedRecord>>,
    pub modal: ModalId,
}

pub struct ArcRecord {
    pub center_mm: [f64; 3],
    pub radius_mm: f64,
    pub end_radius_mm: f64,
    pub start_angle_rad: f64,
    pub sweep_rad: f64,
    pub axial_delta_mm: f64,
    pub format: ArcFormat,
}

pub struct CannedRecord {
    pub cycle: CannedCycle,
    pub hole_index: usize,
    pub r_plane_mm: f64,
    pub bottom_z_mm: f64,
    pub peck_q_mm: Option<f64>,
    pub dwell_s: Option<f64>,
    pub retract_mode: RetractMode,
}

pub struct Annotation {
    pub line: usize,
    pub next_motion_index: usize,
    pub pos_mm: [f64; 3],
    pub kind: AnnotationKind,
}
pub struct Comment { pub line: usize, pub text: String }
```

Invariants:

- `Plane` axis basis: `Xy = (X, Y)`, `Zx = (Z, X)`, `Yz = (Y, Z)`. A positive
  `sweep_rad` is counter-clockwise viewed from the plane's positive normal and
  is the `G3` direction in every plane.
- `|sweep_rad|` is in `(0, 2*pi]`; a full circle is exactly `+/-2*pi`.
- The normal-axis component of `center_mm` equals the start point's component.
- `axial_delta_mm` is the signed travel along the plane normal. Non-zero makes
  the motion a helix.
- Canned cycles store one record per hole. `Program::expanded_motions()`
  generates rapid/feed/retract sub-moves on demand and never stores them.
- `size_of::<MotionRecord>() <= 128` is a test.

### Program

```rust
pub struct Program {
    pub lines: Vec<ProgramLine>,
    pub toolpath: Toolpath,
    pub bounds: Bounds3,
    pub stats: ProgramStats,
    pub state: ParserState,
    pub parameters: Parameters,
    pub offsets: [Position; 9],
    pub warnings: Vec<ParseWarning>,
    pub motions: Vec<MotionRecord>,
    pub annotations: Vec<Annotation>,
    pub comments: Vec<Comment>,
    pub tool_hints: Vec<ToolHint>,
    pub modal_table: Vec<ModalState>,
    pub geometry_uncertain_after_line: Option<usize>,
    pub skipped_blocks: Vec<SkippedBlock>,
}

impl Program {
    pub fn cutting_bounds(&self) -> Bounds3;
    pub fn modal(&self, id: ModalId) -> &ModalState; // out-of-range -> conservative state
    pub fn geometry_uncertain_after(&self) -> Option<usize>;
    pub fn expanded_motions(&self) -> impl Iterator<Item = MotionRecord> + '_;
    pub fn line_for_segment(&self, segment: usize) -> Option<&ProgramLine>;
    pub fn skipped_blocks(&self) -> &[SkippedBlock];
}
```

`MotionRecord` is the analysis/simulation source of truth. `Toolpath` is the
display geometry and a fallback for very large programs.

### Entry points and lossy parsing

```rust
pub fn parse_program(text: &str) -> Result<Program, Vec<ParseError>>;
pub fn parse_program_with(text: &str, config: &ParserConfig) -> Result<Program, Vec<ParseError>>;
pub fn parse_program_lossy(text: &str, config: &ParserConfig) -> LossyProgram;

pub struct LossyProgram {
    pub program: Program,
    pub errors: Vec<ParseError>,
    pub skipped_blocks: Vec<SkippedBlock>,
    pub trusted_geometry_until: Option<usize>,
}
pub struct SkippedBlock {
    pub line: usize,
    pub reason: String,
    pub source: String,
}
```

Lossy policy:

- A failed block is recorded, skipped, and the previous modal state is kept.
- A state-changing unsupported construct (`M98`/`M99`, macros/`G65`, flow
  control) stops new geometry after that line. `trusted_geometry_until` and
  `Program::geometry_uncertain_after_line` carry the first untrusted line.
- G41/G42 are *understood*: they are recorded in `ModalState::cutter_comp`,
  emit a warning, and geometry continues on the programmed centerline. They do
  not truncate geometry; tool-radius-dependent rules must skip while comp is
  active. G40 clears the mode.
- Unknown words and codes are diagnostics, never fatal.
- Input limits: `ParserConfig::max_file_bytes`, `max_line_bytes`, and
  `tool_hint_window` are enforced; exceeding a limit yields a clean
  truncation/refusal diagnostic.

`ParserConfig` additions required by the parser work:

```rust
pub struct ParserConfig {
    // parent fields: offsets, home_position, second_home_position,
    // chord_tolerance_mm, arc_radius_tolerance_mm, max_line_bytes, max_words,
    // max_errors, max_helix_step_mm, max_helix_slices, peck_clearance_mm,
    // max_coordinate_mm
    pub dialect: Dialect,        // default Generic
    pub max_file_bytes: usize,   // DEFAULT_MAX_FILE_BYTES
    pub tool_hint_window: usize, // default 3
}
```

`ParserState` carries `pub cutter_comp: CutterComp`; `ProgramStats::tools` is
`Vec<u32>`; `ParseErrorKind` adds `FileTooLarge` and `key()`; `WarningKind`
adds `CutterCompNotApplied`, `AmbiguousToolHint`, and `MissingFeed`.

### Tool hints

`ToolHint` is defined in `nc-core` to avoid a dependency cycle and re-exported
as `nc_gcode::ToolHint`. Recognizers fill it; the merge precedence is
`ToolTable::resolve`: user config per field, then comment hint, then unknown.

### Test builder

```rust
#[cfg(any(test, feature = "test-util"))]
pub struct ProgramBuilder { /* ... */ }
impl ProgramBuilder {
    pub fn new() -> Self;
    pub fn source(self, line: usize, text: impl Into<String>) -> Self;
    pub fn record(self, motion: MotionRecord) -> Self;
    pub fn motion(self, line: usize, kind: MoveKind, start_mm: [f64; 3],
                  end_mm: [f64; 3], feed_mm_min: Option<f64>) -> Self;
    pub fn annotation(self, annotation: Annotation) -> Self;
    pub fn uncertain_after(self, line: usize) -> Self;
    pub fn build(self) -> Program;
}
```

Consumers enable the `test-util` feature in their dev-dependencies.

## `nc-stock`

```rust
pub enum StockSource { Inferred, Explicit }
pub enum InferenceBasis { ZeroAtTop, AllCutsAboveZero, NoCuttingMoves }
pub struct StockModel {
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
    pub top_z_mm: f64,
    pub source: StockSource,
    pub inference: Option<InferenceBasis>,
}
pub struct Heightmap { /* origin, cell size, cols, rows, f32 cells, version */ }
pub struct Cutter {
    pub shape: ToolShape,
    pub diameter_mm: Option<f64>,
    pub corner_radius_mm: Option<f64>,
    pub tip_angle_deg: Option<f64>,
    pub stickout_mm: Option<f64>,
    pub flute_length_mm: Option<f64>,
    pub shank_diameter_mm: Option<f64>,
    pub holder_diameter_mm: Option<f64>,
    pub assumed: bool,
}
```

Required methods:

```rust
impl StockModel {
    pub fn infer(program: &Program) -> Self;
    pub fn explicit(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, StockError>;
    pub fn top_z(&self) -> f64;
    pub fn xy_extent_mm(&self) -> [f64; 2];
    pub fn contains(&self, point_mm: [f64; 3]) -> bool;
    pub fn to_stock_box(&self) -> Option<nc_viewer::StockBox>;
}
impl Heightmap {
    pub fn new(origin_xy_mm: [f64; 2], size_xy_mm: [f64; 2], cell_mm: f64,
               top_z_mm: f64, max_cells: usize) -> Result<Self, StockError>;
    pub fn cell_mm(&self) -> f64;
    pub fn cols(&self) -> usize;
    pub fn rows(&self) -> usize;
    pub fn len(&self) -> usize;
    pub fn origin_xy_mm(&self) -> [f64; 2];
    pub fn top_z_mm(&self) -> f64;
    pub fn version(&self) -> u64;
    pub fn bump_version(&mut self);
    pub fn get(&self, col: usize, row: usize) -> Option<f32>;
    pub fn cells(&self) -> &[f32];
    pub fn lower_cell(&mut self, col: usize, row: usize, height_mm: f64) -> bool;
    pub fn cell_center_mm(&self, col: usize, row: usize) -> [f64; 2];
    pub fn cell_at(&self, point_xy_mm: [f64; 2]) -> Option<(usize, usize)>;
    pub fn content_hash(&self) -> u64; // FNV-1a over little-endian f32 bits
}
impl Cutter {
    pub fn from_tool(tool: &Tool, assumed_diameter_mm: Option<f64>) -> Self;
    pub fn centerline() -> Self;
    pub fn radius_mm(&self) -> Option<f64>; // None = centerline trace
    pub fn display_radius_mm(&self) -> f64;
    pub fn is_centerline(&self) -> bool;
    pub fn has_conical_tip(&self) -> bool;
}
```

Phase 1 adds the shared sweep API used by both analysis and simulation:

```rust
pub struct SweepParams { /* centreline step, chord tolerance, step cap */ }
impl Heightmap {
    pub fn cut_segment(&mut self, cutter: &Cutter, from_mm: [f64; 3], to_mm: [f64; 3]) -> bool;
    pub fn cut_arc(&mut self, cutter: &Cutter, arc: &ArcRecord, from_mm: [f64;3],
                   to_mm: [f64;3], plane: Plane, tolerance_mm: f64) -> bool;
    pub fn cut_motion(&mut self, cutter: &Cutter, motion: &MotionRecord) -> bool;
    pub fn cut_motion_with(&mut self, cutter: &Cutter, motion: &MotionRecord,
                           params: SweepParams) -> bool;
}
```

`cut_motion` cuts one expanded motion. Straight motions longer than
`cell_mm / 2` are subdivided so a ramp follows the tool Z instead of lowering
every covered cell to the segment's lowest endpoint. Arcs and helixes follow
the exact recorded curve. `SweepParams::max_steps` bounds the subdivision.

## `nc-sim`

```rust
pub struct SimConfig {
    pub cell_mm: f64,
    pub max_cells: usize,
    pub rapid_rate_mm_min: f64,
    pub tool_change_pause_s: f64,
    pub crash_preview: bool,
    pub render_resolution: usize,
    pub max_ticks_per_advance: usize,
    pub assumed_tool_diameter_mm: Option<f64>,
}
pub struct SimSnapshot {
    pub tick: u64,
    pub tick_pose_mm: [f64; 3],
    pub pose_mm: [f64; 3],
    pub tool: u32,
    pub spindle_on: bool,
    pub time_s: f64,
    pub current_line: usize,
    pub current_kind: Option<MoveKind>,
    pub stock_version: u64,
    pub progress: f64,
}
pub struct MotionSpan {
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub start_distance_mm: f64,
    pub end_distance_mm: f64,
    pub line: usize,
    pub kind: MoveKind,
    pub tool: u32,
    pub start_pose_mm: [f64; 3],
    pub end_pose_mm: [f64; 3],
    pub feed_mm_min: Option<f64>,
    pub spindle_on: bool,
}
pub struct Timeline { /* ordered spans, prefix time, line-to-time index */ }
```

`MotionSpan` carries poses, tool, feed, and spindle state in addition to the
fields named in `SPEC.md`; the simulator needs them and no other owner can
supply them without duplicating the timeline.

```rust
impl Simulation {
    pub fn new(program: &Program, stock: StockModel, tools: &ToolTable,
               cfg: &SimConfig) -> Result<Self, SimError>;
    pub fn advance(&mut self, dt_s: f64);
    pub fn seek_to_time(&mut self, time_s: f64);
    pub fn seek_to_line(&mut self, line: usize);
    pub fn step_motion(&mut self);
    pub fn step_line(&mut self);
    pub fn reset(&mut self);
    pub fn snapshot(&self) -> SimSnapshot;
    pub fn heightmap(&self) -> &Heightmap;
    pub fn timeline(&self) -> &Timeline;
    pub fn is_coarse(&self) -> bool;
    pub fn stats(&self) -> SimStats;
}

pub struct SimStats {
    pub cut_volume_mm3: f64,
    pub max_depth_mm: f64,
    pub cut_time_s: f64,
    pub rapid_time_s: f64,
    pub air_time_s: f64,
    pub dwell_time_s: f64,
    pub tool_change_time_s: f64,
    pub volume_assumed: bool,
}
```

`SimStats` time fields are whole-program timeline totals, not elapsed-to-tick
values; `SimSnapshot.time_s` is the elapsed time. Volume and depth are computed
from the current heightmap and cached by heightmap version. `volume_assumed` is
true when any executed cutting pass used a centerline trace or an assumed
diameter.

Defaults (`SimConfig::default`): cell 0.75 mm, 4,000,000 cells, rapid
5000 mm/min, tool-change pause 2 s, crash preview off, render grid 64,
96 ticks per advance, assumed diameter `None`.

`96` is deliberate: at 4x speed and 15 fps a frame needs 32 ticks; the old
default of 8 made 1x and 4x hit the same cap on slow frames.

`TICK_S = 1.0 / 120.0`.

## `nc-analysis`

```rust
pub enum Severity { Error, Warning, Info }
pub enum Basis { Exact, Model }
pub struct RuleId(pub Cow<'static, str>);
pub enum EvidenceValue { Int(i64), Float(f64), Text(String), Bool(bool), Point3([f64; 3]) }

pub struct Diagnostic {
    pub rule: RuleId,
    pub severity: Severity,
    pub basis: Basis,
    pub line: usize,
    pub column: usize,
    pub line_end: usize,
    pub message: String,
    pub evidence: BTreeMap<String, EvidenceValue>,
}
pub struct SkippedRule { pub rule: RuleId, pub missing_input: String, pub reason: String }
pub struct Coverage { pub evaluated: usize, pub total: usize, pub skipped: Vec<SkippedRule> }

pub struct ToolInventoryEntry {
    pub tool: u32,
    pub first_line: usize,
    pub motion_count: usize,
    pub fields: BTreeMap<String, InventoryField>,
}
pub struct InventoryField { pub value: EvidenceValue, pub source: String }
pub struct InputInfo {
    pub file_name: String,
    pub sha256: String,
    pub line_count: usize,
    pub dialect: Dialect,
    pub geometry_uncertain_after: Option<usize>,
}
pub struct Report {
    pub schema: String,
    pub ruleset_version: String,
    pub input: InputInfo,
    pub config_digest: String,
    pub stock_source: StockSource,
    pub tool_inventory: Vec<ToolInventoryEntry>,
    pub diagnostics: Vec<Diagnostic>,
    pub truncated: BTreeMap<String, usize>,
    pub coverage: Coverage,
}
```

Configuration:

```rust
pub struct AnalysisConfig {
    pub machine: Option<MachineConfig>,
    pub stock: Option<StockConfig>,
    pub tools: ToolTable,
    pub material: Option<MaterialPreset>,
    pub tolerances: Tolerances,
    pub rules: RuleConfig,
}
pub struct StockConfig { pub origin_mm: [f64; 3], pub size_mm: [f64; 3] }
pub struct Tolerances {
    pub arc_radius_mm: f64,   // default 0.05
    pub min_segment_mm: f64,  // default 0.01
    pub hole_xy_mm: f64,      // default 0.05
    pub clearance_mm: f64,    // default 2.0
}
pub struct RuleConfig(pub BTreeMap<RuleId, RuleSettings>);
pub struct RuleSettings {
    pub enabled: bool,
    pub severity: Option<Severity>,
    pub thresholds: BTreeMap<String, f64>,
}
```

Rule contract:

```rust
pub trait Rule {
    fn id(&self) -> RuleId;
    fn default_severity(&self) -> Severity;
    fn basis(&self) -> Basis;
    fn requirements(&self) -> Requirements;
    fn evaluate(&self, model: &AnalysisModel<'_>, cfg: &RuleSettings) -> RuleOutcome;
    fn why_it_matters(&self) -> &'static str;
}
pub struct Requirements {
    pub tool_fields: &'static [ToolField],
    pub machine: bool,
    pub material: bool,
    pub stock: StockRequirement,
}
pub enum StockRequirement { None, InferredOk, Explicit }
pub enum RuleOutcome {
    Findings(Vec<Diagnostic>),
    Skipped { missing_input: String, reason: String },
}
pub struct AnalysisModel<'a> {
    pub program: &'a Program,
    pub stock: StockModel,
    pub tools: &'a ToolTable,
    pub machine: Option<&'a MachineConfig>,
    pub material: Option<&'a MaterialPreset>,
    pub tolerances: &'a Tolerances,
    pub engagement: OnceCell<EngagementData>,
    pub dialect_assumed: bool,
}
impl<'a> AnalysisModel<'a> {
    pub fn with_dialect_assumed(self, assumed: bool) -> Self;
}
```

Shared engagement model (one z-map pass, built lazily by `process.*` rules):

```rust
pub struct EngagementData {
    pub final_heightmap: Heightmap,
    pub passes: Vec<PassEngagement>,
    pub coarse: bool,
}
pub struct PassEngagement {
    pub motion_index: usize,
    pub line: usize,
    pub kind: MoveKind,
    pub axial_doc_mm: f64,
    pub radial_doc_mm: f64,
    pub contact_angle_rad: f64,
    pub mrr_mm3_min: f64,
}
```

Report constants: `REPORT_SCHEMA = "nca.report/1"`, `RULESET_VERSION = "1.0.0"`,
`MAX_FINDINGS_PER_RULE = 100`, `MAX_FINDINGS_TOTAL = 1000`.

Engine API:

```rust
pub struct AnalysisRequest<'a> {
    pub file_name: &'a str,      // only the file name is serialized
    pub source: &'a str,         // exact text that produced the program
    pub config: &'a AnalysisConfig,
    pub dialect: Dialect,
    pub ruleset_version: &'a str,
    pub dialect_assumed: bool,   // Generic assumed because none was configured
}
impl<'a> AnalysisRequest<'a> {
    pub fn new(file_name: &'a str, source: &'a str,
               config: &'a AnalysisConfig, dialect: Dialect) -> Self;
    pub fn with_ruleset_version(self, ruleset_version: &'a str) -> Self;
    pub fn with_dialect_assumed(self, assumed: bool) -> Self;
}

pub struct RuleRegistry { /* ordered rules */ }
impl RuleRegistry {
    pub fn new() -> Self;
    pub fn from_rules(rules: impl IntoIterator<Item = Box<dyn Rule>>) -> Self;
    pub fn register(&mut self, rule: Box<dyn Rule>);
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = &dyn Rule> + '_;
}

pub fn analyze(program: &Program, request: &AnalysisRequest<'_>,
               rules: &RuleRegistry) -> Report;

impl Coverage {
    pub fn skipped_count(&self) -> usize;
    pub fn skipped_rule_count(&self) -> usize; // distinct rules, not records
}
impl Report {
    pub fn to_json(&self) -> String;        // canonical compact bytes
    pub fn to_json_pretty(&self) -> String; // canonical pretty bytes
}
```

`Report::to_json`/`to_json_pretty` are the canonical serialization path: they
quantize by evidence key, normalize `-0.0`, and never emit a trailing newline.
`serde` on `Report` remains available for tooling but is not the golden format.

Coverage arithmetic: `Coverage.skipped` holds one record per missing input, so a
single rule with two missing tool fields appears twice. The invariant is
`evaluated + skipped_rule_count() == total`; `evaluated + skipped.len() == total`
holds only when each skipped rule has exactly one missing input.

## Config file schema (TOML)

```toml
[parse]
dialect = "generic"            # generic | grbl | linuxcnc | mach3 | fanuc

[machine]
travel_min_mm = [0, 0, -120]
travel_max_mm = [400, 300, 0]
max_feed_mm_min = 3000
max_spindle_rpm = 24000
rapid_rate_mm_min = 3000
spindle_manual = false

[stock]                        # omit the table to infer stock
origin_mm = [0, 0, -20]
size_mm = [100, 80, 20]

[[tool]]                       # optional; every field except `number` is optional
number = 1
name = "6 mm 2F flat"
shape = "flat"
diameter_mm = 6.0
flutes = 2
stickout_mm = 30
flute_length_mm = 18
right_hand = true

[material]
preset = "aluminum-6061"

[rules."clearance.rapid_at_depth"]
enabled = true
severity = "warning"
clearance_mm = 2.0
```

Config loading belongs to `nc-app` and `nc-cli`. Unknown keys are errors, not
silently ignored, so a typo cannot disable a rule. The shared loader lives in
`nc-analysis::config_file`:

```rust
pub struct LoadedConfig {
    pub analysis: AnalysisConfig,
    pub dialect: Dialect,
    pub dialect_assumed: bool,
}
pub fn parse_config(text: &str) -> Result<LoadedConfig, ConfigError>;
pub fn load_config(path: &Path) -> Result<LoadedConfig, ConfigError>;
```

Schema rules:

- every table is optional;
- `[machine]` requires `travel_min_mm` and `travel_max_mm`;
- `[stock]` requires `origin_mm` and `size_mm`;
- `[[tool]]` requires `number`; every other field is optional and gets
  `FieldSource::Config`;
- `[material]` accepts a built-in `preset` and/or inline fields; inline values
  override the preset;
- `[rules."<id>"]` accepts `enabled` (default true), `severity`, and every other
  numeric key as a flattened threshold;
- `dialect` accepts `generic|grbl|linuxcnc|linux_cnc|mach3|fanuc`; an omitted
  dialect is `Generic` with `dialect_assumed = true`;
- duplicate tool numbers, unknown keys, and unknown `[rules."<id>"]` ids are errors;
- `ConfigError` carries an optional file/line/column location; `UnknownRule`
  names an unregistered rule id.


## Report JSON

- `schema` is `"nca.report/1"`; `ruleset_version` is a semantic version.
- Collections are sorted: tools by number, diagnostics by
  `(severity_rank, line, column, rule_id)`, skips by rule id.
- Quantization: coordinates 0.001 mm, feeds 0.1 mm/min, spindle 1 rpm, angles
  0.01 degrees, percentages 0.1 percent. Serialization uses fixed decimals.
- `input.file_name` is a file name, never a path.
- `config_digest` is lowercase SHA-256 hex over the canonical serialized config.
- Per-rule cap 100 and total cap 1000; dropped counts live in `truncated`.
- `geometry_uncertain_after` is present when the lossy parser stopped geometry.

## `nc-fixtures`

```rust
pub const REQUIRED_FILES: [&str; 3] = ["source.nc", "config.toml", "NOTES.md"];
pub const EXPECTED_FILE: &str = "expected.json";

pub struct Fixture {
    pub dir: PathBuf,
    pub source: String,
    pub config_text: String,
    pub expected_text: Option<String>,
}
impl Fixture {
    pub fn load(dir: &Path) -> Result<Self, FixtureError>;
    pub fn run(&self) -> Result<Report, FixtureError>;
}
pub fn run_fixture(dir: &Path) -> Result<Report, FixtureError>;
pub fn golden_text(report: &Report) -> String; // canonical pretty JSON + '\n'
pub fn compare_reports(expected: &Report, actual: &Report) -> Result<(), FixtureDiff>;

pub struct FixtureDiff {
    pub pointer: String,          // RFC 6901, first difference
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub context: Option<String>,
}
```

`expected.json` is optional at load time so unreviewed fixtures can exist; the
golden test compares only fixtures that have one. Golden files are never
auto-blessed.



## Deliberate naming refinements

| Spec name | Contract name | Reason |
|---|---|---|
| `CoolantState` | `CoolantMode` (aliased as `CoolantState`) | The parser already owns `CoolantMode`; an alias avoids duplicate state. |
| `CannedCycle` | `CannedCycleKind` (aliased as `CannedCycle`) | Matches the existing parser state machine. |
| `CutterComp`, `ArcFormat` | new enums in `nc-gcode::motion` | The parser did not model compensation or arc format before. |
| `ToolHint` in `nc-gcode` | defined in `nc-core`, re-exported by `nc-gcode` | `ToolTable::resolve` lives in `nc-core`; the reverse dependency would be a cycle. |
| `MotionSpan` fields | adds poses, tool, feed, spindle | The simulator needs them; listing them avoids a second lookup table. |

## Implementation status

| Area | Status |
|---|---|
| `nc-core` domain types and math | complete |
| `nc-viewer` `MoveKind`, display stream, render plans | complete; `SegmentKind` removed |
| `nc-gcode` motion records, lossy mode, tool hints, limits, real-file parse | complete |
| `nc-stock` inference, heightmap, cutter shapes, sweep | complete |
| `nc-sim` timeline, tick engine, seek, stats | complete |
| `nc-analysis` engine, coverage, canonical JSON, config loader | complete |
| `nc-analysis` rules | complete catalog: 34 M1 + 11 M2 = 45 rules |
| `nc-app` VIEW shell | complete |
| `nc-app` SIMULATE mode | complete |
| `nc-app` diagnostics/coverage UI | complete |
| `nc-app` setup sheet, config loading, export | complete: TOML load, stock/tools/material/machine/sim/rules, SHA-keyed tool persistence, JSON/Markdown export; lossy partial programs are viewable |
| `nc-cli` commands and `nc-fixtures` harness | complete; 13 reviewed goldens |
| Report goldens | reviewed and committed for the synthetic corpus |
