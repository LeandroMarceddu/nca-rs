# Decisions

Entries are append-only. Each records the decision, its reason, and any
consequence for the contract.

## D001 - Fork the parent crates; do not depend on `../ghs-rs`

- **Decision:** Copy `ghs-viewer` and `ghs-gcode` into this workspace and build
  a minimal `nc-core`.
- **Reason:** The product scope is different (read-only analyzer), and the
  parent is still evolving.
- **Parent commit:** `4bfbda60553661df677eaab15c323ec428c8690b`.

## D002 - `ToolHint` lives in `nc-core`

- **Decision:** Define `ToolHint` in `nc-core` and re-export it from
  `nc-gcode`.
- **Reason:** `ToolTable::resolve(config, hints)` is owned by `nc-core`; putting
  the hint type in `nc-gcode` would require `nc-core -> nc-gcode` while
  `nc-gcode -> nc-core` already exists.
- **Consequence:** `nc_gcode::ToolHint` remains the downstream name.

## D003 - Keep `SegmentKind` during the transition

- **Status:** realized and closed on 2026-11-08. `SegmentKind` is removed;
  `Segment`, `Toolpath`, `Polyline`, and `RenderPlan` carry `MoveKind`
  end-to-end, and display/record kinds are produced by one classifier.
- **Decision:** Add `MoveKind` and its styles to `nc-viewer` now; keep the
  existing display `SegmentKind` until the parser milestone replaces display
  emission.
- **Reason:** Phase 0 must compile and freeze the interface without rewriting
  the entire parser and its golden tests in one step.
- **Consequence:** Phase 1 parser work migrates display segments to `MoveKind`
  (or makes `SegmentKind` a display projection of it), and the app renders from
  the semantic taxonomy.


## D004 - `MotionRecord` is the source of truth; `Toolpath` stays for display

- **Decision:** Keep `Program::toolpath` and add `Program::motions`.
- **Reason:** A flat `f32` chord stream is the right display structure and a poor
  analysis structure; exact arcs and per-move modal data must survive.
- **Consequence:** Rules and the simulator read `motions`; the viewer reads the
  toolpath.

## D005 - `MotionSpan` carries poses, tool, feed, and spindle state

- **Decision:** Extend the seed `MotionSpan` fields.
- **Reason:** The timeline is the only owner of timing; every consumer needs the
  endpoint poses and the tool/spindle state for the same interval.
- **Consequence:** No second parallel array that could drift.

## D006 - G41/G42 is recorded, never applied

- **Decision:** `CutterComp` records the active mode; geometry stays on the
  programmed centerline; tool-radius rules are skipped while it is active.
- **Reason:** True offset geometry is out of scope for v1, and silently drawing
  an uncompensated path as if it were compensated is unsafe.
- **Consequence:** G41/G42 does **not** truncate geometry. The lossy "stop at a
  state-changing unsupported construct" rule applies to unknown constructs
  (M98, macros, flow control), not to a comp mode the parser understands and
  labels. A finding must still tell the user that the displayed path is the
  centerline, not the compensated path.

## D007 - Inferred stock is a model and can be wrong

- **Decision:** Infer from cutting bounds only, expose `InferenceBasis`, and mark
  stock-dependent findings `Basis::Model`.
- **Reason:** Safe-Z rapids must not inflate the block; a program with Z0 at the
  table produces a thin slab, which must be visible rather than hidden.
- **Consequence:** Rules that need a real stock declare
  `StockRequirement::Explicit` and are skipped on inferred stock.

## D008 - Unknown tool sizes are represented, not guessed

- **Decision:** `Cutter::diameter_mm == None` means a centerline trace one cell
  wide, flagged `assumed`.
- **Reason:** Most NC files carry no tool geometry. A guessed diameter would
  produce false volume and collision results.
- **Consequence:** The UI shows `TOOL SIZE ASSUMED` and labels the 3D tool `Ø?`.

## D009 - Dwell units are dialect data

- **Decision:** `Dialect::dwell_p_is_milliseconds()` is true only for `Fanuc`.
- **Reason:** Fanuc-style controls commonly express `G4 P` in milliseconds.
  The table in `docs/DIALECTS.md` marks rows that still need vendor verification.

## D010 - Sender-side G-code rewriting is removed

- **Decision:** Do not carry `compensation.rs`, `rotate.rs`, `scale.rs`, or
  `rewrite.rs` into the product once the parser milestone stabilizes.
- **Reason:** The product never edits G-code. Expression/parameter parsing stays
  because the parser needs it to read programs safely.
- **Status:** The files are present in the Phase 0 fork only so the copied parser
  compiles; Phase 1 prunes them.

## D011 - GPUI Kit 0.8.1 via the workspace

- **Decision:** Pin `gpui-kit = "0.8.1"` in `[workspace.dependencies]`.
- **Reason:** Same facade and version as the parent; a later migration touches
  one line per crate.

## D012 - `nc-cli` never depends on GPUI or `nc-app`

- **Decision:** `nca view` spawns the sibling `nca-gui` binary.
- **Reason:** Headless checking must build and run on machines without a window
  system. A CI test inspects the dependency tree.

## D013 - Report numbers are quantized at serialization

- **Decision:** Keep `f64` internally; round at the report boundary with fixed
  decimal formatting.
- **Reason:** Float formatting is the most likely source of byte differences
  between platforms.

## D014 - `RulesetVersion` and rule ids are API

- **Decision:** `RULESET_VERSION` is `1.0.0`; changing a threshold default or a
  finding's meaning requires a bump. Rule ids never change meaning.
- **Reason:** Golden reports and downstream consumers key on both.

## D015 - Config parsing is strict

- **Decision:** Unknown TOML keys and unknown enum values are errors.
- **Reason:** A typo that silently disables a rule is worse than a failed run.

## D016 - Phase 0 simulator is a scaffold

- **Decision:** The first `Simulation` implementation is a deterministic linear
  scaffold over display segments; the stock/sim milestone replaces the timeline
  and sweep with `Program::expanded_motions()` and exact arc subdivision.
- **Reason:** It proves the API and the heightmap while keeping the milestone
  boundary honest.
- **Consequence:** Do not accept simulator behavior or performance numbers until
  the Phase 1 replacement lands.

## D017 - Header tool hints may use a unit declared later in the header

- **Decision:** For a comment before the first motion, a diameter/radius hint may
  take its unit from a `G20`/`G21` declared anywhere before the first motion,
  even after the comment. A unit declared after the first motion never applies.
- **Reason:** The real `box54+lid 50cm.nc` fixture (PrintNC, Fusion-style post)
  writes `(T1 D=3.1 ...)` and `(T2 D=6 ...)` before `G21`. A strict
  "already declared" rule would drop every header hint on this common post.
- **Consequence:** The hint remains conservative: no unit before the first
  motion means the hint is dropped with `parse.tool_hint_ambiguous`. The rule is
  documented in `docs/DIALECTS.md`.

## D018 - The real fixture is zero-config

- **Decision:** `fixtures/real/box54-lid-50cm` ships with only
  `[parse] dialect = "generic"`; stock is inferred and tool sizes come from the
  header hints.
- **Reason:** This is the normal real-world case and exercises the coverage
  surface (rules skipped for missing machine/material data) on a 119k-line file.
- **Consequence:** A separate configured fixture must cover explicit machine,
  stock, and material data.

## D019 - Commits and pushes are authorized

- **Decision:** The user authorized git commits and pushes on 2026-11-08. The
  remote is `https://github.com/LeandroMarceddu/nca-rs`. The orchestrator commits
  each accepted subagent milestone separately and pushes after a gate.
- **Reason:** The original "no commits unless asked" constraint is superseded by
  an explicit user request.
- **Consequence:** Subagents still never commit; they report changed files. The
  orchestrator reviews the diff, runs the gate, then commits and pushes.

## D020 - The CI license/advisory job is removed at the user's request

- **Decision:** The GitHub Actions `license and advisories` job is removed. The
  `deny.toml` policy and the local command `cargo deny check advisories licenses
  bans sources` remain for manual use.
- **Reason:** The user asked to remove that CI job on 2026-11-08. The action's
  Docker image also failed to build before it could run.
- **Consequence:** CI no longer enforces the license policy. Run the local
  command before releases; the allow-list remains permissive-only.

## D021 - Tool-dependent rules are `Basis::Model`

- **Decision:** `geometry.overlapping_holes`, `geometry.arc_smaller_than_tool`,
  and `modal.spindle_direction` are `Basis::Model` because their required tool
  fields may come from a comment hint. Rules that use only program text, exact
  geometry, or explicit machine/stock configuration stay `Basis::Exact`.
- **Reason:** `SPEC.md` says findings that used comment-derived tool data are
  model findings, but `Rule::basis()` is per rule. A conservative static model
  basis is the honest choice.
- **Consequence:** The docs/RULES.md table is the reference; a future
  per-finding basis would allow these rules to be exact for user-entered tools.

## D022 - Machine-coordinate moves are detected from source words

- **Decision:** `modal.machine_coords` scans `ProgramLine.words` for
  `G53`/`G28`/`G30` because `MotionRecord` has no machine-coordinate flag.
- **Reason:** The parser emits these commands on their source lines, so the
  scan is complete for parsed programs. Adding a flag would grow the 128-byte
  record for a rare case.
- **Consequence:** A hand-built `Program` without words cannot express a
  machine-coordinate move; document the limitation and revisit if a consumer
  needs it.

## D023 - `parse.unsupported` is a real registry rule

- **Decision:** `Program` stores `skipped_blocks`, and `parse.unsupported`
  consumes them as a registry rule. The CLI and fixture loader no longer
  synthesize a second diagnostic.
- **Reason:** Coverage must include every rule, and duplicating the finding
  outside coverage was dishonest.
- **Consequence:** The rule participates in `nca rules`, coverage totals, caps,
  and determinism exactly like every other rule.

## D024 - Spindle-speed checks skip a manual spindle

- **Decision:** `modal.spindle_speed_zero` skips with
  `machine.spindle_manual` when the spindle is started by hand, matching
  `modal.spindle_stopped` and `SPEC.md`.
- **Reason:** A manual spindle has no authoritative programmed speed.

## D025 - M2 engagement applies radial checks only to side cutting

- **Decision:** `process.radial_engagement` ignores plunge/drill/peck/bore
  passes. `process.rest_material` flood-fills the uncut stock from the grid
  border and reports only enclosed regions. `process.thin_wall` requires a cut
  region on both sides of the ridge.
- **Reason:** Flagging a drill as 100% radial engagement is meaningless, and an
  AABB-based rest check reported the square corners left by a round tool as
  false rest material. Both produced noise on correct programs.
- **Consequence:** `clean-pocket` disables the two final-state model rules and
  accepts full-width slotting explicitly; dedicated rest/thin-wall fixtures are
  a follow-up. Coverage reports every disabled rule.

## D026 - The M1 catalog is complete; M2 is model-based

- **Decision:** 34 M1 rules plus 11 M2 rules are registered (45 total). All M2
  rules are `Basis::Model`; M1 rules that require optional tool data are Model
  (D021). `RULESET_VERSION` stays `1.0.0` for the first release.
- **Reason:** The first release has no prior published report format to
  migrate; future behavior changes bump the version.



## D027 - The playback tick budget is 96, not 8

- **Decision:** `SimConfig::max_ticks_per_advance` defaults to 96.
- **Reason:** The app multiplies the frame delta by the transport speed. At
  15 fps, 4x needs 32 ticks per frame; the old 8-tick cap made both 1x and 4x
  saturate, so 4x looked identical to 1x on heavy scenes.
- **Consequence:** 4x reaches real 4x down to about 5 fps. Queued ticks still
  drain on later frames, and the UI work remains bounded per call.

## D028 - Unanchored header hints attach to the first header tool

- **Decision:** A header comment with tool data but no tool number attaches to
  the first `T` selected later in the header region, falling back to the nearest
  preceding header `T`. If no header tool exists, it is dropped.
- **Reason:** Real posts write `(clean pocket, 6 mm 2F flat)` before `T1 M6`.
  Requiring a tool number in the comment discarded usable data.
- **Consequence:** Specific `T<n>` comments win per field; unanchored hints never
  supply a tool name; a `T` after the first motion is never used.

## D029 - The app analyzes with inferred stock until the setup sheet lands

- **Status:** superseded by the setup sheet on 2026-11-08. The app can now load
  a TOML config or edit stock/machine/material/tools in the setup sheet; the
  inferred path remains the default when nothing is set.
- **Decision:** The app resolves comment/user tools, uses inferred stock, and
  leaves machine/material unset in this milestone. Coverage exposes every rule
  that this skips, and the set-value actions for stock/machine/material open a
  labeled placeholder.
- **Reason:** Config loading and the setup sheet are a separate milestone; the
  app must not silently invent stock or machine data.
- **Consequence:** The app report can differ from a CLI run that supplies a TOML
  config. The UI labels the stock source and the assumed dialect.

## D030 - The per-motion memory target misses on word retention

- **Decision:** Keep the 1M-line result (~430 B/motion, ~420 MiB) as the
  documented baseline. The 250 B/motion goal is not met yet; a 2M program is
  projected below the 1 GB ceiling.
- **Reason:** `ProgramLine` keeps its source `String` and a `Vec<Word>` for every
  line, which M1 source-level rules need. `MotionRecord` is still 128 bytes and
  the display toolpath is compact; the line metadata is the overhead.
- **Consequence:** A post-v0.1 optimization can replace per-line words with
  bounded flags while keeping the four source-level rules working. Measure again
  after that change; do not lower the target silently.


## D031 - Tool overlays persist per file hash, not per path

- **Decision:** The app stores edited tool tables in `NcaConfig.tool_tables`
  keyed by the source SHA-256. `LoadedProgram` carries the hash; reports still
  contain only the file name.
- **Reason:** The same file can move; a path key would lose the edits or collide
  across copies.
- **Consequence:** Re-exporting the inventory as TOML `[[tool]]` and migrating
  the old global tool field are follow-ups. Comment-derived fields fall back
  when cleared until a suppression/tombstone state exists.

## D032 - Rule thresholds are editable as key/value pairs

- **Decision:** The setup sheet lists the threshold keys present in config and
  accepts a free-form numeric key/value. `Rule` does not expose its documented
  default threshold keys.
- **Reason:** Adding a `Rule::threshold_keys()` API is a larger interface change
  and would duplicate the docs/RULES.md catalog.
- **Consequence:** Users can override documented keys, but the sheet cannot
  enumerate every possible key. Revisit when rule metadata is centralized.

## D033 - v0.1 fixture scope

- **Decision:** The 13 synthetic fixtures with reviewed `expected.json` are the
  v0.1 golden set. Dedicated fixtures for `arc-radius-mismatch`, `tool-change`,
  `inferred-stock`, `explicit-stock`, `thin-wall`, `rest-material`, and a
  second manual-spindle configuration are post-v0.1 follow-ups.
- **Reason:** The core coverage exists; the missing folders are additional
  regression cases, not unexercised code paths. Several rules are covered by
  unit tests and by `clean-pocket`, `rapid-through-stock`, and `missing-feed`.
- **Consequence:** `docs/FIXTURES.md` marks the list as the target corpus and
  the committed set as the v0.1 subset. Do not claim the full corpus exists.

## D034 - v0.1 worker cancellation is stale-result only

- **Decision:** Analysis and long seeks run on workers with a generation gate;
  a stale result is discarded. There is no mid-run cancel token or progress
  callback in v0.1.
- **Reason:** Adding a cancellation API to `analyze` and `Simulation::seek_to_time`
  changes the frozen interfaces and needs careful testing across long M2 rules.
- **Consequence:** The UI stays responsive, but a long rule/seek already running
  finishes its work. Architecture, simulation, and performance docs state this
  limitation instead of claiming cancellable progress.

## D035 - Entering SIMULATE builds the session synchronously

- **Decision:** v0.1 constructs `Simulation::new` on the UI thread when the user
  switches to SIMULATE. No loading state or worker handoff is implemented.
- **Reason:** The initial build is fast on the measured fixtures; moving it to a
  worker needs a new app state machine and cancellation semantics.
- **Consequence:** A 1M-line file can pause the UI on the first mode switch.
  This is a known v0.2 follow-up; the camera/mesh frame targets remain pending.

## D036 - v0.1 review fixes

- **Decision:** The app uses `parse_program_lossy` and keeps partial geometry;
  travel rules use exact arc extents; CLI and Markdown headlines count distinct
  skipped rules; the app hashes raw file bytes including a BOM; unknown TOML
  rule ids are errors; stock inference uses exact motion extents when motions
  exist; golden coverage is enforced by an explicit 13-name list.
- **Reason:** These were the independent v0.1 review blockers/should-fix items.
- **Consequence:** Unsupported files are viewable and analyzed, arc overtravel
  is caught, coverage arithmetic is honest, and a typo cannot disable a rule.
