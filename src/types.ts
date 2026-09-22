export interface StorySummary {
  id: string;
  title: string;
  chapter_count: number;
  scene_count: number;
  updated_at: string;
}

export interface StoryMetadata {
  genre: string[];
  tags: string[];
  demographic: string;
  content_rating: string;
  tone: string[];
  source_type: string;
  source_title: string;
  inspirations: string[];
}

export interface Character {
  id: string;
  name: string;
  role: string;
  personality: string[];
  appearance: string;
  clothing: string;
  motivations: string[];
  current_state: string;
}

export interface Relationship {
  source_character_id: string;
  target_character_id: string;
  relation_type: string;
  description: string;
}

export interface StoryBible {
  premise: string;
  central_conflict: string;
  themes: string[];
  world_setting: string;
  world_rules: string[];
  locations: string[];
  characters: Character[];
  relationships: Relationship[];
  open_threads: string[];
  continuity_notes: string[];
}

export interface VisualStyleLora {
  id: string;
  name: string;
  weight: number;
  file_name: string;
  activation_prompts: string[];
}

export interface StoryVisualConfig {
  checkpoint_id: string;
  checkpoint_name: string;
  checkpoint_file_name: string;
  style_loras: VisualStyleLora[];
}

export interface SceneLoraSelection {
  id: string;
  name: string;
  role: 'character' | 'concept_pose';
  character: string | null;
  weight: number;
  file_name: string;
  activation_prompts: string[];
  reason: string;
}

export interface Scene {
  id: string;
  order: number;
  description: string;
  location: string;
  time: string;
  characters: string[];
  action: string;
  composition: string;
  dialogue: string;
  positive_prompt: string;
  negative_prompt: string;
  selected_loras: SceneLoraSelection[];
  image_status: 'not_ready' | 'prompt_ready' | 'queued' | 'generated' | 'failed';
  image_url: string | null;
  comfy_prompt_id: string | null;
}

export interface Chapter {
  number: number;
  title: string;
  summary: string;
  text: string;
  user_directive: string | null;
  events: string[];
  continuity_updates: string[];
  scenes: Scene[];
  created_at: string;
}

export interface Story {
  id: string;
  title: string;
  source_prompt: string;
  metadata: StoryMetadata;
  visual_config: StoryVisualConfig;
  introduction: string;
  bible: StoryBible;
  chapters: Chapter[];
  created_at: string;
  updated_at: string;
}

export interface AppSettings {
  llm_base_url: string;
  llm_model: string;
  llm_api_key: string;
  temperature: number;
  story_architect_system_prompt: string;
  continuity_writer_system_prompt: string;
  scene_director_system_prompt: string;
  lora_selector_system_prompt: string;
  image_prompt_generator_system_prompt: string;
  comfyui_url: string;
  comfyui_workflow_json: string;
}

export interface AppState {
  stories: StorySummary[];
  settings: AppSettings;
  llm_configured: boolean;
}

export interface SceneExtractionResult {
  chapter_number: number;
  scenes: Scene[];
}

export interface CommandResult {
  message: string;
}


export type RegistryStatus = 'off' | 'starting' | 'on';

export interface RegistryStatusDto {
  status: RegistryStatus;
  url: string;
  detail: string | null;
}

export interface RegistryModel {
  id: string;
  name: string;
  model_type: string;
  base_model: string | null;
  creator: string | null;
  revision: number;
}

export interface RegistryCatalog {
  checkpoints: RegistryModel[];
  checkpoint_total: number;
  loras: RegistryModel[];
  lora_total: number;
}

export interface StoryVisualSetup {
  checkpoint_id: string;
  style_lora_ids: string[];
}

export type LlmTraceStatus = 'started' | 'token' | 'completed' | 'error';

export interface LlmGenerationEvent {
  generation_id: string;
  stage: string;
  status: LlmTraceStatus;
  model: string;
  system_prompt: string | null;
  user_prompt: string | null;
  delta: string | null;
  response: string | null;
  error: string | null;
}

export type PipelineTraceStatus = 'started' | 'completed' | 'error';

export interface PipelineEvent {
  event_id: string;
  stage: string;
  status: PipelineTraceStatus;
  message: string;
}
