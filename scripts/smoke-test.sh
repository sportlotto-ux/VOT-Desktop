#!/usr/bin/env bash
# Smoke-test: URL -> готовый .mixed.<ext> (end-to-end конвейер).
# Контейнер результата (mp4/webm) выбирается по видеокодеку, как в mixer.rs.
# Использование: scripts/smoke-test.sh <youtube_url> [output_dir]
#
# Требует:
#   - ffmpeg в PATH
#   - yt-dlp в PATH или ~/.cache/votdesktop/binaries
#   - deno в PATH или ~/.cache/votdesktop/binaries для VOT
#   - VOT_MEDIABOT_SRC (опционально)
set -euo pipefail

cd "$(dirname "$0")/.."

URL="${1:-}"
OUT_DIR="${2:-/tmp/vot-desktop-smoke}"
APPIMAGE="src-tauri/target/release/bundle/appimage/VotDesktop_0.1.0_amd64.AppImage"
BINARY="src-tauri/target/release/vot_desktop"
YTDLP="${YTDLP_BIN:-}"
CACHE_BIN="${HOME}/.cache/votdesktop/binaries"

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
  elif [[ -x "${CACHE_BIN}/yt-dlp" ]]; then
    YTDLP="${CACHE_BIN}/yt-dlp"
  else
    echo "ERROR: yt-dlp not found in PATH or ${CACHE_BIN}" >&2
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
  elif [[ -x "${CACHE_BIN}/deno" ]]; then
    DENO="${CACHE_BIN}/deno"
  else
    echo "ERROR: deno not found in PATH or ${CACHE_BIN}" >&2
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

# Выбор контейнера по видеокодеку — повторяет output_profile() из mixer.rs.
CODEC="$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of csv=p=0 "${VIDEO}")"
if [[ "${CODEC}" == "vp8" || "${CODEC}" == "vp9" ]]; then
  CONTAINER="webm"
  AUDIO_CODEC="libopus"
else
  CONTAINER="mp4"
  AUDIO_CODEC="aac"
fi
MIXED="${OUT_DIR}/mixed.${CONTAINER}"

ffmpeg -v error -i "${VIDEO}" -i "${AUDIO}" -i "${TRANSLATION}" \
  -filter_complex "[1:a]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,volume=0.33[en];[2:a]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,pan=stereo|c0=c0|c1=c0[ru];[ru][en]amix=inputs=2:duration=longest:normalize=0[mix]" \
  -map 0:v -map "[mix]" -c:v copy -c:a "${AUDIO_CODEC}" -b:a 192k -y "${MIXED}"

if [[ ! -f "${MIXED}" ]]; then
  echo ">> Smoke-test FAILED: no ${MIXED} produced"
  exit 1
fi

rm -rf "${WORK_DIR}"

echo ""
echo ">> Smoke-test PASSED: ${MIXED} (video codec: ${CODEC})"
ffprobe -v error -show_entries format=duration,size -of default=noprint_wrappers=1 "${MIXED}"
