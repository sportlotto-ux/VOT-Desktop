# Регламент разработки VotDesktop

Десктоп-приложение для скачивания видео с YouTube и автоматическим микшированием голосового перевода Яндекса (VOT).

## 1. Цели

| # | Цель | Метрика успеха |
|---|---|---|
| G1 | Скачивать видео с YouTube в выбранном качестве | Работает без кук и с куками (как Parabolic) |
| G2 | Получать русский голосовой перевод через VOT | MP3 от `vot-cli-live` в течение 5 мин |
| G3 | Микшировать оригинал + перевод в один файл | Готовый `.mixed.<ext>`, оба голоса слышны |
| G4 | Поставляться одним файлом (AppImage) | `chmod +x && ./VotDesktop.AppImage` — работает |
| G5 | Не зависеть от mediabot2.0 в runtime | Self-contained, никаких импортов из bot |

## 2. НЕ-цели (out of scope)

- ❌ GUI для управления плейлистами, библиотекой, AI-описаниями
- ❌ Публикация в Telegram-каналы
- ❌ Поддержка Windows / macOS (только Linux x86_64)
- ❌ Авторизация / OAuth (только cookies-файл от пользователя)
- ❌ Скачивание с VK, OK, Twitch (только YouTube через `vot-cli-live`)
- ❌ Свой переводчик (только Яндекс через VOT)
- ❌ Multi-user / профили (один пользователь = один конфиг)

## 3. Стек

| Слой | Технология | Версия | Обоснование |
|---|---|---|---|
| GUI Framework | Tauri | 2.x | Маленький бинарь, нативный WebView, Rust-perf |
| Frontend | TypeScript + Vite | TS 7.x | Без фреймворков (vanilla) — минимум зависимостей |
| Backend IPC | Tauri Commands (Rust) | — | Прямой вызов Rust-функций из TS |
| Downloader | yt-dlp (системный или bundled) | latest | Стандарт de-facto |
| VOT Translation | vot-cli-live (npm) | 1.7.x | Уже используется в mediabot2.0 |
| Audio Mix | ffmpeg (системный или static) | 6.x+ | filter_complex amix/loudnorm |
| Distribution | AppImage (тонкий) | — | Single-file, no install |

## 4. Структура проекта

```
vot-desktop/
├── docs/
│   └── REGULATION.md              # этот файл
├── src/                           # Frontend (TypeScript)
│   ├── main.ts                    # точка входа, Tauri command bindings
│   ├── ui.ts                      # рендеринг форм, прогресс-бар
│   ├── ipc.ts                     # обёртки над tauri.invoke
│   ├── styles.css                 # нативный вид GTK-like
│   └── types.ts                   # DTO между frontend и Rust
├── src-tauri/                     # Backend (Rust)
│   ├── src/
│   │   ├── main.rs                # Tauri entrypoint
│   │   ├── commands.rs            # #[tauri::command] хендлеры
│   │   ├── downloader.rs          # обёртка над yt-dlp subprocess
│   │   ├── translator.rs          # обёртка над vot-cli-live subprocess
│   │   ├── mixer.rs               # обёртка над ffmpeg subprocess
│   │   ├── deps.rs                # проверка/установка зависимостей
│   │   └── error.rs               # типизированные ошибки для IPC
│   ├── binaries/                  # bundled бинарники (для AppImage)
│   │   └── yt-dlp-x86_64
│   ├── icons/                     # иконки приложения
│   ├── tauri.conf.json
│   ├── Cargo.toml
│   └── build.rs
├── scripts/
│   ├── dev.sh                     # запуск в dev-режиме
│   ├── build-appimage.sh          # сборка .AppImage
│   ├── fetch-yt-dlp.sh            # скачать static yt-dlp в binaries/
│   └── smoke-test.sh              # e2e: URL → готовый .mixed.<ext>
├── tests/
│   ├── unit/
│   │   ├── mixer.test.ts          # парсинг аргументов ffmpeg
│   │   └── translator.test.ts     # парсинг JSON от vot-cli-live
│   └── integration/
│       └── e2e.sh                 # скачать известное видео, проверить выход
├── .gitignore
├── package.json
├── tsconfig.json
├── vite.config.ts
├── index.html
└── README.md
```

## 5. Архитектурные решения

### ADR-001: Tauri-side subprocess, не HTTP-сервер

**Решение:** Все внешние вызовы (yt-dlp, vot-cli-live, ffmpeg) делаются из Rust через `tokio::process::Command`. Tauri-frontend вызывает их через `#[tauri::command]`.

**Альтернативы:**
- Python-sidecar через HTTP — отвергнуто: +200 МБ Python runtime в AppImage, лишний слой
- WASM-порт yt-dlp — не существует

**Последствия:**
- ✅ Один бинарь, нет Python
- ✅ Стриминг stdout в UI через `tauri::Window::emit`
- ⚠️ Все subprocess-вызовы — async, не блокируем UI-поток

### ADR-002: Гибридный AppImage — bundle deno, require system ffmpeg

**Целевые дистрибутивы: rolling-сборки (Fedora, Arch, openSUSE Tumbleweed, Ubuntu 24.04+/Debian 13+).** Старые LTS (Ubuntu 22.04 и ниже) **не поддерживаются**: бинарь линкуется с glibc хоста сборки (≥2.39), сборка в старом контейнере не требуется.

**Решение:** В AppImage бандлим:
- Tauri-бинарь (~8 МБ)
- `yt-dlp` (статический, ~15 МБ) — редко обновляется, автообновление внутри yt-dlp
- `deno` (статический, ~50 МБ) — ни у кого в системе не установлен, тащить обязательно

`ffmpeg` (~80 МБ static build) НЕ бандлим — он есть в 95% десктоп-Linux (Ubuntu/Fedora/Mint). При старте приложения:
- `which ffmpeg` → если нет, показать модалку:  
  `sudo apt install ffmpeg` (или эквивалент для Arch/Fedora) → закрыть приложение
- Никаких runtime-загрузок, никаких checksum, никаких retry-логики

**Альтернативы (отвергнуты):**
- **Полностью тонкий** AppImage + runtime-докачка ffmpeg/deno — отвергнуто: это мини-пакетный-менеджер внутри приложения, требует сетевого кода, валидации, UX прогресса. Больше кода и точек отказа, чем сэкономлено МБ.
- **Полностью толстый** AppImage (~150 МБ со static ffmpeg) — рабочий вариант, но ffmpeg внутри AppImage живёт в squashfs и пути к нему ломаются, нужны костыли (`APPIMAGE`-env-var хаки).
- **.deb** — параллельно, для удобства (`tauri build --bundles appimage,deb`).

**Размер итогового AppImage:** ~75 МБ. Допустимо.

**Последствия:**
- ✅ Нет сетевого кода в рантайме
- ✅ Нет мини-пакетного-менеджера
- ✅ ffmpeg-бинарь из системы — нет проблем glibc-совместимости (статик бы потребовал glibc 2.31+, что исключило бы Ubuntu 18.04)
- ⚠️ Требуем ffmpeg — на серверных/minimal-дистрибутивах без него приложение не стартует (приемлемо: это desktop-приложение)
- ⚠️ Деплой deno в AppImage — нужно проверить работу `deno` из squashfs (может быть `APPIMAGE=1`-aware)

### ADR-003: Переиспользование filter_complex из mediabot2.0 (с версионированием)

**Решение:** Копируем filter_complex из `media_utils.py:369-371` в `src-tauri/src/mixer.rs` с явным версионированием.

**Структура комментария в `mixer.rs`:**
```rust
// MIXER_PRESET_VERSION = 1
// Source: /home/user/podman/triada/workspace/mediabot2.0/src/handlers/media_utils.py
// Source lines: 369-371
// Source sha256: <вычислить при копировании, например 8a3f...>
// Last synced: 2026-07-20
// Synced by:   <username>
//
// При обновлении источника: bump MIXER_PRESET_VERSION, пересчитать sha256, обновить дату.
const MIXER_PRESET_VERSION: u32 = 1;
const FILTER_COMPLEX: &str = "[1:a]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,pan=stereo|c0=c0|c1=c0[ru];\
                              [0:a:0]loudnorm=I=-16:TP=-0.5:LRA=11,aresample=44100,volume=0.33[en];\
                              [ru][en]amix=inputs=2:duration=longest:normalize=0[mix]";
```

**Build-time валидация (опционально, для CI):**
`src-tauri/build.rs` при `cargo build`:
1. Читает исходный `media_utils.py` (путь через env var `VOT_MEDIABOT_SRC`, иначе skip)
2. Считает sha256 строк 369-371
3. Сравнивает с хардкод-значением `MIXER_PRESET_SOURCE_SHA256` в `build.rs`
4. Если не совпадает — `panic!("mixer.rs out of sync with media_utils.py:{})`, иначе cargo собирает нормально

Это превращает «человек должен вспомнить» в «CI ломает сборку» — невозможно задеплоить устаревший миксер.

**Альтернатива (отвергнута):** вынести filter_complex в общий крейт/пакет с mediabot2.0. Сложно, потому что mediabot2.0 — Python, а VotDesktop — Rust. Общий пакет = переписывать mediabot2.0 или поднимать FFI. Overkill.

**Последствия:**
- ✅ Проверенный микс (уже работает у пользователя в боте)
- ✅ Build-time валидация синхронизации
- ⚠️ build.rs зависит от пути к mediabot2.0 — нужен env var + `.cargo/config.toml` для CI
- ⚠️ Если mediabot2.0 — отдельный клон для других разработчиков, build.rs может указывать в пустоту → делаем soft-fail (warning, не panic), если файл не найден

### ADR-004: Без UI-фреймворка (vanilla TS)

**Решение:** Никаких React/Svelte/Vue. Только vanilla TypeScript + DOM API + Vite.

**Обоснование:**
- Окно одно, форм три (URL, качество, прогресс) — фреймворк overkill
- Меньше зависимостей → меньше бинарь → проще аудит
- Все компоненты — функции, рендер — innerHTML/appendChild

**Последствия:**
- ✅ ~50 КБ JS (vs 200+ КБ для Svelte)
- ⚠️ Код рендеринга менее структурирован — компенсируем чётким разделением `ui.ts` / `ipc.ts`

### ADR-005: cookies — только через файл, никаких browser-extensions

**Решение:** Пользователь вручную экспортирует cookies.txt через расширение "Get cookies.txt LOCALLY" и указывает путь в настройках приложения.

**Альтернативы:**
- Интеграция с browser-cookie3 (Python) — не наш стек
- Своё расширение для Chromium — overkill

**Последствия:**
- ✅ Просто, прозрачно, как в Parabolic
- ⚠️ Файл устаревает — показывать дату последней модификации в UI

### ADR-006: Pinned bundled binaries — no runtime downloads

**Решение:** yt-dlp (`2026.07.04`), deno (`2.9.5`), `vot-cli-live@1.7.5` закреплены версиями. `fetch-deno.sh`/`fetch-yt-dlp.sh` качают pinned архив+бинарь и проверяют sha256 (для deno — и бинарь, и архив). В рантайме `binaries.rs` резолвит bundled (`resource_dir/binaries`) или PATH с `probe_version` (≥ минимума). Runtime-загрузки (curl/unzip/`releases/latest`) удалены из Rust.

**Альтернативы:**
- latest + автообновление — отвергнуто: supply-chain риск, ломающие версии
- Только системные бинарники — отвергнуто: deno ни у кого нет, yt-dlp версии расходятся

**Последствия:**
- Воспроизводимые сборки; `build-appimage.sh` поддерживает `SKIP_FETCH=1` с проверкой checksum
- AppImage: `bundle.resources` = `binaries/{deno,yt-dlp}`; bump версии = осознанное действие + обновление sha256

### ADR-007: Sandboxed VOT subprocess — env_clear + scoped permissions

**Решение:** `vot-cli-live` через deno с минимальными правами:
- deno: `--allow-net`, `--allow-env`, scoped `--allow-read=<work_dir>,<HOME>/.cache/deno`, scoped `--allow-write=<work_dir>`
- `env_clear()` + whitelist: `PATH=/usr/bin:/bin`, `HOME`, `TERM=xterm`, `CI=1`, `FORCE_COLOR=1`
- Опытным путём: `vot-cli-live` (chalk/colorette/listr2) требует env-доступ; deno пишет npm-кэш без явного `allow-write`; нужен read кэша для своего package.json

**Альтернативы:**
- `--allow-all` — отвергнуто: полный доступ
- Scoped `--allow-env=NAME` — отвергнуто: whack-a-mole, список переменных нестабилен

**Последствия:**
- Минимальная поверхность атаки для произвольного URL; `VOT_TIMEOUT=300s` + `kill_on_drop`
- Требуется HOME в env для deno-кэша; `work_dir` изолирует вывод

### ADR-008: Isolated work-dir model + graceful VOT fallback

**Решение:** при переводе загрузка идёт в уникальную work-директорию (`.vot-process-{pid}-{attempt}`); финал (mixed) — в `output_dir`; `work_dir` чистится всегда (в т.ч. при ошибке). Поток: video stream (`video_only_format`) → audio (`bestaudio[ext=m4a]/bestaudio`) → translation (в свой `.vot-work-*`) → mix → `output_dir/{stem}.mixed.{ext}`. Fallback: если `vot-cli-live` чисто завершился, но перевода нет (`"not found"` → `Ok(None)`) — качаем комбинированный формат (`format_with_best_audio`) в `output_dir` и возвращаем видео без микса.

**Альтернативы:**
- Всё в `output_dir` — отвергнуто: мусор при падении
- Жёсткая ошибка без fallback — отвергнуто: UX-регресс

**Последствия:**
- Частичные стримы не просачиваются в пользовательскую папку; fallback сохраняет ценность результата

### ADR-009: Typed pipeline — ProcessContext / SelectionKind / Artifact

**Решение:** оркестрация из `commands.rs` перенесена в `pipeline.rs` (`run_process` → `run_video`/`run_audio`). UI-события за трейтом `PipelineEvents` (impl `tauri::Window`, mock в тестах). Артефакты — типизированные `Artifact { path, kind: ArtifactKind }` (Video/Audio/Translation/Mixed); `ProcessResponse` строится через `Artifact::into_response()` по kind. Эвристика «audio-only по строке format_id» удалена — фронт явно шлёт `SelectionKind` (video|audio, serde snake_case).

**Альтернативы:**
- Монолитный `start_process` с эвристиками — отвергнуто: 150+ строк, хрупко, нетестируемо
- События через window прямо в pipeline — отвергнуто: не тестируется без Tauri

**Последствия:**
- `commands.rs` — тонкая обёртка; 10 Rust unit-тестов; поле `request.kind` обязательно с фронта

### ADR-010: Codec-aware mix output container

**Решение:** `mixer.mix()` сам выбирает контейнер/аудиокодек по видеокодеку (`probe_video_codec` → `output_profile`): vp8/vp9 → webm + libopus; остальное (h264, av1, unknown) → mp4 + aac. Видео всегда `-c:v copy`. Выход: `output_dir/{stem}.mixed.{container}`; `mix()` возвращает `PathBuf`.

**Альтернативы:**
- Всегда mp4+aac — отвергнуто: vp8/vp9 копией в mp4 ломаются
- Транскодировать видео — отвергнуто: потеря качества/время

**Последствия:**
- YouTube-стримы (av01/h264/vp9) упаковываются корректно; smoke-test повторяет логику выбора контейнера
- Требуется ffprobe (в составе ffmpeg); unit-тесты `vp9_maps_to_webm` / `h264_and_unknown_map_to_mp4`

### ADR-011: TypeScript 7 (native Go-компилятор)

**Решение:** typescript 5.6 → 7.0.2. TS7 — нативная (Go) реализация, используется только для typecheck (`tsc --noEmit`); транспиляция остаётся на esbuild (Vite 8). Добавлен `src/vite-env.d.ts` с `/// <reference types="vite/client" />` — TS7, в отличие от TS5, не прощает CSS side-effect import (TS2882).

**Последствия:**
- Быстрый typecheck, тот же `tsconfig.json`
- Vite-специфичные модули (CSS-импорты) требуют `vite/client` деклараций
- pin `^7.0.2` в package.json

### ADR-012: Thin AppImage — external yt-dlp/deno, runtime download

**Решение:** yt-dlp и deno **не бандлятся** в AppImage (тонкий, ~88MB). Резолв при запуске (src-tauri/src/binaries.rs):
1. системный бинарь из `PATH` (пользователь обновил сам — новая версия подхватывается без пересборки приложения);
2. кэш `~/.cache/votdesktop/binaries` (pinned, уже скачан);
3. скачивание pinned-версии (yt-dlp 2026.07.04, deno 2.9.5) с **обязательной sha256-проверкой** (архив и бинарь для deno).

**Причина:** выпуск нового yt-dlp/deno не должен требовать пересборки AppImage. Пользователь обновляет системный пакет — приложение подхватывает его; если его нет — приложение само ставит pinned версию в кэш.

**Альтернативы:**
- Bundled (ADR-006) — отвергнуто: каждый выход deno/yt-dlp требует пересборки контейнера
- Только системные, без скачивания — отвергнуто: на чистой системе приложение неработоспособно без deno

**Последствия:**
- AppImage: 88MB (было 156MB), .deb: 2.3MB
- Первый запуск без deno/yt-dlp требует сеть + ~10-30s (скачивание + sha256)
- Нет bundled-бинарей → `bundle.resources` в tauri.conf.json пуст
- `build-appimage.sh` больше не вызывает fetch-скрипты; `SKIP_FETCH` удалён
- Сетевые тесты скачивания — `#[ignore]` (opt-in, не ломают офлайн-CI)

## 6. Фазы разработки

Каждая фаза — атомарная, имеет критерий «готово». **Не переходить к следующей фазе, пока не выполнен критерий.**

**Оценка сроков:**
- С опытом Tauri 2.x: **4-5 дней** (как в первоначальной версии)
- Без опыта Tauri 2.x: **7-10 дней** (x1.5-2, основные грабли: WebKitGTK 4.1 vs 6.0, AppImage glibc mismatch, dev-mode vs bundled-mode поведение, CSP для local resources, `tauri.conf.json` schema-валидация)

### Фаза 0: Скелет (0.5-1 день)

**Задачи:**
- `npm create tauri-app` с шаблоном vanilla TS
- Удалить дефолтный UI
- Настроить `tauri.conf.json` (window title, icon, bundle target=appimage)
- На Linux-хосте проверить версии: `pkg-config --modversion webkit2gtk-4.1` (должна быть ≥ 4.1), `ldd --version | head -1` (glibc ≥ 2.31)

**Критерий готово:**
- `npm run tauri dev` открывает пустое окно с заголовком "VotDesktop"
- `npm run tauri build` создаёт AppImage, который запускается на хост-машине

### Фаза 1: Скачивание (1-2 дня)

**Задачи:**
- `src-tauri/src/downloader.rs`: `pub async fn fetch_formats(url: &str) -> Result<Vec<Format>>` через `yt-dlp --dump-json`
- `src-tauri/src/downloader.rs`: `pub async fn download(url: &str, format_id: &str, cookies: Option<&Path>) -> Result<PathBuf>` через `yt-dlp -f <id> -o <path>`
- Парсинг JSON в Rust-структуру `Format { id, quality, ext, filesize }`
- Стриминг прогресса через `tauri::Window::emit("download-progress", percent)`
- Валидация URL на входе (см. секцию 7 «безопасность subprocess»)

**Критерий готово:**
- В UI ввод URL → парсинг → список качеств в `<select>`
- Выбор качества → скачивание → файл на диске
- Прогресс-бар обновляется в реальном времени
- Без кук и с куками (путь к cookies-файлу из UI через Tauri dialog)

### Фаза 2: Перевод VOT (1-2 дня)

**Задачи:**
- `src-tauri/src/translator.rs`: `pub async fn fetch_translation(video_url: &str, output_dir: &Path) -> Result<Option<PathBuf>>` через pinned `vot-cli-live@1.7.5` в изолированной рабочей директории с `--allow-net`, `--allow-read=<workdir>` и `--allow-write=<workdir>`
- Таймаут 300с, обработка non-zero exit
- Fallback: если перевод не получен → возврат `Ok(None)`, фронт показывает предупреждение и сохраняет видео с оригинальным аудио без микса

**Критерий готово:**
- После скачивания видео автоматически вызывается VOT
- Получаем MP3 в `output_dir/`
- В UI прогресс-бар «Получаю перевод Яндексом...» 0-100%
- Если VOT не нашёл перевод — UI говорит «Перевод недоступен, сохранено без микса»

### Фаза 3: Микширование (0.5-1 день)

**Задачи:**
- `src-tauri/src/mixer.rs`: `pub async fn mix(video: &Path, russian_mp3: &Path, output: &Path) -> Result<()>` через `ffmpeg -i video -i mp3 -filter_complex <...> -y output.mp4`
- Скопировать filter_complex из mediabot2.0 (ADR-003, обновлённый — с `MIXER_PRESET_VERSION`)
- Обработка ошибок ffmpeg (exit != 0)
- `src-tauri/build.rs` — проверка sha256 источника filter_complex, fail-build при изменении (см. ADR-003)

**Критерий готово:**
- На вход: `video.mp4` + `ru.mp3` → на выходе: `video.mixed.mp4` (или `webm`, если видеокодек vp8/vp9)
- В `video.mixed.mp4` оба голоса слышны (ru громче, en тише на фоне)
- Длительность совпадает с оригиналом
- При изменении `media_utils.py` в mediabot2.0 — `cargo build` падает с подсказкой «обнови MIXER_PRESET_VERSION»

### Фаза 4: UI полировка (1-2 дня)

**Задачи:**
- Кнопка выбора cookies-файла (Tauri dialog plugin)
- При старте: проверка `which ffmpeg`, если нет — модалка с командой установки (ADR-002)
- Настройки пути вывода (по умолчанию `~/Videos/VotDesktop/`)
- Лог-панель со stderr subprocess-вызовов
- Горячие клавиши (Ctrl+V — вставить URL, Enter — скачать)

**Критерий готово:**
- Окно не выглядит как MVP
- Все ошибки читаемы для пользователя
- Поведение при отсутствии ffmpeg проверено вручную

### Фаза 5: Упаковка (1-2 дня)

**Задачи:**
- `scripts/fetch-yt-dlp.sh` — скачивает latest static yt-dlp в `src-tauri/binaries/`
- `scripts/fetch-deno.sh` — скачивает latest deno static в `src-tauri/binaries/`
- `scripts/build-appimage.sh` — `npm run tauri build -- --bundles appimage,deb`
- Проверка: AppImage на чистой Ubuntu 22.04 через `podman run --rm -v $PWD:/out ubuntu:22.04 bash -c 'apt-get update && apt-get install -y ffmpeg && /out/VotDesktop.AppImage'`
- Проверка: AppImage на чистом Fedora/Alpine (если есть доступ)

**Критерий готово:**
- `VotDesktop-x86_64.AppImage` размером ~75 МБ
- Запускается на чистой Ubuntu 22.04 (с предустановленным ffmpeg) — без сети в рантайме
- Выдаёт понятную ошибку, если ffmpeg не установлен
- README с инструкцией для пользователя + для разработчика

## 7. Стандарты кода

### Rust
- Edition 2021
- `cargo fmt` + `cargo clippy -- -D warnings` без ошибок
- Все `#[tauri::command]` возвращают `Result<T, AppError>`, где `AppError` сериализуем в JSON
- `tokio::process::Command` для subprocess, **не** `std::process::Command`
- Никаких `unwrap()` в production-коде, только в тестах

### TypeScript
- Strict mode (`"strict": true` в `tsconfig.json`)
- Никаких `any`, используем `unknown` + type guards
- Импорты — ES modules, никаких `require()`
- Форматирование: Prettier defaults (без semicolon, single quotes)

### Именование
- Файлы: snake_case для Rust, kebab-case для TS (`mixer.rs`, `ui-renderer.ts`)
- Функции: snake_case в Rust, camelCase в TS
- Tauri commands: kebab-case в IPC (`invoke('start-download')`)

### Ошибки
- Все ошибки от subprocess — проброс в UI с человеческим сообщением
- Никаких `panic!` в Rust-коде, всегда `Result`
- Логирование: `tracing` (Rust) + `console` (TS)

### Безопасность subprocess (ОБЯЗАТЕЛЬНО)

**Цель:** не допустить command injection через пользовательский ввод (URL, путь к cookies).

**Правила:**

1. **НИКОГДА не использовать shell.** Только прямой вызов бинаря:
   ```rust
   // ✅ ПРАВИЛЬНО
   Command::new("yt-dlp").arg("--dump-json").arg(&url).spawn()
   
   // ❌ ЗАПРЕЩЕНО
   Command::new("sh").arg("-c").arg(format!("yt-dlp --dump-json '{}'", url)).spawn()
   ```

2. **НИКОГДА не собирать команду строковой конкатенацией.** Каждый аргумент — отдельный `.arg()`. Даже для флагов со значением:
   ```rust
   // ✅ ПРАВИЛЬНО
   Command::new("yt-dlp").arg("-f").arg(&format_id).arg("-o").arg(&output_path)
   
   // ❌ ЗАПРЕЩЕНО
   Command::new("sh").arg("-c").arg(format!("yt-dlp -f {} -o {}", format_id, output_path))
   ```

3. **Валидация входов на границе системы.** В `commands.rs`:
   - `validate_url(url: &str) -> Result<()>` — допускает только `https://(www\.)?(youtube\.com|youtu\.be)/...`. Всё остальное → `AppError::InvalidUrl`.
   - `validate_cookies_path(path: &Path) -> Result<()>` — проверяет, что файл существует, это regular file (не symlink на /etc/shadow), owned by текущим UID (через `std::os::unix::fs::MetadataExt::uid()`) и не превышает лимит размера.
   - `validate_format_id(id: &str) -> Result<()>` — ASCII-символы синтаксиса yt-dlp, длина ≤ 128.

4. **Запрещённые символы в URL/format_id/path:** даже после валидации — двойная проверка через `assert!(!arg.contains([';', '|', '`', '$', '\n', '\0']))` перед передачей в `.arg()`. Tokio не интерпретирует эти символы, но это страховка от регрессий при рефакторинге.

5. **Линтер:** `cargo clippy` с линтами `clippy::disallowed_methods` для `Command::new("sh")` / `Command::new("bash")` — запрет вкомпилирован на уровне CI.

6. **При ревью PR:** любой `Command::new(...)` без явного списка аргументов через `.arg()` — бан. Каждый вызов — обоснован, что входы провалидированы.

**Источник угрозы:** пользователь (вы сами) вводит URL в UI. Даже собственный ввод может содержать `; rm -rf ~` если случайно скопировал не туда. Защита должна быть в коде, а не «в голове».

## 8. Поток данных

```
┌──────────┐  invoke('start-download', {url, formatId, cookiesPath})
│ Frontend ├─────────────────────────────────────────┐
│ (TS/Vite) │                                         ▼
└──────────┘  events: 'progress' ◄─────┐  ┌─────────────────────┐
                                     │  │ Rust backend        │
                                     │  │ (Tauri commands)    │
                                     │  ├─────────────────────┤
                                     │  │ 1. yt-dlp --dump-   │
                                     │  │    json → formats   │
                                     │  │ 2. yt-dlp -f ID     │
                                     │  │    -o FILE URL      │
                                     │  │ 3. vot-cli-live     │
                                     │  │    --output DIR URL │
                                     │  │ 4. ffmpeg -i V -i R │
                                     │  │    -filter_complex  │
                                     │  │    → MIXED.mp4      │
                                     │  └─────────────────────┘
                                     │
                          progress: 0%, 30%, 100%
                          log: "vot-cli-live: success"
```

## 9. Зависимости (system requirements для запуска)

| Зависимость | Минимум | Источник | Если отсутствует |
|---|---|---|---|
| glibc | 2.31+ (Ubuntu 20.04+) | Системная | Сообщить пользователю (приложение не запустится) |
| WebKitGTK | 4.1+ (Tauri 2.x) | Системная | `sudo apt install libwebkit2gtk-4.1-dev` (для запуска бинаря нужна runtime-версия) |
| ffmpeg | 6.0+ | **Системная** (ADR-002) | Модалка: `sudo apt install ffmpeg` (или dnf/pacman) → exit |
| deno | 1.40+ | **В AppImage** (ADR-002) | Не нужно — бандлим |
| yt-dlp | latest | **В AppImage** (ADR-002) | Не нужно — бандлим |

## 10. Риски

| Риск | Вероятность | Влияние | Митигация |
|---|---|---|---|
| `vot-cli-live` ломается (новый API Яндекса) | Средняя | Высокое | Версионировать вызов, fallback на оригинал без микса |
| `vot-cli-live` недоступен в npm | Низкая | Критическое | deno кэширует npm-пакет после первого запуска; зеркало через GitHub Releases вручную |
| Tauri 2.x breaking changes (без опыта) | Высокая | Высокое | Закрепить версию в `Cargo.toml` + `package.json`, обновлять осознанно; см. ADR-002 пересмотренный |
| WebKitGTK 4.1 не установлен на хосте | Средняя | Высокое | README с командой `apt install`; pre-flight check в `main.rs` |
| AppImage glibc mismatch на старых дистрибутивах | Средняя | Среднее | Проверка `ldd --version` при старте, понятная ошибка |
| yt-dlp 403 на YouTube (rate limit) | Средняя | Низкое | Retry с backoff внутри `downloader.rs`, инструкция «обновите yt-dlp» |
| 5-минутный таймаут VOT раздражает юзера | Высокая | Низкое | Прогресс-бар + лог + кнопка «Отмена» (через `tokio::select!` с cancel-channel) |
| filter_complex в mediabot2.0 обновился — мы забыли | Средняя | Низкое | build.rs с sha256 валидацией (ADR-003) |
| Command injection через URL/cookies | Низкая (один юзер) | Критическое | Секция 7 «Безопасность subprocess»: валидация + запрет shell + clippy lint |

## 11. Что НЕ делается

Для контроля scope — запрещено без явного запроса:

- ❌ Поддержка нескольких пользователей / профилей
- ❌ Синхронизация с облаком / базой данных
- ❌ AI-описания, AI-модерация
- ❌ Публикация куда-либо (только сохранение на диск)
- ❌ Поддержка платформ кроме YouTube
- ❌ Drag-n-drop URL из браузера
- ❌ Очередь загрузок (одна за раз)
- ❌ История скачанного
- ❌ Настройки громкости микса (хардкод 0.33/1.0)

## 12. Определение «сделано» (Definition of Done)

Проект считается завершённым, когда:

- [ ] `scripts/build-appimage.sh` создаёт AppImage без warnings
- [ ] AppImage запускается на чистом rolling-дистрибутиве (Fedora/Arch; Ubuntu 24.04+ как fallback) в podman-контейнере
- [ ] Без ffmpeg на хосте — приложение показывает модалку с командой установки и exit (НЕ крашится)
- [ ] С ffmpeg — полный цикл: URL → выбор качества → скачивание → VOT → микс → готовый файл
- [ ] Smoke-test проходит: известный URL → готовый `.mixed.<ext>` за <10 мин
- [ ] Без cookies: публичные видео скачиваются
- [ ] С cookies: приватные/age-restricted видео скачиваются
- [ ] VOT-перевод получается для видео из кэша Яндекса
- [ ] Без VOT (видео не в кэше): выдаётся оригинал + предупреждение в UI
- [ ] Микшированный файл проигрывается в mpv/vlc с обоими голосами
- [ ] README с инструкцией для пользователя + для разработчика
- [ ] Все Rust-файлы проходят `cargo clippy -- -D warnings` (включая security-лины из секции 7)
- [ ] Все TS-файлы проходят `tsc --noEmit`
- [ ] `mixer.rs` содержит `MIXER_PRESET_VERSION` + sha256 источника + build.rs валидацию (ADR-003)

---

## История изменений

| Версия | Дата | Что изменилось |
|---|---|---|
| 1.0 | 2026-07-20 | Первая версия |
| 1.1 | 2026-07-20 | Ревью feedback (ADR-002 → гибридный AppImage; сроки x1.5-2 без опыта Tauri; секция «Безопасность subprocess»; build.rs sha256 для mixer.rs) |
| 1.2 | 2026-08-12 | Фазы 1–4 реализованы: pinned бинарники (ADR-006), sandboxed VOT (ADR-007), work-dir + fallback (ADR-008), typed pipeline (ADR-009), codec-aware mix (ADR-010); UI-полировка (hotkeys, cookies age, ffmpeg status) |
| 1.3 | 2026-08-12 | Решение по фаза 5: rolling-only дистрибутивы, старые LTS не поддерживаются (glibc ≥2.39) |
| 1.4 | 2026-08-12 | TypeScript 5.6 → 7.0 (native Go-компилятор); добавлен src/vite-env.d.ts для CSS side-effect импорта |
| 1.5 | 2026-08-12 | AppImage перепакован с xz/1M сжатием (177→156MB, −12%); linuxdeploy zstd/16K блочит размер |
| 1.6 | 2026-08-12 | ADR-012: тонкий AppImage (88MB) — external yt-dlp/deno, runtime-скачивание в ~/.cache/votdesktop/binaries с sha256 |

---

**Текущая версия документа:** 1.6
**Дата:** 2026-08-12
**Автор:** Claude (opencode)
**Связанные проекты:** mediabot2.0 (источник filter_complex и vot-cli-live команды)
