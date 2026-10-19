import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AppSettings, AppState, ComfyGenerationEvent, LlmGenerationEvent, PipelineEvent, RegistryCatalog, RegistryStatusDto, SceneExtractionResult, ServiceStatusBoard, Story, StoryVisualSetup } from './types';
import type { WorkflowBuildResult, WorkflowLoraInput } from './types';

export const isWebApp = typeof window !== 'undefined' && !(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

const command = <T,>(name: string, args: Record<string, unknown> = {}) => {
  if (isWebApp) {
    return Promise.reject(
      new Error(`Raphael Story Generator requires the Tauri desktop runtime. Use "npm run tauri:dev" for development.`),
    );
  }
  return invoke<T>(name, args);
};

export const subscribeToLlm = (handler: (event: LlmGenerationEvent) => void): Promise<UnlistenFn> =>
  isWebApp ? Promise.resolve(() => {}) : listen<LlmGenerationEvent>('raphael:llm', event => handler(event.payload));

export const subscribeToPipeline = (handler: (event: PipelineEvent) => void): Promise<UnlistenFn> =>
  isWebApp ? Promise.resolve(() => {}) : listen<PipelineEvent>('raphael:pipeline', event => handler(event.payload));

export const subscribeToComfy = (handler: (event: ComfyGenerationEvent) => void): Promise<UnlistenFn> =>
  isWebApp ? Promise.resolve(() => {}) : listen<ComfyGenerationEvent>('raphael:comfyui', event => handler(event.payload));

export const api = {
  isWebApp,
  getState: () => command<AppState>('get_app_state'),
  getStory: (id: string) => command<Story>('get_story', { id }),
  createStory: (prompt: string, visualSetup: StoryVisualSetup) => command<Story>('create_story', { prompt, visualSetup }),
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
  getRegistryModelThumbnails: (modelIds: string[]) =>
    command<Record<string, string>>('get_registry_model_thumbnails', { modelIds }),
  getServiceStatus: () => command<ServiceStatusBoard>('get_service_status'),
  buildComfyUiWorkflow: (workflow: unknown, loraStack: WorkflowLoraInput[], checkpointNode: string | null, imageWidth: number, imageHeight: number) =>
    command<WorkflowBuildResult>('build_comfyui_workflow', { workflow, loraStack, checkpointNode, imageWidth, imageHeight }),
  testPrivateWebResearch: () => command<string>('test_private_web_research'),
  getSceneImage: (storyId: string, chapterNumber: number, sceneId: string) =>
    command<string | null>('get_scene_image', { storyId, chapterNumber, sceneId }),
  clearLlmApiKey: () => command<string>('clear_llm_api_key'),
};
