# arc-smaller-than-tool

A 2 mm radius internal arc cut with a 6 mm cutter. The arc is smaller than the
3 mm cutter radius, so `geometry.arc_smaller_than_tool` must warn. A 3.1 mm
radius arc would be the near-miss negative; this fixture does not commit a
second source for it.

The source comment says 1.5 mm, but `G3 X34 Y10 I2 J0` is a 2.0 mm radius arc.
The golden reports the programmed geometry.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Finding:

- `geometry.arc_smaller_than_tool` warning on line 9: radius 2.000 mm is
  smaller than the 3.000 mm cutter radius of T1.

## Config override

`process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
slotting in this focused fixture. It is not the subject here.

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
