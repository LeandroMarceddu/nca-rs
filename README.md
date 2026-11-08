# nca-rs

A read-only NC viewer, deterministic preflight analyzer, and stock simulator
for 3-axis CNC milling G-code.

Open a `.nc`, `.gcode`, `.tap`, `.ngc`, or `.cnc` file. The app parses it without a
controller, renders the toolpath in 2D and 3D, runs explicit rules over the
program and geometry, shows every finding with its source line and evidence,
and can replay the toolpath as a 2.5D stock-removal preview.

It never connects to a machine, never sends motion, never edits the G-code, and
never claims a program is safe.

## Workspace

| Crate | Purpose |
|---|---|
| `nc-core` | coordinates, units, WCS, sparse tool overlay, machine limits, material presets, deterministic libm-backed math |
| `nc-viewer` | dependency-free geometry, cameras, render plans, semantic move taxonomy |
| `nc-gcode` | parser, lossy mode, exact motion records, canned cycles, tool hints |
| `nc-stock` | stock inference, 2.5D heightmap, cutter shapes, sweep |
| `nc-sim` | deterministic 1/120 s playback engine |
| `nc-analysis` | rule contract, coverage, engagement pass, versioned report |
| `nc-app` | GPUI desktop shell (`nca-gui`) |
| `nc-cli` | headless `nca` checker |
| `nc-fixtures` | fixture loader and golden-report harness |

## Build and test

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --workspace --no-deps
```

Rust 1.98.1 is pinned in `rust-toolchain.toml`.

## Run

```sh
cargo run -p nc-app            # nca-gui
cargo run -p nc-cli -- --help  # nca
```

## Releases

Public releases are tagged `vMAJOR.MINOR.PATCH`, for example `v0.5.0`, and are
built by the tag-driven [release workflow](.github/workflows/release.yml) for
Linux x86_64, Windows x86_64, and macOS (the archive name records the runner
architecture). Each archive contains `nca`, `nca-gui`, the README, both
licenses, `NOTICE`, and `PACKAGING.md`; `SHA256SUMS.txt` is published beside
the archives. Verify it before running a downloaded archive.

Builds are unsigned and there is no installer. The Linux GUI uses the host
Wayland/X11 and Vulkan libraries. See [`docs/RELEASE.md`](docs/RELEASE.md) for
the maintainer runbook, and [`packaging/README.md`](packaging/README.md) for
archive contents and runtime requirements. The analyzer is advisory: a clean
report is not a safety verdict.

## Documentation

- `docs/SPEC.md` - the product specification.
- `docs/INTERFACES.md` - the frozen shared-type contract.
- `docs/ARCHITECTURE.md` - crates, data flow, threading.
- `docs/DETERMINISM.md` - what must be byte-identical and how.
- `docs/RULES.md` - rule catalog, defaults, and skip reasons.
- `docs/DIALECTS.md`, `docs/SIMULATION.md`, `docs/FIXTURES.md`,
  `docs/PERFORMANCE.md`, `docs/REUSE-MAP.md`, `docs/DECISIONS.md`,
  `docs/PROGRESS.md`.

## Scope

3-axis milling only. No lathe, plasma, 4th axis, ATC, probing UI, controller
connection, telemetry, or cloud. The analyzer is advisory: it catches G-code
programming mistakes, not chatter, cutter force, material defects, workholding
slip, or fixture collisions.

## License

MIT OR Apache-2.0. See `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE`. The
parent project and commit are recorded in `PROVENANCE.md`.
