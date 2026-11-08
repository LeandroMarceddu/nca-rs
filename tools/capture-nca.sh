#!/usr/bin/env bash
# Capture nca-gui in an isolated nested KWin session.
#
# The development desktop runs a KWin script that restores remembered window
# sizes and overrides the app's --size. This script avoids the host session
# entirely: private XDG directories, a private D-Bus session, and a virtual
# KWin output.
#
# Usage:
#   tools/capture-nca.sh [SIZE] [OUT]
#   SIZE defaults to 1280x800; OUT defaults to /tmp/opencode/nca-SIZE.png
#
# The caller must build first:
#   cargo build -p nc-app
#
# Never run the app in the foreground on the host desktop. The trap kills the
# nested compositor and the app. The script prints the identify result and
# checks that no nca-gui process remains.

set -euo pipefail

SIZE="${1:-1280x800}"
OUT="${2:-/tmp/opencode/nca-${SIZE}.png}"
BIN="${BIN:-target/debug/nca-gui}"

if [[ ! -x "$BIN" ]]; then
  echo "missing binary: $BIN (run: cargo build -p nc-app)" >&2
  exit 1
fi

W="${SIZE%x*}"
H="${SIZE#*x}"
ROOT="/tmp/opencode/nca-shot-env-${SIZE}"
rm -rf "$ROOT"
mkdir -p "$ROOT/config" "$ROOT/data" "$ROOT/cache" "$ROOT/runtime"
chmod 700 "$ROOT/runtime"

export XDG_RUNTIME_DIR="$ROOT/runtime"
export XDG_CONFIG_HOME="$ROOT/config"
export XDG_DATA_HOME="$ROOT/data"
export XDG_CACHE_HOME="$ROOT/cache"
export NCA_W="$W"
export NCA_H="$H"
export NCA_OUT="$OUT"
export NCA_BIN="$(realpath "$BIN")"
export NCA_ARGS="${NCA_ARGS:-}"

cleanup() {
  pkill -f 'kwin_wayland --virtual' 2>/dev/null || true
  pkill -f "$NCA_BIN" 2>/dev/null || true
}
trap cleanup EXIT

timeout 90 dbus-run-session -- bash -c '
  set -u
  export WAYLAND_DISPLAY=wayland-0
  export XDG_CURRENT_DESKTOP=KDE

  kwin_wayland --virtual --no-lockscreen \
    --width "$NCA_W" --height "$NCA_H" --scale 1 \
    --socket wayland-0 >"$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
  KWIN=$!
  sleep 2

  "$NCA_BIN" --fullscreen $NCA_ARGS >"$XDG_RUNTIME_DIR/app.log" 2>&1 &
  APP=$!
  sleep 4

  spectacle --fullscreen --background --nonotify --output "$NCA_OUT" \
    >"$XDG_RUNTIME_DIR/spectacle.log" 2>&1 || true

  kill "$APP" 2>/dev/null || true
  kill "$KWIN" 2>/dev/null || true
  wait "$APP" 2>/dev/null || true
  wait "$KWIN" 2>/dev/null || true
' || {
  echo "nested session failed; inspect $ROOT" >&2
  exit 1
}

if ! identify "$OUT" 2>/dev/null; then
  echo "capture failed; check $ROOT/spectacle.log and $ROOT/app.log" >&2
  exit 1
fi

if pgrep -af 'nca-gui' | grep -v pgrep >/dev/null; then
  echo "WARNING: an nca-gui process remains" >&2
  pgrep -af 'nca-gui' || true
else
  echo "no nca-gui process remains"
fi
