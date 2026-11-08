# Changelog

All notable changes to nca-rs are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
nca-rs is pre-1.0: every version before 1.0.0 may change without notice and
carries no compatibility guarantee. Versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) from 1.0.0 onward.

## [Unreleased]

No changes yet after `v0.5.0`.

## [0.5.0] - 2026-11-08

First public release. nca-rs is a read-only NC viewer, deterministic preflight
analyzer, and 2.5D stock simulator for 3-axis CNC milling. It never connects to
a controller, never sends motion, and never edits G-code. Every release archive
is unsigned and there is no installer.

### Added

- **45-rule analyzer** - 34 M1 rules for parsing, modal state, travel,
  clearance, and exact geometry, plus 11 M2 process rules for chipload, surface
  speed, axial depth, radial engagement, removal rate, flute length, stickout,
  plunge feed, shank collision, thin walls, and rest material. Every finding
  carries a stable rule id, severity, basis, source line, and evidence.
- **Coverage honesty** - the report lists every rule as evaluated or skipped,
  with the missing input and reason. Rules never invent tool, stock, machine,
  or material data to force a result. There is no coverage percentage,
  progress bar, or green "SAFE" state.
- **VIEW and SIMULATE in a GPUI desktop app** - `nca-gui` opens `.nc`,
  `.gcode`, `.tap`, `.ngc`, and `.cnc` files, renders semantic toolpaths in 2D
  top and 3D orbit views, and replays the program as a 2.5D heightfield stock
  removal.
- **Diagnostics and coverage UI** - severity filters, per-finding details and
  evidence, source-line jump, click-to-zoom, severity gutter, and a coverage
  panel that names every skip.
- **Setup, config, and export** - TOML configuration for machine, stock, tools,
  material, tolerances, and rule overrides; a setup sheet for editable values
  and provenance; per-file tool tables keyed by source SHA-256; and JSON or
  Markdown report export.
- **CLI** - `nca check`, `nca view`, `nca simulate`, and `nca rules` with
  JSON/Markdown/text reports, `--fail-on`, `--to-line`, and PNG heightmap
  output. Exit codes are 0 (no finding at the threshold), 1 (finding at the
  threshold), and 2 (no report).
- **Fixtures and goldens** - 13 reviewed synthetic fixtures with byte-exact
  golden reports, positive and near-miss coverage, and a harness that prints
  the first differing JSON pointer. The real `box54` file is a determinism,
  cap, and performance regression; the Makera `LatchBase.nc` file is the first
  real vendor-post fixture.
- **Makera Z1 dialect and `.cnc` files** - a `makera` dialect for
  Smoothieware-derived `M400`, Makera `M851 S<n>`/`M852` auxiliary outputs, and
  controller-defined clearance `G28`/`G30`, plus `.cnc` file discovery and
  header tool-table hints.
- **Stock fill modes** - SIMULATE cycles GHOST (translucent block), SOLID
  (opaque block), and CUT ONLY (no uncut cap).
- **Tool shape in the simulation HUD** - the full label shows `FLAT`, `BALL`,
  `BULL`, `DRILL 118°`, or `VBIT 90°` with comment provenance, and an unknown
  shape is labeled `SHAPE? (FLAT)` instead of looking like a real flat cutter.
- **AUTO dialect detection in the app** - deterministic detection from strong
  vendor code markers and header comments before the first motion, with the
  evidence line recorded. Ambiguous or absent evidence keeps the assumed
  default, and an explicit configured dialect always wins.
- **Deterministic core** - libm-backed trigonometry, canonical JSON, stable
  ordering, SHA-256 over raw source bytes, and byte-identical reports and
  heightmaps for the same input, config, and ruleset version.
- **Release automation** - a tag-driven workflow builds `nca` and `nca-gui` for
  Linux x86_64, Windows x86_64, and macOS, writes `SHA256SUMS.txt`, and
  publishes the archives; `docs/RELEASE.md` documents the release gates.

### Changed

- The app uses lossy parsing so a partially supported program still renders
  geometry; unsupported blocks stay visible as parse diagnostics and in
  coverage.
- Semantic move kinds (13 in total) now drive painting, dash patterns, toggles,
  and hole markers instead of a generic segment kind.
- Tool overlays persist per source SHA-256 instead of per path, so moving or
  copying a file does not lose or collide the edited tool table.
- Analysis and long seeks use a worker generation gate: a stale result is
  discarded, and the UI stays responsive while work runs.
- The app defaults to inferred stock, labels the stock source and assumed
  dialect, and leaves machine/material unset until the operator supplies them.
- Release packaging uses the single `[workspace.package]` version and produces
  a normal GitHub Release rather than a draft.

### Fixed

- Lossy feedless cutting moves keep a motion with no feed; canned-cycle upward
  returns are classified as retracts; skipped blocks are retained on `Program`.
- Travel and stock checks use exact arc extents, including sweeps whose
  endpoints are inside a boundary but whose arc leaves it.
- Coverage headlines count evaluated and skipped rules without hiding skips;
  unknown rule ids are rejected instead of silently ignored.
- The source hash covers the raw bytes, including a UTF-8 BOM, so two files
  that differ only by a BOM do not share a tool overlay.
- Stock extents in the setup sheet and report are exact rather than rounded
  display values.
- Shaped cutter sweeps evaluate the profile in the moving-tool frame, so
  horizontal and angled Drill, V-bit, Ball, and Bull cuts no longer mirror
  about the segment start.
- The 3D vertical orbit direction was inverted to match pointer movement.

### Safety

- Read-only by construction: no serial port, controller connection, streaming,
  jog, DRO, or network path exists in the product.
- No green "SAFE" state and no coverage percentage. A clean report means no
  retained finding for the evaluated rules; it is not a safety verdict.
- Missing inputs produce a named skip, never an invented value. Model rules
  state their dependence on inferred or explicit stock.
- Input is bounded: file-size and coordinate limits, property tests for
  arbitrary bytes, no `unwrap` or panic on input in library code, and
  deterministic libm math on every platform.
- G-code is never rewritten. Export writes a report or heightmap image, not a
  modified program.

### Known limitations

- Camera 60 fps at 1M segments and the 15-30 Hz stock-mesh update target are
  unmeasured.
- Analysis and long seeks have no mid-run cancellation or progress; only stale
  results are discarded (D034).
- Entering SIMULATE builds the timeline and initial heightmap synchronously on
  the UI thread; a 1M-line first switch can pause the interface (D035).
- The 1M-line synthetic run takes 0.745 s but uses about 420 MiB, roughly
  430 B/motion, above the 250 B/motion goal (D030).
- The real `box54` file produces 477 findings from 31/45 evaluated rules; the
  model-rule findings are noisy and are not blessed as a golden report.
- Dedicated fixtures for arc-radius mismatch, tool change, inferred stock,
  explicit stock, thin wall, rest material, and a second manual-spindle
  configuration are still missing (D033).
- Dialect semantics outside `generic` and `makera` remain unverified against
  vendor manuals.
- 3-axis milling only. No lathe, plasma, 4th axis, ATC UI, probing UI,
  controller connection, telemetry, cloud, or auto-fix.
- Advisory only: the analyzer does not model chatter, cutter force, material
  defects, workholding slip, or fixture collisions. It is not a safety
  guarantee and has no hardware validation path.
- Builds are unsigned and have no installer. Linux GUI builds need host
  Wayland/X11 and Vulkan libraries; macOS and Windows builds are not signed.
  The macOS archive contains bare binaries, not a `.app` bundle.

[Unreleased]: https://github.com/LeandroMarceddu/nca-rs/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/LeandroMarceddu/nca-rs/releases/tag/v0.5.0
