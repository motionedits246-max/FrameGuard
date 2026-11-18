#!/bin/sh
set -eu

runtime_dir=$(mktemp -d -t frameguard-session.XXXXXX)
cleanup() {
  fusermount3 -u "$runtime_dir/doc" 2>/dev/null || fusermount -u "$runtime_dir/doc" 2>/dev/null || true
  rm -rf "$runtime_dir"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$runtime_dir"
export XDG_CURRENT_DESKTOP=XFCE
xvfb-run -a dbus-run-session -- sh -c '
  openbox > /tmp/frameguard-window-manager.log 2>&1 &
  python3 "$1"
' sh "$(dirname "$0")/check-linux-open.py"
