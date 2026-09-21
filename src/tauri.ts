import { invoke } from '@tauri-apps/api/core';
import type { AppSettings, AppState, SceneExtractionResult, Story } from './types';

export const isWebApp = typeof window !== 'undefined' && !(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

async function webCommand<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const response = await fetch('/api/command/' + encodeURIComponent(command), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args),
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) throw new Error(payload?.error || `Web API request failed: ${response.status}`);
  return payload as T;
}

const command = <T,>(name: string, args: Record<string, unknown> = {}) =>
  isWebApp ? webCommand<T>(name, args) : invoke<T>(name, args);

export const api = {
  isWebApp,
  getState: () => command<AppState>('get_app_state'),
  getStory: (id: string) => command<Story>('get_story', { id }),
  createStory: (prompt: string) => command<Story>('create_story', { prompt }),
  generateNextChapter: (storyId: string, userPrompt: string) =>
    command<Story>('generate_next_chapter', { storyId, userPrompt }),
  extractScenes: (storyId: string, chapterNumber: number) =>
    command<SceneExtractionResult>('extract_scenes', { storyId, chapterNumber }),
  buildScenePrompt: (storyId: string, chapterNumber: number, sceneId: string) =>
    command<Story>('build_scene_prompt', { storyId, chapterNumber, sceneId }),
  saveSettings: (settings: AppSettings) => command<AppSettings>('save_settings', { settings }),
  getSettings: () => command<AppSettings>('get_settings'),
};
