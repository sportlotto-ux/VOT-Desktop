# VotDesktop

Десктоп-приложение для скачивания видео с YouTube с микшированием голосового перевода Яндекса (VOT) и переводом описания на русский через Google AI Studio.

**Статус:** Фазы 0–4 завершены. Регламент — [docs/REGULATION.md](docs/REGULATION.md).

## Зависимости

Приложение использует **системные библиотеки** (тонкий AppImage ~3 MB):

| Зависимость | Назначение | Fedora | Ubuntu/Debian | Arch |
|---|---|---|---|---|
| **webkit2gtk-4.1** | GUI (обязательно) | `sudo dnf install webkit2gtk4.1` | `sudo apt install libwebkit2gtk-4.1-0` | `sudo pacman -S webkit2gtk-4.1` |
| **ffmpeg** (+ ffprobe) | микширование перевода (обязательно для VOT) | `sudo dnf install ffmpeg` | `sudo apt install ffmpeg` | `sudo pacman -S ffmpeg` |

Без ffmpeg скачивание работает, но микширование недоступно. Без webkit2gtk окно не откроется вовсе.

**yt-dlp** и **deno** не требуются заранее: при первом запуске берутся из `PATH`, иначе автоматически скачиваются pinned-версии с sha256-проверкой в `~/.cache/votdesktop/binaries` (нужна сеть, ~10–30 c).

Для **перевода описаний** опционально укажите бесплатный API-ключ [Google AI Studio](https://aistudio.google.com/apikey) в настройках приложения.

## Установка

```bash
# 1. Поставьте зависимости (см. таблицу выше)

# 2. Скачать AppImage со страницы Releases
chmod +x VotDesktop_*.AppImage
./VotDesktop_*.AppImage
```

Если ffmpeg не установлен — приложение покажет модалку с командой установки.

## Стек

- **GUI:** Tauri 2.x + TypeScript (vanilla)
- **Загрузчик:** yt-dlp (pinned 2026.07.04, авто-скачивание)
- **Перевод:** vot-cli-live через Deno (авто-скачивание)
- **Описание:** Gemini API (ключ пользователя)
- **Микширование:** ffmpeg (системный, filter_complex amix)
- **Дистрибутив:** AppImage + .deb (Linux x86_64)

## Использование

1. Вставьте YouTube-ссылку
2. Нажмите **Получить форматы**
3. Выберите контейнер (mp4/webm/mkv), тип (Video+Audio / Video / Audio) и качество
4. Опционально: выберите cookies-файл и папку загрузки
5. Включите чекбокс **Микшировать с русской озвучкой (Яндекс VOT)** для перевода дорожки
6. Нажмите **Скачать**

### Перевод описания (опционально)

1. Получите бесплатный ключ на [aistudio.google.com/apikey](https://aistudio.google.com/apikey) и вставьте его в поле **API-ключ** — список моделей подтянется автоматически
2. Выберите модель в выпадающем списке
3. После скачивания нажмите **Перевести описание** — перевод появится на экране и сохранится рядом с видео

Временные сбои Gemini (503 перегрузка) обрабатываются автоматически — до 3 попыток с паузами.

Результат — папка на каждое видео:

```
~/Videos/VotDesktop/
└── Название видео_<ID>/
    ├── Название видео_<ID>.mp4        # видео
    ├── ...mixed.webm                  # микс с переводом (если включён VOT)
    └── description.ru.txt             # перевод описания (по кнопке)
```

## Интерфейс

- **Компоненты** — версии yt-dlp / deno / ffmpeg, проверяются при запуске
- **Обновления** — раз в сутки приложение проверяет новые релизы yt-dlp/deno и предлагает обновиться одной кнопкой
- Баннер в правом верхнем углу ведёт на [aiera.uz](https://aiera.uz)
- Версия приложения показана в правом нижнем углу

## Разработка

```bash
# Зависимости (Fedora)
sudo dnf install webkit2gtk4.1-devel openssl-devel patchelf

# Запуск dev-режима
npm run tauri:dev

# Сборка AppImage + deb (тонкий AppImage на системных библиотеках)
bash scripts/build-appimage.sh
```

### Важные замечания

- **patchelf ≥0.16** обязателен на хосте (`sudo dnf install patchelf`). Без него `linuxdeploy` корраптит `.init` секции в `.so`
- **deno** и **yt-dlp** НЕ бандлятся в AppImage (тонкий, ~3MB). При запуске: используются системные из `PATH`, иначе скачиваются pinned-версии с sha256-проверкой в `~/.cache/votdesktop/binaries`. Первый запуск без них требует сеть (~10-30s)
- AppImage также не бандлит webkit2gtk/GTK — используются системные библиотеки (иначе бандл из чужого дистрибутива даёт белый webview)
- Детали архитектурных решений (ADR) и внутренние детали сборки — в [docs/REGULATION.md](docs/REGULATION.md)
- `scripts/fetch-deno.sh` / `scripts/fetch-yt-dlp.sh` — опционально предзагружают кэш до первого запуска

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
