# Dialects

The dialect selects a small set of semantics. It never supplies tool data,
stock, or machine limits, and it never makes an unknown code safe.

## Verification status

No vendor documentation could be consulted while this table was first written.
Rows marked **unverified** must be checked against the vendor manual before the
rule that depends on them is considered stable. `Generic` is an explicit
conservative profile, not a claim about a real control.

## Semantics

| Dialect | `G4 P` unit | Canned retract | Arc formats | `G28`/`G30` | `G53` | Status |
|---|---|---|---|---|---|---|
| `generic` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R` | machine home via stored position | machine-coordinate move | defined profile |
| `grbl` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; `G18`/`G19` supported | stored home position | machine-coordinate move | unverified |
| `linuxcnc` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | stored home position | machine-coordinate move | unverified |
| `mach3` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | stored home position | machine-coordinate move | unverified |
| `fanuc` | milliseconds (`P`) | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | reference return | machine-coordinate move | unverified |

Notes:

- A `G4` without `P` is not a dwell; it is reported as an incomplete block.
- `G98` returns to the initial Z; `G99` returns to the R plane. A cycle that
  changes the R plane mid-run is reported when it is not representable.
- Radius-format arcs that are geometrically impossible are errors, never a
  guessed center.
- `G28`/`G30` and `G53` always produce machine-coordinate targets. They are
  flagged by `modal.machine_coords`; the toolpath shows the programmed target,
  not a simulated home search.
- `G93` inverse-time feed is recorded but feed-based rules skip it.

## Default dialect

`ParseConfig::dialect` defaults to `Generic`, and the parser emits one Info
`parse.dialect_assumed` finding per file. A configured dialect suppresses it.

## Tool comment conventions

Recognizers are conservative and fixture-driven. A recognizer must be justified
by a real fixture; this list is the complete generic set for v1:

| Form | Example | Unit required |
|---|---|---|
| `D=` / `DIA` | `D=6`, `DIA 3.175` | yes |
| Diameter symbol | `Ø6`, `⌀6` | yes |
| Millimetre suffix | `6mm`, `6 mm` | yes (self-declared) |
| Inch suffix | `1/4"`, `0.125 in`, `0.125"` | yes (self-declared) |
| Corner radius | `CR=0`, `CR 0.5` | yes, same rule as diameter |
| Flutes | `2F`, `3 flute`, `3FL` | no |
| Shape words | `flat`, `ball`, `bull`, `drill`, `v-bit` | no |

Rules:

- A bare number without a unit is ambiguous unless the program declares
  `G20`/`G21`. Ambiguous hints are dropped and produce
  `parse.tool_hint_ambiguous`.
- **Header-unit rule:** for a comment before the first motion, the unit may come
  from any `G20`/`G21` declared anywhere before the first motion, even if it
  appears after the comment. Fusion-style posts emit the tool table first and
  the modal preamble second; requiring an earlier declaration would discard
  every real header hint. A unit declared after the first motion never
  retroactively validates a header hint.
- **Unanchored header hint:** a header comment with a dimension/shape/flute
  count but no tool number attaches to the first `T` selected later in the
  header region (before the first motion), falling back to the nearest preceding
  header `T`. If no header `T` exists, the hint is dropped. A comment that names
  a tool (`T3`) is specific and wins per field over an unanchored hint. The
  header text is not used as a tool name.
- Only header comments before the first motion, and comments within
  `tool_hint_window` lines (default 3) of a `T`/`M6` block, are scanned.
- A comment may name a tool number (`T3`, `tool 3`); otherwise it attaches to the
  nearest preceding `T` word.
- Hints never override user configuration. `ToolTable::resolve` merges per field.
- Never infer tool size from toolpath geometry.
- Post-specific recognizers are added only after a real fixture proves them, and
  are documented in this table with the post name and source.

## Verified real-world examples

| Source | Comment | Result |
|---|---|---|
| `box54+lid 50cm.nc` (PrintNC, Fusion-style header, CRLF) | `(T1  D=3.1 CR=0 - ZMIN=-10 - flat end mill)` | T1 diameter 3.1 mm, flat, header-unit rule uses the later `G21` before the first motion |
| same | `(T2  D=6 CR=0 - ZMIN=-18 - flat end mill)` | T2 diameter 6.0 mm, flat |

