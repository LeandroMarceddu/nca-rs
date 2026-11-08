# Architecture

## Product shape

One read-only application with two modes over the same canvas:

```text
                 +-------------------------------+
  file bytes --> | nc-gcode: Program + MotionRecord |
                 +---------------+---------------+
                                 |
              +------------------+------------------+
              |                                     |
     display Toolpath                     MotionRecord stream
              |                                     |
     nc-app 2D/3D canvas                 +-----------+-----------+
                                         |                       |
                                   nc-analysis               nc-sim
                                   rules -> Report        ticks -> Heightmap
                                         |                       |
                                         +-----------+-----------+
                                                     |
                                              nc-app panels
```

## Crates

| Crate | Responsibility |
|---|---|
| `nc-core` | Coordinates, units, WCS, tool overlay, machine limits, material presets, dialects, libm-backed math |
| `nc-viewer` | Dependency-free geometry, cameras, render plans, `MoveKind` taxonomy and styles |
| `nc-gcode` | Lexer, block parser, planner, modal state, exact motion records, lossy mode, comments and tool hints |
| `nc-stock` | Stock inference, 2.5D heightmap, cutter shapes, deterministic sweep |
| `nc-sim` | Timeline, 1/120 s tick engine, snapshots, seek |
| `nc-analysis` | Rule contract, requirements/skip handling, engagement pass, deterministic report |
| `nc-app` | GPUI shell, canvas, panels, sheets, file IO, persistence, worker threads, playback clock |
| `nc-cli` | `nca check`, `nca view`, `nca simulate`, `nca rules`; no GPUI |
| `nc-fixtures` | Fixture loading and golden-report comparison |

Dependency direction is acyclic and points downward only:

```text
nc-core <- nc-viewer <- nc-gcode <- nc-stock <- nc-sim
                              \         \
                               \         +-> nc-analysis
                                +-------------> nc-app
                                               nc-cli
```

`nc-gcode` re-exports `MoveKind`, `AnnotationKind`, and `Bounds3` so downstream
crates do not import `nc-viewer` directly.

## Threading

- **UI thread:** GPUI rendering and input only. It never parses a file, runs a
  rule, or replays a long seek.
- **Worker thread:** file read, parse, analysis, and long simulation seeks. It
  reports progress and accepts a cancel flag.
- **Playback clock:** the app's frame loop advances the `Simulation` with the
  elapsed wall time; the simulation itself is pure and deterministic.
- No async runtime. A bounded `crossbeam-channel` plus a worker thread is
  enough for a local desktop tool.

## Data flow

1. The user opens a file. The app reads it with a size limit and computes its
   SHA-256.
2. The parser produces a `Program` with display geometry, `MotionRecord`s,
   annotations, comments, and tool hints.
3. The viewer builds a `RenderPlan` from the display toolpath; the plan cache
   rebuilds only on program, camera, or viewport change.
4. The analyzer builds one `AnalysisModel`, evaluates rules, and returns a
   `Report`.
5. `SIMULATE` constructs a `Simulation` from the same records and the inferred
   or explicit stock.

## Determinism boundaries

| Boundary | Rule |
|---|---|
| Math | All trig through `nc_core::math` |
| Ordering | `BTreeMap`/`BTreeSet` or sorted `Vec` for any output path |
| Report | Quantized values, fixed decimals, file name only |
| Simulation | State changes only on 1/120 s ticks; heightmap hash is the oracle |
| UI | Interpolation is display-only and never feeds the report |

## Failure policy

- A malformed block is a finding, not a crash. The lossy parser keeps the last
  valid modal state and records the block.
- An unsupported state-changing construct stops new geometry and marks the rest
  uncertain.
- A rule that cannot run is skipped with a stable missing-input key.
- A file over the size limit is refused cleanly (exit code 2 in the CLI) or
  truncated with a visible notice.
- Library code has no `unwrap`, `expect`, or `panic!` on input data.

## Persistence

Local only, next to the platform config directory:

- Window state, recent files, theme, last opened sheet.
- Per-file tool inventory keyed by the input SHA-256, never by path.
- No telemetry, no history uploaded anywhere.

## Testing layers

1. Unit tests per crate and per rule (positive plus near-miss).
2. Property tests for the parser and for report line ranges.
3. Golden reports per fixture folder.
4. Determinism tests (report bytes, heightmap hash at fixed tick).
5. Screenshot tests on Linux at 1280x800 and 1920x1080.
6. Cross-platform CI for fmt, clippy, test, build, and license policy.
