import type { Format, UpdateInfo } from './types';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { getVersion } from '@tauri-apps/api/app';
import {
  fetchFormats,
  startProcess,
  pickCookiesFile,
  pickOutputDir,
  cookiesInfo,
  runtimeVersions,
  geminiModels,
  translateDescription,
  updateBinary,
} from './ipc';

const DEFAULT_OUTPUT_DIR = '~/Videos/VotDesktop';
const STALE_COOKIES_DAYS = 30;
const AI_KEY_STORAGE = 'vot-ai-api-key';
const AI_MODEL_STORAGE = 'vot-ai-model';

let currentFormats: Format[] = [];
let currentDescription: string | null = null;
/** Parent dir of the last downloaded artifact — where description.ru.txt goes. */
let lastResultDir: string | null = null;
let cookiesPath: string | null = null;
let outputDir: string | null = null;
let isProcessing = false;
let isTranslating = false;
/** True when Yandex has no cached translation — the file is saved without  озвучки. */
let missingTranslation = false;

function getAiKey(): string {
  return localStorage.getItem(AI_KEY_STORAGE)?.trim() ?? '';
}

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

export function init(): void {
  render();
  bindEvents();
  bindRuntimeListeners();
  void refreshComponentVersions();
  if (getAiKey()) void loadAiModels();
  void getVersion()
    .then((v) => {
      $<HTMLElement>('app-version').textContent = `v${v}`;
    })
    .catch(() => {
      /* version display is cosmetic */
    });
}

function render(): void {
  const app = $<HTMLElement>('app');
  app.innerHTML = `
    <div class="container">
      <div class="header-row">
        <h1>VotDesktop</h1>
        <img id="aieauz-banner" src="/aieauz.png" alt="aiera.uz"
          title="aiera.uz" draggable="false" />
      </div>

      <!-- Остров 1: ссылка YouTube -->
      <section class="card">
        <label for="url-input">YouTube URL</label>
        <input type="url" id="url-input"
          placeholder="https://youtube.com/watch?v=..." />

        <div class="row">
          <button id="fetch-btn">Получить форматы</button>
        </div>
      </section>

      <!-- Остров 2: форматы -->
      <section id="format-section" class="hidden">
        <div class="format-row">
          <div>
            <label>Container</label>
            <div class="radio-group" id="container-group"></div>
          </div>
          <div>
            <label>Type</label>
            <div class="radio-group" id="type-group"></div>
          </div>
          <div>
            <label>Quality</label>
            <div class="radio-group" id="quality-group"></div>
          </div>
        </div>
      </section>

      <!-- Остров 3: озвучка VOT -->
      <section class="card vot-card">
        <div class="checkbox-row">
          <input type="checkbox" id="mix-check" checked />
          <label for="mix-check">Микшировать с русской озвучкой (Яндекс VOT)</label>
        </div>
      </section>

      <!-- Остров 4: перевод описания через AI Studio -->
      <section class="card ai-card">
        <div class="option-row">
          <span class="label">API-ключ</span>
          <input type="password" id="ai-key-input"
            placeholder="Вставьте ключ с aistudio.google.com/apikey" autocomplete="off" />
        </div>
        <p class="hint left">Нужен только для перевода описания. Получить бесплатно: aistudio.google.com/apikey</p>
        <div class="option-row">
          <span class="label">Модель ИИ</span>
          <select id="ai-model-select" disabled>
            <option value="">Введите API-ключ…</option>
          </select>
          <button id="translate-desc-btn" class="small" disabled>Перевести описание</button>
        </div>
      </section>

      <!-- Острова 5+: настройки -->
      <section class="card">
        <div class="option-row">
          <span class="label">Cookies</span>
          <span id="cookies-status" class="value">нет</span>
          <button id="cookies-btn" class="small">Выбрать файл...</button>
          <button id="cookies-clear" class="small hidden">Сбросить</button>
        </div>
      </section>

      <section class="card">
        <div class="option-row">
          <span class="label">Папка загрузки</span>
          <span id="output-path" class="value">${DEFAULT_OUTPUT_DIR}</span>
          <button id="output-btn" class="small">Выбрать...</button>
        </div>
      </section>

      <section class="card">
        <div class="option-row">
          <span class="label">Компоненты</span>
          <span id="components-status" class="value">проверка…</span>
        </div>
      </section>

      <section id="update-section" class="hidden">
        <div class="option-row">
          <span class="label">Обновления</span>
          <span id="update-status" class="value"></span>
          <button id="update-btn" class="small">Обновить</button>
        </div>
      </section>

      <button id="process-btn" disabled class="primary">Скачать</button>
      <div class="hint">Enter — скачать · Ctrl+V — вставить URL</div>
      <div id="app-version" class="version-corner"></div>

      <div id="progress-section" class="hidden">
        <label>Progress</label>
        <progress id="progress-bar" value="0" max="100"></progress>
        <span id="progress-text"></span>
      </div>

      <div id="log-header" class="log-header hidden">
        <span class="label">Log</span>
        <button id="log-clear" class="small">Clear</button>
      </div>
      <pre id="log-area" class="hidden"></pre>
      <div id="error-box" class="error hidden"></div>
      <div id="result-box" class="result hidden"></div>
    </div>
  `;
}

function bindEvents(): void {
  $<HTMLButtonElement>('fetch-btn').addEventListener('click', onFetch);
  $<HTMLButtonElement>('cookies-btn').addEventListener('click', onPickCookies);
  $<HTMLButtonElement>('cookies-clear').addEventListener('click', onClearCookies);
  $<HTMLButtonElement>('output-btn').addEventListener('click', onPickOutput);
  $<HTMLButtonElement>('process-btn').addEventListener('click', onProcess);
  $<HTMLButtonElement>('log-clear').addEventListener('click', clearLog);
  $<HTMLButtonElement>('update-btn').addEventListener('click', onUpdateBinary);
  $<HTMLButtonElement>('translate-desc-btn').addEventListener('click', onTranslateDescription);
  $<HTMLImageElement>('aieauz-banner').addEventListener('click', () => {
    void openUrl('https://aiera.uz');
  });
  // Suppress the webview's image context menu (save/copy image) —
  // the banner is an interactive link, not content.
  $<HTMLImageElement>('aieauz-banner').addEventListener('contextmenu', (e) => {
    e.preventDefault();
  });

  const urlInput = $<HTMLInputElement>('url-input');
  const aiKeyInput = $<HTMLInputElement>('ai-key-input');
  aiKeyInput.value = localStorage.getItem(AI_KEY_STORAGE) ?? '';
  let modelsDebounce: ReturnType<typeof setTimeout> | undefined;
  aiKeyInput.addEventListener('input', () => {
    localStorage.setItem(AI_KEY_STORAGE, aiKeyInput.value.trim());
    updateTranslateDescAvailability();
    clearTimeout(modelsDebounce);
    modelsDebounce = setTimeout(loadAiModels, 700);
  });
  const aiModelSelect = $<HTMLSelectElement>('ai-model-select');
  aiModelSelect.addEventListener('change', () => {
    localStorage.setItem(AI_MODEL_STORAGE, aiModelSelect.value);
  });
  urlInput.addEventListener('input', () => {
    $<HTMLButtonElement>('fetch-btn').disabled = urlInput.value.trim() === '';
  });
  urlInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      if ($<HTMLButtonElement>('process-btn').disabled) {
        onFetch();
      } else {
        onProcess();
      }
    }
  });

  // Ctrl+V — вставить URL в поле ввода из буфера обмена.
  document.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 'v') return;
    if (isProcessing) return;
    const target = e.target as HTMLElement | null;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
    e.preventDefault();
    const input = $<HTMLInputElement>('url-input');
    navigator.clipboard
      .readText()
      .then((text) => {
        input.value = text.trim();
        $<HTMLButtonElement>('fetch-btn').disabled = input.value === '';
        input.focus();
      })
      .catch(() => {
        input.focus();
      });
  });
}

function bindRuntimeListeners(): void {
  // Component versions are queried actively in init(); these listeners keep
  // the row fresh if ffmpeg status changes or updates arrive later.
  const setFfmpeg = (text: string, missing: boolean): void => {
    componentVersions.ffmpeg = text;
    renderComponents();
    $<HTMLElement>('components-status').classList.toggle('missing', missing);
  };
  void listen<string>('ffmpeg-status', (event) => setFfmpeg(event.payload, false));
  void listen<string>('ffmpeg-missing', () => setFfmpeg('не найден', true));

  // Surface description translation failures instead of failing silently.
  void listen<string>('translation-error', (event) => {
    log(`Ошибка перевода описания: ${event.payload}`);
  });

  // Yandex has no cached  озвучка for this video — explain the fallback so the
  // user knows why the mix checkbox produced a file without  озвучки.
  void listen('translation-not-found', () => {
    missingTranslation = true;
    log('Озвучка Яндекса для этого видео не найдена — будет сохранена оригинальная дорожка.');
  });

  // Available runtime binary updates (yt-dlp/deno), ADR-012.
  void listen<UpdateInfo[]>('update-available', (event) => {
    pendingUpdates = event.payload;
    renderUpdates();
  });
}

const componentVersions = { ytdlp: '…', deno: '…', ffmpeg: '…' };

function renderComponents(): void {
  $<HTMLElement>('components-status').textContent =
    `yt-dlp ${componentVersions.ytdlp} · deno ${componentVersions.deno} · ffmpeg ${componentVersions.ffmpeg}`;
}

async function refreshComponentVersions(): Promise<void> {
  try {
    const v = await runtimeVersions();
    componentVersions.ytdlp = v.ytdlp ?? 'не найден';
    componentVersions.deno = v.deno ?? 'не найден';
    componentVersions.ffmpeg = v.ffmpeg ?? 'не найден';
  } catch {
    componentVersions.ytdlp = '?';
    componentVersions.deno = '?';
    componentVersions.ffmpeg = '?';
  }
  renderComponents();
}

// ---- AI description translation (manual) ----

async function loadAiModels(): Promise<void> {
  const key = getAiKey();
  const select = $<HTMLSelectElement>('ai-model-select');
  if (!key) {
    select.disabled = true;
    select.innerHTML = '<option value="">Введите API-ключ…</option>';
    return;
  }
  select.disabled = true;
  select.innerHTML = '<option value="">Загрузка моделей…</option>';
  try {
    const models = await geminiModels(key);
    if (models.length === 0) throw new Error('пустой список моделей');
    const saved = localStorage.getItem(AI_MODEL_STORAGE) ?? '';
    select.innerHTML = models
      .map((m) => `<option value="${m}"${m === saved ? ' selected' : ''}>${m}</option>`)
      .join('');
    if (!models.includes(saved)) localStorage.setItem(AI_MODEL_STORAGE, models[0]);
    select.disabled = false;
  } catch (err: unknown) {
    const msg = errMsg(err);
    select.innerHTML = `<option value="">Не удалось получить модели (${msg})</option>`;
    log(`Не удалось получить список моделей: ${msg}`);
  }
}

function updateTranslateDescAvailability(): void {
  $<HTMLButtonElement>('translate-desc-btn').disabled =
    isTranslating || !currentDescription || !getAiKey();
}

async function onTranslateDescription(): Promise<void> {
  const btn = $<HTMLButtonElement>('translate-desc-btn');
  if (isTranslating || !currentDescription || !getAiKey()) return;

  isTranslating = true;
  btn.disabled = true;
  btn.textContent = 'Перевод...';
  hideError();
  try {
    const text = await translateDescription({
      description: currentDescription,
      api_key: getAiKey(),
      model: $<HTMLSelectElement>('ai-model-select').value || undefined,
      save_dir: lastResultDir ?? undefined,
    });
    showResult(text);
    log(
      lastResultDir
        ? `Перевод сохранён: ${lastResultDir}/description.ru.txt`
        : 'Видео ещё не скачивалось — перевод показан здесь и не сохранён на диск',
    );
  } catch (err: unknown) {
    showError(err);
    log(`Ошибка перевода описания: ${errMsg(err)}`);
  } finally {
    isTranslating = false;
    btn.textContent = 'Перевести описание';
    updateTranslateDescAvailability();
  }
}

let pendingUpdates: UpdateInfo[] = [];
let updatingName: string | null = null;

function renderUpdates(): void {
  const section = $<HTMLElement>('update-section');
  const status = $<HTMLElement>('update-status');
  const btn = $<HTMLButtonElement>('update-btn');

  const target = pendingUpdates.find((u) => u.name === updatingName) ?? pendingUpdates[0];
  if (!target) {
    section.classList.add('hidden');
    return;
  }
  section.classList.remove('hidden');
  btn.textContent = updatingName === target.name && updatingName ? 'Updating...' : 'Update';
  btn.disabled = updatingName !== null;
  const cur = target.current ? `v${target.current}` : 'none';
  status.textContent = `${target.name}: ${cur} → v${target.latest}`;
}

async function onUpdateBinary(): Promise<void> {
  const target = pendingUpdates[0];
  if (!target || updatingName) return;
  updatingName = target.name;
  renderUpdates();
  try {
    const installed = await updateBinary(target.name);
    pendingUpdates = pendingUpdates.filter((u) => u.name !== target.name);
    log(`${target.name} updated to v${installed}`);
  } catch (err: unknown) {
    showError(err);
    log(`update failed: ${errMsg(err)}`);
  } finally {
    updatingName = null;
    renderUpdates();
  }
}

// ---- Fetch ----

async function onFetch(): Promise<void> {
  const url = $<HTMLInputElement>('url-input').value.trim();
  if (!url) return;

  setProgress('Получаю форматы...');
  hideError();
  hideResult();

  try {
    const response = await fetchFormats(url, cookiesPath ?? undefined);
    currentFormats = response.formats;
    currentDescription = response.description;
    updateTranslateDescAvailability();
    setProgress(`Found ${currentFormats.length} formats`);
    populateSelectors(currentFormats);
    enableProcess();
  } catch (err: unknown) {
    showError(err);
    clearProgress();
  }
}

// ---- Format selectors ----

let currentContainer = '';
let currentType = '';

function populateSelectors(formats: Format[]): void {
  const containers = [...new Set(formats.map((f) => f.ext))].sort();
  const types = ['Video+Audio', 'Video', 'Audio'];
  // only show types that actually exist
  const availableTypes = types.filter((t) =>
    formats.some((f) => classify(f) === t),
  );

  currentType = availableTypes[0] ?? '';
  currentContainer =
    containers.find((container) =>
      formats.some((format) => format.ext === container && classify(format) === currentType),
    ) ?? containers[0] ?? '';

  const syncContainerWithType = (): void => {
    const typeContainers = containers.filter((container) =>
      formats.some((format) => format.ext === container && classify(format) === currentType),
    );
    if (!typeContainers.includes(currentContainer)) {
      currentContainer = typeContainers[0] ?? '';
      renderRadioList(
        $('container-group'),
        containers.map((container) => ({ id: container, label: container })),
        currentContainer,
        () => {
          currentContainer = getSelectedFromGroup('container-group');
          renderQuality();
        },
      );
    }
  };

  renderRadioList($('container-group'), containers.map(c => ({ id: c, label: c })), currentContainer, () => {
    currentContainer = getSelectedFromGroup('container-group');
    renderQuality();
  });
  renderRadioList($('type-group'), availableTypes.map(t => ({ id: t, label: t })), currentType, () => {
    currentType = getSelectedFromGroup('type-group');
    syncContainerWithType();
    renderQuality();
  });

  $<HTMLElement>('format-section').classList.remove('hidden');
  renderQuality();
}

const QUALITY_LEVELS = ['2160p', '1440p', '1080p', '720p', '480p', '360p'];

function getSelectedFromGroup(groupId: string): string {
  const checked = document.querySelector<HTMLInputElement>(`#${groupId} input:checked`);
  return checked?.value ?? '';
}

function renderQuality(): void {
  const container = $<HTMLElement>('quality-group');
  container.innerHTML = '';

  if (currentType === 'Audio') {
    // Show bitrate options from yt-dlp format list
    const audioFormats = currentFormats.filter(
      (f) => classify(f) === 'Audio' && f.ext === currentContainer,
    );
    if (audioFormats.length === 0) return;

    const items = audioFormats.map((f) => ({
      id: f.id,
      label: `${f.quality} (${f.filesize})`,
    }));
    renderRadioList(container, items, items[0].id, () => {});
    return;
  }

  // Video: show resolution presets + "Best" option
  const hasMatchingVideo = currentFormats.some((format) => {
    if (format.ext !== currentContainer || !format.has_video) return false;
    return currentType !== 'Video' || !format.has_audio;
  });
  if (!hasMatchingVideo) {
    container.textContent = '— no matching video formats —';
    return;
  }
  const extFilter = currentContainer ? `[ext=${currentContainer}]` : '';
  const videoSelector = `bestvideo${extFilter}`;
  const bestSelector =
    currentType === 'Video'
      ? videoSelector
      : `${videoSelector}+bestaudio/best${extFilter}`;
  const bestItems = [
    { id: bestSelector, label: 'Best (auto)' },
    ...QUALITY_LEVELS.filter((q) => {
      const h = parseInt(q);
      return currentFormats.some((f) => {
        if (f.ext !== currentContainer) return false;
        if (currentType === 'Video') {
          return f.has_video && !f.has_audio && (f.quality.includes(`${h}p`) || f.quality.includes(`${h}0p`));
        }
        return (f.has_video) && (f.quality.includes(`${h}p`) || f.quality.includes(`${h}0p`));
      });
    }).map((q) => ({
      id:
        currentType === 'Video'
          ? `${videoSelector}[height<=${q.slice(0, -1)}]`
          : `${videoSelector}[height<=${q.slice(0, -1)}]+bestaudio/best${extFilter}`,
      label: q,
    })),
  ];
  if (bestItems.length === 0) {
    container.textContent = '— no matching quality —';
    return;
  }
  renderRadioList(container, bestItems, bestItems[0].id, () => {});
}

function renderRadioList(
  container: HTMLElement,
  items: { id: string; label: string }[],
  selected: string,
  onChange: () => void,
  groupName?: string,
): void {
  container.innerHTML = '';
  for (const item of items) {
    const lbl = document.createElement('label');
    lbl.className = 'radio-label';
    if (item.id === selected) lbl.classList.add('active');
    const inp = document.createElement('input');
    inp.type = 'radio';
    inp.name = groupName ?? container.id;
    inp.value = item.id;
    inp.checked = item.id === selected;
    inp.addEventListener('change', () => {
      container.querySelectorAll('.radio-label').forEach((e) => e.classList.remove('active'));
      lbl.classList.add('active');
      onChange();
    });
    lbl.appendChild(inp);
    lbl.appendChild(document.createTextNode(` ${item.label}`));
    container.appendChild(lbl);
  }
}

function classify(f: Format): string {
  if (f.has_video && f.has_audio) return 'Video+Audio';
  if (f.has_video) return 'Video';
  return 'Audio';
}

function getSelectedFormatId(): string {
  const checked = document.querySelector<HTMLInputElement>(
    '#quality-group input:checked',
  );
  return checked?.value ?? '';
}

// ---- Options ----

async function onPickCookies(): Promise<void> {
  const path = await pickCookiesFile();
  if (path) {
    cookiesPath = path;
    const status = $<HTMLElement>('cookies-status');
    const name = path.split('/').pop()!;
    try {
      const info = await cookiesInfo(path);
      const when = info.modified_secs != null ? formatAge(info.modified_secs) : '?';
      const stale =
        info.modified_secs != null &&
        Date.now() / 1000 - info.modified_secs > STALE_COOKIES_DAYS * 86400;
      status.textContent = `${name} (${when})`;
      status.classList.toggle('stale', stale);
      status.title = stale
        ? 'Cookies файл устарел — экспортируйте свежий из браузера'
        : `Изменён: ${new Date(info.modified_secs! * 1000).toLocaleString()}`;
    } catch {
      status.textContent = name;
      status.classList.remove('stale');
    }
    $<HTMLButtonElement>('cookies-clear').classList.remove('hidden');
  }
}

function onClearCookies(): void {
  cookiesPath = null;
  const status = $<HTMLElement>('cookies-status');
  status.textContent = 'none';
  status.classList.remove('stale');
  status.title = '';
  $<HTMLButtonElement>('cookies-clear').classList.add('hidden');
}

async function onPickOutput(): Promise<void> {
  const path = await pickOutputDir();
  if (path) {
    outputDir = path;
    $<HTMLElement>('output-path').textContent = path;
  }
}

function enableProcess(): void {
  $<HTMLButtonElement>('process-btn').disabled = false;
}

function formatAge(secs: number): string {
  const days = Math.max(0, Math.floor((Date.now() / 1000 - secs) / 86400));
  if (days === 0) return 'today';
  if (days === 1) return 'yesterday';
  if (days < 30) return `${days} days ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months} mo ago`;
  return `${Math.floor(months / 12)} y ago`;
}

// ---- Process ----

async function onProcess(): Promise<void> {
  if (isProcessing) return;

  const url = $<HTMLInputElement>('url-input').value.trim();
  const formatId = getSelectedFormatId();
  if (!url || !formatId) return;

  const dir = outputDir ?? DEFAULT_OUTPUT_DIR;
  const doTranslate = $<HTMLInputElement>('mix-check').checked;
  const kind: 'video' | 'audio' = currentType === 'Audio' ? 'audio' : 'video';
  isProcessing = true;
  missingTranslation = false;

  const btn = $<HTMLButtonElement>('process-btn');
  btn.disabled = true;
  btn.textContent = 'Обработка...';

  const progressSection = $<HTMLElement>('progress-section');
  const progressBar = $<HTMLProgressElement>('progress-bar');
  const progressText = $<HTMLElement>('progress-text');
  progressSection.classList.remove('hidden');
  progressBar.value = 0;
  progressText.textContent = 'Starting...';

  hideError();
  hideResult();

  try {
    const result = await startProcess(
      url,
      formatId,
      kind,
      dir,
      doTranslate,
      { cookiesPath: cookiesPath ?? undefined },
      (msg) => {
        progressText.textContent = msg;
        log(msg);
      },
      (pct) => {
        progressBar.value = pct;
      },
    );

    progressBar.value = 100;
    progressText.textContent = 'Готово!';

    if (result.mixed_path) {
      showResult(`Файл с озвучкой сохранён: ${result.mixed_path}`);
    } else {
      let resultMsg = `Файл сохранён: ${result.video_path}`;
      if (result.translation_path) {
        resultMsg += `\nОзвучка: ${result.translation_path}`;
      } else if (missingTranslation) {
        resultMsg +=
          '\n⚠ Озвучка Яндекса для этого видео не найдена — сохранена оригинальная дорожка. Озвучка доступна для видео из кэша Яндекса.';
      }
      showResult(resultMsg);
    }
    // Remember the per-video folder so "Перевести описание" can save there.
    const artifactPath = result.mixed_path ?? result.video_path;
    lastResultDir = artifactPath.replace(/[/\\][^/\\]+$/, '') || null;
    updateTranslateDescAvailability();
  } catch (err: unknown) {
    showError(err);
    progressText.textContent = 'Ошибка';
  } finally {
    isProcessing = false;
    btn.disabled = false;
    btn.textContent = 'Скачать';
  }
}

// ---- Status helpers ----

function setProgress(msg: string): void {
  const section = $<HTMLElement>('progress-section');
  const text = $<HTMLElement>('progress-text');
  section.classList.remove('hidden');
  text.textContent = msg;
}

function clearProgress(): void {
  $<HTMLElement>('progress-section').classList.add('hidden');
}

const LOG_MAX_LINES = 500;
const LOG_FLUSH_INTERVAL_MS = 200;
let logBuffer: string[] = [];
let logFlushTimer: ReturnType<typeof setTimeout> | undefined;
let logLineCount = 0;

function flushLogBuffer(): void {
  logFlushTimer = undefined;
  if (logBuffer.length === 0) return;
  const header = $<HTMLElement>('log-header');
  const area = $<HTMLElement>('log-area');
  header.classList.remove('hidden');
  area.classList.remove('hidden');
  // Single textContent write per flush instead of O(n²) appends —
  // thousands of yt-dlp/VOT lines used to freeze the webview.
  const wasAtBottom = area.scrollTop + area.clientHeight >= area.scrollHeight - 4;
  area.textContent += logBuffer.join('\n') + '\n';
  logLineCount += logBuffer.length;
  logBuffer = [];
  if (logLineCount > LOG_MAX_LINES) {
    const lines = area.textContent.split('\n');
    area.textContent = lines.slice(-LOG_MAX_LINES).join('\n');
    logLineCount = LOG_MAX_LINES;
  }
  if (wasAtBottom) area.scrollTop = area.scrollHeight;
}

function log(line: string): void {
  logBuffer.push(line);
  if (logFlushTimer === undefined) {
    logFlushTimer = setTimeout(flushLogBuffer, LOG_FLUSH_INTERVAL_MS);
  }
}

function clearLog(): void {
  if (logFlushTimer !== undefined) {
    clearTimeout(logFlushTimer);
    logFlushTimer = undefined;
  }
  logBuffer = [];
  logLineCount = 0;
  $<HTMLElement>('log-area').textContent = '';
  $<HTMLElement>('log-header').classList.add('hidden');
  $<HTMLElement>('log-area').classList.add('hidden');
}


/** Tauri IPC errors arrive as {kind, message} objects — extract human text. */
function errMsg(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  if (typeof err === 'object' && err !== null && 'message' in err) {
    return String((err as { message: unknown }).message);
  }
  return JSON.stringify(err);
}

function showError(err: unknown): void {
  const box = $<HTMLElement>('error-box');
  box.classList.remove('hidden');
  box.textContent = `Ошибка: ${errMsg(err)}`;
}

function hideError(): void {
  $<HTMLElement>('error-box').classList.add('hidden');
}

function showResult(msg: string): void {
  const box = $<HTMLElement>('result-box');
  box.classList.remove('hidden');
  box.textContent = msg;
}

function hideResult(): void {
  $<HTMLElement>('result-box').classList.add('hidden');
}
