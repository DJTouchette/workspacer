#!/usr/bin/env bash
# Restage every native-app capture on the landing page (shots/native-*.webp).
#
# The window is the real wks-native binary, drawn into a private Xvfb display
# at 2x and driven by apps/native/scripts/smoke.py. What it shows is the
# fixture hub from `native-harness serve`: the same fixture `wks-native --demo`
# runs in-process, plus --catalog and --rich-transcript so the sessions have
# believable names and the transcript has a spawn card and its wakes in it.
# No agent is launched, no credentials are read and no model is called.
#
# Needs: Xvfb, openbox, xdotool, cwebp, python3, and a Vulkan driver the
# virtual display can use (Mesa's lavapipe: mesa-vulkan-drivers on Debian,
# vulkan-swrast on Arch). Build the two binaries first:
#
#   (cd apps/native && cargo build --locked --bin wks-native --bin native-harness)
#
# A capture's size is the app window in logical pixels; the file is twice
# that. Keep the sizes in step with the width/height attributes in index.html.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
native=$root/apps/native
bin=${WKS_NATIVE_BIN:-$native/target/debug/wks-native}
harness=${WKS_HARNESS_BIN:-$native/target/debug/native-harness}
out=$root/landing/shots
display=:${LANDING_SHOTS_DISPLAY:-97}

for tool in Xvfb openbox xdotool cwebp python3; do
  command -v "$tool" >/dev/null || { echo "capture-shots: missing $tool" >&2; exit 1; }
done
if [ ! -x "$bin" ] || [ ! -x "$harness" ]; then
  echo "capture-shots: build wks-native and native-harness first (see the header of this script)" >&2
  exit 1
fi

work=$(mktemp -d)
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$work"
}
trap cleanup EXIT

Xvfb "$display" -screen 0 2800x1700x24 >/dev/null 2>&1 & pids+=($!)
sleep 2
export DISPLAY=$display GPUI_X11_SCALE_FACTOR=2
lavapipe=/usr/share/vulkan/icd.d/lvp_icd.json
[ -n "${VK_ICD_FILENAMES:-}" ] || { [ -f "$lavapipe" ] && export VK_ICD_FILENAMES=$lavapipe; }
openbox >/dev/null 2>&1 & pids+=($!)

# Two fixtures: the second one has an unanswered question set on the first
# session, which replaces the approval card every other capture relies on.
fixture="--rich-transcript --catalog --sidebar-mix --sessions 9 --turns 4"
"$harness" serve --bind 127.0.0.1:7906 $fixture >/dev/null 2>&1 & pids+=($!)
"$harness" serve --bind 127.0.0.1:7907 $fixture --pending-questions >/dev/null 2>&1 & pids+=($!)
sleep 2
plain=ws://127.0.0.1:7906/bus
asking=ws://127.0.0.1:7907/bus

# shot <name> <bus> <logical width> <logical height> [smoke.py options...]
shot() {
  local name=$1 bus=$2 width=$3 height=$4
  shift 4
  # smoke.py fails on a blank window; its report and the app's own startup
  # chatter are only worth reading when it does.
  python3 "$native/scripts/smoke.py" --binary "$bin" --bus "$bus" --no-input \
    --width $((width * 2)) --height $((height * 2)) \
    --settle-seconds 6 --sample-seconds 1 --output "$work/$name.png" "$@" \
    >"$work/$name.log" 2>&1 || { cat "$work/$name.log" >&2; exit 1; }
  cwebp -quiet -q 86 -m 6 "$work/$name.png" -o "$out/$name.webp"
  echo "captured $name.webp (${width}x${height} at 2x)"
}

# The two wide frames. --scroll-pages is in half-windows from the bottom, so
# these counts only land on the spawn card and the wake at this window height.
shot native-spawn       "$plain"  1280 760 --scroll-pages 5
shot native-wake        "$plain"  1280 760 --scroll-pages 3
# The feature rows.
shot native-questions   "$asking"  900 600
shot native-new-agent   "$plain"   900 600 --new-session
shot native-changes     "$plain"   900 600 --screen changes
# The three-up gallery.
shot native-terminal    "$plain"   820 550 --keys ctrl+grave
shot native-jobs        "$plain"   820 550 --screen jobs
shot native-everforest  "$plain"   820 550 --theme everforest
# The docs pages.
shot native-history     "$plain"   900 600 --screen history
shot native-projects    "$plain"   900 600 --screen projects
shot native-settings    "$plain"   900 600 --screen settings
shot native-light       "$plain"   900 600 --theme light
