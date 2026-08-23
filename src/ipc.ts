import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { CookieInfo, FetchFormatsResponse, ProcessRequest, ProcessResponse, TranslateDescriptionRequest, UpdateInfo } from './types';

export async function fetchFormats(
  url: string,
  cookiesPath?: string,
): Promise<FetchFormatsResponse> {
  return await invoke<FetchFormatsResponse>('fetch_formats', {
    request: { url, cookies_path: cookiesPath },
  });
}

export async function startProcess(
  url: string,
  formatId: string,
  kind: 'video' | 'audio',
  outputDir: string,
  doTranslate: boolean,
  options?: {
    cookiesPath?: string;
  },
  onStep?: (msg: string) => void,
  onProgress?: (pct: number) => void,
): Promise<ProcessResponse> {
  const req: ProcessRequest = {
    url,
    format_id: formatId,
    kind,
    output_dir: outputDir,
    do_translate: doTranslate,
  };
  if (options?.cookiesPath) {
    req.cookies_path = options.cookiesPath;
  }

  const unlistenStep = await listen<string>('process-step', (e) => {
    onStep?.(e.payload);
  });
  const unlistenProgress = await listen<{ operation: string; percent: number; message: string }>(
    'process-progress',
    (e) => {
      const p = e.payload;
      onStep?.(p.message);
      onProgress?.(p.percent);
    },
  );

  try {
    return await invoke<ProcessResponse>('start_process', { request: req });
  } finally {
    unlistenStep();
    unlistenProgress();
  }
}

export async function pickCookiesFile(): Promise<string | null> {
  return await open({
    multiple: false,
    filters: [{ name: 'Cookies', extensions: ['txt'] }],
  });
}

export async function cookiesInfo(path: string): Promise<CookieInfo> {
  return await invoke<CookieInfo>('cookies_info', { path });
}

export interface RuntimeVersions {
  ytdlp: string | null;
  deno: string | null;
  ffmpeg: string | null;
}

export async function runtimeVersions(): Promise<RuntimeVersions> {
  return await invoke<RuntimeVersions>('runtime_versions');
}

export async function geminiModels(apiKey: string): Promise<string[]> {
  return await invoke<string[]>('gemini_models', { apiKey });
}

export async function translateDescription(
  request: TranslateDescriptionRequest,
): Promise<string> {
  return await invoke<string>('translate_description', { request });
}

export async function checkUpdates(): Promise<UpdateInfo[]> {
  return await invoke<UpdateInfo[]>('check_updates');
}

export async function updateBinary(name: string): Promise<string> {
  return await invoke<string>('update_binary', { name });
}

export async function pickOutputDir(): Promise<string | null> {
  return await open({ directory: true, multiple: false });
}
