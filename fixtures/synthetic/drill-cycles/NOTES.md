# drill-cycles

G81, G83 with peck Q, and G85 across several holes. Expected: one MotionRecord
per hole, expanded lazily into rapid/feed/retract; `Drill`, `Peck`, and `Bore`
kinds. Duplicate-hole detection must use the hole records, not the expansion.

Golden draft: `clearance.rapid_at_depth` reports eight warnings (four per
cycle line) from the expanded cycle motions. The expansion's Z endpoints
(0, -2, -4, -6 mm) do not match the programmed `R2`/`Z-4` values and need
review before this golden is accepted.
