# Progress

This file mirrors the OpenChamber STATE note so progress survives even if the
project knowledge store is lost.

## Current

- Phase: **v0.5.0 released**
- Status: signed tag `v0.5.0` on `76d0129`; release workflow
  `42520583813` published the Linux, Windows, and macOS archives plus
  `SHA256SUMS.txt` as **Latest**. Full local gate green with 45 suites / 812
  tests.
- Next (post-0.5.0): mid-run analysis/seek progress and cancellation (D034),
  asynchronous SIMULATE session build (D035), camera/stock-mesh measurements,
  the dedicated fixtures in D033, and `cargo deny`/doc CI.
- Release: <https://github.com/LeandroMarceddu/nca-rs/releases/tag/v0.5.0>

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
| 2026-11-08 | app SIMULATE: clippy + `cargo test -p nc-app` | pass, 66 tests; 4 captures at 1280x800 |
| 2026-11-08 | M2 process: clippy + `cargo test -p nc-analysis` | pass, 46 unit + 33 process tests |
| 2026-11-08 | full workspace gate after M2 | fmt, clippy, tests, docs pass |
| 2026-11-08 | sim speed fix: `cargo test -p nc-sim` | 31 tests, 4x/15 fps = 32 ticks |
| 2026-11-08 | unanchored header hints: `cargo test -p nc-gcode` | pass, 9 new tests; real file unchanged |
| 2026-11-08 | diagnostics/coverage UI: clippy + `cargo test -p nc-app` | pass, 77 tests; 2 captures |
| 2026-11-08 | real box54: release check, 3 runs | median 0.509 s, ~103 MiB RSS, 477 findings, 31/45 |
| 2026-11-08 | setup/config/export: clippy + `cargo test -p nc-app` | pass, 90 tests; 4 captures refreshed |
| 2026-11-08 | v0.1 review fixes: workspace fmt/clippy/test/doc | pass; docs-vs-registry test added |
| 2026-11-08 | 1M-line synthetic: release check | 0.745 s, ~420 MiB RSS; memory target miss recorded (D030) |
| 2026-11-08 | parser corrections: clippy + `cargo test -p nc-gcode -p nc-stock -p nc-sim` | pass; real file 119,393 records, 0 errors, 336 ms |
| 2026-11-08 | Makera dialect: `cargo test -p nc-gcode -p nc-fixtures` | pass, 263 tests; LatchBase 7,550 lines, 0 skipped, 0 parse findings, 31/45 |
| 2026-11-08 | Makera + .cnc + orbit drag: workspace fmt/clippy/test/doc + deny | pass, 44 suites, deny ok |
| 2026-11-08 | stock fill + AUTO dialect + shaped sweep: workspace gate | pass, 45 suites / 812 tests, docs + deny clean |
| 2026-11-08 | release scaffolding v0.5.0: workflow YAML, changelog, runbook | yaml + metadata + fmt pass |
| 2026-11-08 | v0.5.0 release: signed tag, CI 42520422295, release 42520583813 | Latest; 3 archives + SHA256SUMS; checksums verified |

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
- **M1 rule catalog** — 34 rules across modal/parse/travel/clearance/geometry,
  with positive/near-miss tests, coverage, and 13 reviewed synthetic goldens
  (2026-11-08).
- **App SIMULATE** — transport, frame-clock playback, heightfield stock mesh,
  traversed/future path, tool chip, assumed-size badge, HUD stats, 66 tests,
  four 1280x800 captures (2026-11-08).
- **M2 process rules** — shared engagement pass and 11 process rules; radial
  checks are side-cutting only; rest material uses enclosed regions; 33 tests
  (2026-11-08).
- **Diagnostics + coverage UI** — analysis worker, filters/details/evidence,
  click-to-zoom, warning overlay, severity gutter, grouped skip actions, 77 app
  tests, two captures (2026-11-08).
- **Setup, config, export** — TOML load, full setup sheet, SHA-keyed tool
  persistence, JSON/Markdown export, 90 app tests, setup/export captures
  (2026-11-08).
- **v0.1 review fixes** — lossy app parse, exact arc extents, coverage
  headlines, raw-byte BOM hash, unknown rule ids, exact stock extents, enforced
  golden set, fixture notes (2026-11-08).
- **Post-v0.1: Makera Z1** — `makera` dialect (Smoothieware `M400`, Makera
  `M851`/`M852`, clearance `G28`), real `LatchBase.nc` fixture, `.cnc` file
  extension, inverted 3D vertical orbit drag (2026-11-08).
- **Post-v0.1: stock display and AUTO dialect** — GHOST/SOLID/CUT ONLY stock
  fill with SOLID as the SIMULATE default (D039), evidence-based AUTO dialect
  detection in the app (D040), and the shaped-sweep frame fix (D041).
- **Full workspace gate** — fmt, clippy `-D warnings`, tests, and docs clean
  (2026-11-08).

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
