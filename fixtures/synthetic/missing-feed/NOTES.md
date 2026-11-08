# missing-feed

The first `G1` has no `F` anywhere before it (line 7), so the first cutting
motion must be reported as `modal.feed_missing`. The parser accepts the
feedless block and records the cutting motion; it does not reject the program.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **33/45 evaluated, 12 skipped rules (12 records)**.

Findings:

- `modal.feed_missing` error on line 7: the first cutting motion has no feed.
- `clearance.tool_end_in_material` warning on line 8: the program ends at
  Z -1.000 mm, below the stock top 0.000 mm.
- `modal.end_state` warning on line 8: the program ends below the safe Z.

The last two findings are true for this source: the program stops after
`G1 X20` at Z -1 without lifting.

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
- `process.plunge_feed`: `feed.reference`
- `process.shank_collision`: `tool.flute_length:T1`
- `process.stickout_ratio`: `tool.stickout:T1`
- `process.surface_speed`: `material`
- `travel.machine_envelope`: `machine`
- `travel.z_below_table`: `machine`
