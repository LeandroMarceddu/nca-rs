# unsupported-code

`M98 P1000` is a subprogram call. v1 does not execute subprograms or flow
control, so the lossy parser records the block and stops new geometry at that
line. Expected: a `parse.unsupported` error and `geometry_uncertain_after` set
to the M98 line; geometry rules must not evaluate past it.

G41/G42 is a different case: see `cutter-comp`. It keeps the programmed
centerline and does not truncate.
