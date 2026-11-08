# Repository instructions for agents

`nca-rs` is a read-only NC viewer, deterministic preflight analyzer, and stock
simulator for 3-axis CNC milling G-code. It never connects to a machine, never
sends motion, never edits G-code, and never claims a program is safe.

Read these first, in order:

1. `docs/SPEC.md` - the product specification. Read only the sections your task
   cites; do not paste the whole file into a brief.
2. `docs/INTERFACES.md` - the frozen shared-type contract. It wins over the
   spec if they ever disagree; record the reason in `docs/DECISIONS.md`.
3. `docs/ARCHITECTURE.md` - crates, dependency direction, threading.
4. `docs/DETERMINISM.md` - what must be byte-identical and how.
5. `docs/RULES.md`, `docs/DIALECTS.md`, `docs/SIMULATION.md` - domain details.
6. `docs/PROGRESS.md` - the current milestone and accepted work.

## Scope guardrails

- **Read-only.** No serial port, controller connection, streaming, jog, DRO, or
  network. No G-code writing or auto-fix.
- **3-axis mill only.** No lathe, plasma, drag knife, 4th axis, or ATC UI.
- **Local only.** No telemetry, cloud, or network calls at runtime.
- **Advisory, not authoritative.** Every finding is explainable and carries a
  stable rule id, severity, basis, source line, and evidence.
- **Coverage honesty.** A rule that cannot run is skipped with a reason. Never
  show a green "SAFE" badge, a coverage percentage, or a progress bar.
- **Zero-config first.** Tool, stock, machine, and material data are optional.
  Never invent a value to make a rule run; skip it and name the missing input.

## Ownership and workflow

- The orchestrator owns the architecture, public APIs, root `Cargo.toml`, docs,
  CI, integration, and commits.
- A subagent may edit **only** the paths named in its brief. Do not touch other
  crates, root files, or docs unless the brief says so.
- Do not create git commits yourself. Report changed files and test results;
  the orchestrator reviews and commits. The orchestrator may commit and push
  this repository: the user authorized it on 2026-11-08, with the remote
  `https://github.com/LeandroMarceddu/nca-rs`.
- If a task needs an interface change in another crate, stop and report the
  needed change instead of editing that crate.
- Build and test with the default `target/` directory in the repository. Never
  set `CARGO_TARGET_DIR`; parallel agents share Cargo's lock, so serialize heavy
  builds when needed.
- Prefer small, reviewable changes over broad refactors.

## Code standards

- Edition 2024, Rust 1.98.1. Format with `cargo fmt`; keep lines <= 100 columns.
- Library crates use typed errors with `thiserror`; no `anyhow` in libraries.
- No `unwrap`/`expect`/`panic!` on input data in library code. Tests may panic.
- Never index with an untrusted length; bound every loop over input.
- Use `tracing` for diagnostics in libraries; no `println!` except in binaries.
- Public items get doc comments. Units are explicit in names or docs.
- Trigonometry goes through `nc_core::math` (libm-backed), never `f64::sin` and
  friends, so reports and heightmaps are platform-stable.
- No `f32` arithmetic in library code. Heightmap cells are *stored* as `f32`;
  computation is `f64` with a deterministic rounding on store.
- No `HashMap`/`HashSet` iteration on a path that affects output. Use
  `BTreeMap`/`BTreeSet` or sorted vectors.
- Derive `Debug`, `Clone`, `PartialEq` where useful; use `serde` only at
  persistence, config, and report boundaries.
- ASCII-only source and default UI strings.

## Testing

- Every rule needs a positive fixture and a near-miss negative fixture.
- Parser and lossy mode must never panic on arbitrary bytes; add property tests.
- Determinism: reports and heightmap hashes must match byte for byte for the
  same input, config, and ruleset version.
- Required commands before reporting:

```sh
cargo fmt --all
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
```

The orchestrator runs the full workspace gates before accepting a milestone.

## Reporting format

Return a concise report with:

1. **What changed** - files and public API.
2. **Tests** - commands run and their output.
3. **Interface change requests** - exact type/function changes needed in
   `docs/INTERFACES.md`, if any.
4. **Open questions / risks**.
5. **Follow-ups** - intentionally deferred work.

Do not claim completion without test output. Do not hide failing tests.
