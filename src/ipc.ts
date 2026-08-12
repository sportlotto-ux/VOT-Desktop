import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { Format, ProcessRequest, ProcessResponse } from './types';

export async function fetchFormats(
  url: string,
  cookiesPath?: string,
): Promise<Format[]> {
  return await invoke<Format[]>('fetch_formats', {
    request: { url, cookies_path: cookiesPath },
  });
}

export async function startProcess(
  url: string,
  formatId: string,
  kind: 'video' | 'audio',
  outputDir: string,
  doTranslate: boolean,
  cookiesPath?: string,
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
  if (cookiesPath) {
    req.cookies_path = cookiesPath;
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

export async function pickOutputDir(): Promise<string | null> {
  return await open({ directory: true, multiple: false });
}
