> **Project addendum (2026-11-08, user-approved).** The "no commits, pushes,
> tags, or PRs unless the user asks" constraint is superseded for this
> repository: the user has explicitly authorized committing and pushing to
> `https://github.com/LeandroMarceddu/nca-rs`. The orchestrator commits each
> accepted subagent milestone separately; subagents still do not commit.
> Everything below is the original specification text, preserved verbatim.

## Mission

Build a **read-only NC viewer, deterministic preflight analyzer, and stock
simulator** for 3-axis CNC milling G-code.

The user opens a `.nc` / `.gcode` / `.tap` / `.ngc` / `.cnc` file. The app:

1. Parses it, including errors, without a controller.
2. Renders the toolpath in 2D top view and 3D orbit view.
3. Shows a rich, color-coded move taxonomy, not just rapid/feed/plunge.
4. Runs explicit, reproducible rules over the program and geometry.
5. Shows every finding with its source line, affected segment(s), severity,
   evidence, and a plain-language explanation.
6. Shows which rules were **not evaluated** and why, so "no warnings" is never
   mistaken for "safe".
7. Can **simulate** the tool moving along the path and cutting the stock, with
   the stock inferred from the toolpath when the user has not set it.
8. Exports a machine-readable report.

It never connects to a machine, never sends motion, never edits the G-code, and
never claims a program is safe.

## Product principles

1. **Rendering first.** The viewer and simulator are the product. The rest of
   the app is a compact shell around them.
2. **Deterministic.** Same input + same config + same ruleset version =
   byte-identical report. Same input + same simulated tick = the same stock
   state, regardless of frame rate. The exact scope of this guarantee is in
   `## Determinism contract`.
3. **Explainable.** Every finding has a stable rule ID, severity, basis
   (`exact` or `model`), source line, and evidence values.
4. **Coverage honesty.** A rule that cannot run is reported as skipped with a
   reason. The UI shows evaluated and skipped counts. Never show a green
   "SAFE" badge, a percentage, or a progress bar for coverage.
5. **Advisory, not authoritative.** The tool catches G-code programming
   mistakes. It cannot predict chatter, cutter force, material defects,
   workholding slip, or fixture collisions without fixture geometry.
6. **Local only.** No telemetry, no network calls, no cloud.
7. **3-axis mill only.** No lathe, plasma, drag knife, 4th axis, or ATC UI.
8. **Zero-config first.** Real-world NC files rarely say anything about tool
   sizes, stock, or the machine; at best a comment near the top names a tool.
   A bare file must still open, render, simulate, and run every rule that needs
   no setup. Tool, stock, machine, and material data are optional inputs, from
   the user or from file comments, always labeled with their source, and they
   only *unlock more rules*. Never invent a value to make a rule run: skip the
   rule and say what would enable it.

## Non-goals for v1

- No serial port, controller connection, streaming, jogging, or DRO.
- No G-code editing, post-processing, or auto-fix that writes a file.
- No full physics simulation (cutter force, deflection, chatter, thermal).
- No full 3D voxel/octree material removal; a 2.5D z-map is enough.
- No fixture/clamp collision unless the user supplies fixture geometry.
- No subprogram/flow-control execution (`M98`, macros, `IF/WHILE/GOTO`).
- No G41/G42 true offset geometry in v1.
- No network, accounts, telemetry, or cloud.

## Repository shape

Create a Cargo workspace at the path above. Working title `nca-rs`; the product
name is a placeholder.

```
nca-rs/
  Cargo.toml
  rust-toolchain.toml
  AGENTS.md
  LICENSE-MIT
  LICENSE-APACHE
  NOTICE
  PROVENANCE.md
  docs/
    SPEC.md              # this document's spec sections, verbatim
    ARCHITECTURE.md
    INTERFACES.md
    DECISIONS.md
    PROGRESS.md
    RULES.md
    DIALECTS.md
    DETERMINISM.md
    SIMULATION.md
    REUSE-MAP.md
    FIXTURES.md
    PERFORMANCE.md
    screenshots.md
  crates/
    nc-core/       # minimal domain types copied from ghs-core
    nc-viewer/     # copy of ghs-viewer + richer MoveKind/styles
    nc-gcode/      # fork of ghs-gcode + lossy mode + MotionRecord
    nc-stock/      # NEW: stock model, z-map, cutter shapes, sweep
    nc-sim/        # NEW: deterministic playback engine
    nc-analysis/   # NEW: rule engine, models, report
    nc-app/        # NEW: GPUI desktop app (library + `nca-gui` binary)
    nc-cli/        # NEW: headless checker (`nca` binary)
    nc-fixtures/   # fixture loader + golden-report test harness
  fixtures/
    README.md
    synthetic/<name>/{source.nc,config.toml,expected.json,NOTES.md}
    real/<name>/{source.nc,config.toml,expected.json,NOTES.md}
```

Dependency direction (acyclic):

- `nc-core`: no internal deps. Owns the `math` module (see Determinism contract).
- `nc-viewer`: no deps; owns `MoveKind`, `Segment`, `Toolpath`, `Bounds3`,
  `StockBox` (display-only box).
- `nc-gcode`: `nc-core`, `nc-viewer`; owns `Program`, `ProgramLine`,
  `MotionRecord`, `ArcRecord`, `CannedRecord`, `ModalState`, parse errors.
  **Re-exports** `MoveKind`, `AnnotationKind`, `Bounds3` from `nc-viewer` so
  downstream crates need no direct `nc-viewer` dependency.
- `nc-stock`: `nc-core`, `nc-viewer`, `nc-gcode`; owns `StockModel`,
  `StockSource`, `Heightmap`, `Cutter`, and the one-way conversion
  `StockModel::to_stock_box() -> nc_viewer::StockBox`.
- `nc-sim`: `nc-core`, `nc-gcode`, `nc-stock`; owns `Simulation`, `SimSnapshot`,
  `Timeline`, `SimConfig`.
- `nc-analysis`: `nc-core`, `nc-gcode`, `nc-stock`; owns `AnalysisConfig`,
  `Diagnostic`, `Report`, `Rule`, `Coverage`.
- `nc-app`: all crates; owns dialogs, persistence, worker threads, rendering,
  and the playback clock. Builds the `nca-gui` binary.
- `nc-cli`: `nc-analysis` (+ `nc-sim` for `simulate`). It must **not** depend on
  GPUI or `nc-app`. `nca view <file>` spawns the sibling `nca-gui` executable
  via `std::process::Command` and reports a clear error if it is not found.

Put `MoveKind` in `nc-viewer` so that crate stays dependency-free; `nc-gcode`
already depends on `nc-viewer`.

Preserve the parent's MIT/Apache-2.0 licensing and `NOTICE`. Add a
`PROVENANCE.md` naming the source repo, commit, copied crates, and changes.
Keep the parent's code standards: edition 2024, Rust 1.98.1 (pin it in
`rust-toolchain.toml`), `thiserror` for library errors, no
`unwrap`/`expect`/`panic!` on input data in library code, `tracing` for
diagnostics, doc comments on public items, `cargo fmt`,
`cargo clippy -- -D warnings`, and tests. Add a `cargo-deny` (or equivalent)
license/advisory check to CI; permissive licenses only.

## Shared types to freeze before spawning implementers

**Rule:** `docs/INTERFACES.md` must define **every** type named in this
document. Phase 0 is not done while any type is undefined, any item is marked
TODO, or `cargo doc --workspace --no-deps` has warnings. The definitions below
are **seed definitions**: refine them in Phase 0 if you must, then freeze them
before Phase 1. Implementers code against `INTERFACES.md`.

### `nc-core`

```rust
pub enum ToolShape { Flat, Ball, Bull, Drill, VBit, Unknown }

pub enum Dialect { Generic, Grbl, LinuxCnc, Mach3, Fanuc, Makera } // default Generic

pub enum ToolField {
    Name, Shape, Diameter, Flutes, Stickout, FluteLength, TipAngle,
    CornerRadius, ShankDiameter, HolderDiameter, RightHand,
}
pub enum FieldSource { Config, Comment { line: usize } }  // Config = user entry or TOML

/// A sparse overlay, not a tool library. Most programs use tools that have no
/// entry at all, and most entries know only a number and maybe a diameter.
/// Every field except `number` is optional; unknown means "skip the rules that
/// need it", never "assume a default".
pub struct Tool {
    pub number: u32,
    pub name: Option<String>,
    pub shape: ToolShape,                 // Unknown when not known
    pub diameter_mm: Option<f64>,
    pub flutes: Option<u32>,
    pub stickout_mm: Option<f64>,         // tip to holder face
    pub flute_length_mm: Option<f64>,
    pub tip_angle_deg: Option<f64>,       // Drill, VBit
    pub corner_radius_mm: Option<f64>,    // Bull
    pub shank_diameter_mm: Option<f64>,   // None -> same as diameter
    pub holder_diameter_mm: Option<f64>,  // None -> holder checks skipped
    pub right_hand: Option<bool>,         // None -> handedness rules skipped
    pub sources: BTreeMap<ToolField, FieldSource>,  // provenance of every known field
}
pub struct ToolTable { /* sorted by number; may be empty */ }
// ToolTable::resolve(config: &ToolTable, hints: &[ToolHint]) -> ToolTable
// Per-field precedence: Config (user) > Comment hint > unknown.

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
    pub spindle_manual: bool,             // spindle is started by hand; no M3 expected
}
```

### `nc-viewer`

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
```

### `nc-gcode`

```rust
pub enum Plane { Xy, Zx, Yz }                 // G17, G18, G19
pub struct ModalId(pub u32);                  // index into Program::modal_table

pub struct ModalState {
    pub units: Option<Units>,                 // None until G20/G21
    pub distance: Option<DistanceMode>,       // None until G90/G91
    pub feed_mode: FeedMode,                  // PerMinute, InverseTime, PerRev
    pub wcs: Wcs,
    pub coolant: CoolantState,
    pub cutter_comp: CutterComp,              // Off, Left, Right
    pub g92_active: bool,
    pub retract_mode: RetractMode,            // G98 / G99
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
    pub arc: Option<Box<ArcRecord>>,          // boxed to keep the common record small
    pub canned: Option<Box<CannedRecord>>,
    pub modal: ModalId,                       // interned; never a per-record copy
}

/// Angles use the plane basis (first, second): Xy = (X, Y), Zx = (Z, X),
/// Yz = (Y, Z). Increasing angle in that basis is counter-clockwise viewed
/// from the plane's +normal axis, which is the G3 direction in every plane.
pub struct ArcRecord {
    pub center_mm: [f64; 3],        // machine coords; normal-axis value = start value
    pub radius_mm: f64,             // center -> start point, in plane
    pub end_radius_mm: f64,         // center -> end point, in plane (for radius-mismatch checks)
    pub start_angle_rad: f64,       // atan2(second, first) of start about center
    pub sweep_rad: f64,             // signed: > 0 CCW, < 0 CW; |sweep| in (0, 2*pi]; full circle = +/-2*pi
    pub axial_delta_mm: f64,        // signed travel along the plane normal; 0 = planar arc
    pub format: ArcFormat,          // Center (I/J/K) or Radius (R)
}

pub struct CannedRecord {
    pub cycle: CannedCycle,         // G73, G81, G82, G83, G85, G86, G89
    pub hole_index: usize,          // order within the cycle's run of holes
    pub r_plane_mm: f64,
    pub bottom_z_mm: f64,
    pub peck_q_mm: Option<f64>,
    pub dwell_s: Option<f64>,
    pub retract_mode: RetractMode,
}

pub struct Annotation {
    pub line: usize,
    pub next_motion_index: usize,   // index of the next MotionRecord
    pub pos_mm: [f64; 3],
    pub kind: AnnotationKind,
}

pub struct Comment { pub line: usize, pub text: String }

/// Best-effort tool data a post wrote into a comment. Never authoritative.
pub struct ToolHint {
    pub tool: u32,
    pub line: usize,                      // the comment's source line
    pub name: Option<String>,
    pub shape: Option<ToolShape>,
    pub diameter_mm: Option<f64>,         // only when the unit was unambiguous
    pub flutes: Option<u32>,
}
```

`Program` gains `motions: Vec<MotionRecord>`, `annotations: Vec<Annotation>`,
`comments: Vec<Comment>`, `tool_hints: Vec<ToolHint>`,
`modal_table: Vec<ModalState>` (interned), and:

- `cutting_bounds() -> Bounds3` (Feed/Plunge/Ramp/Arc/Helix/Drill/Peck/Bore only).
- `modal(&self, ModalId) -> &ModalState`.
- `expanded_motions(&self) -> impl Iterator<Item = MotionRecord>`: lazily expands
  each canned-cycle record (one per hole) into its rapid/feed/retract/peck
  sub-moves. The simulator and timeline consume this iterator; analysis rules
  that reason about holes use the unexpanded hole records.
- `geometry_uncertain_after() -> Option<usize>`.

A test asserts `size_of::<MotionRecord>() <= 128`.

### `nc-stock`

```rust
pub enum StockSource { Inferred, Explicit }
pub enum InferenceBasis { ZeroAtTop, AllCutsAboveZero, NoCuttingMoves }

pub struct StockModel {
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
    pub top_z_mm: f64,
    pub source: StockSource,
    pub inference: Option<InferenceBasis>,   // Some(..) only when source == Inferred
}
pub struct Heightmap { /* grid, cell size, f32 cells, dirty version */ }
// Heightmap::content_hash() -> u64 : FNV-1a over little-endian f32 bit patterns,
// used by determinism tests.
pub struct Cutter { /* shape, diameter, corner radius, tip angle, stickout, shank/holder */ }
```

### `nc-sim`

```rust
pub struct SimConfig {
    pub cell_mm: f64,
    pub max_cells: usize,
    pub rapid_rate_mm_min: f64,
    pub tool_change_pause_s: f64,
    pub crash_preview: bool,
    pub render_resolution: usize,
    pub max_ticks_per_advance: usize,   // bounds work per UI frame; excess stays queued
    pub assumed_tool_diameter_mm: Option<f64>,  // None = centerline trace, one cell wide
}
pub struct SimSnapshot {
    pub tick: u64,                      // exact simulated tick count
    pub tick_pose_mm: [f64; 3],         // exact pose at `tick` (used by tests)
    pub pose_mm: [f64; 3],              // display pose, interpolated between ticks
    pub tool: u32,
    pub spindle_on: bool,
    pub time_s: f64,
    pub current_line: usize,
    pub current_kind: Option<MoveKind>,
    pub stock_version: u64,
    pub progress: f64,
}
```

### `nc-analysis`

```rust
pub enum Severity { Error, Warning, Info }
pub enum Basis { Exact, Model }

pub struct RuleId(pub Cow<'static, str>);        // serializes as a string; deserializes owned

pub enum EvidenceValue { Int(i64), Float(f64), Text(String), Bool(bool), Point3([f64; 3]) }

pub struct Diagnostic {
    pub rule: RuleId,
    pub severity: Severity,
    pub basis: Basis,
    pub line: usize,
    pub column: usize,      // 0 = whole line
    pub line_end: usize,
    pub message: String,
    pub evidence: BTreeMap<String, EvidenceValue>,
}

pub struct SkippedRule { pub rule: RuleId, pub missing_input: String, pub reason: String }
// missing_input is a stable key, e.g. "tool.flutes:T3" or "stock.explicit".

pub struct Coverage {
    pub evaluated: usize,
    pub total: usize,
    pub skipped: Vec<SkippedRule>,   // sorted by rule id
}

/// What the report knew about each tool the program uses, and where it came from.
pub struct ToolInventoryEntry {
    pub tool: u32,
    pub first_line: usize,
    pub motion_count: usize,
    pub fields: BTreeMap<String, InventoryField>,   // field name -> value + source
}
pub struct InventoryField { pub value: EvidenceValue, pub source: String }
// source is "config" or "comment:L<line>"

pub struct InputInfo {
    pub file_name: String,           // file name only; never an absolute path (determinism)
    pub sha256: String,
    pub line_count: usize,
    pub dialect: Dialect,
    pub geometry_uncertain_after: Option<usize>,
}

pub struct Report {
    pub schema: String,              // "nca.report/1"
    pub ruleset_version: String,
    pub input: InputInfo,
    pub config_digest: String,
    pub stock_source: StockSource,
    pub tool_inventory: Vec<ToolInventoryEntry>,   // sorted by tool number
    pub diagnostics: Vec<Diagnostic>,
    pub truncated: BTreeMap<String, usize>,   // rule id -> findings dropped by caps
    pub coverage: Coverage,
}

pub struct AnalysisConfig {
    pub machine: Option<MachineConfig>,
    pub stock: Option<StockConfig>,     // None -> use inferred stock
    pub tools: ToolTable,
    pub material: Option<MaterialPreset>,
    pub tolerances: Tolerances,
    pub rules: RuleConfig,
}
pub struct StockConfig { pub origin_mm: [f64; 3], pub size_mm: [f64; 3] }
pub struct Tolerances {
    pub arc_radius_mm: f64,
    pub min_segment_mm: f64,
    pub hole_xy_mm: f64,
    pub clearance_mm: f64,
    // every default is documented in docs/RULES.md
}
pub struct RuleConfig(pub BTreeMap<RuleId, RuleSettings>);
pub struct RuleSettings {
    pub enabled: bool,
    pub severity: Option<Severity>,                 // override
    pub thresholds: BTreeMap<String, f64>,
}
pub struct AnalysisModel<'a> {
    pub program: &'a Program,
    pub stock: StockModel,
    pub tools: &'a ToolTable,
    pub machine: Option<&'a MachineConfig>,
    pub material: Option<&'a MaterialPreset>,
    pub tolerances: &'a Tolerances,
    pub engagement: OnceCell<EngagementData>,       // one shared z-map pass for all process.* rules
}
```

### Config and report sketches (for `docs/INTERFACES.md`)

TOML config (human-edited):

```toml
[parse]
dialect = "generic"            # generic | grbl | linuxcnc | mach3 | fanuc | makera

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
preset = "aluminum-6061"       # built-in name, or inline fields

[rules."clearance.rapid_at_depth"]
enabled = true
severity = "warning"
clearance_mm = 2.0
```

JSON report (excerpt; numbers quantized as in `## Analysis model`):

```json
{
  "schema": "nca.report/1",
  "ruleset_version": "1.0.0",
  "input": { "file_name": "part.nc", "sha256": "…", "line_count": 120,
             "dialect": "generic", "geometry_uncertain_after": null },
  "stock_source": "inferred",
  "tool_inventory": [
    { "tool": 1, "first_line": 8, "motion_count": 412,
      "fields": { "diameter_mm": { "value": 6.0, "source": "comment:L4" } } },
    { "tool": 2, "first_line": 530, "motion_count": 88, "fields": {} }
  ],
  "diagnostics": [
    { "rule": "clearance.rapid_at_depth", "severity": "warning", "basis": "model",
      "line": 42, "column": 0, "line_end": 42,
      "message": "Rapid move below stock top + clearance.",
      "evidence": { "stock_source": "inferred", "end_mm": [10.0, 5.0, -1.0] } }
  ],
  "coverage": { "evaluated": 31, "total": 44,
                "skipped": [ { "rule": "process.chipload",
                               "missing_input": "tool.flutes:T3",
                               "reason": "tool T3 has no flute count" } ] }
}
```

## Determinism contract

Document this in `docs/DETERMINISM.md` and enforce it in tests.

- **Scope.** Reports and heightmap hashes are byte-identical for the same input,
  config, and ruleset version **on the same OS/architecture**. Cross-platform
  identity is a goal: CI compares golden reports on Linux, macOS, and Windows,
  and any boundary-case difference is recorded in `DECISIONS.md` rather than
  hidden.
- **Math.** All trig (`sin`, `cos`, `atan2`, …) goes through `nc-core::math`,
  which wraps the `libm` crate, so results do not depend on the platform's libm.
  Library code does not use `f32` arithmetic (heightmap cells are *stored* as
  `f32`; computation is `f64`; the rounding on store is deterministic). No
  fused-multiply-add tricks, no platform intrinsics.
- **Ordering.** No `HashMap`/`HashSet` iteration anywhere that affects output.
  Parallel code (if any) uses fixed chunking and an ordered combine.
- **No ambient inputs.** No wall clock, randomness, locale, or absolute paths in
  reports. `InputInfo` carries the file name only.
- **Simulator ticks.** See `## Simulator`.

## App design: reuse the ghs-rs language, trim the sender

The app should feel like `ghs-rs`: the same dark shop-floor visual language,
typography, spacing, chips, cards, touch targets, and three-column layout. It
should be **much smaller** and centered on rendering and simulation.

Create `docs/REUSE-MAP.md` with this table filled in and verified (the Explorer
adds file/symbol/line-range evidence for each row):

| Parent asset | Decision | Notes |
|---|---|---|
| `ghs-app/src/theme.rs` | Copy + adapt | Keep tokens; replace `ghs_config::ThemeMode` with a local enum; add move/cut colors |
| `ghs-app/src/layout.rs` | Copy + simplify | Keep wide/compact/portrait breakpoints; remove six-workspace assumptions |
| `ghs-app/src/viewer.rs` | Copy + refactor | Keep cameras, plan cache, gestures, stock box, marker; replace DRO marker with sim pose; expand styles; add stock mesh |
| `ghs-app/src/shell.rs` | Selective copy | Copy generic `card`, `chip`, `section_label`, `overlay_layer`, `chip_button`, list-row patterns; do not copy the 26k-line `Shell` struct or sender panels |
| `ghs-app/src/file_browser.rs` | Adapt | Keep recent-file/list patterns; simplify filters |
| Program list (inside `shell.rs`) | Adapt | Virtualized rows, search/filter, current line, severity gutter |
| `ghs-app/src/state.rs` | Do not copy | 11k-line sender state; build a small `NcaState` |
| `ghs-app/src/live.rs`, `session.rs`, `connection.rs` | Do not copy | Controller/session code |
| `ghs-app/src/probe.rs`, `settings.rs`, `offsets.rs`, `tools.rs`, `utilities.rs` | Do not copy | Sender features; extract `ProgramCheck` ideas into rules; build the small sparse tool overlay (see `Tool`), not a tool library |
| `ghs-app/src/i18n.rs` | Defer | English-only v1; centralize strings so i18n can be added later |
| `ghs-app/src/config.rs` | Do not copy | Build `NcaConfig` for stock/tools/rules/UI |
| `ghs-gcode` | Copy + extend | Parser foundation; lossy mode + motion records |
| `ghs-viewer` | Copy | Dependency-free geometry/render |
| `ghs-core` | Copy minimal | Position, units, WCS, tool types; drop controller settings |

### Keep

- The dark/light/high-contrast theme and its tokens.
- The 48 px touch targets, 11 px labels, 14-16 px body, large numeric readouts.
- The top bar, status chips, cards, sheets, chips, and virtualized list style.
- The 2D/3D canvas, camera fit, pan/zoom/orbit, plan caching, stock box, and
  tool marker.
- The program list with search/filter and current-line highlight.
- The responsive wide/compact/portrait layout.

### Trim or remove

- Connection panel, baud/port, controller discovery, DRO, jog, MDI, console,
  overrides, spindle/coolant, signals, alarms, reset, job controls.
- Probing, height maps, settings editor, offsets, macros, startup lines,
  keymap, pins, utilities, tool-change flow, session recording, job history.
- The six-workspace top bar. Replace it with `VIEW` / `SIMULATE`, file actions,
  view controls, analyze, export, and settings.

### Add

- `VIEW` / `SIMULATE` modes using the same canvas.
- Transport bar: play/pause, stop, step, speed, scrub, time, current line.
- Move-type legend and per-kind visibility toggles.
- Diagnostics panel with severity/rule filters, evidence, and click-to-zoom.
- Coverage panel with evaluated/skipped rules and reasons.
- Setup sheet: stock, tools, material, machine, simulation, rules.
- Export report.

The shell should be roughly one main workspace, two modes, and a handful of
sheets, not fourteen workspaces. The viewer must dominate the window. Aim for
roughly 10-15k lines of new/adapted app code total, not 70k.

## Move taxonomy and color coding

The current `ghs-viewer` has five `SegmentKind` variants and the app legend only
shows Cut, Rapid, and Plunge. Expand this into a semantic `MoveKind` shared by
the parser, viewer, simulator, and analysis.

Required motion kinds:

| Kind | Meaning | Default style |
|---|---|---|
| `Rapid` | G0 travel | cool gray, dashed 5/4 |
| `Retract` | vertical up, no cut | light gray, dashed 2/3 |
| `Feed` | G1 XY/3D cut | teal, solid 1.6 |
| `Plunge` | vertical down cut | amber, solid 1.9 |
| `Ramp` | G1 with XY + Z down | gold, solid 1.8 |
| `ArcCw` | G2 cut | blue, solid 1.6 |
| `ArcCcw` | G3 cut | violet, solid 1.6 |
| `HelixCw` | G2 with Z change | magenta, solid 1.8 |
| `HelixCcw` | G3 with Z change | purple, solid 1.8 |
| `Drill` | G81/G82 drill | cyan, solid 2.0 + ring marker |
| `Peck` | G73/G83 peck | sky, solid 2.0 + dashed ring |
| `Bore` | G85/G86/G89 | navy, solid 2.0 + double ring |
| `Probe` | G38.x | yellow, dotted 2/3 |

Annotations, not segments:

| Annotation | Meaning | Style |
|---|---|---|
| `Dwell` | G4 | clock icon at the position |
| `ToolChange` | M6 | tool icon at the position |
| `Spindle` | M3/M4/M5 | small state marker, optional |
| `Coolant` | M7/M8/M9 | small state marker, optional |

Render-time overlays, not separate kinds:

- `Rapid at depth`: danger red, dashed, with a glow; toggleable.
- `Air cut`: feed above the inferred stock top, teal at 35% alpha or dotted.
- `Warning segment`: red outline/glow from a diagnostic.
- `Current segment`: thicker/brighter while simulating.
- `Future path`: dimmed ghost in simulate mode; traversed path solid.

Rules:

- Color is never the only signal. Every kind also has a stroke style and a
  legend label.
- Group the legend by family (Travel, Cutting, Arcs, Holes, Special) and allow
  per-kind visibility toggles. Collapse the legend on compact/portrait.
- Every `MoveKind` has a stable label, color token, stroke, and legend entry.
  Add a test that iterates all kinds and asserts a non-empty style.
- Use the theme's dark/light/high-contrast palettes; red is reserved for
  errors/unsafe motion, not ordinary cutting.
- The parser must tag kinds semantically, not guess in the renderer:
  - G0 Z up -> `Retract`; G0 otherwise -> `Rapid`.
  - G1 Z-only down -> `Plunge`; G1 XY+Z down (both > 1e-3 mm) -> `Ramp`;
    G1 Z-only up -> `Retract`; other G1 -> `Feed`.
  - G2/G3 with Z change -> `HelixCw`/`HelixCcw`; otherwise `ArcCw`/`ArcCcw`.
    (For G18/G19 "Z change" means travel along the plane's normal axis, i.e.
    `axial_delta_mm != 0`.)
  - G81/G82 -> `Drill`; G73/G83 -> `Peck`; G85/G86/G89 -> `Bore`.
  - G38.x -> `Probe`.
  - G4 -> `Dwell` annotation; M6 -> `ToolChange` annotation.

## Required parser work

The current parser is all-or-nothing: `parse_program_with` returns `Err` if any
line fails. A viewer needs a lossy mode. Add:

1. **`parse_program_lossy(text, config) -> LossyProgram`**
   - Returns the parsed `Program` (or a partial one), `Vec<ParseError>`, a list
     of skipped blocks, and `trusted_geometry_until: Option<usize>`.
   - On a failed block: record it, skip the block, keep the previous modal
     state, continue.
   - If a state-changing unsupported construct appears (`G41/G42`, `M98`,
     macros, flow control), stop generating new geometry after that line and
     mark the rest `geometry_uncertain`. Source-level exact rules may still run.
     This prevents a clean-looking toolpath after an unknown modal change.
2. **`MotionRecord` stream** (see the shared types above)
   - One record per real motion, in machine coordinates, `f64`.
   - Canned cycles: one record per hole carrying a `CannedRecord`; sub-moves
     are produced lazily by `Program::expanded_motions()`. Do not store the
     expansion.
   - Modal state is interned into `Program::modal_table`; records hold a
     `ModalId`.
   - Keep the display `Toolpath`; `MotionRecord` is the analysis/simulation
     source of truth.
3. **Widen tool IDs**
   - The viewer uses `u16` and the parser rejects tool numbers above
     `u16::MAX`. Use `u32` in `MotionRecord` and analysis; map to a display
     index if needed.
4. **Permissive dialect policy**
   - Unknown words/codes are diagnostics, not fatal.
   - `ParseConfig::dialect` selects dialect semantics (default `Generic`; also
     `Grbl`, `LinuxCnc`, `Mach3`, `Fanuc`, and `Makera`). An omitted dialect
     emits an Info `parse.dialect_assumed` once per file. `docs/DIALECTS.md`
     lists, per dialect: the unit of `G4 P` (seconds in Grbl/LinuxCNC/Mach3/
     Generic/Makera; Fanuc-style milliseconds), canned-cycle retract behavior
     (G98/G99), accepted arc formats, and the meaning of G28/G30/G53. Verify
     each row against vendor documentation and mark unverified rows as such.
   - `G41/G42`: v1 shows the programmed centerline, does not apply cutter
     compensation. Emit a warning and skip tool-radius-dependent geometry
     rules while active. Do not pretend the offset was applied.
   - `G18/G19` arcs: support them in `MotionRecord`.
   - Helical arcs: preserve the helix in `ArcRecord`; do not reduce to chords
     for analysis.
   - `G93` inverse-time feed: flag; feed-based rules skip it.
5. **`Program::cutting_bounds()`**
   - Feed/Plunge/Ramp/Arc/Helix/Drill/Peck/Bore only. Keep `Program::bounds` for
     display and fallback.
6. **Input limits (no OOM, no panic)**
   - Configurable maximum file size and maximum line length; exceeding either
     yields a diagnostic and a clean refusal or truncation notice, never an
     allocation blow-up.
   - Property tests: arbitrary bytes never panic; every diagnostic line number
     is in range.
7. **Tool hints from comments (tool data is usually unknown)**
   - Keep every comment (`Program::comments`) with its line number.
   - Extract `ToolHint`s only from (a) header comments before the first motion
     and (b) comments within N lines (configurable, default 3) of a `T`/`M6`
     block.
   - Recognizers are conservative and fixture-driven: start with a small
     generic set (diameter tokens such as `D=6`, `Ø6`, `6mm`, `1/4"`,
     `0.125 in`) and add post-specific recognizers **only against real files**.
     Document each in `docs/DIALECTS.md` under "Tool comment conventions".
   - A diameter counts only if the comment carries a unit or the program has
     already declared G20/G21. Otherwise drop the hint and emit an Info
     `parse.tool_hint_ambiguous`. Never guess.
   - Never infer tool size from toolpath geometry. That is out of scope.
   - Hints are advisory: `ToolTable::resolve` merges them under user-entered
     values, and anything that used one is labeled with its comment line.

## Stock model and inference

`nc-stock` owns:

- `StockSource::Inferred | Explicit`.
- `StockModel` (min/max/top/source/inference basis).
- A 2.5D heightmap grid with cell size, max cell cap, dirty version, `f32`
  cells, and `content_hash()`.
- Cutter shapes: flat endmill (cylinder), ball nose (sphere), bull nose
  (cylinder + torus), drill (cone tip + cylinder), V-bit (cone). Unknown shape
  falls back to a flat cylinder with a clear label.
- A deterministic `cut_segment`/`sweep_capsule` operation that lowers cells.

Inference (reuse the parent's idea, with one correction):

- Use the **cutting** bounds, not all path bounds, so a safe-Z rapid does not
  inflate the stock.
- XY margin = `max(2% of the larger cutting XY extent, 1 mm)`.
- If any cutting Z is below 0, top Z = 0 (`InferenceBasis::ZeroAtTop`); otherwise
  top Z = max cutting Z + 2 mm (`AllCutsAboveZero`).
- Bottom = min cutting Z - 2 mm.
- An entirely above-zero flat cut gets a 10 mm block beneath the top.
- If there are no cutting moves, fall back to all path bounds
  (`NoCuttingMoves`).
- Inference **assumes Z0 is the stock top** (or that cuts go below it). Programs
  with Z0 at the table will infer a thin slab. Surface `InferenceBasis` in the
  `STOCK: INFERRED` chip tooltip, in the report, and in stock-dependent
  evidence, and document the limitation in `SIMULATION.md`.
- Show a `STOCK: INFERRED` chip with an edit action. An explicit stock overrides
  inference and is labeled `STOCK: SET`.
- In the report, include `stock_source` in evidence for stock-dependent rules.
  Inferred stock is `Basis::Model`, not `Exact`.
- Because inferred stock is fitted to the cuts, some rules are meaningless
  against it. Rules that need a real stock declare `stock: explicit` (see the
  catalog) and are **skipped with a reason** when the stock is inferred.

## Simulator

`nc-sim` is a pure, deterministic playback engine. The app owns the wall clock
and passes time steps.

### Timeline

Precompute a `Timeline` from `Program::expanded_motions()`:

- One `MotionSpan` per motion: `start_time_s`, `end_time_s`,
  `start_distance_mm`, `end_distance_mm`, source line, kind.
- Feed moves: `length / feed_mm_min * 60`; arcs/helixes use exact arc length
  (`radius * |sweep|` combined with `axial_delta_mm`), not chord sums.
- Rapids: `length / rapid_rate_mm_min * 60`; `rapid_rate_mm_min` is a config
  value with a documented default.
- Dwell: G4 P converted per dialect.
- Tool change: a configurable pause (default 2 s) or zero; show a marker.
- Prefix sums for binary search. A `line_to_time` map for jump-to-diagnostic.
- Memory: the timeline stores spans, not copies of motions.

### Simulation state

- `tool_pose` (X, Y, Z), `tool_number`, `spindle_on`, `elapsed_s`,
  `path_distance`, `current_line`, `current_kind`.
- `heightmap` (from `nc-stock`), `heightmap_version`, dirty cells.
- Stats for the UI: cut volume estimate, max depth, air time, rapid time.

### API

```rust
pub struct Simulation { /* ... */ }

impl Simulation {
    pub fn new(
        program: &Program,
        stock: StockModel,
        tools: &ToolTable,
        cfg: &SimConfig,
    ) -> Result<Self, SimError>;

    pub fn advance(&mut self, dt_s: f64);
    pub fn seek_to_time(&mut self, time_s: f64);
    pub fn seek_to_line(&mut self, line: usize);
    pub fn step_motion(&mut self);
    pub fn step_line(&mut self);
    pub fn reset(&mut self);
    pub fn snapshot(&self) -> SimSnapshot;
    pub fn heightmap(&self) -> &Heightmap;
}
```

### Tick contract (this is what makes frame-rate independence true)

- The internal tick is exactly 1/120 s of **simulated** time.
- `advance(dt)` adds `dt` to an accumulator and runs `floor(acc / tick)` ticks,
  at most `max_ticks_per_advance` per call. The remainder (and any queued ticks)
  stays in the accumulator; time is never dropped or rounded away.
- The heightmap, `stock_version`, stats, and `tick_pose_mm` change **only on
  ticks** and are a pure function of the tick count. `seek_to_time(t)` seeks to
  tick `floor(t / tick)`.
- `SimSnapshot::pose_mm` is interpolated between the last two tick poses for
  smooth display. It is not part of determinism tests; `tick_pose_mm` is.
- Between consecutive tick poses the cutter is applied as a **swept capsule**
  (not point stamps), so there are no gaps at high feed (at 50 mm/s one tick
  moves about 0.4 mm, roughly one cell). Arcs/helixes are subdivided along the
  exact curve so chord error is at most `cell_mm / 4`.
- `seek_to_time` forward is incremental. Backward resets and replays; for long
  jumps, fast-forward without rendering. If replay takes more than ~100 ms,
  show a progress indicator. Do not block the UI thread: long seeks run on the
  worker thread and are cancellable.

### Cutting rules

- Feed/Plunge/Ramp/Arc/Helix/Drill/Peck/Bore cut.
- Rapid/Retract/Probe/Dwell/ToolChange do not cut by default.
- Optional "crash preview" toggle: rapids below the stock top cut (show the
  gouge) and are highlighted red. Default off.
- Apply the cutter at the current tool position as the tool advances, so the
  cut follows the tool. Do not pre-cut the whole segment at once.
- For arcs/helixes, step along the exact arc/helix; do not use the display
  chords.
- For drills, lower the hole to the drill tip depth; honor peck retracts.

### Grid and tool

- **Grid:** default cell 0.5-1.0 mm; cap total cells (e.g., 4M). If the
  inferred stock is too large, coarsen the cell size and record it. If the cell
  size would exceed a maximum (e.g., 2 mm), simulate but label it "coarse."
- **Unknown tools are the normal case.** A bare NC file has no tool sizes. Use
  the resolved tool table (user entry, else comment hint). When a diameter is
  unknown, cut with `SimConfig::assumed_tool_diameter_mm`; by default (`None`)
  that is a **centerline trace one cell wide**, which shows where the tool
  went without pretending to know how wide it is. Unknown shape is a flat
  cylinder. Never use a zero-diameter tool, and never present an assumed size
  as real:
  - show a `TOOL SIZE ASSUMED` badge in the sim HUD while an assumed tool is
    active, and label the 3D tool "Ø?";
  - flag cut-volume and other size-dependent stats as "assumed";
  - drill holes with unknown diameter lower one cell (or the assumed
    diameter), and the hole marker is drawn at that assumed size and labeled.
  - The user fixes this in two taps via the tool chip (see UI requirements).
- **Determinism tests:** simulate to tick T with different `dt` sequences and
  compare `Heightmap::content_hash()`; seek backward vs fresh replay; arc/helix
  cuts; drill holes; stock inference; crash preview; `max_ticks_per_advance`
  splitting does not change the result.

### Rendering the cut stock

- 3D: render the stock as a heightfield mesh over the inferred box. Use a
  separate render grid (e.g., 64x64 while playing, 128x128 when paused) sampled
  from the simulation heightmap. Draw back-to-front quads with a neutral
  blue-gray depth ramp. Red is reserved for warnings. Rebuild only when the
  heightmap version changes; update at 15-30 Hz while playing, full resolution
  when paused. If 64x64 is too slow in GPUI, drop to 48x48 or render the top as
  a 2D depth overlay in 3D; measure and record.
- 2D: shade cut cells by depth over the stock footprint; uncut stock is a light
  panel. This is the cheap, always-smooth fallback.
- Tool: 3D cylinder/holder (diameter from the tool table) at the current pose;
  2D tool circle + crosshair. Show the cutting edge at the path Z and the shank
  above it. If the tool shape is unknown, draw a thin cylinder and label it.
- Path: traversed path solid; future path dimmed ghost; current segment
  highlighted. In `VIEW` mode, show the full path normally.
- Legend: move kinds + cut depth. Toggles for stock, rapids, air cuts, future
  path, tool, warnings.

## Analysis model

`AnalysisConfig`, `AnalysisModel`, and the report types are defined in
`## Shared types`. The engine builds one `AnalysisModel` per run and shares it
across rules; the z-map engagement pass (`EngagementData`) is computed lazily,
once, and only if a `process.*` rule needs it.

Report rules:

- Use `BTreeMap`/sorted `Vec`, never `HashMap` iteration, for report order.
- Quantize reported numbers: coordinates 0.001 mm, feeds 0.1 mm/min, spindle
  1 rpm, angles 0.01°, percentages 0.1%. Keep full `f64` internally. Serialize
  quantized values with a fixed decimal format so bytes do not depend on float
  printing.
- Sort diagnostics by `(severity_rank, line, column, rule_id)`.
- Deduplicate identical findings. Cap per rule (e.g. 100) and total (e.g.
  1000); record dropped counts in `Report::truncated`.
- `Coverage` lists every skipped rule with a stable `missing_input` key and a
  human reason, e.g. `"process.chipload": "tool T3 has no flute count"`.
- Include the config snapshot or a stable digest (`config_digest`, SHA-256 over
  the canonical serialized config).
- Include `geometry_uncertain_after` when the lossy parser stopped geometry.
- Include only the input file **name**, never its absolute path.
- Add a determinism test: run twice, compare serialized JSON bytes.
- Use `sha2` for hashes; no network.

## Rule contract

```rust
pub trait Rule {
    fn id(&self) -> RuleId;
    fn default_severity(&self) -> Severity;
    fn basis(&self) -> Basis;
    fn requirements(&self) -> Requirements;
    fn evaluate(&self, model: &AnalysisModel, cfg: &RuleSettings) -> RuleOutcome;
}

pub struct Requirements {
    pub tool_fields: &'static [ToolField],   // e.g. Diameter, Flutes, HolderDiameter, RightHand
    pub machine: bool,
    pub material: bool,
    pub stock: StockRequirement,             // None | InferredOk | Explicit
}
```

- `RuleOutcome` is `Findings(Vec<Diagnostic>)` or `Skipped { missing_input, reason }`.
- The engine checks `requirements()` against the config **before** calling
  `evaluate` and records a uniform `SkippedRule` when they are unmet.
  `evaluate` may also return `Skipped` for data-dependent reasons (cutter
  compensation active, geometry uncertain at that line, inverse-time feed).
- `nca rules` and the Coverage panel are generated from `requirements()`, so
  they can never disagree with behavior.
- Never panic on malformed geometry. Every rule has a stable ID, documented
  threshold, positive and near-miss fixtures, and a `Basis`.
- Each rule module exposes `pub fn rules() -> Vec<Box<dyn Rule>>`; the
  orchestrator owns `registry.rs`, which aggregates the modules.

## V1 rule catalog

Start with the M1 set, then M2. Default severity is shown; it is configurable.
Rules that need missing data are skipped with a reason.

**Stock column:** `—` = stock not used; `inferred ok` = runs on inferred stock
but reports `stock_source` and `Basis::Model`; `explicit` = **skipped when the
stock is inferred**, because inference is fitted to the cuts and the result
would be meaningless.

### M1 — exact program, modal, travel, and clearance

| ID | Severity | Stock | Catches |
|---|---|---|---|
| `parse.unsupported` | Error/Info | — | Unparseable/unsupported block; raw line |
| `parse.dialect_assumed` | Info | — | No dialect configured; `Generic` semantics used |
| `parse.tool_hint_ambiguous` | Info | — | Tool comment with a dimension whose unit could not be established; hint dropped |
| `modal.units_missing` | Warning | — | Motion before G20/G21 |
| `modal.distance_missing` | Warning | — | Motion before G90/G91 |
| `modal.feed_missing` | Error | — | First cutting move with no F |
| `modal.feed_nonpositive` | Error | — | F0 or negative feed |
| `modal.spindle_stopped` | Error | — | Cutting with spindle stopped (skipped when `machine.spindle_manual`) |
| `modal.spindle_speed_zero` | Warning | — | M3/M4 with S0 or no S (skipped when `machine.spindle_manual`) |
| `modal.spindle_direction` | Info | — | M4 with a right-hand tool (needs `tool.right_hand`) |
| `modal.tool_unknown` | Warning | — | T not in the tool table. Evaluated only when a tool table or tool hints exist; otherwise skipped ("no tool data"), because on a bare file it would fire for every tool |
| `modal.tool_zero` | Warning | — | Cutting with T0 |
| `modal.tool_change_mismatch` | Warning | — | T without M6 or M6 without T |
| `modal.machine_coords` | Warning | — | G53/G28/G30 or machine-coordinate targets |
| `modal.probe_in_job` | Warning | — | G38.x probe in a normal job |
| `modal.g92_unrestored` | Warning | — | G92 active at program end |
| `modal.wcs_change` | Warning | — | WCS change after cutting starts |
| `modal.no_program_end` | Warning | — | No M2/M30 |
| `modal.end_state` | Warning | inferred ok | Spindle/coolant on or tool below safe Z at end (safe Z = stock top + clearance) |
| `modal.block_delete_unsafe` | Warning | — | `/` on a retract/spindle/tool-change line |
| `travel.machine_envelope` | Error | — | Motion outside configured machine travel |
| `travel.z_below_table` | Error | — | Z below the configured table/bed |
| `travel.stock_bounds` | Warning | explicit | Cutting outside stock XY |
| `clearance.rapid_at_depth` | Warning | inferred ok | G0 below stock_top + clearance |
| `clearance.rapid_through_stock` | Warning | inferred ok | XY rapid inside the stock |
| `clearance.retract_below_stock` | Warning | inferred ok | Retract plane below stock top |
| `clearance.tool_end_in_material` | Warning | inferred ok | Program ends with tool below stock top |
| `geometry.duplicate_hole` | Warning | — | Same tool + same XY + overlapping Z (double drilling). Needs no tool size; runs on any file |
| `geometry.overlapping_holes` | Warning | — | Hole centers closer than tool diameter (needs `tool.diameter`) |
| `geometry.tiny_segment` | Info | — | Cutting segment below minimum length |
| `geometry.arc_radius_mismatch` | Error | — | `radius_mm` vs `end_radius_mm` beyond tolerance |
| `geometry.arc_smaller_than_tool` | Warning | — | Internal arc radius smaller than tool radius (needs `tool.diameter`; skipped while cutter comp active) |
| `geometry.feed_out_of_range` | Warning | — | Feed above machine/tool maximum |
| `geometry.spindle_out_of_range` | Warning | — | Spindle above machine/tool maximum |

### M2 — model-based process rules (need stock/tool/material data)

| ID | Severity | Stock | Catches |
|---|---|---|---|
| `process.chipload` | Warning | — | Feed / (RPM x flutes) outside material range |
| `process.surface_speed` | Warning | — | RPM x pi x D outside material range |
| `process.axial_depth` | Warning | inferred ok | Axial DOC above the limit |
| `process.radial_engagement` | Warning | inferred ok | Contact angle/radial DOC above the limit (z-map) |
| `process.mrr` | Warning | inferred ok | Material removal rate above the limit |
| `process.flute_length` | Warning | inferred ok | Axial depth greater than flute length |
| `process.stickout_ratio` | Warning | — | Stickout / diameter above the limit |
| `process.plunge_feed` | Warning | — | Plunge feed above the configured ratio |
| `process.shank_collision` | Warning | inferred ok | Shank (above `flute_length`) or holder (above `stickout`, needs `holder_diameter`) enters the stock |
| `process.thin_wall` | Warning | explicit | Standing wall narrower than the minimum in the final-state z-map |
| `process.rest_material` | Info | explicit | Stock left standing inside the cutting footprint higher than final pass depth + threshold in the final-state z-map |

`thin_wall` and `rest_material` run only after the full-program z-map pass.
Document their exact definitions and thresholds in `docs/RULES.md` before
implementing.

**Tool data is usually missing.** On a bare file, most `process.*` rules and
the tool-size rules above will be skipped. That is correct, and it must be
visible: each skip names its missing input (e.g. `tool.diameter:T3`). Findings
that used comment-derived tool data carry `tool_source: comment` evidence and
`Basis::Model`. Rules that need no tool data (`geometry.duplicate_hole`,
`geometry.tiny_segment`, `geometry.arc_radius_mismatch`, `modal.*` except
`tool_unknown`, `clearance.*`) must never be gated on a tool table.

### Out of scope for v1

- Chatter, cutter force, deflection, thermal prediction.
- Fixture/clamp collision without user-supplied fixture geometry.
- Full 3D undercut/5-axis collision.
- G41/G42 true offset geometry.
- Subprograms/macros/flow control.
- 4th axis and non-milling processes.

## Material engagement model (M2)

Use `nc-stock`'s 2.5D z-map, not a separate simulation:

- Grid over the stock XY; default 0.5-1.0 mm; cap cells (e.g., 4M).
- Initialize to `stock.top_z`.
- For each cutting motion, sweep the tool footprint and lower cells.
- Before lowering, sample the stock height around the tool circumference at the
  current Z to estimate contact angle. A reasonable estimate:
  `theta` from the fraction of sampled circumference with stock above the cut
  Z; `radial_DOC ~= R(1 - cos(theta/2))`, capped at `D`.
- Axial DOC = pre-cut height at the tool center minus the cut Z.
- Use `MotionRecord` feed/RPM/tool for chipload and MRR.
- Mark findings `Basis::Model`; call it "material engagement estimate", not a
  simulation, in the analysis UI.
- If stock/tool/material is missing, skip the rule. If stock is inferred,
  include `stock_source: inferred` in evidence.
- The pass is shared with `nc-sim` code (`Heightmap`, `Cutter`, capsule sweep)
  but runs headless and without ticks: it walks `expanded_motions()` in order.

## UI requirements

- File open (native dialog if available, otherwise an in-app browser), drag-
  and-drop, recent files. No network.
- `VIEW` mode: full static toolpath, 2D top and 3D orbit, fit, stock box,
  bounds, legend, diagnostics.
- `SIMULATE` mode: same canvas, stock cutting, tool pose, transport bar,
  traversed/future path, cut-depth legend.
- Program list: virtualized, line numbers, source text, severity gutter;
  click a diagnostic to highlight the line and zoom to its segments.
- Diagnostics panel: severity/category/rule filters, search, counts; details
  show rule ID, basis, evidence, and "why this matters".
- Coverage panel: `N evaluated · M skipped` as plain counts (never a
  percentage, progress bar, or green state), every skipped rule and its reason,
  visible without a submenu. Group skipped rules by missing input
  (e.g. `tool.diameter:T3`) and give each group a "set value" shortcut that
  opens the right field and lists the rules it would unlock.
- Tool chip (top bar and simulation HUD): shows `T3 · Ø6 · comment L4` or
  `T3 · Ø?`. Tapping it opens a quick popover to set the diameter (and
  optionally shape/flutes) for that tool without opening the full sheet.
- Tool inventory: built automatically from the program (every T number used,
  in order of first use, with move counts). Values from comments are prefilled
  and labeled with their source line; everything else is blank and optional.
  Entries persist locally keyed by the file's SHA-256 (not its path) and export
  to the TOML `[[tool]]` format.
- Setup sheet: stock (inferred/explicit, origin/size/top), tools (the
  inventory above: diameter,
  shape, flutes, stickout, flute length, shank/holder diameter, handedness),
  material preset, machine travel/limits and `spindle_manual`, dialect,
  simulation grid/rapid rate/crash preview, rule thresholds.
- Export: JSON report and Markdown summary. HTML is a plus.
- Parse/analyze/simulate off the UI thread; show progress and allow cancel.
  The UI thread only renders.
- Persist window state, recent files, config locally.
- Keep the ghs-rs theme, spacing, typography, chip/card language, 48 px touch
  targets, and responsive layout. Use screenshots next to ghs-rs to verify.
- No Start/Connect/Jog/Reset. This app is read-only.

## CLI requirements

```
nca check <file> [--config <toml>] [--format json|markdown|text]
                 [--output <path>] [--fail-on warning|error]   # default: error
nca view <file>                      # spawns the sibling `nca-gui` binary
nca simulate <file> [--config <toml>] [--to-line N] [--out-heightmap <png>]
nca rules
```

Exit codes:

- `0` — no findings at or above `--fail-on`.
- `1` — at least one finding at or above `--fail-on`. G-code the lossy parser
  could not understand is a `parse.unsupported` **finding**, not a tool failure.
- `2` — the tool could not produce a report: unreadable file, invalid config,
  input limit exceeded, internal error.

JSON is the stable report interface; keep it versioned. `nca rules` lists every
rule ID, severity, basis, stock requirement, and required inputs, generated
from `Rule::requirements()`. `nc-cli` has no GPUI dependency; a test checks
`cargo tree -p nc-cli` for it.

## Performance targets

Write `docs/PERFORMANCE.md` first, then fill in measurements. Record the
reference machine (CPU, GPU, RAM, OS) before measuring anything. The numbers
below are **initial targets**: measure, then revise them in `DECISIONS.md` with
the evidence.

- `MotionRecord` ≤ 128 bytes (size test). Modal state interned; arcs and canned
  data boxed.
- Parse + motion records + timeline: 100k-line file ≤ 1 s; 1M-line file ≤ 5 s.
- Peak memory: ≤ ~250 bytes per motion all-in; a 2M-motion program stays under
  ~1 GB.
- Analysis M1 on 1M lines ≤ 10 s; the M2 engagement pass is allowed to be
  slower but must be cancellable and show progress.
- Camera orbit/pan/zoom at 60 fps at 1920x1080 with a 1M-segment path, via the
  plan cache. Stock mesh update 15-30 Hz while playing.
- Above a configurable size, degrade gracefully (coarser grid, skip the M2
  engagement pass with a coverage skip reason) instead of stalling.

## Testing

- Unit test per rule: positive and near-miss negative fixture.
- Golden reports for: clean pocket, duplicate holes, overlapping holes,
  rapid-through-stock, missing feed, arc-smaller-than-tool, tool change,
  unsupported code, no tool/stock config, inferred stock, explicit stock,
  manual-spindle program with and without `spindle_manual`.
- Include fixtures from at least three real post-processors (Fusion 360 grbl,
  Estlcam, VCarve/Aspire, Carbide Create). The user can provide real files.
- Property tests: parser never panics on arbitrary bytes; diagnostic lines are
  in range; report determinism; bounds containment.
- Fuzz the parser and lossy mode.
- Parser/geometry tests: arc angle/sweep convention in all three planes
  (G17/G18/G19), full-circle arcs, R-format arcs, helixes, `expanded_motions`
  for G73/G81/G82/G83/G85/G86/G89, `G4 P` per dialect, modal interning,
  `size_of::<MotionRecord>() <= 128`.
- Simulator tests: stock removal, arcs/helixes, drills, seek backward, frame-
  rate independence (different `dt` sequences, same tick), `max_ticks_per_advance`
  splitting, capsule sweep leaves no gaps at high feed, crash preview, stock
  inference, grid cap, performance.
- Rendering tests: every `MoveKind` has style + legend; 2D/3D screenshots at
  1280x800 and 1920x1080; simulate at 0%, 25%, 50%, 100%; 60 fps path and
  stock-update target. Screenshot tests run on one reference OS (Linux); other
  OSes compile and run non-visual tests.
- Cross-platform CI: Linux, macOS, Windows; fmt, clippy `-D warnings`, test,
  build, license check. Golden reports and heightmap hashes are compared on all
  three.
- A test proving unsupported state-changing code stops geometry and is reported
  as `geometry_uncertain`.
- A test proving skipped rules appear in coverage, and that `explicit`-stock
  rules are skipped (with reason) when the stock is inferred.
- A test proving inferred stock ignores safe-Z rapids.
- A test proving the simulator does not cut on rapids by default.
- A test proving reports contain no absolute paths.
- A bare-file golden: no comments, no config. It opens, renders, simulates with
  the labeled centerline cutter, runs every no-setup rule, and lists each skipped
  rule with its missing input.
- Tool-hint tests: header-comment fixtures per post; ambiguous-unit comments are
  dropped with `parse.tool_hint_ambiguous`; user config overrides a comment;
  `modal.tool_unknown` is skipped when there is no tool table or hint; findings
  that used a hint carry `tool_source: comment` and `Basis::Model`.

## Fixtures

Create `docs/FIXTURES.md` and `fixtures/README.md`.

- Use synthetic fixtures first, plus the parent's
  `PROGRAM_CHECK_DEMO_SOURCE` as a starting point.
- Ask the user for real files after the vertical slice:
  - 3-5 files from their actual post-processors, with the post name and machine.
  - Duplicate holes / repeated drilling.
  - Deep pocket / heavy stepdown.
  - 3D surfacing / ball nose.
  - Helical bore / arcs.
  - G41/G42.
  - Inch units.
  - Macros/subprograms.
  - A manual-spindle post (no M3) and, if available, a Fanuc-style post.
  - Files with tool info in header comments (name the post) **and** files with
    no tool info at all; the latter are the common case.
- Store under `fixtures/real/<name>/` with:
  - `source.nc`
  - `config.toml` (stock/tools/material/machine)
  - `expected.json` (reviewed golden report)
  - `NOTES.md` (post, machine, source, license/permission)
- Do not auto-bless a golden report; the orchestrator reviews it first.
- Do not block on real fixtures; synthetic coverage is enough to start.

## Decisions to make explicitly

Defaults unless the user says otherwise:

- **Fork now.** Copy the crates into `nca-rs`; do not depend on `../ghs-rs`.
- **GPUI native app.** Reuse `ghs-viewer` and the existing canvas. A web/WASM
  viewer is a later option.
- **Permissive viewer mode.** Unknown codes are diagnostics; stop geometry at
  the first unsupported state-changing construct.
- **G41/G42 centerline in v1.** Warn and skip tool-radius-dependent checks.
- **Inferred stock by default.** Use cutting bounds, show `STOCK: INFERRED`,
  and allow an explicit override. Stock-dependent findings are `Basis::Model`;
  rules that need a real stock are skipped while stock is inferred.
- **Tool data is optional.** Sizes come from the user or from comments, never
  from guessing. Unknown tools simulate as a labeled centerline trace; rules
  that need tool data are skipped with the missing input named.
- **Generic dialect by default**, with an explicit `dialect` setting and an
  Info finding when the default is assumed.
- **Determinism scope:** same OS/architecture guaranteed; cross-platform
  verified in CI; `libm` for trig.
- **TOML config, JSON report.** Human config, machine output.
- **2.5D z-map.** No voxel engine in v1.
- **English-only v1.** Centralize strings; i18n later.
- **No auto-fix.**

## What not to do

- Do not copy the sender shell, serial code, simulator, probing, settings,
  controller panels, DRO, jog, console, or job controls.
- Do not add a controller connection "for testing."
- Do not claim the analyzer is a physical simulator. The simulator is a visual
  stock-removal preview; the analyzer is advisory.
- Do not add ML/LLM scoring or opaque "confidence" numbers, or any coverage
  percentage that could read as one.
- Do not silently ignore unsupported codes or compensate for them without
  saying so.
- Do not hardcode thresholds without config defaults and docs.
- Do not implement full 3D collision/force simulation in v1.
- Do not let subagents edit outside their paths.
