# unsupported-code

`M98 P1000` is a subprogram call. v1 does not execute subprograms or flow
control, so the lossy parser records the block and stops new geometry at that
line. `geometry_uncertain_after` is 9; the later G1 lines are not treated as
trusted geometry.

G41/G42 is a different case: see `cutter-comp`. It keeps the programmed
centerline and does not truncate.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Findings:

- `parse.unsupported` error on line 9: `M98 P1000` is skipped as
  `unsupported_code`.
- `clearance.tool_end_in_material` warning on line 8: the last trusted motion
  ends at Z -1.000 mm, below the stock top.
- `modal.end_state` warning on line 8: the last trusted motion ends below safe
  Z.

The two end-state findings use line 8 because no trusted cutting motion exists
after the M98 and its G1 blocks are not analyzed.

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
