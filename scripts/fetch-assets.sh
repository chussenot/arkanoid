#!/usr/bin/env bash
# Fetches Kenney's "Puzzle Pack" (CC0), picks 6 of its sprites, and lays
# them out under the fixed filenames this game's asset-pack loader expects
# (`pack_filename` in src/assets.rs) -- one PNG per `SpriteId`, any size:
# the loader (`assets::TextureSource::Pack`) decodes and resizes each to
# the atlas's cell size at runtime, so this script doesn't need to.
# Also (re)writes ATTRIBUTION at the repo root, crediting Kenney -- not
# required by CC0, just good practice.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-${REPO_ROOT}/assets/pack}"
ATTRIBUTION_FILE="${REPO_ROOT}/ATTRIBUTION"

SOURCE_URL="https://kenney.nl/media/pages/assets/puzzle-pack-1/627a04c0d3-1774770940/kenney_puzzle-pack-1.zip"
SOURCE_SHA256="0d143f821cff40eec16b3d28c4d377cea7460d8796f6c7346b178b1534827a7c"

# Kenney filename -> this game's fixed pack filename (`pack_filename` in
# src/assets.rs), one pair per `SpriteId::ALL` entry. Bases picked to
# loosely match the flat colors render.rs's procedural fallback already
# uses (see assets.rs's `BRICK_PALETTE`), so a pack-textured run doesn't
# look like an unrelated palette swap.
SPRITE_MAP=(
  "PNG/Default/paddleBlu.png:paddle.png"
  "PNG/Default/ballBlue.png:ball.png"
  "PNG/Default/element_red_rectangle.png:brick_normal.png"
  "PNG/Default/element_blue_rectangle_glossy.png:brick_armored_intact.png"
  "PNG/Default/element_yellow_rectangle.png:brick_armored_hit.png"
  "PNG/Default/element_grey_square.png:brick_indestructible.png"
)

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

archive="${workdir}/puzzle-pack-1.zip"

echo "Downloading Kenney Puzzle Pack..."
curl -fsSL "${SOURCE_URL}" -o "${archive}"

echo "Verifying checksum..."
if command -v sha256sum >/dev/null 2>&1; then
  actual_sha256="$(sha256sum "${archive}" | cut -d' ' -f1)"
else
  actual_sha256="$(shasum -a 256 "${archive}" | cut -d' ' -f1)"
fi

if [[ "${actual_sha256}" != "${SOURCE_SHA256}" ]]; then
  echo "error: checksum mismatch for ${SOURCE_URL}" >&2
  echo "  expected: ${SOURCE_SHA256}" >&2
  echo "  actual:   ${actual_sha256}" >&2
  exit 1
fi

echo "Extracting sprites to ${OUT_DIR}..."
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"
for pair in "${SPRITE_MAP[@]}"; do
  src="${pair%%:*}"
  dst="${pair##*:}"
  unzip -p "${archive}" "${src}" >"${OUT_DIR}/${dst}"
done

count="$(find "${OUT_DIR}" -type f -name '*.png' | wc -l | tr -d ' ')"
if [[ "${count}" -ne "${#SPRITE_MAP[@]}" ]]; then
  echo "error: expected ${#SPRITE_MAP[@]} sprites, got ${count} -- upstream pack layout may have changed" >&2
  exit 1
fi

# Repo-relative for a default (or otherwise-nested) output dir, so
# ATTRIBUTION reads the same regardless of where this checkout happens to
# live; falls back to the raw path for a genuinely external -o override.
if [[ "${OUT_DIR}" == "${REPO_ROOT}"/* ]]; then
  display_dir="${OUT_DIR#"${REPO_ROOT}"/}"
else
  display_dir="${OUT_DIR}"
fi

{
  echo "Third-party texture asset attribution"
  echo "======================================"
  echo
  echo "${display_dir}/ (gitignored if under the repo root; populated by"
  echo "scripts/fetch-assets.sh) is a 6-sprite subset of Kenney's Puzzle Pack:"
  echo
  echo "  Source:  https://kenney.nl/assets/puzzle-pack-1"
  echo "  License: CC0 1.0 Universal (public domain) -- see the pack's own"
  echo "           License.txt for the full text."
  echo
  echo "This repo is MIT-licensed; CC0 content may be redistributed freely,"
  echo "but the fetched pack is still left gitignored (like this project's"
  echo "other fetched-asset script, scripts/fetch-levelsets.sh) rather than"
  echo "committed as binary blobs. Crediting Kenney here isn't required by"
  echo "CC0 -- it's just good practice."
  echo
  echo "Sprites used (Kenney filename -> this game's sprite file):"
  echo
  for pair in "${SPRITE_MAP[@]}"; do
    echo "  - ${pair%%:*} -> ${pair##*:}"
  done
} >"${ATTRIBUTION_FILE}"

echo "Wrote ${count} sprite(s) to ${OUT_DIR}"
echo "Wrote ${ATTRIBUTION_FILE}"
echo "Run with: cargo run -- --assets ${display_dir}"
