# Fixtures

Fixtures are the regression spine of the project. Each fixture folder is a
complete, reproducible input:

```text
fixtures/synthetic/<name>/
  source.nc       program text
  config.toml     parse, machine, stock, tools, material, rule overrides
  expected.json   reviewed golden report (or absent until reviewed)
  NOTES.md        what the fixture proves and where it came from
```

## Rules

- A golden report is never auto-blessed. The orchestrator reviews the report
  against the source and the rule definitions before committing it.
- `expected.json` contains the full report, including `coverage` and `truncated`.
- A fixture that is expected to skip rules must name each missing input in
  `NOTES.md`; the golden coverage is part of the test.
- Fixtures that use a real post name it in `NOTES.md`, with permission and
  license information.
- Synthetic fixtures are deterministic and self-contained. They must not depend
  on a machine, a controller, or a clock.

## Required synthetic coverage

| Fixture | Proves |
|---|---|
| `bare-file` | no comments, no config; opens, renders, simulates with centerline; skipped rules named |
| `clean-pocket` | no diagnostics on a correct program |
| `missing-feed` | first cutting move without feed |
| `duplicate-holes` | same tool/XY/Z drilled twice |
| `overlapping-holes` | centres closer than the tool diameter |
| `rapid-through-stock` | rapid below safe Z and XY rapid inside stock |
| `arc-smaller-than-tool` | internal arc below tool radius |
| `arc-radius-mismatch` | start/end radius disagreement |
| `tool-change` | T/M6 pairing and tool inventory |
| `unsupported-code` | lossy parse, geometry stops, coverage remains honest |
| `inferred-stock` | inference basis and stock_source evidence |
| `explicit-stock` | explicit-stock rules now run |
| `manual-spindle` | spindle rules skipped with `spindle_manual`, and evaluated without |
| `inch-program` | G20 units and tool comment units |
| `helical-bore` | exact helix length and timeline |
| `drill-cycles` | G73/G81/G82/G83/G85/G86/G89 expansion |
| `thin-wall` | explicit stock and wall detection threshold |
| `rest-material` | explicit stock and standing material |

## Real fixtures

Ask the user for, at minimum:

- 3-5 files from their real post-processors, naming the post and machine.
- duplicate/repeated drilling;
- deep pocket or heavy stepdown;
- 3D surfacing with a ball nose;
- helical bore or arcs;
- a G41/G42 program;
- inch units;
- macros or subprograms;
- a manual-spindle post and, if available, a Fanuc-style post;
- header comments with tool data **and** files with no tool data.

Real files are stored under `fixtures/real/<name>/` with the same four files.
The user is not blocked on providing them; synthetic coverage starts the suite.

`box54-lid-50cm` currently has no `expected.json`. Its report is large and
model-heavy (hundreds of clearance/engagement findings), so it is used as a
determinism, cap and performance regression (`crates/nc-fixtures/tests/real_file.rs`)
until the model rules are tuned and the report is reviewed for a golden.

## Comparison

`nc-fixtures` loads a folder, parses `config.toml`, runs analysis, and compares
the serialized report with `expected.json` byte for byte. A mismatch prints the
first differing JSON pointer and the surrounding values, never a whole-file
diff. Golden reports use pretty JSON with a trailing newline.

## Determinism

Run each fixture twice and compare bytes. Reports never contain an absolute
path, a timestamp, a hostname, or a locale-formatted number.
