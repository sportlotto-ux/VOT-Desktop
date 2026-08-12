#!/usr/bin/env bash
# Скачивает зафиксированный static-бинарь yt-dlp в src-tauri/binaries/.
# Запускать перед сборкой AppImage (Фаза 5). В dev-режиме используется
# системный yt-dlp из PATH.
#
# Использование: scripts/fetch-yt-dlp.sh
set -euo pipefail

cd "$(dirname "$0")/.."

DEST_DIR="src-tauri/binaries"
DEST="${DEST_DIR}/yt-dlp-x86_64-x86_64-unknown-linux-gnu"
VERSION="2026.07.04"
URL="https://github.com/yt-dlp/yt-dlp/releases/download/${VERSION}/yt-dlp_linux"
SHA256="6bbb3d314cde4febe36e5fa1d55462e29c974f63444e707871834f6d8cc210ae"

mkdir -p "${DEST_DIR}"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

echo ">> Fetching yt-dlp from ${URL}"
curl --connect-timeout 15 --max-time 600 --retry 2 -fsSL -o "${TMP}/yt-dlp" "${URL}"
printf '%s  %s\n' "${SHA256}" "${TMP}/yt-dlp" | sha256sum -c -
install -m 0755 "${TMP}/yt-dlp" "${DEST}"

echo ">> Installed: ${DEST}"
"${DEST}" --version
