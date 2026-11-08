# box54 + lid 50 cm

Real part program, supplied by the project owner on 2026-11-08. The original
file is `/home/leandro/Desktop/Dev/box54+lid 50cm.nc`; it is stored here
unmodified as `source.nc`.

## Facts

- 119,433 lines, 2,415,803 bytes, ASCII, CRLF line endings.
- Millimetres (`G21`), absolute (`G90`), feed per minute (`G94`), `G17` initial
  plane with `G18`/`G19` sections.
- Machine vendor comment: `PrintNC`.
- Post: not stated in the file. The header comment style
  (`(T1  D=3.1 CR=0 - ZMIN=-10 - flat end mill)`) is Fusion 360-like.
- Tools: `T2` = 6 mm flat, `T1` = 3.1 mm flat, both named only in header
  comments. The program has two `M6` changes and one `G81`/`G80` group.
- Explicit coordinate extremes: X -3.61..503.9, Y -6.875..202.583,
  Z -18.0..15.0. Feeds F1000/F3000, spindle S24000.
- No `G41`/`G42`, no macros, no subprograms.

## Purpose

The primary real-world regression and performance fixture:

- CRLF input, header tool hints with an explicit `D=` dimension, unknown stock
  and machine data;
- `G18`/`G19` arcs and full-circle helixes in a real post;
- a large program (target: parse plus records under 1 s, analysis M1 under
  10 s);
- zero-config behavior: inferred stock, comment-derived tool sizes, and a
  coverage list that names every skipped rule.

`expected.json` is intentionally absent for now. The report is large and
model-heavy (hundreds of clearance and engagement findings), so this fixture is
used as a determinism, cap and performance regression instead. A golden is added
only after the model rules are tuned and the report is reviewed line by line;
it is never auto-blessed.

Measured 2026-11-08: release `nca check` median 0.509 s, ~103 MiB RSS; debug
1.80 s. See `docs/PERFORMANCE.md`.

## Permission

Provided by the project owner for use as a test fixture. No third-party
license is known; do not redistribute this file outside the project without
the owner's permission.
