# helical-bore

Three full-circle descents and one flat full circle, all G3. Because G3 is
counter-clockwise, the descending arcs are `HelixCcw` and the final flat circle
is `ArcCcw`. Each descent combines a 3 mm radius sweep with an axial drop, so
its exact path length is radius x sweep plus axial travel.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**. There are
no diagnostics: this well-formed helical bore is not a clearance, feed, or
geometry violation. The golden locks the parsed motion stream and coverage.

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
