import { listen } from '@tauri-apps/api/event';
import { init } from './ui';
import './styles.css';

async function bootstrap(): Promise<void> {
  try {
    // Register before rendering so startup events cannot be overwritten by init().
    await listen<string>('ffmpeg-missing', (event) => {
      const app = document.getElementById('app');
      if (!app) return;
      const div = document.createElement('div');
      div.className = 'error ffmpeg-error';
      div.textContent = `Missing dependency: ${event.payload}`;
      app.prepend(div);
    });
  } catch (error: unknown) {
    console.error('failed to register ffmpeg status listener', error);
  }

  init();
}

void bootstrap();
