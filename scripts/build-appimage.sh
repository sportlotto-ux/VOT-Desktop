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

# ---- 6b. Перепаковка AppImage с xz-сжатием ----
# linuxdeploy-plugin-appimage пакует squashfs с zstd и block 16K, что даёт
# ~177MB. xz с block 1M сжимает лучше (~156MB, −12%) — перепаковываем из
# готового AppDir. Проверено: работает, время ~45s.
APPIMAGETOOL="$(command -v appimagetool || true)"
APPIMAGE_OUT="$(find src-tauri/target/release/bundle/appimage -maxdepth 1 -name '*.AppImage' | head -1)"
APPDIR="src-tauri/target/release/bundle/appimage/VotDesktop.AppDir"

if [[ -n "${APPIMAGETOOL}" && -n "${APPIMAGE_OUT}" && -d "${APPDIR}" ]]; then
  echo ">> Repacking ${APPIMAGE_OUT} with xz/1M compression..."
  ARCH=x86_64 "${APPIMAGETOOL}" --comp xz \
    --mksquashfs-opt="-b" --mksquashfs-opt="1048576" \
    --no-appstream "${APPDIR}" "${APPIMAGE_OUT}.xz" > /dev/null
  mv -f "${APPIMAGE_OUT}.xz" "${APPIMAGE_OUT}"
  chmod +x "${APPIMAGE_OUT}"
  echo ">> Repacked: $(ls -lh "${APPIMAGE_OUT}" | awk '{print $5}')"
else
  echo ">> SKIP repack: appimagetool or AppDir missing"
fi

# ---- 7. Результат ----
echo ""
echo "Build artifacts:"
find src-tauri/target/release/bundle -type f \( -name "*.AppImage" -o -name "*.deb" \) -exec ls -lh {} \;
