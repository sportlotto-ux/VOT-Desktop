#!/usr/bin/env bash
# Сборка AppImage + .deb для x86_64.
# Использование: scripts/build-appimage.sh
#
# Опциональная переменная окружения:
#   VOT_MEDIABOT_SRC — путь к mediabot2.0/src/handlers/media_utils.py
#                      для build-time валидации filter_complex (ADR-003).
#   SKIP_FETCH      — если установлена, использовать уже скачанные yt-dlp/deno.
set -euo pipefail

cd "$(dirname "$0")/.."

# ---- 1. Проверка и фикс linuxdeploy (patchelf ≥0.16 для .relr.dyn) ----
echo ">> Fixing linuxdeploy..."
scripts/fix-linuxdeploy.sh

# ---- 2. VOT_MEDIABOT_SRC ----
if [[ -n "${VOT_MEDIABOT_SRC:-}" ]]; then
  export VOT_MEDIABOT_SRC
  echo ">> VOT_MEDIABOT_SRC=${VOT_MEDIABOT_SRC}"
else
  echo ">> VOT_MEDIABOT_SRC not set — filter_complex sync check will be skipped"
fi

# ---- 4. npm install ----
if [[ -n "${SKIP_FETCH:-}" ]]; then
  DENO_BIN="src-tauri/binaries/deno-x86_64-x86_64-unknown-linux-gnu"
  YTDLP_BIN="src-tauri/binaries/yt-dlp-x86_64-x86_64-unknown-linux-gnu"
  printf '%s  %s\n' \
    "dc480c462c8c3582524f3e75c160613d0a975e1f66b5465995d58bae236da7d3" \
    "${DENO_BIN}" | sha256sum -c -
  printf '%s  %s\n' \
    "6bbb3d314cde4febe36e5fa1d55462e29c974f63444e707871834f6d8cc210ae" \
    "${YTDLP_BIN}" | sha256sum -c -
else
  echo ">> Fetching pinned runtime binaries"
  scripts/fetch-deno.sh
  scripts/fetch-yt-dlp.sh
fi

# ---- 5. npm install ----
echo ">> npm ci"
npm ci

# ---- 6. Сборка Tauri ----
echo ">> npm run tauri:build"
npm run tauri:build

# ---- 7. Результат ----
echo ""
echo "Build artifacts:"
find src-tauri/target/release/bundle -type f \( -name "*.AppImage" -o -name "*.deb" \) -exec ls -lh {} \;
