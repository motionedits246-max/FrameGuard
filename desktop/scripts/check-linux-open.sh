#!/bin/sh
set -eu

runtime_dir=$(mktemp -d -t frameguard-session.XXXXXX)
trap 'rm -rf "$runtime_dir"' EXIT
export XDG_RUNTIME_DIR="$runtime_dir"
export XDG_CURRENT_DESKTOP=XFCE
xvfb-run -a dbus-run-session -- python3 "$(dirname "$0")/check-linux-open.py"
