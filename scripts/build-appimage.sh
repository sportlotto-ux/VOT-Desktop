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

# ---- 5. Перепаковка AppImage в "тонкий" вариант ----
# Бандл linuxdeploy тянет webkit2gtk/GTK из хоста сборки; на машинах с другим
# стеком драйверов это даёт белый webview (EGL_BAD_PARAMETER). Вырезаем
# usr/lib целиком — AppImage использует системные библиотеки (как .deb).
APPIMAGETOOL="${APPIMAGETOOL:-/tmp/appimagetool-x86_64.AppImage}"
if [[ ! -f "${APPIMAGETOOL}" ]]; then
  echo ">> downloading appimagetool..."
  curl -fsSL -o "${APPIMAGETOOL}" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "${APPIMAGETOOL}"
fi
APPIMAGE_OUT="$(find src-tauri/target/release/bundle/appimage -maxdepth 1 -name '*.AppImage' | head -1)"

if [[ -n "${APPIMAGE_OUT}" ]]; then
  cd "$(dirname "${APPIMAGE_OUT}")"
  echo ">> Repacking $(basename "${APPIMAGE_OUT}") as thin AppImage (system libs)..."
  ./"$(basename "${APPIMAGE_OUT}")" --appimage-extract > /dev/null
  rm -rf squashfs-root/usr/lib
  ARCH=x86_64 "${APPIMAGETOOL}" --no-appstream squashfs-root "$(basename "${APPIMAGE_OUT}")" > /dev/null
  rm -rf squashfs-root
  chmod +x "${APPIMAGE_OUT}"
  echo ">> Repacked: $(ls -lh "${APPIMAGE_OUT}" | awk '{print $5}')"
else
  echo ">> SKIP repack: AppImage missing"
fi

# ---- 6. Результат ----
echo ""
echo "Build artifacts:"
find src-tauri/target/release/bundle -type f \( -name "*.AppImage" -o -name "*.deb" \) -exec ls -lh {} \;
