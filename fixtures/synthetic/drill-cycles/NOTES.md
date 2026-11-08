# drill-cycles

G81 across two holes, G83 with a peck Q, and G85 across one hole. Each hole is
stored as one `MotionRecord`; the rapid/feed/retract sub-moves are generated on
demand. Duplicate and clearance checks read the stored records, so internal
peck retracts are not treated as clearance findings.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **34/45 evaluated, 11 skipped rules (11 records)**. There are
no diagnostics: the 4 mm drill, the R2 safe plane, and the intentional deep
G83 pass do not violate the enabled rules under this config.

## Config overrides

- `process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
  slotting in this focused fixture.
- `process.axial_depth.axial_doc_ratio = 2.0` accepts the 8 mm G83 hole, which
  is 2.0 drill diameters deep.

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
