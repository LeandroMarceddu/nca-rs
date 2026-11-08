# rapid-through-stock

A safe cut followed by a rapid below the stock top, first plunging down and then
crossing the stock in XY.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Findings:

- `clearance.rapid_at_depth` warning on line 11: the Z-only rapid ends at
  Z -1.500 mm, below safe Z 2.000 mm.
- `clearance.rapid_at_depth` warning on line 12: the XY rapid also ends below
  safe Z.
- `clearance.rapid_through_stock` warning on line 12: the XY rapid crosses the
  stock footprint at Z -1.500 mm, below the stock top 0.000 mm.

The Z-only rapid on line 11 is not a through-stock XY rapid, so
`clearance.rapid_through_stock` does not report it.

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
