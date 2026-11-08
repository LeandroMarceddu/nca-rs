# Screenshots

Visual review is part of acceptance. The reference OS is Linux; other platforms
must compile and pass non-visual tests.

## Required captures

| View | Size | State |
|---|---|---|
| VIEW, 2D top | 1280x800 | a file with arcs and holes |
| VIEW, 3D orbit | 1280x800 | stock box visible |
| VIEW, 2D top | 1920x1080 | same file |
| SIMULATE | 1280x800 | 0%, 25%, 50%, 100% |
| SIMULATE, assumed tool | 1280x800 | `TOOL SIZE ASSUMED` visible |
| Diagnostics | 1280x800 | one warning selected, evidence visible |
| Coverage | 1280x800 | evaluated/skipped counts and reasons |
| Setup sheet | 1280x800 | tools/stock provenance and editable fields |
| Export sheet | 1280x800 | JSON selected, path field, write action |
| Partial geometry | 1280x800 | unsupported block, parse.unsupported selected |
| Compact | 1024x600 | two columns |
| Portrait | 1080x1920 | single scrolling column |

## Never capture on the host desktop

The development desktop runs a KWin script that remembers and restores window
sizes. It overrides the app's `--size`; do not try to fix that in the app.

Always capture in an isolated nested session:

- empty `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_CACHE_HOME`;
- a private `XDG_RUNTIME_DIR` with mode 700;
- a private `dbus-run-session`;
- `kwin_wayland --virtual --no-lockscreen --scale 1` at the target size;
- the app launched with `--fullscreen`.

The repo helper does all of this:

```sh
cargo build -p nc-app
tools/capture-nca.sh 1280x800 /tmp/opencode/nca-1280x800.png
tools/capture-nca.sh 1920x1080 /tmp/opencode/nca-1920x1080.png
```

Rules:

- Never leave `cargo run` or the app in the foreground. Build first, launch in
  the background, capture, kill, and verify.
- Verify the PNG with `identify`; a capture command can exit 0 and write
  nothing.
- Use `timeout 90` around the nested session. Never leave a long `sleep` inside
  the `dbus-run-session` wrapper; it prevents the private bus from being reaped.
- After a capture, check `pgrep -af 'nca-gui|kwin_wayland --virtual|spectacle'`.
  Kill only nested `kwin_wayland --virtual`, the test `nca-gui`, and
  `spectacle`. Never kill the host `kwin_wayland`, its Xwayland, or the real
  session bus.
- If the nested capture fails, inspect `<root>/spectacle.log` and
  `<root>/app.log`. The fallback is host X11/Xwayland with the actual restored
  size recorded, not a fake target size.

## Nested method

`tools/capture-nca.sh [SIZE] [OUT]` is the verified path. It wraps the whole
session in `timeout 90` and uses `dbus-run-session`, because `spectacle` can
exit 0 without writing a file when no capture service is available.

If a capture tool changes, the equivalent manual sequence is:

```sh
SIZE=1280x800
W=${SIZE%x*}; H=${SIZE#*x}
ROOT=/tmp/opencode/nca-shot-env
rm -rf "$ROOT"; mkdir -p "$ROOT"/{config,data,cache,runtime}
chmod 700 "$ROOT/runtime"
export XDG_CONFIG_HOME="$ROOT/config" XDG_DATA_HOME="$ROOT/data" \
       XDG_CACHE_HOME="$ROOT/cache" XDG_RUNTIME_DIR="$ROOT/runtime"
export W H
timeout 90 dbus-run-session -- bash -c '
  export WAYLAND_DISPLAY=wayland-0 XDG_CURRENT_DESKTOP=KDE
  kwin_wayland --virtual --no-lockscreen --width "$W" --height "$H" --scale 1 \
    --socket wayland-0 >"$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
  KWIN=$!; sleep 2
  target/debug/nca-gui --fullscreen >"$XDG_RUNTIME_DIR/app.log" 2>&1 &
  APP=$!; sleep 4
  spectacle --fullscreen --background --nonotify \
    --output /tmp/opencode/nca-live.png >"$XDG_RUNTIME_DIR/spectacle.log" 2>&1 || true
  kill "$APP" "$KWIN" 2>/dev/null || true
'
identify /tmp/opencode/nca-live.png
```

## Method

1. Build `nc-app` first.
2. Load the fixture named in the capture list (via the app's file browser or a
   future `--open` flag).
3. Capture the virtual output, not a desktop region.
4. Save under `docs/images/` with a descriptive name, for example
   `view-2d-1280x800.png`.
5. Never commit an image that contains a user's private path, file content, or
   workstation chrome.

## Visual checks

- The viewer dominates the window; the shell is compact.
- Text is readable at 11 px labels and 14-16 px body.
- Touch targets are at least 48 px in wide and compact layouts.
- Color is never the only signal: every motion kind has a stroke pattern and a
  legend label.
- Red is reserved for errors and unsafe motion.
- Coverage shows plain counts and a skipped list; no percentage, progress bar,
  or green "safe" state.
- The dark, light, and high-contrast palettes all keep contrast.

Screenshot comparison is manual in v1. Automated pixel comparison is a later
option because GPU and font rendering differ across machines.
