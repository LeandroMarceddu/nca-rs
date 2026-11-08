# missing-feed

The first `G1` has no `F` anywhere before it. Expected:
`modal.feed_missing` on the plunge line.

Golden draft: the parser currently rejects a feedless cutting block with
`feed_rate_undefined`, so both `G1 Z-1` and `G1 X20` are skipped. The draft
therefore contains two `parse.unsupported` errors and no
`modal.feed_missing` finding. This is a parser/rule integration gap to review,
not a clean result.
