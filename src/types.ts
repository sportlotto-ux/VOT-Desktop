export interface Format {
  id: string;
  ext: string;
  quality: string;
  filesize: string;
  has_video: boolean;
  has_audio: boolean;
}

export interface FetchFormatsResponse {
  formats: Format[];
  description: string | null;
}

export interface ProcessRequest {
  url: string;
  format_id: string;
  kind: 'video' | 'audio';
  output_dir: string;
  cookies_path?: string;
  do_translate: boolean;
}

export interface TranslateDescriptionRequest {
  description: string;
  api_key: string;
  model?: string;
  save_dir?: string;
}

export interface ProcessResponse {
  video_path: string;
  translation_path: string | null;
  mixed_path: string | null;
}

export type AppError = {
  kind: 'invalid_input' | 'subprocess' | 'io' | 'tauri';
  message: string;
};

export interface CookieInfo {
  path: string;
  modified_secs: number | null;
}

export interface UpdateInfo {
  name: string;
  current: string | null;
  latest: string;
}
