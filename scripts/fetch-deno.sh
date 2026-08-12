#!/usr/bin/env bash
# Скачивает зафиксированный static-бинарь deno в src-tauri/binaries/.
# Используется в AppImage для запуска vot-cli-live (npm-пакет).
#
# Использование: scripts/fetch-deno.sh
set -euo pipefail

cd "$(dirname "$0")/.."

DEST_DIR="src-tauri/binaries"
DEST="${DEST_DIR}/deno-x86_64-x86_64-unknown-linux-gnu"
VERSION="2.9.5"
URL="https://github.com/denoland/deno/releases/download/v${VERSION}/deno-x86_64-unknown-linux-gnu.zip"
ARCHIVE_SHA256="8b010a3b1a4a0188a67cdb8a7a27348b2a501af78aec7fc74f2ace167368d530"
BINARY_SHA256="dc480c462c8c3582524f3e75c160613d0a975e1f66b5465995d58bae236da7d3"

mkdir -p "${DEST_DIR}"

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

echo ">> Fetching deno from ${URL}"
curl --connect-timeout 15 --max-time 600 --retry 2 -fsSL -o "${TMP}/deno.zip" "${URL}"
printf '%s  %s\n' "${ARCHIVE_SHA256}" "${TMP}/deno.zip" | sha256sum -c -

if command -v unzip >/dev/null 2>&1; then
  unzip -o -d "${TMP}" "${TMP}/deno.zip"
elif command -v python3 >/dev/null 2>&1; then
  python3 -c "import zipfile, sys; zipfile.ZipFile('${TMP}/deno.zip').extractall('${TMP}')"
else
  echo "ERROR: need unzip or python3 to extract deno archive" >&2
  exit 1
fi

install -m 0755 "${TMP}/deno" "${DEST}"
printf '%s  %s\n' "${BINARY_SHA256}" "${DEST}" | sha256sum -c -

echo ">> Installed: ${DEST}"
"${DEST}" --version
