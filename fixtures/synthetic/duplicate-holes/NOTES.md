# duplicate-holes

Drills (20, 20) twice with the same tool and Z range, then drills a different
hole at (40, 20). The first repeat is the positive case; the third hole is the
near-miss negative.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**.

Finding:

- `geometry.duplicate_hole` warning on line 11: it repeats the hole from line 8
  with tool T1, distance 0.000 mm, both at Z -6.000 mm.
- The hole at line 14 is 20 mm away and produces no finding.

## Config overrides

- `process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
  slotting in this focused fixture.
- `process.axial_depth.axial_doc_ratio = 2.0` accepts the deliberate 6 mm
  drilling depth with the 5 mm drill.

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
