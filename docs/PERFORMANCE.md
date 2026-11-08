# Performance

## Reference machine

Recorded before any measurement, as required:

| Component | Value |
|---|---|
| CPU | 13th Gen Intel Core i7-13650HX, 14 cores / 20 threads |
| GPU | Intel Raptor Lake-S UHD Graphics + NVIDIA GeForce RTX 4060 Max-Q |
| RAM | 23 GiB |
| OS | CachyOS Linux, kernel 7.3.1 |
| Rust | 1.98.1, edition 2024 |
| Build | release, thin LTO, codegen-units 1 |

## Targets (initial, to be revised with evidence)

| Metric | Target |
|---|---|
| `MotionRecord` size | <= 128 bytes |
| Parse + motion records + timeline, 100k lines | <= 1 s |
| Parse + motion records + timeline, 1M lines | <= 5 s |
| Peak memory | <= ~250 bytes per motion; 2M motions under ~1 GB |
| Analysis M1, 1M lines | <= 10 s |
| M2 engagement pass | target: cancellable with progress; current: stale-result cancellation only, no mid-run progress (D034) |
| Camera orbit/pan/zoom, 1M segments at 1920x1080 | 60 fps via the plan cache |
| Stock mesh update while playing | 15-30 Hz |
| Heightmap grid | <= 4M cells by default |

## Measurement protocol

1. Build with `cargo build --release`.
2. Generate deterministic inputs with the fixture generator, not a real clock.
3. Measure wall time over at least three runs and report the median.
4. Report peak RSS from `/usr/bin/time -v`.
5. Measure UI frame time with the GPUI frame timer over a recorded 10 s orbit;
   report the 95th percentile, not only the average.
6. Record the exact command and the fixture name next to every number.

## Measurements

### 2026-11-08 - real `box54` file

Input: `fixtures/real/box54-lid-50cm/source.nc`, 119,433 lines, 2,415,803 bytes,
119,393 motion records. Command:

```sh
./target/release/nca check fixtures/real/box54-lid-50cm/source.nc   --config fixtures/real/box54-lid-50cm/config.toml --format json
```

| Run | Wall time | Exit |
|---|---:|---:|
| 1 | 0.522 s | 0 |
| 2 | 0.501 s | 0 |
| 3 | 0.509 s | 0 |

Median **0.509 s**, peak RSS **105,516 KB (~103 MiB)**. This includes parsing,
motion records, the 45-rule analysis and JSON serialization. Debug build:
**1.80 s** for the same workload.

Report: 477 retained diagnostics, `31/45` rules evaluated, per-rule caps active
(`truncated` records dropped findings for `clearance.rapid_at_depth`,
`geometry.arc_smaller_than_tool`, `process.radial_engagement` and
`process.plunge_feed`). No absolute path appears in the JSON.

### Against the initial targets

| Metric | Target | Measured | Verdict |
|---|---|---|---|
| Parse + records + timeline, 100k lines | <= 1 s | 0.51 s release, including analysis | pass |
| Peak memory, 119k motions | <= ~250 B/motion | ~903 B/motion (~103 MiB total) | **miss; D030 accepted** |
| M1 analysis, 1M lines | <= 10 s | 0.745 s for 34 M1 rules (M2 skipped) | pass |
| MotionRecord size | <= 128 bytes | enforced by test | pass |
| Camera 60 fps, 1M segments | 60 fps | not measured | pending |
| Stock mesh update | 15-30 Hz | not measured | pending |

Release UI frame-time and stock-mesh update measurements are still required
before the performance section is final. The 1M-line measurement is below.

Known UI gap: entering SIMULATE builds the `Simulation` (timeline + initial
heightmap) synchronously on the UI thread. For 1M-line files this is an
unmeasured pause; moving it to a worker with a loading state is a v0.2
follow-up (D035).

### One-million-line synthetic

Generated deterministically with `tools/gen-perf-1m.py` (19,000,055 bytes,
1,000,000 motion lines). Bare config: no tool data, stock, machine or material,
so the M2 engagement pass is skipped by missing inputs; the 34 M1 rules and the
report path run.

| Run | Wall time | Exit | Peak RSS |
|---|---:|---:|---:|
| 1 | 0.745 s | 0 | 430,656 KB (~420 MiB) |

This meets the 1M-line <= 5 s target comfortably. It is above the ~250 B/motion
memory goal (~430 B/motion all-in), because `ProgramLine` retains its source text
and `Word` vector for every line. A 2M-motion program is projected at roughly
860 MiB, still below the 1 GB ceiling; D030 records the miss and the optimization
follow-up.

## Degradation policy

Above a configurable input size:

- coarsen the heightmap cell size instead of allocating past the cap;
- skip the M2 engagement pass with a coverage skip reason, never silently;
- decimate display geometry through the render plan, never in the stored
  toolpath;
- keep the UI responsive by moving parse, analysis, and long seeks off the UI
  thread.
