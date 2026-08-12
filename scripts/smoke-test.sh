#!/usr/bin/env bash
# Smoke-test: URL -> готовый .mixed.mp4 (end-to-end конвейер).
# Использование: scripts/smoke-test.sh <youtube_url> [output_dir]
#
# Требует:
#   - ffmpeg в PATH
#   - yt-dlp в PATH (или bundled)
#   - deno в PATH (или bundled) для VOT
#   - VOT_MEDIABOT_SRC (опционально)
set -euo pipefail

cd "$(dirname "$0")/.."

URL="${1:-}"
OUT_DIR="${2:-/tmp/vot-desktop-smoke}"
APPIMAGE="src-tauri/target/release/bundle/appimage/VotDesktop_0.1.0_amd64.AppImage"
BINARY="src-tauri/target/release/vot_desktop"
YTDLP="${YTDLP_BIN:-}"

if [[ -z "${URL}" ]]; then
  echo "Usage: $0 <youtube_url> [output_dir]" >&2
  echo ""
  echo "Example: $0 'https://youtube.com/watch?v=dQw4w9WgXcQ'" >&2
  exit 1
fi

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "ERROR: ffmpeg not found in PATH" >&2
  exit 1
fi

if [[ -z "${YTDLP}" ]]; then
  if command -v yt-dlp >/dev/null 2>&1; then
    YTDLP="$(command -v yt-dlp)"
  elif [[ -x "src-tauri/binaries/yt-dlp-x86_64-x86_64-unknown-linux-gnu" ]]; then
    YTDLP="src-tauri/binaries/yt-dlp-x86_64-x86_64-unknown-linux-gnu"
  else
    echo "ERROR: yt-dlp not found in PATH or src-tauri/binaries" >&2
    exit 1
  fi
fi

if [[ ! -f "${BINARY}" ]] && [[ ! -f "${APPIMAGE}" ]]; then
  echo "ERROR: build the project first: npm run tauri:build" >&2
  exit 1
fi

if [[ -z "${DENO:-}" ]]; then
  if command -v deno >/dev/null 2>&1; then
    DENO="$(command -v deno)"
  elif [[ -x "src-tauri/binaries/deno-x86_64-x86_64-unknown-linux-gnu" ]]; then
    DENO="src-tauri/binaries/deno-x86_64-x86_64-unknown-linux-gnu"
  else
    echo "ERROR: deno not found in PATH or src-tauri/binaries" >&2
    exit 1
  fi
fi

mkdir -p "${OUT_DIR}"
WORK_DIR="${OUT_DIR}/.work"
rm -rf "${WORK_DIR}"
mkdir -p "${WORK_DIR}"
echo ">> output dir: ${OUT_DIR}"
echo ">> URL: ${URL}"
echo ""

echo ">> [1/5] Checking ffmpeg..."
ffmpeg -version 2>&1 | head -1

echo ">> [2/5] Downloading video..."
"${YTDLP}" -f "bestvideo[height<=360]/bestvideo" \
  -o "${WORK_DIR}/video.%(ext)s" \
  --no-playlist --newline "${URL}" >/dev/null 2>&1

echo ">> [3/5] Downloading audio..."
"${YTDLP}" -f "bestaudio[ext=m4a]/bestaudio" \
  -o "${WORK_DIR}/audio.%(ext)s" \
  --no-playlist --newline "${URL}" >/dev/null 2>&1

echo ">> [4/5] Fetching VOT translation (up to 300s)..."
if ! timeout 300 env -i PATH=/usr/bin:/bin HOME="${HOME:-/home/$(id -un)}" TERM=xterm CI=1 FORCE_COLOR=1 \
  "${DENO}" run --allow-net --allow-env \
  "--allow-read=${WORK_DIR},${HOME:-/home/$(id -un)}/.cache/deno" \
  "--allow-write=${WORK_DIR}" \
  "npm:vot-cli-live@1.7.5" --quiet --output "${WORK_DIR}" --voice-style live "${URL}" \
  >/dev/null 2>&1; then
  echo "   WARN: VOT failed or timed out — mixing will be skipped"
  echo ">> Smoke-test FAILED: no translation available"
  exit 1
fi

TRANSLATION="$(find "${WORK_DIR}" -maxdepth 1 -name '*.mp3' -o -name '*.m4a' | head -1)"
if [[ -z "${TRANSLATION}" ]]; then
  echo ">> Smoke-test FAILED: VOT produced no audio output"
  exit 1
fi

echo ">> [5/5] Mixing video + original audio + translation..."
VIDEO="$(find "${WORK_DIR}" -maxdepth 1 -name 'video.*' | head -1)"
AUDIO="$(find "${WORK_DIR}" -maxdepth 1 -name 'audio.*' | head -1)"
ffmpeg -v error -i "${VIDEO}" -i "${AUDIO}" -i "${TRANSLATION}" \
  -filter_complex "[1:a]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,volume=0.33[en];[2:a]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,pan=stereo|c0=c0|c1=c0[ru];[ru][en]amix=inputs=2:duration=longest:normalize=0[mix]" \
  -map 0:v -map "[mix]" -c:v copy -c:a aac -b:a 192k -y "${OUT_DIR}/mixed.mp4"

if [[ ! -f "${OUT_DIR}/mixed.mp4" ]]; then
  echo ">> Smoke-test FAILED: no mixed.mp4 produced"
  exit 1
fi

rm -rf "${WORK_DIR}"

echo ""
echo ">> Smoke-test PASSED: ${OUT_DIR}/mixed.mp4"
ffprobe -v error -show_entries format=duration,size -of default=noprint_wrappers=1 "${OUT_DIR}/mixed.mp4"
