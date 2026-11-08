# inch-program

Inch mode and a header comment with `1/4 inch`. Expected: coordinates converted
to millimetres in records; the tool hint resolves to 6.35 mm; `D=0.25` without a
unit would be ambiguous and must be dropped.

Golden draft: the program contains no `T`/`M6`, so `tool_inventory` reports
tool 0 with no fields; configured tool 1 is unused. The header comment has no
recognizable tool number, so it does not create a hint.
