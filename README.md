# VotDesktop

Десктоп-приложение для скачивания видео с YouTube с микшированием голосового перевода Яндекса (VOT).

**Статус:** Фазы 0–4 завершены. Регламент — [docs/REGULATION.md](docs/REGULATION.md).

## Стек

- **GUI:** Tauri 2.x + TypeScript (vanilla)
- **Загрузчик:** yt-dlp 2026.07.04 (bundled в AppImage или системный)
- **Перевод:** vot-cli-live 1.7.5 через Deno 2.9.5 (bundled в AppImage)
- **Микширование:** ffmpeg (системный, filter_complex amix)
- **Дистрибутив:** AppImage + .deb (Linux x86_64)

## Установка

```bash
# Требования: ffmpeg
sudo dnf install ffmpeg          # Fedora
sudo apt install ffmpeg          # Ubuntu/Debian

# Скачать AppImage
chmod +x VotDesktop_*.AppImage
./VotDesktop_*.AppImage
```

Если ffmpeg не установлен — приложение покажет модалку с командой установки.

## Использование

1. Вставьте YouTube-ссылку
2. Нажмите **Fetch Formats**
3. Выберите контейнер (mp4/webm/mkv), тип (Video+Audio/Video only/Audio only) и качество
4. Опционально: выберите cookies-файл и путь сохранения
5. Включите чекбокс **Mix with Russian voice translation** для VOT+микширования
6. Нажмите **Start**

Результат: скачанный файл + (опционально) `.mixed.<ext>` с голосовым переводом. Контейнер результата выбирается по видеокодеку: vp8/vp9 → webm, иначе mp4.

## Разработка

```bash
# Зависимости (Fedora)
sudo dnf install webkit2gtk4.1-devel openssl-devel patchelf

# Запуск dev-режима
VOT_MEDIABOT_SRC=/path/to/mediabot2.0/src/handlers/media_utils.ts \
  npm run tauri:dev

# Сборка AppImage + deb
VOT_MEDIABOT_SRC=/path/to/mediabot2.0/src/handlers/media_utils.ts \
  bash scripts/build-appimage.sh
```

### Важные замечания

- **patchelf ≥0.16** обязателен на хосте (`sudo dnf install patchelf`). Без него `linuxdeploy` корраптит `.init` секции в `.so`
- **VOT_MEDIABOT_SRC** — опциональный путь к `media_utils.py` для build-time sha256-валидации filter_complex (ADR-003). Если не задан — soft-fail
- **deno** и **yt-dlp** скачиваются на этапе сборки из зафиксированных release assets с проверкой SHA-256 и бандлятся в AppImage/.deb. В dev-режиме используются системные версии
- `SKIP_FETCH=1` разрешен только при наличии бинарников с ожидаемыми checksum

### Проверка качества

```bash
npm run build
npm test
npm run lint:rust
```

## Структура проекта

```
vot-desktop/
├── docs/REGULATION.md        # регламент
├── src/                      # TypeScript (vanilla, Vite)
│   ├── main.ts               # точка входа
│   ├── ui.ts                 # рендеринг/события
│   ├── ipc.ts                # обёртки над invoke
│   ├── types.ts              # DTO
│   └── styles.css
├── src-tauri/                # Rust (Tauri 2)
│   ├── src/
│   │   ├── main.rs           # entrypoint
│   │   ├── lib.rs            # Builder, setup
│   │   ├── commands.rs       # #[tauri::command]
│   │   ├── downloader.rs     # yt-dlp subprocess
│   │   ├── translator.rs     # vot-cli-live/deno
│   │   ├── mixer.rs          # ffmpeg filter_complex
│   │   ├── deps.rs           # проверка ffmpeg
│   │   ├── error.rs          # AppError
│   │   └── types.rs          # Format, YtDlpFormat
│   ├── build.rs              # sha256 валидация (ADR-003)
│   ├── binaries/             # bundled yt-dlp/deno
│   ├── icons/
│   └── tauri.conf.json
├── scripts/
│   ├── build-appimage.sh     # полная сборка
│   ├── dev.sh                # dev-режим
│   ├── fix-linuxdeploy.sh    # патч patchelf в кэше
│   ├── fetch-yt-dlp.sh
│   ├── fetch-deno.sh
│   └── smoke-test.sh
├── package.json
├── tsconfig.json
└── vite.config.ts
```
