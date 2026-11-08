# Dialects

The dialect selects a small set of semantics. It never supplies tool data,
stock, or machine limits, and it never makes an unknown code safe.

## Verification status

The `generic` profile is an explicit conservative profile, not a claim about a
real control. The `makera` row was checked against the Makera and Smoothieware
tables on 2026-11-08, and the `grbl` additional-code list was checked against
the grblHAL wiki. Rows marked **unverified** still need their vendor manual
checked before the rule that depends on them is considered stable. Sources are
listed at the end of the Makera section.

## Semantics

| Dialect | `G4 P` unit | Canned retract | Arc formats | `G28`/`G30` | `G53` | Status |
|---|---|---|---|---|---|---|
| `generic` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R` | machine home via stored position | machine-coordinate move | defined profile |
| `grbl` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; `G18`/`G19` supported | stored home position | machine-coordinate move | unverified |
| `linuxcnc` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | stored home position | machine-coordinate move | unverified |
| `mach3` | seconds | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | stored home position | machine-coordinate move | unverified |
| `fanuc` | milliseconds (`P`) | honor `G98`/`G99` | `I`/`J`/`K` and `R`; all planes | reference return | machine-coordinate move | unverified |
| `makera` | seconds | honor `G98`/`G99` (not listed by the vendor) | `I`/`J`/`K`; `G17`/`G18`/`G19` | controller-defined clearance move, no invented target | machine-coordinate move | vendor M/G table checked 2026-11-08; canned cycles unlisted |

Notes:

- A `G4` without `P` is not a dwell; it is reported as an incomplete block.
- `G98` returns to the initial Z; `G99` returns to the R plane. A cycle that
  changes the R plane mid-run is reported when it is not representable.
- Radius-format arcs that are geometrically impossible are errors, never a
  guessed center.
- For every dialect except `makera`, `G28`/`G30` and `G53` produce
  machine-coordinate targets. They are flagged by `modal.machine_coords`; the
  toolpath shows the programmed target, not a simulated home search. In
  `makera`, `G28`/`G30` are a clearance command and are neither drawn nor
  flagged as a machine-coordinate move.
- `G93` inverse-time feed is recorded but feed-based rules skip it.

## Makera Z1 / Carvera

Makera firmware is Smoothieware-derived, so the Makera profile accepts the
Smoothieware motion codes already covered by the shared parser and adds the
auxiliary-output family used by Makera posts:

| Code | Meaning | Viewer handling |
|---|---|---|
| `M400` | wait for queued moves to finish | non-motion; no modal change |
| `M851 S<n>` | extended-port power on; `S` is PWM percent | non-motion; `S` never changes spindle RPM |
| `M852` | extended-port power off | non-motion |
| `M7` | airflow on | mapped to the coolant-mist state |
| `M9` | airflow off | mapped to coolant off |
| `G28` | controller-defined clearance position | non-motion; the target is not invented |

Unknown Makera auxiliary codes (vacuum, light, probe charging, laser, ATC, and
so on) are not silently accepted. A lossy run reports the skipped block. The
laser and ATC families are out of scope for this 3-axis mill viewer.

`G28` is the only vendor-documented clearance form. `G30` is treated the same
way for safety; the vendor page does not list it.

Sources checked 2026-11-08:

- Makera supported codes: <https://wiki.makera.com/en/supported-codes>
- Smoothieware supported G-codes: <https://smoothieware.org/supported-g-codes>
- grblHAL additional G/M codes (for the existing `grbl` profile):
  <https://github.com/grblHAL/core/wiki/Additional-G--and-M-codes>

## Automatic detection

The app's setup sheet defaults to `AUTO` and calls
`nc_gcode::detect_dialect(source)` on load. Detection is evidence-based:

- `M851`/`M852` outside comments selects `makera`.
- Header comments with `makera`, `carvera`, `model: z1`, or
  `community post process utility` select `makera`.
- Header comments naming `grbl`/`grblhal`, `linuxcnc`, `mach3`/`mach4`, or
  `fanuc` select that dialect.
- `M400` alone is not Makera evidence; it is shared by several controls.
- Conflicting header markers return no detection. The app then keeps `Generic`
  with the assumed chip, rather than guessing.

The app shows the detected dialect and the evidence line, and a manual choice
overrides AUTO. The CLI remains explicit: `[parse] dialect` selects the
dialect; `generic` with an omitted key is assumed.

## Default dialect

The parser's `ParseConfig::dialect` defaults to `Generic`, and the parser emits
one Info `parse.dialect_assumed` finding per file. A configured dialect
suppresses it.

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

