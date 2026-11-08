# Simulation

`nc-sim` is a deterministic stock-removal preview, not a physical simulation.
It shows where the tool goes and what material the program leaves behind. It
does not model cutter force, deflection, chatter, heat, or workholding.

## Time

- Internal tick: exactly `1 / 120` s of simulated time.
- `advance(dt)` accumulates time and runs whole ticks, up to
  `max_ticks_per_advance` per call. The remaining fraction is never discarded.
- State that matters (`tick`, `tick_pose_mm`, heightmap, stock version, stats)
  changes only on a tick.
- The display pose is interpolated between ticks for smooth rendering. It is not
  used by tests.
- `seek_to_time(t)` targets `floor(t / TICK_S)`. Forward seeks are incremental;
  backward seeks reset and replay. A long seek runs on the worker thread; stale
  results are discarded, but v0.1 cannot interrupt one already in progress (D034).
- `step_motion()` advances to the end of the current span; `step_line()`
  advances to the next line with motion.

## Timeline

Built from `Program::expanded_motions()`:

| Motion | Duration |
|---|---|
| Feed/plunge/ramp cut | length / feed x 60 |
| Arc/helix | `hypot(radius x abs(sweep), axial_delta)` / feed x 60 |
| Rapid/retract | length / `rapid_rate_mm_min` x 60 |
| Dwell | `P` converted by dialect |
| Tool change | `tool_change_pause_s` (default 2 s) |

Prefix sums support binary search. A line-to-time index supports jump-to-line.
The timeline stores spans, not copies of motions.

## Cutting rules

- Cutting kinds: `Feed`, `Plunge`, `Ramp`, `ArcCw`, `ArcCcw`, `HelixCw`,
  `HelixCcw`, `Drill`, `Peck`, `Bore`.
- Non-cutting by default: `Rapid`, `Retract`, `Probe`, dwell, tool change.
- Crash preview (off by default) cuts rapids below the stock top and highlights
  them red.
- The cutter is applied between consecutive tick poses as a swept capsule, so a
  fast move leaves no gap. Arcs and helixes are subdivided along the exact curve
  with chord error at most `cell_mm / 4`.
- Shaped cutters evaluate their profile in the moving-tool frame
  (`cell - (from + d*t)`): Drill and V-bit cones, Ball spheres, Bull flat tip
  plus torus. Flat and centerline cutters are unaffected.
- Drills lower the hole to the tip depth and honor peck retracts.

## Grid and cutter

- Default cell 0.75 mm; the requested range is 0.5-1.0 mm. The total cell count
  is capped at 4,000,000.
- If the requested grid exceeds the cap, the cell size doubles until it fits.
  Above 2 mm the simulation still runs but is labeled **coarse**.
- A known tool uses its resolved diameter and shape. An unknown diameter uses
  `SimConfig::assumed_tool_diameter_mm`; the default `None` is a centerline
  trace one cell wide.
- An unknown shape is a flat cylinder, labeled `SHAPE? (FLAT)` in the
  simulation HUD.
- There is never a zero-diameter cutter.

## Stock

Stock comes from `StockModel`:

- Explicit stock is the user's origin and size.
- Inferred stock uses **cutting** bounds, so safe-Z rapids do not inflate it.
  XY margin is `max(2% of the larger cutting extent, 1 mm)`.
- If any cut is below Z0, the top is Z0 (`ZeroAtTop`).
- If all cuts are above Z0, the top is the highest cut + 2 mm, and the block is
  10 mm thick (`AllCutsAboveZero`).
- With no cutting moves, all path bounds are used (`NoCuttingMoves`).

### Limitation: inference assumes Z0 is the stock top

A program whose Z0 sits on the table, and whose cuts stay above it, will infer a
thin slab around the toolpath. This is a model, not a fact. The UI shows
`STOCK: INFERRED` with the inference basis in its tooltip, and every
stock-dependent finding reports `stock_source` and `Basis::Model`. Rules that
need a real stock declare `Explicit` and are skipped with reason
`stock.explicit`.

## Rendering

- 3D: heightfield mesh over the stock box, sampled from the simulation
  heightmap. 64x64 while playing, 128x128 when paused. Rebuild only when the
  heightmap version changes; update at 15-30 Hz while playing.
- 2D: shade cut cells by depth over the stock footprint; uncut stock is a light
  panel. This is the always-smooth fallback.
- Tool: a cylinder/holder at the current pose; a circle plus crosshair in 2D.
  Unknown size draws thin and is labeled `Ø?`.
- Path: traversed solid, future dimmed, current segment highlighted. `VIEW` mode
  shows the whole path normally.
- Legend: move kinds plus cut depth; toggles for stock, rapids, air cuts, future
  path, tool, warnings.

## HUD

- `TOOL SIZE ASSUMED` while an assumed size is active.
- Cut-volume and size-dependent statistics are marked "assumed".
- Drill markers use the assumed size and are labeled.
- The tool chip opens a popover with diameter, shape, and flutes so the user can
  fix the assumption without opening the setup sheet.

## Defaults

| Setting | Default |
|---|---|
| Cell size | 0.75 mm |
| Cell cap | 4,000,000 |
| Rapid rate | 5000 mm/min |
| Tool-change pause | 2 s |
| Crash preview | off |
| Render grid | 64 |
| Ticks per advance | 96 |
| Assumed diameter | none (centerline) |

## Determinism tests

- Different `dt` sequences to tick T produce the same `content_hash()`.
- Backward seek versus fresh replay to T produces the same hash.
- Different `max_ticks_per_advance` values produce the same hash.
- Arc, helix, and drill cuts hash the same on repeat runs.
- Rapids do not cut by default.
- Crash preview cuts and is reported.
- Stock inference ignores safe-Z rapids.
- A grid above the cap coarsens instead of failing.

## Performance

- Stock mesh updates at 15-30 Hz while playing; full resolution when paused.
- The path plan is not rebuilt on a pose-only update.
- Long seeks run off the UI thread; stale results are discarded. Mid-run progress and cancellation are a v0.2 follow-up (D034).
- Above a configurable size, coarsen the grid or skip the engagement pass with
  a coverage skip reason rather than stalling.
