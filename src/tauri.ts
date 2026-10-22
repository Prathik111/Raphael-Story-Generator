import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AppSettings, AppState, ComfyGenerationEvent, LlmGenerationEvent, OllamaLanHostStatus, OllamaModelDetection, PipelineEvent, RegistryCatalog, RegistryStatusDto, SceneExtractionResult, ServiceStatusBoard, Story, StoryVisualSetup } from './types';
import type { WorkflowBuildResult, WorkflowLoraInput } from './types';

export const isWebApp = typeof window !== 'undefined' && !(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

const command = async <T,>(name: string, args: Record<string, unknown> = {}): Promise<T> => {
  if (!isWebApp) return invoke<T>(name, args);

  const response = await fetch('/api/' + encodeURIComponent(name), {
    method: 'POST',
    headers: { 'content-type': 'application/json', accept: 'application/json' },
    body: JSON.stringify(args),
  });
  const text = await response.text();
  let value: unknown = null;
  try { value = text ? JSON.parse(text) : null; } catch { value = text; }
  if (!response.ok) {
    const message = typeof value === 'object' && value && 'error' in value
      ? String((value as { error: unknown }).error)
      : text || response.statusText;
    throw new Error(message);
  }
  return value as T;
};

function subscribeToWebEvent<T>(name: string, handler: (event: T) => void): Promise<UnlistenFn> {
  const source = new EventSource('/api/events');
  source.onmessage = event => {
    try {
      const message = JSON.parse(event.data) as { name?: string; payload?: T };
      if (message.name === name && message.payload !== undefined) handler(message.payload);
    } catch {
      // Ignore malformed event frames; the stream will continue.
    }
  };
  return Promise.resolve(() => source.close());
}

export const subscribeToLlm = (handler: (event: LlmGenerationEvent) => void): Promise<UnlistenFn> =>
  isWebApp
    ? subscribeToWebEvent<LlmGenerationEvent>('raphael:llm', handler)
    : listen<LlmGenerationEvent>('raphael:llm', event => handler(event.payload));

export const subscribeToPipeline = (handler: (event: PipelineEvent) => void): Promise<UnlistenFn> =>
  isWebApp
    ? subscribeToWebEvent<PipelineEvent>('raphael:pipeline', handler)
    : listen<PipelineEvent>('raphael:pipeline', event => handler(event.payload));

export const subscribeToComfy = (handler: (event: ComfyGenerationEvent) => void): Promise<UnlistenFn> =>
  isWebApp
    ? subscribeToWebEvent<ComfyGenerationEvent>('raphael:comfyui', handler)
    : listen<ComfyGenerationEvent>('raphael:comfyui', event => handler(event.payload));

export const api = {
  isWebApp,
  getWebHostStatus: () => command<WebHostStatus>('get_web_host_status'),
  startWebHost: () => command<WebHostStatus>('start_web_host'),
  stopWebHost: () => command<WebHostStatus>('stop_web_host'),
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
  detectOllamaModels: (configuredBaseUrl: string) =>
    command<OllamaModelDetection>('detect_ollama_models', { configuredBaseUrl }),
  getSceneImage: (storyId: string, chapterNumber: number, sceneId: string) =>
    command<string | null>('get_scene_image', { storyId, chapterNumber, sceneId }),
  clearLlmApiKey: () => command<string>('clear_llm_api_key'),
};
