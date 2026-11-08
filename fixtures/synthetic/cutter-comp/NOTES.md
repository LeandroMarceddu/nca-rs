# cutter-comp

`G41 D1` is recorded and **not applied**. The programmed centerline continues
through the compensated section, so the geometry after `G41` is present and
`geometry_uncertain_after` stays null. Tool-radius-dependent rules must skip
while compensation is active.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **35/45 evaluated, 10 skipped rules (10 records)**. There are
no report diagnostics. The parser's centerline-compensation warning is not part
of the canonical report.

## Config override

`process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
slotting in this focused fixture.

## Coverage skips

- `geometry.feed_out_of_range`: `machine`
- `geometry.spindle_out_of_range`: `machine`
- `process.chipload`: `material`
- `process.flute_length`: `tool.flute_length:T1`
- `process.mrr`: `material`
- `process.shank_collision`: `tool.flute_length:T1`
- `process.stickout_ratio`: `tool.stickout:T1`
- `process.surface_speed`: `material`
- `travel.machine_envelope`: `machine`
- `travel.z_below_table`: `machine`
