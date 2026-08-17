#!/usr/bin/env bash
# Fetches the official LBreakoutHD levelset archive, verifies its checksum,
# and extracts the levelsets into ./levels/ (gitignored: this repo is MIT
# and the fetched data is GPLv3, so it must never be committed here).
# Also (re)writes ATTRIBUTION at the repo root, crediting the original
# levelset authors and the GPL source, as required by the GPL -- without
# reproducing any of the level data itself.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEVELS_DIR="${REPO_ROOT}/levels"
ATTRIBUTION_FILE="${REPO_ROOT}/ATTRIBUTION"

SOURCE_COMMIT="3cd2a6160941557f48c49b184f0ad47ddd882c23"
SOURCE_URL="https://github.com/midzer/lbreakouthd/archive/${SOURCE_COMMIT}.tar.gz"
SOURCE_SHA256="0302394efa6d11c1b8714f3a04596dedc62c7a77a2fcfc56cdcc54714c27a6a0"
ARCHIVE_PREFIX="lbreakouthd-${SOURCE_COMMIT}"

# Fan-made recreation of the original Taito Arkanoid brick layouts, shipped
# upstream as its own levelset file. Excluded on purpose: this repo must
# never reproduce Arkanoid/Taito layouts anywhere, gitignored or not.
EXCLUDE_LEVELSET="Arkanoid"

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

archive="${workdir}/lbreakouthd.tar.gz"

echo "Downloading LBreakoutHD @ ${SOURCE_COMMIT}..."
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

echo "Extracting levelsets..."
tar -xzf "${archive}" -C "${workdir}" "${ARCHIVE_PREFIX}/src/levels"

rm -rf "${LEVELS_DIR}"
mkdir -p "${LEVELS_DIR}"

authors_file="${workdir}/authors.txt"
: >"${authors_file}"

for levelset in "${workdir}/${ARCHIVE_PREFIX}"/src/levels/*; do
  name="$(basename "${levelset}")"
  [[ -f "${levelset}" ]] || continue
  [[ "${name}" == "${EXCLUDE_LEVELSET}" ]] && continue
  cp "${levelset}" "${LEVELS_DIR}/${name}"
  # Each level's author line is the line right after its "Level:" marker.
  awk '/^Level:/ { getline a; if (length(a) > 0) print a }' "${levelset}" >>"${authors_file}"
done

level_count="$(find "${LEVELS_DIR}" -type f | wc -l | tr -d ' ')"
if [[ "${level_count}" -eq 0 ]]; then
  echo "error: no levelset files extracted -- upstream layout may have changed" >&2
  exit 1
fi

{
  echo "Third-party level data attribution"
  echo "==================================="
  echo
  echo "levels/ (gitignored, never committed to this repo) is populated by"
  echo "scripts/fetch-levelsets.sh from the LBreakoutHD project:"
  echo
  echo "  Source:  https://github.com/midzer/lbreakouthd"
  echo "  Commit:  ${SOURCE_COMMIT} (v1.1.8)"
  echo "  License: GNU GPLv3 (see the project's own COPYING file)"
  echo
  echo "This repo is MIT-licensed; the fetched levelset data is GPL and is"
  echo "never committed here -- levels/ is gitignored. This file exists to"
  echo "credit the original levelset authors as required by the GPL, without"
  echo "reproducing any of their level data."
  echo
  echo "The '${EXCLUDE_LEVELSET}' levelset (a fan recreation of the original"
  echo "Taito Arkanoid brick layouts) is deliberately excluded from the fetch"
  echo "and never touches this repo, even in the gitignored levels/ directory."
  echo
  echo "Levelset authors (each file's 'Level:' author line, deduplicated):"
  echo
  sort -u "${authors_file}" | sed 's/^/  - /'
  echo
  echo "${level_count} levelset file(s) fetched at commit ${SOURCE_COMMIT}."
} >"${ATTRIBUTION_FILE}"

echo "Wrote ${level_count} levelset file(s) to ${LEVELS_DIR}"
echo "Wrote ${ATTRIBUTION_FILE}"
