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

## Accepted milestones

None yet. Phase 0 is the first gate.

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
