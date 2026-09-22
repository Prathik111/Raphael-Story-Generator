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
  image_status: 'not_ready' | 'prompt_ready' | 'queued' | 'running' | 'generated' | 'failed';
  image_url: string | null;
  image_path: string | null;
  image_mime: string | null;
  image_error: string | null;
  image_width: number;
  image_height: number;
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

export interface ResearchSource {
  id: string;
  title: string;
  url: string;
  snippet: string;
  content: string;
}

export interface ResearchFact {
  claim: string;
  evidence: string;
  source_ids: string[];
  confidence: string;
}

export interface ResearchBundle {
  queries: string[];
  sources: ResearchSource[];
  facts: ResearchFact[];
  retrieved_at: string;
}

export interface Story {
  id: string;
  title: string;
  source_prompt: string;
  research: ResearchBundle;
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
  web_research_enabled: boolean;
  web_search_url: string;
  web_proxy_url: string;
  web_search_max_results: number;
  web_fetch_max_chars: number;
  web_context_max_chars: number;
  web_research_system_prompt: string;
  comfyui_url: string;
  comfyui_workflow_json: string;
}

export interface AppState {
  stories: StorySummary[];
  settings: AppSettings;
  llm_configured: boolean;
  llm_api_key_configured: boolean;
}

export interface SceneExtractionResult {
  chapter_number: number;
  scenes: Scene[];
}

export interface CommandResult {
  message: string;
}


export type RegistryStatus = 'off' | 'starting' | 'on';
export type ServiceHealthStatus = 'online' | 'offline' | 'disabled' | 'checking';

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

export interface ServiceStatusDto {
  service: 'registry' | 'searxng' | 'comfyui';
  status: ServiceHealthStatus;
  url: string;
  detail: string | null;
}

export interface ServiceStatusBoard {
  registry: ServiceStatusDto;
  searxng: ServiceStatusDto;
  comfyui: ServiceStatusDto;
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

export interface ComfyGenerationEvent {
  story_id: string;
  chapter_number: number;
  scene_id: string;
  prompt_id: string;
  status: 'queued' | 'running' | 'completed' | 'failed';
  progress: number | null;
  current_node: string | null;
  current_step: number | null;
  total_steps: number | null;
  queue_remaining: number | null;
  image_url: string | null;
  error: string | null;
  message: string;
}

export interface PipelineEvent {
  event_id: string;
  stage: string;
  status: PipelineTraceStatus;
  message: string;
}
