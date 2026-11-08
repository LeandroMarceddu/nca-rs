# Rules

This document is the authority for rule ids, defaults, requirements, and skip
reasons. `Rule::requirements()` must agree with this table; the Coverage panel
and `nca rules` are generated from it, so they cannot drift.

## Conventions

- Rule ids are stable. A change in meaning requires a new id and a ruleset
  version bump.
- Every rule has a default severity, a `Basis`, and a stock requirement.
- `Exact` rules depend only on program text or exact geometry.
- `Model` rules depend on inferred stock, engagement, or material data.
- A rule that cannot run returns `Skipped { missing_input, reason }`; the engine
  records it in coverage. Disabled rules are skipped with
  `missing_input = "rule.disabled"`.
- Missing-input keys are stable and use the form `tool.<field>:T<n>`,
  `machine`, `material`, `stock`, `stock.explicit`, `rule.disabled`,
  `machine.spindle_manual`, `machine.max_feed_mm_min`,
  `machine.max_spindle_rpm`, `tool.data`, `feed.reference`, or
  `feed.inverse_time`.
  `stock` means the stock extents are degenerate or the grid cannot resolve the
  feature; `stock.explicit` means a rule requires user-supplied stock and the
  stock is inferred.
- Rules whose `requirements()` include a tool field are `Basis::Model` when the
  value can be comment-derived (`geometry.overlapping_holes`,
  `geometry.arc_smaller_than_tool`, `modal.spindle_direction`). Rules that use
  only program text, exact geometry, or explicit machine/stock configuration
  are `Basis::Exact`.
- Tools are referenced by `u32` ids from `MotionRecord.tool`.
- `clearance.rapid_at_depth` reads `thresholds["clearance_mm"]` and falls back
  to `Tolerances::clearance_mm` when absent.
- `travel.machine_envelope` emits one finding per violating axis on a motion.

## Default tolerances (`Tolerances`)

| Field | Default | Meaning |
|---|---|---|
| `arc_radius_mm` | 0.05 | maximum start/end radius difference |
| `min_segment_mm` | 0.01 | shorter cutting segments are "tiny" |
| `hole_xy_mm` | 0.05 | XY distance below which two holes are the same |
| `clearance_mm` | 2.0 | safe clearance above the stock top |

These match `Tolerances::default()`. A malformed value falls back to the
default rather than disabling a rule.

## M1 - program, modal, travel, clearance, geometry

| Rule id | Default | Basis | Stock | Required inputs | Catches |
|---|---|---|---|---|---|
| `parse.unsupported` | Error | Exact | - | - | A block the lossy parser skipped; raw line in evidence |
| `parse.dialect_assumed` | Info | Exact | - | - | No dialect configured |
| `parse.tool_hint_ambiguous` | Info | Exact | - | - | Comment dimension with no determinable unit |
| `modal.units_missing` | Warning | Exact | - | - | Motion before `G20`/`G21` |
| `modal.distance_missing` | Warning | Exact | - | - | Motion before `G90`/`G91` |
| `modal.feed_missing` | Error | Exact | - | - | First cutting move with no feed |
| `modal.feed_nonpositive` | Error | Exact | - | - | `F0` or negative feed |
| `modal.spindle_stopped` | Error | Exact | - | `machine` (for `spindle_manual`; otherwise evaluated) | Cutting with spindle stopped |
| `modal.spindle_speed_zero` | Warning | Exact | - | - | `M3`/`M4` with `S0` or no `S` |
| `modal.spindle_direction` | Info | Model | - | `tool.right_hand:T<n>` | `M4` with a right-hand tool |
| `modal.tool_unknown` | Warning | Exact | - | tool table or hints non-empty | `T` not in the resolved table |
| `modal.tool_zero` | Warning | Exact | - | - | Cutting with `T0` |
| `modal.tool_change_mismatch` | Warning | Exact | - | - | `T` without `M6`, or `M6` without `T` |
| `modal.machine_coords` | Warning | Exact | - | - | `G53`/`G28`/`G30` or machine-coordinate targets (not Makera clearance `G28`/`G30`) |
| `modal.probe_in_job` | Warning | Exact | - | - | `G38.x` probe in a normal job |
| `modal.g92_unrestored` | Warning | Exact | - | - | `G92` active at program end |
| `modal.wcs_change` | Warning | Exact | - | - | WCS change after cutting starts |
| `modal.no_program_end` | Warning | Exact | - | - | No `M2`/`M30` |
| `modal.end_state` | Warning | Model | inferred ok | stock | Spindle/coolant on, or tool below safe Z at end |
| `modal.block_delete_unsafe` | Warning | Exact | - | - | `/` on a retract, spindle, or tool-change line |
| `travel.machine_envelope` | Error | Exact | - | `machine` | Motion outside configured travel |
| `travel.z_below_table` | Error | Exact | - | `machine` | Z below the configured table |
| `travel.stock_bounds` | Warning | Exact | explicit | `stock.explicit` | Cutting outside stock XY |
| `clearance.rapid_at_depth` | Warning | Model | inferred ok | stock | `G0` below `stock_top + clearance` |
| `clearance.rapid_through_stock` | Warning | Model | inferred ok | stock | XY rapid inside the stock |
| `clearance.retract_below_stock` | Warning | Model | inferred ok | stock | Retract plane below the stock top |
| `clearance.tool_end_in_material` | Warning | Model | inferred ok | stock | Program ends with the tool below the stock top |
| `geometry.duplicate_hole` | Warning | Exact | - | - | Same tool, same XY, overlapping Z |
| `geometry.overlapping_holes` | Warning | Model | - | `tool.diameter:T<n>` | Hole centres closer than the tool diameter |
| `geometry.tiny_segment` | Info | Exact | - | - | Cutting segment under `min_segment_mm` |
| `geometry.arc_radius_mismatch` | Error | Exact | - | - | Start/end radius differ beyond `arc_radius_mm` |
| `geometry.arc_smaller_than_tool` | Warning | Model | - | `tool.diameter:T<n>` | Internal arc radius below the tool radius |
| `geometry.feed_out_of_range` | Warning | Exact | - | `machine` (max feed) | Programmed feed above the maximum |
| `geometry.spindle_out_of_range` | Warning | Exact | - | `machine` (max rpm) | Spindle above the maximum |

### M1 definitions

- **Cutting move:** any `MotionRecord` whose `kind.is_cutting()` is true.
- **Safe Z:** `stock.top_z_mm + tolerances.clearance_mm`.
- **Rapid at depth:** a `Rapid` whose endpoint Z is below safe Z and whose path
  is not a pure retract that starts and ends above safe Z. A rapid that only
  moves upward is a retract; it is not flagged.
- **Rapid through stock:** a `Rapid` whose XY segment intersects the stock
  footprint while its Z is below the stock top. Z-only rapids are exempt.
- **Exact travel extent:** every travel rule uses the motion's exact swept
  extent: the start/end points plus `ArcRecord` extrema for that plane. An arc
  whose endpoints are inside the envelope but whose sweep leaves it is a
  finding. The same rule applies to stock XY and table-Z checks.
- **Retract below stock:** a `Retract` whose end Z is below the stock top.
- **Duplicate hole:** same tool, XY distance `<= hole_xy_mm`, and the Z ranges
  overlap. Canned records use their bottom Z; drilling moves use their end Z.
- **Overlapping holes:** different centres with XY distance `< tool diameter`,
  evaluated only between holes that use the same tool.
- **Arc smaller than tool:** the arc radius minus tool radius is negative beyond
  `arc_radius_mm`; skipped while cutter compensation is active.
- **Tool change mismatch:** a `T` block without `M6` or an `M6` without a
  selected `T`; evaluate within the same tool-change group.
- **Feed missing:** only the first cutting motion is checked. A present but
  non-positive feed belongs to `modal.feed_nonpositive`, not this rule. An
  inverse-time first cut (`G93`) is skipped with `feed.inverse_time` because a
  mm/min feed is undefined by design.
- **Spindle stopped:** `modal.spindle_stopped` is skipped when
  `machine.spindle_manual` is true; `modal.spindle_speed_zero` is skipped for
  the same reason because a manually started spindle has no authoritative
  programmed speed.
- **Machine coordinates:** `modal.machine_coords` scans source words for
  `G53`/`G28`/`G30`. `MotionRecord` has no machine-coordinate flag, so a
  hand-built program cannot express this without source words; see D022.
  In the `makera` dialect, `G28`/`G30` are controller-defined clearance moves
  and are excluded; `G53` and the `G28.1`/`G30.1` set-home forms still fire.
- **WCS change after cutting:** a `G54`-`G59.3` change whose line is after the
  first cutting motion.
- **End state:** spindle on, coolant on, or the final tool Z below safe Z.

## M2 - process rules

| Rule id | Default | Basis | Stock | Required inputs | Catches |
|---|---|---|---|---|---|
| `process.chipload` | Warning | Model | - | material, `tool.flutes:T<n>` | Feed / (RPM x flutes) outside the material range |
| `process.surface_speed` | Warning | Model | - | material, `tool.diameter:T<n>` | RPM x pi x D outside the material range |
| `process.axial_depth` | Warning | Model | inferred ok | stock, tool diameter | Axial DOC above `axial_doc_ratio x D` |
| `process.radial_engagement` | Warning | Model | inferred ok | stock, tool diameter | Contact/radial engagement above `radial_engagement_ratio x D` |
| `process.mrr` | Warning | Model | inferred ok | stock, material, tool diameter | Removal rate above the material maximum |
| `process.flute_length` | Warning | Model | inferred ok | `tool.flute_length:T<n>` | Axial depth greater than the flute length |
| `process.stickout_ratio` | Warning | Model | - | `tool.stickout:T<n>`, `tool.diameter:T<n>` | Stickout / diameter above `stickout_ratio` |
| `process.plunge_feed` | Warning | Model | - | feed and spindle | Plunge feed above `plunge_feed_ratio` of the cut feed |
| `process.shank_collision` | Warning | Model | inferred ok | `tool.flute_length:T<n>`, shank/holder data | Shank or holder enters the stock |
| `process.thin_wall` | Warning | Model | explicit | `stock.explicit`, tool diameter | Standing wall thinner than `min_wall_mm` |
| `process.rest_material` | Info | Model | explicit | `stock.explicit` | Stock left above `rest_material_mm` inside the cutting footprint |

### M2 threshold defaults

All are configurable through `RuleSettings::thresholds`.

`process.radial_engagement` evaluates only side-cutting kinds (`Feed`, `Ramp`,
`ArcCw`, `ArcCcw`, `HelixCw`, `HelixCcw`). A plunge, drill, peck, or bore is
fully engaged by definition and is not a radial-engagement finding.

| Key | Default | Rule |
|---|---|---|
| `axial_doc_ratio` | 1.0 | `process.axial_depth` (1 x tool diameter) |
| `radial_engagement_ratio` | 0.5 | `process.radial_engagement` (50% of diameter) |
| `stickout_ratio` | 8.0 | `process.stickout_ratio` |
| `plunge_feed_ratio` | 0.5 | `process.plunge_feed` |
| `min_wall_mm` | 0.8 | `process.thin_wall` |
| `rest_material_mm` | 0.5 | `process.rest_material` |
| `chipload_margin` | 0.0 | `process.chipload` (0 = use the material range exactly) |
| `surface_speed_margin` | 0.0 | `process.surface_speed` |
| `mrr_margin` | 0.0 | `process.mrr` |

### `process.thin_wall` definition

After the full-program z-map pass, scan the final heightmap for a run of
standing cells between **two cut regions**. A run is a wall only when both
sides are cut material; a cut-to-stock boundary is the part edge, not a wall.
The reported width is the run length in cells times the cell size. If the grid
cell is coarser than `min_wall_mm`, the rule skips with `stock` because the
resolution cannot support the measurement. It requires explicit stock because
inferred stock is fitted to the cuts.

### `process.rest_material` definition

After the full-program pass, a cell is *cut* when its final height is below the
stock top. Flood-fill the uncut cells from the grid border; only uncut cells
that are unreachable are **enclosed** by cuts. A connected enclosed region
whose height exceeds `lowest_final_height + rest_material_mm` is reported once,
with area, highest height, centroid, and the nearest cutting line. Material
outside the swept footprint (for example the square corner left by a round tool)
is not counted.

## Evidence keys

Common evidence keys, typed as `EvidenceValue`:

| Key | Type | Used by |
|---|---|---|
| `stock_source` | Text (`inferred`/`explicit`) | every stock-dependent rule |
| `inference_basis` | Text (`zero_at_top`/`all_cuts_above_zero`/`no_cutting_moves`) | stock-dependent rules on inferred stock |
| `tool` | Int | tool-dependent rules |
| `tool_source` | Text (`config`/`comment:L<n>`) | findings that used a hint |
| `feed_mm_min` | Float | feed rules |
| `spindle_rpm` | Float | spindle rules |
| `start_mm`, `end_mm` | Point3 | motion rules |
| `axis` | Text (`X`/`Y`/`Z`) | `travel.machine_envelope`, `travel.z_below_table` |
| `value_mm`, `min_mm`, `max_mm` | Float | `travel.machine_envelope`, `travel.z_below_table` |
| `extent_min_mm`, `extent_max_mm` | Float (envelope/table) or Point3 (stock bounds) | exact swept extent including arcs |
| `kind` | Text (`MoveKind::label`) | motion rules |
| `depth_mm` | Float | depth and clearance rules |
| `first_line`, `second_line` | Int | `geometry.duplicate_hole`, `geometry.overlapping_holes` |
| `first_mm`, `second_mm` | Point3 | hole rules |
| `distance_mm`, `tolerance_mm` | Float | hole rules |
| `diameter_mm`, `tool_diameter_mm` | Float | tool-diameter rules |
| `length_mm`, `threshold` | Float | `geometry.tiny_segment` |
| `radius_mm`, `end_radius_mm`, `delta_mm` | Float | `geometry.arc_radius_mismatch` |
| `shortfall_mm` | Float | `geometry.arc_smaller_than_tool` |
| `axial_doc_mm`, `radial_doc_mm` | Float | `process.axial_depth`, `process.radial_engagement` |
| `contact_angle_deg` | Float | `process.radial_engagement` |
| `mrr_mm3_min` | Float | `process.mrr` |
| `ratio` | Float | process threshold rules |
| `area_mm2`, `region_cells` | Float/Int | rest and thin-wall regions |
| `highest_mm`, `height_above_lowest_mm`, `lowest_final_height_mm` | Float | `process.rest_material` |
| `wall_mm`, `cell_mm` | Float | `process.thin_wall` |
| `centroid_mm` | Point3 | rest and thin-wall regions |
| `measured` | Float | every threshold rule |
| `threshold` | Float | threshold rules |

## Why this matters

Every rule carries one or two plain-language sentences for the details panel.
They describe the physical consequence, not the implementation. Example:
"Rapids below the stock top can drag the cutter through material at rapid rate;
the machine has no chance to react."

## Coverage

- `total` is the number of rules in the registry, including disabled ones.
- `evaluated` is the number that returned `Findings` (zero findings included).
- Every other rule appears in `skipped` with a stable key and a reason.
- The UI shows plain counts. It never shows a percentage, a progress bar, or a
  green "safe" state.
