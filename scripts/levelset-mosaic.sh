#!/usr/bin/env bash
# Renders a labelled mosaic of levels from the fetched levelsets (see
# scripts/fetch-levelsets.sh) by running the real game and screenshotting
# it: docs/levelsets-mosaic.png in the README is this script's output.
#
#   scripts/levelset-mosaic.sh [OUT.png] [SET:LEVEL ...]
#
# Each SET:LEVEL is a file under levels/ and a 1-based level number in it.
# The level is copied alone into a temporary levelset, played on
# `--autopilot` (so the menu overlay is gone and the ball is in flight), and
# captured just after its first frame. Needs an X display: it starts Xvfb on
# :99 when DISPLAY is unset. Also needs ImageMagick (import, convert,
# montage) and a release build (`cargo build --release`).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${REPO_ROOT}/target/release/arkanoid"
OUT="${1:-${REPO_ROOT}/docs/levelsets-mosaic.png}"
shift || true

DEFAULT_LEVELS=(
  AllNighter:230 Gatinha:40 Hieroglyphs:25 AllWorld:180
  Color:1 Flower:1 Lattices:1 Maze:1
  Popcorn:1 Smilies:1 Zigies:1 Chaos:1
)
if [[ $# -gt 0 ]]; then LEVELS=("$@"); else LEVELS=("${DEFAULT_LEVELS[@]}"); fi

[[ -x "${BIN}" ]] || { echo "error: build first: cargo build --release" >&2; exit 1; }
[[ -d "${REPO_ROOT}/levels" ]] || { echo "error: fetch first: scripts/fetch-levelsets.sh" >&2; exit 1; }

workdir="$(mktemp -d)"
xvfb_pid=""
cleanup() {
  [[ -n "${xvfb_pid}" ]] && kill "${xvfb_pid}" 2>/dev/null
  rm -rf "${workdir}"
}
trap cleanup EXIT

if [[ -z "${DISPLAY:-}" ]]; then
  Xvfb :99 -screen 0 1024x768x24 >/dev/null 2>&1 &
  xvfb_pid=$!
  export DISPLAY=:99
  sleep 1
fi

# Brightness of the top-left 800x600 of the screen; the window draws black
# until wgpu presents its first frame.
mean() {
  import -window root -crop 800x600+0+0 "${workdir}/probe.png"
  convert "${workdir}/probe.png" -format "%[fx:mean]" info:
}

montage_args=()
for spec in "${LEVELS[@]}"; do
  set_name="${spec%%:*}"
  level="${spec##*:}"
  set_file="${REPO_ROOT}/levels/${set_name}"
  single="${workdir}/${set_name}-${level}"

  # The header line, then the Nth `Level:` block alone.
  awk -v n="${level}" '
    NR == 1 && /^Version:/ { print; next }
    /^Level:/ { count++ }
    count == n { print }
  ' "${set_file}" >"${single}"
  grep -q '^Level:' "${single}" || { echo "error: ${set_name} has no level ${level}" >&2; exit 1; }

  "${BIN}" --autopilot --levelset "${single}" >/dev/null 2>&1 &
  game_pid=$!
  for _ in $(seq 1 75); do
    sleep 0.2
    awk -v m="$(mean)" 'BEGIN { exit !(m > 0.05) }' && break
  done
  sleep 0.25
  # Drop the HUD strip: a single-level set always reads "Level 1".
  import -window root -crop 800x548+0+52 +repage "${single}.png"
  kill "${game_pid}" 2>/dev/null || true
  wait "${game_pid}" 2>/dev/null || true
  sleep 0.5

  montage_args+=(-label "${set_name} #${level}" "${single}.png")
done

montage "${montage_args[@]}" -tile 4x -geometry 400x274+6+6 \
  -background '#1b1e27' -fill '#e6e8ee' -pointsize 17 "${workdir}/mosaic.png"
# The game draws a handful of flat colors: a palette PNG is a tenth the size.
convert "${workdir}/mosaic.png" -strip -depth 8 -colors 128 \
  -define png:compression-level=9 "${OUT}"
echo "Wrote ${OUT}"
