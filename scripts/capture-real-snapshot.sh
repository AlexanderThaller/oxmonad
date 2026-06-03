#!/bin/sh
set -eu

name="${1:-session}"
snapshot_dir="tests/real-snapshots"
golden="$snapshot_dir/$name.png"
actual="$snapshot_dir/$name.actual.png"
out="$actual"

if [ "${UPDATE_REAL_SNAPSHOTS:-}" ]; then
    out="$golden"
fi

if ! command -v grim >/dev/null 2>&1; then
    echo "error: grim is required for real compositor snapshots" >&2
    echo "install grim, then run: just real-snapshot" >&2
    exit 127
fi

cargo build
mkdir -p "$snapshot_dir"

tmpdir="$(mktemp -d)"
log="$tmpdir/oxmonad.log"
client_pid=""

cleanup() {
    if [ -n "$client_pid" ]; then
        kill "$client_pid" >/dev/null 2>&1 || true
    fi
    if [ -n "${oxmonad_pid:-}" ]; then
        kill "$oxmonad_pid" >/dev/null 2>&1 || true
    fi
    rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

RUST_LOG="${RUST_LOG:-info}" target/debug/oxmonad >"$log" 2>&1 &
oxmonad_pid="$!"

display=""
i=0
while [ "$i" -lt 100 ]; do
    if ! kill -0 "$oxmonad_pid" >/dev/null 2>&1; then
        echo "error: oxmonad exited before becoming ready" >&2
        sed -n '1,160p' "$log" >&2
        exit 1
    fi

    display="$(sed -n 's/^OXMONAD_WAYLAND_DISPLAY=//p' "$log" | tail -n 1)"
    if [ -n "$display" ]; then
        break
    fi

    i=$((i + 1))
    sleep 0.05
done

if [ -z "$display" ]; then
    echo "error: timed out waiting for oxmonad Wayland socket" >&2
    sed -n '1,160p' "$log" >&2
    exit 1
fi

if [ "${CLIENT_CMD:-}" ]; then
    WAYLAND_DISPLAY="$display" sh -c "$CLIENT_CMD" &
    client_pid="$!"
elif command -v alacritty >/dev/null 2>&1; then
    WAYLAND_DISPLAY="$display" alacritty --title oxmonad-snapshot >/dev/null 2>&1 &
    client_pid="$!"
fi

# Give the compositor and optional client a few frames to map and render.
sleep "${REAL_SNAPSHOT_DELAY:-1}"

WAYLAND_DISPLAY="$display" grim "$out" || {
    status="$?"
    echo "error: grim failed to capture from oxmonad" >&2
    sed -n '1,220p' "$log" >&2
    exit "$status"
}
echo "wrote $out"

if [ -z "${UPDATE_REAL_SNAPSHOTS:-}" ] && [ -f "$golden" ]; then
    echo "compare against $golden with your image diff tool or git diff"
elif [ -z "${UPDATE_REAL_SNAPSHOTS:-}" ]; then
    echo "no golden exists yet; run: just update-real-snapshot"
fi
