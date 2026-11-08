# inch-program

A G20 program with a header comment that says "1/4 inch flat". Programmed
coordinates are converted to millimetres in the motion records. The header
comment has no recognizable tool number, so it creates no tool hint; T1 is
resolved from `config.toml` at 6.350 mm.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Finding:

- `process.rest_material` info on line 10: 239.1 mm2 of material remains up to
  1.270 mm above the lowest final height inside the cleared footprint. This is
  an informational model result, not an error.

## Config override

`process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
slotting in this focused fixture.

## Coverage skips

- `geometry.feed_out_of_range`: `machine`
- `geometry.spindle_out_of_range`: `machine`
- `modal.spindle_direction`: `tool.right_hand:T1`
- `process.chipload`: `material`
- `process.flute_length`: `tool.flute_length:T1`
- `process.mrr`: `material`
- `process.shank_collision`: `tool.flute_length:T1`
- `process.stickout_ratio`: `tool.stickout:T1`
- `process.surface_speed`: `material`
- `travel.machine_envelope`: `machine`
- `travel.z_below_table`: `machine`
