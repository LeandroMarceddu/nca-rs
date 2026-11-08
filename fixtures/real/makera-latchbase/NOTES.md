# Makera Z1 `LatchBase.nc`

- Source: user-provided `LatchBase.nc` (7,550 lines, 168,946 bytes), generated
  by Makera Community Post Process Utility 0.9.1-RC1 for a Makera Z1 3-axis.
- SHA-256: `7aae6e520d549929ecc19fc869c82af192b648f4eb567921d0b4a68cf99ddb66`
- Dialect: `makera` (Makera firmware is Smoothieware-derived).

## Codes used

The post emits ordinary G-code plus:

- `M400` (Smoothieware): wait for queued moves to finish. No geometry.
- `M851 S100` (Makera): extended-port power on, `S` is PWM percent, not spindle
  RPM. The parser keeps the modal spindle speed unchanged.
- `M852` (Makera): extended-port power off.
- `G28` (Makera): controller-defined clearance position. The viewer records it
  as a non-motion command and does not invent a machine target.

Tool comments carry `D=`, `FL=`, `SL=`, and `TAPER=` fields. `G21` is declared
in the header before the first motion, so the bare millimetre values are
accepted under the header-unit rule.

## Golden status

No `expected.json`. This is a large real post used as a parse/analysis
regression: it must load with zero skipped blocks and no parse diagnostics.
The report is not blessed until the model rules are reviewed on this post.

## Review

Added and reviewed 2026-11-08.
