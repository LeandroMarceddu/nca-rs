# helical-bore

Three full-circle helixes at decreasing Z and one flat full circle. Expected:
`HelixCw` for the descending arcs (G3 is CCW, so `HelixCcw`) and `ArcCcw` for the
final flat circle. Exact length is radius x sweep combined with axial travel.

Golden draft: no implemented rule reads helix geometry yet, so the draft has no
diagnostics; it records the parsed motion stream and coverage only.
