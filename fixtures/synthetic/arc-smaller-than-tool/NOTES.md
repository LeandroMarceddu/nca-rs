# arc-smaller-than-tool

A 2 mm radius internal arc cut with a 6 mm cutter (3 mm radius). Expected:
`geometry.arc_smaller_than_tool` warning. Near-miss: a 4 mm radius arc would
still be smaller; a 6.1 mm radius arc would not.
