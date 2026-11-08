# Progress

This file mirrors the OpenChamber STATE note so progress survives even if the
project knowledge store is lost.

## Current

- Phase: **0 - scaffold, docs, frozen interfaces**
- Status: Phase 0 work complete; gate verification in progress.
- Next: review the Phase 0 gate, then spawn the parser, stock/sim, and app-shell
  milestones in parallel.

## Phase 0 checklist

- [x] Continuity anchors created (plan, todos, STATE note).
- [x] `docs/SPEC.md` saved from the mission through "What not to do".
- [x] Workspace and `rust-toolchain.toml` created.
- [x] Parent commit recorded in `PROVENANCE.md`.
- [x] `nc-viewer` forked and renamed; `MoveKind` taxonomy added.
- [x] `nc-gcode` forked and renamed; motion records and builder added.
- [x] Minimal `nc-core` created with libm-backed math.
- [x] `nc-stock`, `nc-sim`, `nc-analysis`, `nc-app`, `nc-cli`, `nc-fixtures`
      scaffolds created.
- [x] Every shared type defined in `docs/INTERFACES.md`.
- [x] `cargo check` green for the workspace.
- [ ] `cargo doc --workspace --no-deps` clean.
- [ ] Phase 0 gate accepted.

## Gate log

| Date | Gate | Result |
|---|---|---|
| 2026-11-08 | `cargo check --offline --workspace` | pass; GPUI 0.8.1 builds |
| 2026-11-08 | `cargo test -p nc-core -p nc-viewer -p nc-stock -p nc-analysis` | pass (unit tests; one doctest fixed) |
| 2026-11-08 | `cargo test -p nc-gcode --lib` | pass, 54 tests |
| 2026-11-08 | `cargo check -p nc-app` | pass (`nca-gui` stub window) |
| 2026-11-08 | stock/sim core: clippy + `cargo test -p nc-stock -p nc-sim` | pass, 30 + 35 tests after stats/ramp follow-up |
| 2026-11-08 | parser: clippy + `cargo test -p nc-gcode` | pass, 46 unit + 184 integration/doctests |
| 2026-11-08 | real `box54` parse | 119,433 lines, 119,393 records, 0 errors, 327 ms debug |
| 2026-11-08 | analysis skeleton: clippy + `cargo test -p nc-analysis` | pass, 18 + 25 + doctest; deterministic canonical JSON |
| 2026-11-08 | app shell: clippy + `cargo test -p nc-app` | pass, 44 tests; 5 isolated nested captures at target sizes |
| 2026-11-08 | Phase 1 gate: fmt + workspace clippy + workspace test | pass |
| 2026-11-08 | first three rules: clippy + `cargo test -p nc-analysis` | pass, 18 + 25 + 14 + doctest |
| 2026-11-08 | semantic rendering: viewer/gcode/app tests | pass, 70 viewer + 504 workspace |
| 2026-11-08 | CLI/config/fixtures: clippy + tests | pass, 35+25+14+5+9+2 |
| 2026-11-08 | GitHub CI run 42467554129 | success on all jobs |

## Accepted milestones

- **Phase 0** — workspace, docs, interfaces, ProgramBuilder (2026-11-08).
- **Parser** — lossy mode, motion records, tool hints, real-file parse (2026-11-08).
- **Stock/sim** — inference, heightmap, sweep, timeline, tick engine, stats (2026-11-08).
- **Analysis skeleton** — engine, coverage, canonical JSON (2026-11-08).
- **App shell** — GPUI VIEW mode, program list, 2D/3D canvas, legend, file
  browser, settings sheet, 44 tests, screenshots (2026-11-08).
- **Phase 1 gate** — workspace fmt, clippy `-D warnings`, and tests green
  (2026-11-08).
- **Vertical-slice rules** — `modal.feed_missing`, `travel.machine_envelope`,
  `clearance.rapid_at_depth` with positive/near-miss tests and coverage
  (2026-11-08).
- **Semantic rendering** — `MoveKind` end-to-end; `SegmentKind` removed; the
  painter, toggles, dashes, and hole markers use all 13 kinds (2026-11-08).
- **CLI and fixtures** — strict TOML loader, `nca check|view|simulate|rules`,
  fixture harness and golden comparison (2026-11-08). Goldens are drafts pending
  the feedless/peck parser fixes and review.
- **Parser correction** — lossy feedless cuts keep a motion with no feed;
  canned upward returns are `Retract`; `Program` stores skipped blocks
  (2026-11-08).
- **CI** — Linux/macOS/Windows builds, Linux clippy/test, GUI build all green
  (run 42467554129).

## Version control

- Repository initialized on `main`.
- Remote: `https://github.com/LeandroMarceddu/nca-rs`.
- Commit `49273d1` — initial workspace, pushed 2026-11-08.
- Policy D019: commits and pushes are authorized; one commit per accepted
  subagent milestone; subagents still never commit.

## Changed files (Phase 0)

- Root: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`,
  `AGENTS.md`, `PROVENANCE.md`, licences, `NOTICE`, `.gitignore`.
- `crates/nc-core` (new): `lib`, `math`, `tool`, `dialect`, `machine`,
  `material`, `error`, copied `axis`/`position`/`units`/`wcs`.
- `crates/nc-viewer` (fork): all parent files plus `move_kind.rs`.
- `crates/nc-gcode` (fork): all parent files plus `motion.rs`, `builder.rs`,
  `Program` motion fields and methods.
- `crates/nc-stock` (new): `stock`, `heightmap`, `cutter`.
- `crates/nc-sim` (new): `config`, `timeline`, `simulation`, `error`.
- `crates/nc-analysis` (new): `types`, `config`, `report`, `rule`.
- `crates/nc-app` (new): Phase 0 window shell.
- `crates/nc-cli` (new): `nca` stub.
- `crates/nc-fixtures` (new): loader scaffold.
- Docs: `SPEC`, `INTERFACES`, `ARCHITECTURE`, `DECISIONS`, `REUSE-MAP`,
  `DETERMINISM`, `RULES`, `DIALECTS`, `SIMULATION`, `FIXTURES`, `PERFORMANCE`,
  `screenshots`, this file.
- `fixtures/README.md`.

## Known gaps at the Phase 0 boundary

- The parser still emits empty `motions`; Phase 1 fills them.
- `parse_program_lossy` is specified but not implemented.
- The simulator timeline uses display segments; Phase 1 replaces it.
- The analysis engine has types but no registry or evaluator.
- `nc-app` is a single-window stub; the real shell is Phase 1.
- `cargo deny` is not wired yet (Phase 4 CI).
