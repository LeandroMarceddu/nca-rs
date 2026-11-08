# Reuse map

Evidence was gathered read-only from `ghs-rs` at commit
`4bfbda60553661df677eaab15c323ec428c8690b`. Line ranges name the parent file
and the symbol; they were spot-checked before this map was accepted.

## Decisions

| Parent asset | Decision | Evidence and notes |
|---|---|---|
| `ghs-app/src/theme.rs` | Copy + adapt | `Palette` at 23-52; dark 84-113, light 115-144, high-contrast 152-181; `Palette::apply` 189-255 mutates the global GPUI theme. Replace `ghs_config::ThemeMode` (import at 17) with a local enum and add move/cut color tokens. |
| `ghs-app/src/layout.rs` | Copy + simplify | Breakpoints at 20-72: `GAP=12`, `WIDE_MIN_WIDTH=1120`, `WIDE_LEFT_MIN_WIDTH=1600`, `PORTRAIT_ASPECT=1.15`, `TOUCH_TARGET=48`, top-bar heights 56/108. `ShellLayout::for_viewport` 152-208. Remove the six-workspace width assumptions. |
| `ghs-app/src/viewer.rs` | Copy + refactor | `ViewerHandle` 141-336 (`Rc<RefCell<Viewer>>`, not Send); `view` 405-528; `Viewer::prepare` 811-874; `paint` 877-928; plan caches 664-740. Keep cameras, gestures, stock box, marker; replace the DRO marker with the simulation pose; add a heightfield stock mesh. |
| `ghs-app/src/shell.rs` | Selective copy | Generic helpers only: `card` 24675-24683, `overlay_layer` 24702-24719, `section_label` 24906-24912, `chip` 24925-24940, `chip_button` 25019-25042, list-row helpers 23796-24124. Do not copy the 26,516-line `Shell` struct or its controller panels. |
| Program list in `shell.rs` | Adapt | `fn program` 7846-7920 with `uniform_list("program-list", ...)` 7889-7905; row painting 23796-23912. Current-line/search model lives in `state.rs` 745-937 and 4108-4117; rebuild a small version. |
| `ghs-app/src/file_browser.rs` | Adapt | `MAX_PROGRAM_BYTES`/extension list 26-33; `ProgramFile` 141-154; directory listing and bounded read 159-358; `FileBrowser` model 365-576. Drop i18n and engine handoff. |
| `ghs-app/src/state.rs` | Do not copy | Controller/telemetry view-model; incompatible with a read-only app. Build a small `NcaState`. |
| `ghs-app/src/{live,session,connection}.rs` | Do not copy | Controller/session/live-link code. |
| `ghs-app/src/{probe,settings,offsets,tools,utilities}.rs` | Do not copy | Sender features. Extract the `ProgramCheck` idea (utilities 2195-2416, scan 2418-2679) into deterministic rules; build the sparse `Tool` overlay, not a tool library. |
| `ghs-app/src/i18n.rs` | Defer | English-only v1; centralize strings for a later adapter. |
| `ghs-app/src/config.rs` | Do not copy | Build `NcaConfig` for stock/tools/rules/UI. |
| `ghs-app/src/main.rs` | Do not copy | Replace with the `nca-gui` bootstrap; keep the window-size flag pattern 247-278. |
| `ghs-gcode` | Copy + extend | Parser, lexer, block model, planner, state: `lib.rs` 177-220 lists the modules. Add lossy mode, `MotionRecord`, `MoveKind`, tool hints, input limits. Remove the write-side transforms. |
| `ghs-viewer` | Copy | Dependency-free geometry and cameras; `lib.rs` 50-68 lists exports. `StockBox` is static (stock.rs 45-265), not a stock-removal model. |
| `ghs-core` | Copy minimal | `Position` position.rs 20-156, `Units` units.rs 13-79, `Wcs`/`CoordinateSystem` wcs.rs 14-189, `Axis`/`Axes` axis.rs 15-264. Controller report models are not copied. |

## What the parent does not provide

- There is **no stock-removal simulator**. `ghs-sim` is a sans-IO controller
  protocol simulator; `StockBox` is a static display box. `nc-stock` and
  `nc-sim` are new code.
- There is **no tool table in `ghs-core`**; only `ToolEntry` (tools.rs 15-76).
  The sender's table is `ghs-config/controller.rs:151-219`. The sparse `Tool`
  overlay in `nc-core` is new.
- There is **no deterministic report model**. `ProgramCheck` (utilities.rs
  2195-2416) is the closest inspiration: it is read-only, bounded, and
  explanatory, but it is not a rule engine and has no coverage surface.

## Code removed from the fork

| Parent module | Reason |
|---|---|
| `ghs-gcode/src/compensation.rs` | Height-map rewrite; writes G-code. |
| `ghs-gcode/src/rotate.rs`, `scale.rs`, `rewrite.rs` | Program rewriting; writes G-code. |
| `ghs-app` sender modules | Controller connection, DRO, jog, MDI, probing, settings, offsets, macros, session recording. |
| `ghs-gcode` flow-control and macro execution | Out of scope; the lossy parser reports and stops. |

Expression and parameter evaluation stays: the parser needs it to read valid
programs without executing macros or subprograms.

## Visual language to preserve

- Dark shop-floor palette with light and high-contrast variants.
- 48 px touch targets, 11 px labels, 14-16 px body, large numeric readouts.
- Card, chip, sheet, and virtualized list patterns.
- Three-column wide layout; two-column compact; single-column portrait.
- The viewer dominates the window; controls stay in the top bar and sheets.
