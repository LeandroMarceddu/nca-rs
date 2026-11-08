# overlapping-holes

Holes at (10, 10) and (12, 10) with a 6 mm tool overlap by 4 mm. Holes at
(30, 10) and (37, 10) are 7 mm apart, so their centres are farther apart than
the tool diameter and they are the near-miss negative.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Finding:

- `geometry.overlapping_holes` warning on line 8: the hole is 2.000 mm from the
  hole at line 7 with tool T1, closer than its 6.000 mm diameter.
- The hole at line 12 is 7 mm from line 11 and produces no finding.

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
