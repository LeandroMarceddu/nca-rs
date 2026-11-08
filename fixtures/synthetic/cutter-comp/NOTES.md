# cutter-comp

`G41 D1` is recorded, warned about, and **not applied**. The programmed
centerline continues through the compensated section, so geometry after `G41`
is present and `geometry_uncertain_after` stays null. Tool-radius-dependent
checks must skip while compensation is active.

Expected: no truncation and no error; the report may carry an Info/warning for
the centerline policy once that rule lands.
