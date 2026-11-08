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


