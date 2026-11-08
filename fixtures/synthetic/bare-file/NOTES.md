# bare-file

Proves the zero-config path: no comments, no tool data, no explicit stock, and
no machine or material configuration. The program opens, runs every rule that
needs no setup, and lists each missing input. The simulator uses a labeled
centerline cutter because no diameter is known.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **26/45 evaluated, 19 skipped rules (24 records)**. Five rules
contribute two records each, so the distinct rule count is smaller than the
record count.

Findings:

- `modal.spindle_stopped` error on lines 4, 5, 6, 7, and 8: cutting motions
  run without the spindle commanded on.
- `modal.tool_zero` warning on lines 4, 5, 6, 7, and 8: cutting motions use T0
  and no tool is selected.

## Config override

`process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
slotting. Without a tool diameter the rule skips anyway.

## Coverage skips

- `geometry.arc_smaller_than_tool`: `tool.diameter:T0`
- `geometry.feed_out_of_range`: `machine`
- `geometry.overlapping_holes`: `tool.diameter:T0`
- `geometry.spindle_out_of_range`: `machine`
- `modal.spindle_direction`: `tool.right_hand:T0`
- `modal.tool_unknown`: `tool.data`
- `process.axial_depth`: `tool.diameter:T0`
- `process.chipload`: `material`
- `process.chipload`: `tool.flutes:T0`
- `process.flute_length`: `tool.flute_length:T0`
- `process.mrr`: `material`
- `process.mrr`: `tool.diameter:T0`
- `process.radial_engagement`: `tool.diameter:T0`
- `process.rest_material`: `stock.explicit`
- `process.shank_collision`: `tool.flute_length:T0`
- `process.stickout_ratio`: `tool.diameter:T0`
- `process.stickout_ratio`: `tool.stickout:T0`
- `process.surface_speed`: `material`
- `process.surface_speed`: `tool.diameter:T0`
- `process.thin_wall`: `stock.explicit`
- `process.thin_wall`: `tool.diameter:T0`
- `travel.machine_envelope`: `machine`
- `travel.stock_bounds`: `stock.explicit`
- `travel.z_below_table`: `machine`
