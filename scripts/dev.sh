#!/usr/bin/env bash
# Запуск VotDesktop в dev-режиме (Vite + Tauri).
# Использование: scripts/dev.sh
#
# Опциональная переменная окружения:
#   VOT_MEDIABOT_SRC — абсолютный путь к mediabot2.0/src/handlers/media_utils.py
#                      для build-time валидации filter_complex (ADR-003).
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -n "${VOT_MEDIABOT_SRC:-}" ]]; then
  export VOT_MEDIABOT_SRC
  echo ">> VOT_MEDIABOT_SRC=${VOT_MEDIABOT_SRC}"
else
  echo ">> VOT_MEDIABOT_SRC not set — filter_complex sync check will be skipped"
fi

exec npm run tauri:dev
