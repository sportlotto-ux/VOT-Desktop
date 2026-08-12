import type { Format } from './types';
import { fetchFormats, startProcess, pickCookiesFile, pickOutputDir } from './ipc';

const DEFAULT_OUTPUT_DIR = '~/Videos/VotDesktop';

let currentFormats: Format[] = [];
let cookiesPath: string | null = null;
let outputDir: string | null = null;
let isProcessing = false;

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

export function init(): void {
  render();
  bindEvents();
}

function render(): void {
  const app = $<HTMLElement>('app');
  app.innerHTML = `
    <div class="container">
      <h1>VotDesktop</h1>

      <section class="input-section">
        <label for="url-input">YouTube URL</label>
        <input type="url" id="url-input"
          placeholder="https://youtube.com/watch?v=..." />

        <div class="row">
          <button id="fetch-btn">Fetch Formats</button>
        </div>
      </section>

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

      <section class="options-section">
        <div class="checkbox-row">
          <input type="checkbox" id="mix-check" checked />
          <label for="mix-check">Mix with Russian voice translation (VOT)</label>
        </div>

        <div class="option-row">
          <span class="label">Cookies</span>
          <span id="cookies-status" class="value">none</span>
          <button id="cookies-btn" class="small">Choose file...</button>
          <button id="cookies-clear" class="small hidden">Clear</button>
        </div>

        <div class="option-row">
          <span class="label">Output</span>
          <span id="output-path" class="value">${DEFAULT_OUTPUT_DIR}</span>
          <button id="output-btn" class="small">Choose...</button>
        </div>
      </section>

      <button id="process-btn" disabled class="primary">Start</button>

      <div id="progress-section" class="hidden">
        <label>Progress</label>
        <progress id="progress-bar" value="0" max="100"></progress>
        <span id="progress-text"></span>
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

  $<HTMLInputElement>('url-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') onFetch();
  });
}

// ---- Fetch ----

async function onFetch(): Promise<void> {
  const url = $<HTMLInputElement>('url-input').value.trim();
  if (!url) return;

  setProgress('Fetching formats...');
  hideError();
  hideResult();

  try {
    currentFormats = await fetchFormats(url, cookiesPath ?? undefined);
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
  const types = ['Video+Audio', 'Video only', 'Audio only'];
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

  if (currentType === 'Audio only') {
    // Show bitrate options from yt-dlp format list
    const audioFormats = currentFormats.filter(
      (f) => classify(f) === 'Audio only' && f.ext === currentContainer,
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
    return currentType !== 'Video only' || !format.has_audio;
  });
  if (!hasMatchingVideo) {
    container.textContent = '— no matching video formats —';
    return;
  }
  const extFilter = currentContainer ? `[ext=${currentContainer}]` : '';
  const videoSelector = `bestvideo${extFilter}`;
  const bestSelector =
    currentType === 'Video only'
      ? videoSelector
      : `${videoSelector}+bestaudio/best${extFilter}`;
  const bestItems = [
    { id: bestSelector, label: 'Best (auto)' },
    ...QUALITY_LEVELS.filter((q) => {
      const h = parseInt(q);
      return currentFormats.some((f) => {
        if (f.ext !== currentContainer) return false;
        if (currentType === 'Video only') {
          return f.has_video && !f.has_audio && (f.quality.includes(`${h}p`) || f.quality.includes(`${h}0p`));
        }
        return (f.has_video) && (f.quality.includes(`${h}p`) || f.quality.includes(`${h}0p`));
      });
    }).map((q) => ({
      id:
        currentType === 'Video only'
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
  if (f.has_video) return 'Video only';
  return 'Audio only';
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
    $<HTMLElement>('cookies-status').textContent = path.split('/').pop()!;
    $<HTMLButtonElement>('cookies-clear').classList.remove('hidden');
  }
}

function onClearCookies(): void {
  cookiesPath = null;
  $<HTMLElement>('cookies-status').textContent = 'none';
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

// ---- Process ----

async function onProcess(): Promise<void> {
  if (isProcessing) return;

  const url = $<HTMLInputElement>('url-input').value.trim();
  const formatId = getSelectedFormatId();
  if (!url || !formatId) return;

  const dir = outputDir ?? DEFAULT_OUTPUT_DIR;
  const doTranslate = $<HTMLInputElement>('mix-check').checked;
  isProcessing = true;

  const btn = $<HTMLButtonElement>('process-btn');
  btn.disabled = true;
  btn.textContent = 'Processing...';

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
      dir,
      doTranslate,
      cookiesPath ?? undefined,
      (msg) => {
        progressText.textContent = msg;
        log(msg);
      },
      (pct) => {
        progressBar.value = pct;
      },
    );

    progressBar.value = 100;
    progressText.textContent = 'Done!';

    if (result.mixed_path) {
      showResult(`Mixed video saved: ${result.mixed_path}`);
    } else {
      let resultMsg = `Video saved: ${result.video_path}`;
      if (result.translation_path) {
        resultMsg += `\nTranslation: ${result.translation_path}`;
      }
      showResult(resultMsg);
    }
  } catch (err: unknown) {
    showError(err);
    progressText.textContent = 'Failed';
  } finally {
    isProcessing = false;
    btn.disabled = false;
    btn.textContent = 'Start';
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

function log(line: string): void {
  const area = $<HTMLElement>('log-area');
  area.classList.remove('hidden');
  area.textContent += line + '\n';
}

function showError(err: unknown): void {
  const box = $<HTMLElement>('error-box');
  box.classList.remove('hidden');
  const msg =
    err instanceof Error
      ? err.message
      : typeof err === 'string'
        ? err
        : JSON.stringify(err);
  box.textContent = `Error: ${msg}`;
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
