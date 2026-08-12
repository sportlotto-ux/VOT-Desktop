#!/usr/bin/env bash
# Ensure the cached linuxdeploy AppImage has a patchelf that understands
# .relr.dyn sections (≥0.16). Fedora 44+ ships binutils with .relr.dyn
# support, but the bundled patchelf (0.15.0) inside Tauri's linuxdeploy
# predates this and corrupts .init sections when rewriting DT_RUNPATH.
#
# Run this before `npm run tauri:build` whenever the cache is cleared
# or linuxdeploy is re-downloaded.
set -euo pipefail

CACHE_DIR="${HOME}/.cache/tauri"
APPIMAGE="${CACHE_DIR}/linuxdeploy-x86_64.AppImage"

if [[ ! -f "${APPIMAGE}" ]]; then
  echo ">> linuxdeploy not in cache — will be patched after first download"
  exit 0
fi

WORKDIR="$(mktemp -d)"
trap 'rm -rf "${WORKDIR}"' EXIT

cd "${WORKDIR}"
"${APPIMAGE}" --appimage-extract > /dev/null 2>&1
EXTRACTED="${WORKDIR}/squashfs-root"

BUNDLED_VER="$("${EXTRACTED}/usr/bin/patchelf" --version 2>/dev/null || echo "unknown")"

# Extract numeric version (0.15.0 -> 15, 0.18.0 -> 18)
BUNDLED_NUM="$(echo "${BUNDLED_VER}" | sed -n 's/patchelf 0\.\([0-9]*\)\..*/\1/p')"

if [[ -n "${BUNDLED_NUM}" ]] && [[ "${BUNDLED_NUM}" -ge 16 ]] 2>/dev/null; then
  echo ">> bundled patchelf ${BUNDLED_VER} is OK (≥0.16), no fix needed"
  exit 0
fi

SYSTEM_PATCHELF="$(command -v patchelf || true)"
if [[ -z "${SYSTEM_PATCHELF}" ]]; then
  echo "ERROR: patchelf ≥0.16 required on host. Install: sudo dnf install -y patchelf" >&2
  exit 1
fi

SYSTEM_VER="$("${SYSTEM_PATCHELF}" --version 2>/dev/null || echo "unknown")"
echo ">> replacing bundled patchelf (${BUNDLED_VER}) with system (${SYSTEM_VER})"

cp "${SYSTEM_PATCHELF}" "${EXTRACTED}/usr/bin/patchelf"

APPIMAGETOOL="/tmp/appimagetool-x86_64.AppImage"
if [[ ! -f "${APPIMAGETOOL}" ]]; then
  echo ">> downloading appimagetool..." >&2
  curl -fsSL -o "${APPIMAGETOOL}" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "${APPIMAGETOOL}"
fi

ARCH=x86_64 "${APPIMAGETOOL}" -n "${EXTRACTED}" "${APPIMAGE}" > /dev/null 2>&1
chmod +x "${APPIMAGE}"
echo ">> linuxdeploy patched successfully"
