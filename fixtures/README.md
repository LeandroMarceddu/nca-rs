# Fixtures

Fixture folders live under `fixtures/`:

```text
fixtures/
  synthetic/<name>/{source.nc,config.toml,expected.json,NOTES.md}
  real/<name>/{source.nc,config.toml,expected.json,NOTES.md}
```

Rules:

- `source.nc` is the complete program; never truncate it silently.
- `config.toml` uses the schema in `docs/INTERFACES.md`. Omit tables you do not
  need; an omitted `[stock]` means inferred stock.
- `expected.json` is the full golden report. It is never auto-blessed: the
  orchestrator reviews it against the source, config, and rule definitions
  before committing it.
- `NOTES.md` is required, states what the fixture proves, records the actual
  coverage, and names each missing input behind a skip.

See `docs/FIXTURES.md` for the target synthetic coverage and the review
process.

## Review status

All 13 synthetic fixtures carry an `expected.json` reviewed on 2026-11-08
against ruleset 1.0.0, which contains 45 rules. `nc-fixtures/tests/golden.rs`
requires exactly this set of names: a missing folder, a missing required file,
a missing synthetic golden, or an unexpected synthetic folder fails the suite.
`fixtures/real/box54-lid-50cm` is the only named exemption from the golden
requirement; it is locked by `crates/nc-fixtures/tests/real_file.rs` for
determinism, caps, and performance instead.

The committed corpus is:

| Fixture | Proves |
|---|---|
| `arc-smaller-than-tool` | internal arc below the cutter radius |
| `bare-file` | zero-config program, inferred stock, centerline simulation, explicit skips |
| `clean-pocket` | a complete setup produces no diagnostics |
| `cutter-comp` | G41 is recorded but centerline geometry continues |
| `drill-cycles` | G81/G83/G85 canned-cycle records |
| `duplicate-holes` | a repeated hole is reported once |
| `helical-bore` | descending G3 helixes and a flat full circle |
| `inch-program` | G20 conversion and a millimetre tool table |
| `manual-spindle` | spindle rules skip with `machine.spindle_manual` |
| `missing-feed` | the first cutting move has no feed |
| `overlapping-holes` | holes closer than the tool diameter |
| `rapid-through-stock` | rapid below safe Z and XY rapid inside stock |
| `unsupported-code` | lossy parse stops geometry and coverage stays honest |

## Config overrides

Overrides are intentional and are repeated in each `NOTES.md`:

- every focused fixture sets `process.radial_engagement.radial_engagement_ratio = 1.0`
  so full-width slotting is not the subject of the fixture;
- `clean-pocket` disables `process.thin_wall` and `process.rest_material`, which
  appear as `rule.disabled` skips;
- `drill-cycles` and `duplicate-holes` set
  `process.axial_depth.axial_doc_ratio = 2.0` because their drilling depth is
  deliberate.

`docs/FIXTURES.md` also lists target fixtures that are not committed yet; the
manifest and this table describe only the current, reviewed corpus.
