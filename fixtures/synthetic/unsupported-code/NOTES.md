# unsupported-code

`G41` is recognized but not applied. Per D006 it is a supported modal state,
so it does not truncate geometry and `geometry_uncertain_after` stays null.

Golden draft: the cutter-compensation rule and `parse.unsupported` are not in
the current registry, so the draft has no diagnostics. It still proves that
`G41` is accepted without stopping or changing the centerline geometry.
