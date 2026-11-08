# clean-pocket

A correct rectangular pocket with a complete setup: machine, explicit stock,
tool, and material. The fixture proves that a well-formed program with enough
inputs is not punished by the rules.

## Golden

`expected.json` was reviewed on 2026-11-08 against the 45-rule registry.
Actual coverage: **43/45 evaluated, 2 skipped rules (2 records)**. There are no
diagnostics. The two skips are disabled rules, not missing inputs.

## Config overrides

- `process.radial_engagement.radial_engagement_ratio = 1.0` accepts full-width
  slotting for this roughing program.
- `process.thin_wall` is disabled. A square-corner pocket cut with a round tool
  inherently leaves corner rest and edge ridges inside the AABB cutting
  envelope; thin-wall coverage has its own target fixture.
- `process.rest_material` is disabled for the same reason.

Disabling the two rules is deliberate and is recorded in coverage as
`rule.disabled`.

## Coverage skips

- `process.rest_material`: `rule.disabled`
- `process.thin_wall`: `rule.disabled`
