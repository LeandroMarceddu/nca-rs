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
| M2 engagement pass | slower is allowed; must be cancellable and show progress |
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

## Status

No numbers are recorded yet. Phase 0 only established the crate graph and the
scaffold. Measurements are taken after the parser and simulator milestones land,
and every deviation from the initial targets is recorded in `DECISIONS.md`.

## Degradation policy

Above a configurable input size:

- coarsen the heightmap cell size instead of allocating past the cap;
- skip the M2 engagement pass with a coverage skip reason, never silently;
- decimate display geometry through the render plan, never in the stored
  toolpath;
- keep the UI responsive by moving parse, analysis, and long seeks off the UI
  thread.
