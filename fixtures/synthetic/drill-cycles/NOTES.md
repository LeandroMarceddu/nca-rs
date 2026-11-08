# drill-cycles

G81, G83 with peck Q, and G85 across several holes. Expected: one MotionRecord
per hole, expanded lazily into rapid/feed/retract; `Drill`, `Peck`, and `Bore`
kinds. Duplicate-hole detection must use the hole records, not the expansion.
