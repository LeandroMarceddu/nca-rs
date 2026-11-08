# Determinism

## Guarantee

For the same input bytes, the same configuration, and the same ruleset version:

- the serialized JSON report is byte-identical on the same OS and architecture;
- the heightmap `content_hash()` at the same simulated tick is identical on the
  same OS and architecture.

Cross-platform identity is a goal, not yet a claim. CI compares golden reports
and heightmap hashes on Linux, macOS, and Windows. Any boundary-case difference
is recorded in `DECISIONS.md`; it is never hidden by loosening a tolerance.

## Math

- Every trig call (`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`), plus
  `sqrt`, `hypot`, `pow`, `floor`, `ceil`, `round`, `trunc`, and `abs`, goes
  through `nc_core::math`, which wraps `libm`.
- No `f32` arithmetic in library code. Heightmap cells are **stored** as `f32`;
  computation is `f64`, and the store conversion is the only rounding boundary.
- No fused-multiply-add intrinsics, no platform-specific vector math, and no
  fast-math flags.

## Ordering

- Any collection that reaches a report, a hash, or a rendered legend is a
  `BTreeMap`, `BTreeSet`, or a sorted `Vec`.
- `HashMap`/`HashSet` may be used for membership only when iteration order never
  reaches output.
- Diagnostics sort by `(severity_rank, line, column, rule_id)`; skips sort by
  `(rule_id, missing_input)`.
- Tool inventory sorts by tool number; evidence sorts by key.

## No ambient inputs

- No wall clock, randomness, locale, process id, hostname, or environment value
  appears in a report.
- `InputInfo.file_name` is the file name only. Absolute paths are forbidden.
- The report carries no frame rate, render size, or UI state.
- Persistence may read the clock for UI timestamps, but those values never enter
  a report or a golden fixture.

## Report quantization

| Value | Precision | Example |
|---|---|---|
| Coordinates | 0.001 mm | `12.345` |
| Feeds | 0.1 mm/min | `800.0` |
| Spindle | 1 rpm | `12000` |
| Angles | 0.01 degrees | `90.00` |
| Percentages | 0.1 percent | `12.5` |

Values are kept as `f64` internally. At the serialization boundary they are
rounded and written with a fixed decimal count; the formatting never depends on
the shortest-round-trip float algorithm.

## Simulator ticks

- The internal tick is exactly `1.0 / 120.0` simulated seconds.
- `advance(dt)` accumulates wall time and runs whole ticks. At most
  `max_ticks_per_advance` run per call; the remainder stays in the accumulator.
- `tick_pose_mm`, the heightmap, the stock version, and statistics change only
  on a tick.
- `pose_mm` is display interpolation and is excluded from determinism tests.
- `seek_to_time(t)` targets tick `floor(t / TICK_S)`. Forward seeks are
  incremental; backward seeks reset and replay.

## Required tests

- Report determinism: analyze twice, compare JSON bytes.
- Report path hygiene: no absolute path appears in the serialized report.
- Heightmap determinism: reach tick T with different `dt` sequences and compare
  `content_hash()`.
- Seek determinism: backward seek versus fresh replay to the same tick.
- Tick splitting: different `max_ticks_per_advance` values reach the same hash.
- Arc/helix and drill cuts reach the same hash on repeat runs.
- Stock inference ignores safe-Z rapids.
- Parser property test: arbitrary bytes never panic.
- Diagnostic line numbers are always within `1..=line_count`.

## Known non-goals

- Cross-platform bit identity for inputs that exercise libm boundary cases is
  not claimed until CI proves it.
- Rendering is not pixel-deterministic across GPUs, font stacks, or window
  systems. Screenshot tests run on one reference OS.
