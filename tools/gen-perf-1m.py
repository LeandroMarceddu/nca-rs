#!/usr/bin/env python3
"""Generate a deterministic large NC file for performance checks.

Usage:
  tools/gen-perf-1m.py /tmp/opencode/nca-1m.nc [lines]

The program is a bare file: no tool data, stock, machine or material, so the
M2 engagement pass is skipped by missing inputs. It is intended to measure the
parser, M1 rule pass, and report serialization.
"""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    target = Path(sys.argv[1])
    lines = int(sys.argv[2]) if len(sys.argv) == 3 else 1_000_000
    if lines < 1:
        print("lines must be positive", file=sys.stderr)
        return 2
    with target.open("w", encoding="ascii", newline="\n") as handle:
        handle.write("G21 G90\nT1 M6\nG0 Z5\nM3 S10000\nG1 Z-1 F100\n")
        for index in range(lines):
            x = (index % 200) / 10.0
            y = ((index // 200) % 200) / 10.0
            handle.write(f"G1 X{x:.1f} Y{y:.1f} F600\n")
        handle.write("G0 Z5\nM5\nM30\n")
    print(f"{target}: {target.stat().st_size} bytes, {lines} motion lines")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
