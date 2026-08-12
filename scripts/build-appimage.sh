#!/usr/bin/env bash
# Сборка AppImage + .deb для x86_64.
# Использование: scripts/build-appimage.sh
#
# Бинарники yt-dlp/deno НЕ бандлятся (тонкий AppImage) — они резолвятся
# из PATH или скачиваются pinned-версии в ~/.cache/votdesktop/binaries
# при первом запуске (см. src-tauri/src/binaries.rs, ADR-012).
#
# Опциональная переменная окружения:
#   VOT_MEDIABOT_SRC — путь к mediabot2.0/src/handlers/media_utils.py
#                      для build-time валидации filter_complex (ADR-003).
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

# ---- 3. npm install ----
echo ">> npm ci"
npm ci

# ---- 4. Сборка Tauri ----
echo ">> npm run tauri:build"
npm run tauri:build

# ---- 5. Перепаковка AppImage с xz-сжатием ----
# linuxdeploy-plugin-appimage пакует squashfs с zstd и block 16K, что даёт
# больший размер. xz с block 1M сжимает лучше — перепаковываем из готового
# AppDir. Проверено: работает, время ~45s.
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

# ---- 6. Результат ----
echo ""
echo "Build artifacts:"
find src-tauri/target/release/bundle -type f \( -name "*.AppImage" -o -name "*.deb" \) -exec ls -lh {} \;
