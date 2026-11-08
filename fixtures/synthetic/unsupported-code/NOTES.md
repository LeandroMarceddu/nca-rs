# unsupported-code

`G41` is recognized but not applied. Geometry after it is not trusted. Expected:
a cutter-compensation warning and `geometry_uncertain_after` set to the `G41`
line. Source-level rules may still run after it; geometry rules must not.
