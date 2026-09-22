import { invoke } from '@tauri-apps/api/core';
import type { AppSettings, AppState, RegistryCatalog, RegistryStatusDto, SceneExtractionResult, Story } from './types';

export const isWebApp = typeof window !== 'undefined' && !(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

const command = <T,>(name: string, args: Record<string, unknown> = {}) => {
  if (isWebApp) {
    return Promise.reject(
      new Error(`Raphael Story Generator requires the Tauri desktop runtime. Use "npm run tauri:dev" for development.`),
    );
  }
  return invoke<T>(name, args);
};

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
  queueSceneImage: (storyId: string, chapterNumber: number, sceneId: string) =>
    command<Story>('queue_scene_image', { storyId, chapterNumber, sceneId }),
  saveSettings: (settings: AppSettings) => command<AppSettings>('save_settings', { settings }),
  getSettings: () => command<AppSettings>('get_settings'),
  ensureRegistry: () => command<RegistryStatusDto>('ensure_registry'),
  getRegistryStatus: () => command<RegistryStatusDto>('get_registry_status'),
  getRegistryModels: () => command<RegistryCatalog>('get_registry_models'),
};
