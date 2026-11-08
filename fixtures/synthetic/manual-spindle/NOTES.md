# manual-spindle

No `M3`/`M4`, because the spindle is started by hand. With
`spindle_manual = true`, `modal.spindle_stopped` and `modal.spindle_speed_zero`
must skip and name `machine.spindle_manual` as the missing input.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Finding:

- `process.rest_material` info on line 7: 324.0 mm2 of material remains up to
  1.000 mm above the lowest final height inside the cleared footprint.

The two spindle rules are skipped for the manual-spindle reason. This fixture
commits only the `spindle_manual = true` variant; the evaluated-without variant
is not a separate golden.

## Config override

`process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
slotting in this focused fixture.

## Coverage skips

- `geometry.feed_out_of_range`: `machine.max_feed_mm_min`
- `geometry.spindle_out_of_range`: `machine.max_spindle_rpm`
- `modal.spindle_direction`: `tool.right_hand:T1`
- `modal.spindle_speed_zero`: `machine.spindle_manual`
- `modal.spindle_stopped`: `machine.spindle_manual`
- `process.chipload`: `material`
- `process.flute_length`: `tool.flute_length:T1`
- `process.mrr`: `material`
- `process.shank_collision`: `tool.flute_length:T1`
- `process.stickout_ratio`: `tool.stickout:T1`
- `process.surface_speed`: `material`
