#!/usr/bin/env bash
# Spike 06: mount a fake camera card (FAT32 image with DCIM/) through udisks, exactly like a
# real card insertion, while the watcher runs; then unmount and detach.
#
# Run from a terminal in your *desktop* session (polkit only lets the active session mount),
# or with sudo. Image: /tmp/mm-card.img (built by the spike; see docs/spikes/06-card-detection.md).
set -euo pipefail
IMG=${1:-/tmp/mm-card.img}
BIN="$(dirname "$0")/target/release/card-detect"
LOG=/tmp/mm-card-detect.log

"$BIN" >"$LOG" 2>&1 &
WATCHER=$!
trap 'kill $WATCHER 2>/dev/null || true' EXIT
sleep 0.5

DEV=$(udisksctl loop-setup -f "$IMG" | grep -o '/dev/loop[0-9]*')
echo "attached $DEV"
sleep 2  # give a desktop automounter the chance to mount it first
if ! findmnt -n "$DEV" >/dev/null; then
  udisksctl mount -b "$DEV"
fi
findmnt -n -o TARGET,FSTYPE,OPTIONS "$DEV"
sleep 2
udisksctl unmount -b "$DEV"
sleep 1
udisksctl loop-delete -b "$DEV"
sleep 0.5
echo "---- watcher log ($LOG)"
cat "$LOG"
