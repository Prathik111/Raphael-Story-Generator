mod comfyui;
mod privacy_gateway;
mod registry;
mod research;
mod service_status;
mod workflow_builder;
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use research::ResearchBundle;
use serde::{Deserialize, Serialize};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{collections::HashMap, env, fs, path::{Path, PathBuf}, sync::RwLock};
use tauri::{AppHandle, Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
enum AppError {
    #[error("story not found: {0}")]
    StoryNotFound(String),
    #[error("invalid model response: {0}")]
    ModelResponse(String),
    #[error("LLM request failed: {0}")]
    Llm(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("ComfyUI error: {0}")]
    ComfyUi(String),
    #[error("Registry error: {0}")]
    Registry(String),
    #[error("Web research error: {0}")]
    WebResearch(String),
}
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {
        serializer.serialize_str(&self.to_string())
    }
}
type AppResult<T> = Result<T, AppError>;

const DEFAULT_STORY_ARCHITECT_SYSTEM_PROMPT: &str = r#"You are Raphael Story Architect. Convert a user's natural-language story request into a structured story bible and opening chapter.
Return ONLY valid JSON matching the requested schema. Do not wrap it in markdown.
Extract explicit facts faithfully. You may invent missing details, but make them internally consistent and suitable for future visual generation.
The story must include metadata: anime/show source information if present, genre, tags, demographic, content rating, tone. Characters require stable visual details: personality, appearance, clothing, motivations.
Relationships must use character names from the characters array. Chapter 1 must establish canon without contradicting the request."#;

const DEFAULT_CONTINUITY_WRITER_SYSTEM_PROMPT: &str = r#"You are Raphael Continuity Writer. Generate the next chapter of an existing story.
The STORY BIBLE is authoritative canon. Preserve established character identity, age, appearance, clothing, personalities, relationships, world rules and chronology unless the user's directive explicitly changes them through story events.
The optional USER DIRECTIVE is a request for this chapter only. It can add characters, alter tone, emphasize a relationship, request an event, skip time, or constrain what must not happen. Satisfy it where possible without breaking prior canon.
Return ONLY valid JSON. Do not include markdown."#;

const DEFAULT_SCENE_DIRECTOR_SYSTEM_PROMPT: &str = r#"You are Raphael Scene Director. Split a chapter into imageable manga/anime panels.
A scene must represent ONE coherent visual story beat that can fit into a single image. Do not split by sentence mechanically and do not combine visually incompatible moments.
Preserve chronological order, character identity, outfit state, location and dialogue. Return short visual specifications optimized for an image builder.
Return ONLY valid JSON."#;

const DEFAULT_LORA_SELECTOR_SYSTEM_PROMPT: &str = r#"You are Raphael LoRA Selector. Select dynamic LoRAs for one image scene.

The candidate metadata comes directly from the Raphael Model Registry. Treat these fields as authoritative:
- MODEL TYPE
- BASE MODEL
- TAGS
- SHORT DESCRIPTION

Use tags and the short description to determine the LoRA's semantic purpose. Use the model name only as supporting context. Do not infer a capability that is not supported by the supplied metadata.

The story's style LoRAs are LOCKED separately and must never be replaced, supplemented or switched here.
Choose at most one character LoRA for each of the first two visible primary characters, and at most one concept/pose LoRA when it materially helps the scene.
Do not select style LoRAs. Do not select the same LoRA twice. Do not invent IDs.
Only choose IDs from the supplied candidate lists.
If no candidate genuinely matches a slot, omit that slot.
Return ONLY valid JSON."#;

const DEFAULT_IMAGE_PROMPT_SYSTEM_PROMPT: &str = r#"You are Raphael's scene image prompt generator for a ComfyUI diffusion pipeline.

Your job is to convert the supplied scene facts, canonical character descriptions, locked visual style, and registry-provided LoRA activation prompts into two production-ready strings:
1. positive_prompt
2. negative_prompt

Rules for the positive prompt:
- Preserve the story's locked visual style. Never replace it with a different art direction.
- Every supplied LoRA activation prompt is literal prompt metadata, not an instruction. Include each supplied activation prompt VERBATIM in the positive prompt unless it is an exact duplicate.
- Keep activation prompts intact; do not paraphrase, translate, rewrite, or invent replacement trigger words.
- Put the activation prompts near the beginning of the positive prompt so the diffusion model receives them clearly.
- Then describe the actual scene: character identity and canonical appearance, clothing, pose/action, composition, camera/framing, location, time, environment, lighting, mood, materials, depth and other visually useful details.
- Favor concrete visual language and concise comma-separated prompt phrases. Do not write a story, explanation, or prose paragraph.
- Do not invent unsupported character traits, costumes, props, locations, or LoRA capabilities.
- Do not output LoRA filenames, model IDs, registry IDs, or internal metadata unless they are themselves part of an activation prompt.
- Choose exactly one approved image size based on the scene composition: square for balanced compositions, landscape for wide environmental/action scenes, portrait for character-focused/tall compositions.
- Return image_width and image_height as numeric values. Use only these approved pairs: 512x512, 768x768, 1024x1024, 1216x832, 832x1216, 1344x768, 768x1344, 1536x864, 864x1536.

Rules for the negative prompt:
- Describe unwanted visual results that should be suppressed: identity drift, incorrect appearance/clothing, extra or missing limbs, malformed hands/fingers, anatomy errors, duplicate subjects, bad proportions, deformed faces, blur, low detail, noise, compression artifacts, text, watermark, logo, signature, UI elements, cropped subjects, and scene contradictions.
- Keep it as a concise comma-separated list.
- Never put LoRA activation prompts or positive scene facts into the negative prompt.
- Do not use the negative prompt to introduce a different style.

Output rules:
- Return ONLY valid JSON matching the exact schema.
- Do not wrap the JSON in Markdown fences.
- Do not add commentary before or after the JSON."#;

fn default_image_dimension() -> u32 { 1024 }

fn default_web_research_enabled() -> bool { true }
fn default_web_search_url() -> String { "http://127.0.0.1:8080".into() }
fn default_web_proxy_url() -> String { "socks5h://127.0.0.1:9050".into() }
fn default_web_search_max_results() -> usize { 8 }
fn default_web_fetch_max_chars() -> usize { 12_000 }
fn default_web_context_max_chars() -> usize { 36_000 }
fn default_web_research_system_prompt() -> String {
    research::DEFAULT_WEB_RESEARCH_SYSTEM_PROMPT.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_api_key: String,
    pub temperature: f32,
    pub story_architect_system_prompt: String,
    pub continuity_writer_system_prompt: String,
    pub scene_director_system_prompt: String,
    pub lora_selector_system_prompt: String,
    pub image_prompt_generator_system_prompt: String,
    #[serde(default = "default_web_research_enabled")]
    pub web_research_enabled: bool,
    #[serde(default = "default_web_search_url")]
    pub web_search_url: String,
    #[serde(default = "default_web_proxy_url")]
    pub web_proxy_url: String,
    #[serde(default = "default_web_search_max_results")]
    pub web_search_max_results: usize,
    #[serde(default = "default_web_fetch_max_chars")]
    pub web_fetch_max_chars: usize,
    #[serde(default = "default_web_context_max_chars")]
    pub web_context_max_chars: usize,
    #[serde(default = "default_web_research_system_prompt")]
    pub web_research_system_prompt: String,
    pub comfyui_url: String,
    pub comfyui_workflow_json: String,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            llm_base_url: "http://127.0.0.1:11434/v1".into(),
            llm_model: "qwen3:8b".into(),
            llm_api_key: String::new(),
            temperature: 0.8,
            story_architect_system_prompt: DEFAULT_STORY_ARCHITECT_SYSTEM_PROMPT.into(),
            continuity_writer_system_prompt: DEFAULT_CONTINUITY_WRITER_SYSTEM_PROMPT.into(),
            scene_director_system_prompt: DEFAULT_SCENE_DIRECTOR_SYSTEM_PROMPT.into(),
            lora_selector_system_prompt: DEFAULT_LORA_SELECTOR_SYSTEM_PROMPT.into(),
            image_prompt_generator_system_prompt: DEFAULT_IMAGE_PROMPT_SYSTEM_PROMPT.into(),
            web_research_enabled: true,
            web_search_url: default_web_search_url(),
            web_proxy_url: default_web_proxy_url(),
            web_search_max_results: default_web_search_max_results(),
            web_fetch_max_chars: default_web_fetch_max_chars(),
            web_context_max_chars: default_web_context_max_chars(),
            web_research_system_prompt: default_web_research_system_prompt(),
            comfyui_url: "http://127.0.0.1:8188".into(),
            comfyui_workflow_json: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StoryMetadata {
    pub genre: Vec<String>,
    pub tags: Vec<String>,
    pub demographic: String,
    pub content_rating: String,
    pub tone: Vec<String>,
    pub source_type: String,
    pub source_title: String,
    pub inspirations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub role: String,
    pub personality: Vec<String>,
    pub appearance: String,
    pub clothing: String,
    pub motivations: Vec<String>,
    pub current_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub source_character_id: String,
    pub target_character_id: String,
    pub relation_type: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoryBible {
    pub premise: String,
    pub central_conflict: String,
    pub themes: Vec<String>,
    pub world_setting: String,
    pub world_rules: Vec<String>,
    pub locations: Vec<String>,
    pub characters: Vec<Character>,
    pub relationships: Vec<Relationship>,
    pub open_threads: Vec<String>,
    pub continuity_notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VisualStyleLora {
    pub id: String,
    pub name: String,
    pub weight: f32,
    pub file_name: String,
    pub activation_prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StoryVisualConfig {
    pub checkpoint_id: String,
    pub checkpoint_name: String,
    pub checkpoint_file_name: String,
    pub style_loras: Vec<VisualStyleLora>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SceneLoraSelection {
    pub id: String,
    pub name: String,
    pub role: String,
    pub character: Option<String>,
    pub weight: f32,
    pub file_name: String,
    pub activation_prompts: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Scene {
    pub id: String,
    pub order: usize,
    pub description: String,
    pub location: String,
    pub time: String,
    pub characters: Vec<String>,
    pub action: String,
    pub composition: String,
    pub dialogue: String,
    pub positive_prompt: String,
    pub negative_prompt: String,
    pub selected_loras: Vec<SceneLoraSelection>,
    pub image_status: String,
    pub image_url: Option<String>,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default)]
    pub image_mime: Option<String>,
    #[serde(default)]
    pub image_error: Option<String>,
    #[serde(default = "default_image_dimension")]
    pub image_width: u32,
    #[serde(default = "default_image_dimension")]
    pub image_height: u32,
    pub comfy_prompt_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub number: usize,
    pub title: String,
    pub summary: String,
    pub text: String,
    pub user_directive: Option<String>,
    pub events: Vec<String>,
    pub continuity_updates: Vec<String>,
    pub scenes: Vec<Scene>,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub id: String,
    pub title: String,
    pub source_prompt: String,
    #[serde(default)]
    pub research: research::ResearchBundle,
    pub metadata: StoryMetadata,
    #[serde(default)]
    pub visual_config: StoryVisualConfig,
    pub introduction: String,
    pub bible: StoryBible,
    pub chapters: Vec<Chapter>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorySummary {
    pub id: String,
    pub title: String,
    pub chapter_count: usize,
    pub scene_count: usize,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryVisualSetup {
    pub checkpoint_id: String,
    pub style_lora_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateDto {
    pub stories: Vec<StorySummary>,
    pub settings: AppSettings,
    pub llm_configured: bool,
    pub llm_api_key_configured: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct InitialResponse {
    title: String,
    metadata: StoryMetadata,
    premise: String,
    central_conflict: String,
    themes: Vec<String>,
    world_setting: String,
    world_rules: Vec<String>,
    locations: Vec<String>,
    characters: Vec<CharacterDraft>,
    relationships: Vec<RelationshipDraft>,
    open_threads: Vec<String>,
    introduction: String,
    chapter: ChapterDraft,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CharacterDraft {
    name: String,
    role: String,
    personality: Vec<String>,
    appearance: String,
    clothing: String,
    motivations: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct RelationshipDraft {
    source: String,
    target: String,
    relation_type: String,
    description: String,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ChapterDraft {
    title: String,
    summary: String,
    text: String,
    events: Vec<String>,
    continuity_updates: Vec<String>,
    new_characters: Vec<CharacterDraft>,
    character_state_updates: Vec<CharacterStateUpdate>,
    relationship_updates: Vec<RelationshipUpdate>,
    open_threads: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct CharacterStateUpdate {
    character_id: String,
    current_state: String,
    clothing: Option<String>,
}
#[derive(Debug, Deserialize)]
struct RelationshipUpdate {
    source_character: String,
    target_character: String,
    relation_type: String,
    description: String,
}
#[derive(Debug, Deserialize)]
struct SceneDraft {
    description: String,
    location: String,
    time: String,
    characters: Vec<String>,
    action: String,
    composition: String,
    dialogue: String,
}
#[derive(Debug, Deserialize)]
struct SceneResponse { scenes: Vec<SceneDraft> }
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ImagePromptResponse {
    positive_prompt: String,
    negative_prompt: String,
    image_width: u32,
    image_height: u32,
}

fn normalize_image_size(width: u32, height: u32) -> AppResult<(u32, u32)> {
    const ALLOWED: &[(u32, u32)] = &[
        (512, 512), (768, 768), (1024, 1024),
        (1216, 832), (832, 1216), (1344, 768), (768, 1344),
        (1536, 864), (864, 1536),
    ];
    if ALLOWED.contains(&(width, height)) {
        Ok((width, height))
    } else {
        Err(AppError::ModelResponse(format!(
            "LLM selected unsupported image size {}x{}; allowed sizes: {}",
            width, height,
            ALLOWED.iter().map(|(w, h)| format!("{}x{}", w, h)).collect::<Vec<_>>().join(", "),
        )))
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SceneLoraSelectorResponse {
    selections: Vec<SceneLoraDraft>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SceneLoraDraft {
    id: String,
    role: String,
    character: Option<String>,
    weight: f32,
    reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoreData { stories: HashMap<String, Story> }

struct Store {
    app: AppHandle,
    root: PathBuf,
    data: RwLock<StoreData>,
    settings: RwLock<AppSettings>,
}
impl Store {
    fn app(&self) -> &AppHandle { &self.app }
}
impl Store {
    fn new(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app.path().app_data_dir().map_err(|e| AppError::Storage(e.to_string()))?;
        let root = data_dir.join("story-generator");
        fs::create_dir_all(root.join("stories")).map_err(|e| AppError::Storage(e.to_string()))?;
        let settings_path = root.join("settings.json");
        let settings = if settings_path.exists() {
            load_json::<AppSettings>(&settings_path).map_err(|e| {
                AppError::Storage(format!("failed to load settings.json: {e}"))
            })?
        } else {
            AppSettings::default()
        };

        let mut data = StoreData::default();
        let entries = fs::read_dir(root.join("stories"))
            .map_err(|e| AppError::Storage(format!("failed to read stories directory: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("failed to read story entry: {e}")))?;
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }

            let story = load_json::<Story>(&path).map_err(|e| {
                AppError::Storage(format!("failed to load story file {}: {e}", path.display()))
            })?;
            validate_story_identity(&story)?;
            data.stories.insert(story.id.clone(), story);
        }

        Ok(Self { app: app.clone(), root, data: RwLock::new(data), settings: RwLock::new(settings) })
    }
    fn persist_settings(&self) -> AppResult<()> {
        let settings = self.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
        save_json(&self.root.join("settings.json"), &settings)
    }
    fn persist_story(&self, story: &Story) -> AppResult<()> {
        save_json(&self.root.join("stories").join(format!("{}.json", story.id)), story)
    }
}
fn load_json<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> AppResult<T> {
    fn try_read<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> Result<T, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    match try_read(path) {
        Ok(value) => Ok(value),
        Err(primary_error) => {
            let backup = PathBuf::from(format!("{}.bak", path.display()));
            if let Ok(value) = try_read(&backup) {
                let _ = fs::copy(&backup, path);
                return Ok(value);
            }

            let tmp = PathBuf::from(format!("{}.tmp", path.display()));
            if let Ok(value) = try_read(&tmp) {
                let _ = fs::copy(&tmp, path);
                return Ok(value);
            }

            Err(AppError::Storage(primary_error))
        }
    }
}

fn save_json<T: Serialize>(path: &PathBuf, value: &T) -> AppResult<()> {
    use std::io::Write;

    let text = serde_json::to_string_pretty(value).map_err(|e| AppError::Storage(e.to_string()))?;
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let backup = PathBuf::from(format!("{}.bak", path.display()));

    let mut file = fs::File::create(&tmp).map_err(|e| AppError::Storage(e.to_string()))?;
    file.write_all(text.as_bytes()).map_err(|e| AppError::Storage(e.to_string()))?;
    file.sync_all().map_err(|e| AppError::Storage(e.to_string()))?;
    drop(file);

    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|e| AppError::Storage(e.to_string()))?;
    }

    match fs::rename(&tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            if !path.exists() && backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            Err(AppError::Storage(error.to_string()))
        }
    }
}
fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("unix:{}", seconds)
}

fn normalize_name(value: &str) -> String {
    value.trim().to_lowercase()
}

fn validate_story_identity(story: &Story) -> AppResult<()> {
    if Uuid::parse_str(&story.id).is_err() {
        return Err(AppError::Storage(format!("invalid story ID: {}", story.id)));
    }
    if story.title.trim().is_empty() {
        return Err(AppError::Storage(format!("story {} has an empty title", story.id)));
    }
    Ok(())
}

fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::Llm(format!("failed to create HTTP client: {e}")))
}

#[derive(Debug, Clone, Serialize)]
struct LlmGenerationEvent {
    generation_id: String,
    stage: String,
    status: String,
    model: String,
    system_prompt: Option<String>,
    user_prompt: Option<String>,
    delta: Option<String>,
    response: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PipelineEvent {
    event_id: String,
    stage: String,
    status: String,
    message: String,
}

fn emit_llm(app: &AppHandle, event: LlmGenerationEvent) {
    let _ = app.emit("raphael:llm", event);
}

fn emit_pipeline(app: &AppHandle, stage: &str, status: &str, message: impl Into<String>) {
    let _ = app.emit("raphael:pipeline", PipelineEvent {
        event_id: Uuid::new_v4().to_string(),
        stage: stage.to_string(),
        status: status.to_string(),
        message: message.into(),
    });
}
fn emit_llm_error(app: &AppHandle, generation_id: &str, stage: &str, model: &str, response: Option<String>, message: String) {
    emit_llm(app, LlmGenerationEvent {
        generation_id: generation_id.to_string(),
        stage: stage.to_string(),
        status: "error".into(),
        model: model.to_string(),
        system_prompt: None,
        user_prompt: None,
        delta: None,
        response,
        error: Some(message),
    });
}

async fn chat(
    app: &AppHandle,
    settings: &AppSettings,
    stage: &str,
    system: &str,
    user: &str,
) -> AppResult<String> {
    validate_settings(settings)?;
    let generation_id = Uuid::new_v4().to_string();
    let base = settings.llm_base_url.trim().trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let client = http_client()?;

    emit_llm(app, LlmGenerationEvent {
        generation_id: generation_id.clone(),
        stage: stage.into(),
        status: "started".into(),
        model: settings.llm_model.trim().into(),
        system_prompt: Some(system.to_string()),
        user_prompt: Some(user.to_string()),
        delta: None,
        response: Some(String::new()),
        error: None,
    });

    let body = json!({
        "model": settings.llm_model.trim(),
        "temperature": settings.temperature,
        "stream": true,
        "messages": [
            {"role":"system","content":system},
            {"role":"user","content":user}
        ]
    });

    let mut req = client.post(url.clone()).header(CONTENT_TYPE, "application/json").json(&body);
    if !settings.llm_api_key.trim().is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {}", settings.llm_api_key.trim()));
    }

    let response = match req.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            emit_pipeline(app, stage, "error", message.clone());
            emit_llm(app, LlmGenerationEvent {
                generation_id: generation_id.clone(),
                stage: stage.into(),
                status: "error".into(),
                model: settings.llm_model.trim().into(),
                system_prompt: None,
                user_prompt: None,
                delta: None,
                response: None,
                error: Some(message.clone()),
            });
            return Err(AppError::Llm(message));
        }
    };

    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "request rejected".into());
        let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({"error": body}));
        let message = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request rejected")
            .to_string();
        emit_pipeline(app, stage, "error", message.clone());
        emit_llm(app, LlmGenerationEvent {
            generation_id: generation_id.clone(),
            stage: stage.into(),
            status: "error".into(),
            model: settings.llm_model.trim().into(),
            system_prompt: None,
            user_prompt: None,
            delta: None,
            response: None,
            error: Some(message.clone()),
        });
        return Err(AppError::Llm(message));
    }

    fn extract_content(value: &Value) -> Option<&str> {
        value
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| {
                v.get("delta")
                    .and_then(|d| d.get("content"))
                    .or_else(|| v.get("message").and_then(|m| m.get("content")))
            })
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    }

    let mut full_response = String::new();
    let mut buffer = Vec::<u8>::new();
    let mut parsed_any = false;
    let mut saw_sse = false;

    let mut consume_payload = |payload: &str| {
        let data = payload.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }

        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };

        if let Some(text) = extract_content(&value) {
            parsed_any = true;
            full_response.push_str(text);
            emit_llm(app, LlmGenerationEvent {
                generation_id: generation_id.clone(),
                stage: stage.into(),
                status: "token".into(),
                model: settings.llm_model.trim().into(),
                system_prompt: None,
                user_prompt: None,
                delta: Some(text.to_string()),
                response: None,
                error: None,
            });
        }
    };

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let message = error.to_string();
                emit_llm(app, LlmGenerationEvent {
                    generation_id: generation_id.clone(),
                    stage: stage.into(),
                    status: "error".into(),
                    model: settings.llm_model.trim().into(),
                    system_prompt: None,
                    user_prompt: None,
                    delta: None,
                    response: Some(full_response.clone()),
                    error: Some(message.clone()),
                });
                return Err(AppError::Llm(message));
            }
        };

        buffer.extend_from_slice(&chunk);

        while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=index).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();

            if let Some(data) = line.strip_prefix("data:") {
                saw_sse = true;
                consume_payload(data);
            } else if line.starts_with('{') {
                // Support providers that return NDJSON despite advertising a JSON response.
                consume_payload(line);
            }
        }
    }

    if !buffer.is_empty() {
        let leftover = String::from_utf8_lossy(&buffer);
        let leftover = leftover.trim();

        if let Some(data) = leftover.strip_prefix("data:") {
            saw_sse = true;
            consume_payload(data);
        } else if !leftover.is_empty() {
            consume_payload(leftover);
        }
    }

    if !parsed_any {
        let kind = if saw_sse {
            "SSE"
        } else if content_type.contains("ndjson") {
            "NDJSON"
        } else {
            "JSON/stream"
        };
        let message = format!(
            "LLM returned a {} response, but no choices[0].message.content or choices[0].delta.content was found (content-type: '{}'). Check the provider endpoint and model response format.",
            kind,
            if content_type.is_empty() { "unknown" } else { content_type.as_str() }
        );
        emit_pipeline(app, stage, "error", message.clone());
        emit_llm_error(
            app,
            &generation_id,
            stage,
            settings.llm_model.trim(),
            Some(full_response.clone()),
            message.clone(),
        );
        return Err(AppError::Llm(message));
    }

    emit_llm(app, LlmGenerationEvent {
        generation_id,
        stage: stage.into(),
        status: "completed".into(),
        model: settings.llm_model.trim().into(),
        system_prompt: None,
        user_prompt: None,
        delta: None,
        response: Some(full_response.clone()),
        error: None,
    });

    Ok(full_response)
}

fn clean_json(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') { return trimmed; }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end { return &trimmed[start..=end]; }
    }
    trimmed
}
fn validate_initial_response(parsed: &InitialResponse) -> AppResult<()> {
    if parsed.title.trim().is_empty() {
        return Err(AppError::ModelResponse("generated story title is empty".into()));
    }
    if parsed.introduction.trim().is_empty() {
        return Err(AppError::ModelResponse("generated story introduction is empty".into()));
    }
    if parsed.chapter.title.trim().is_empty() || parsed.chapter.text.trim().is_empty() {
        return Err(AppError::ModelResponse("generated Chapter 1 is incomplete".into()));
    }
    Ok(())
}

fn validate_scene_response(parsed: &SceneResponse) -> AppResult<()> {
    if parsed.scenes.is_empty() {
        return Err(AppError::ModelResponse("scene extraction returned no scenes".into()));
    }
    for (index, scene) in parsed.scenes.iter().enumerate() {
        if scene.description.trim().is_empty() {
            return Err(AppError::ModelResponse(format!(
                "scene {} has an empty description",
                index + 1
            )));
        }
    }
    Ok(())
}

fn validate_chapter_draft(parsed: &ChapterDraft, chapter_number: usize) -> AppResult<()> {
    if parsed.title.trim().is_empty() || parsed.text.trim().is_empty() {
        return Err(AppError::ModelResponse(format!(
            "generated Chapter {} is incomplete",
            chapter_number
        )));
    }
    Ok(())
}

fn unique_nonempty(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() || result.iter().any(|existing| existing == &value) {
            continue;
        }
        result.push(value);
    }
    result
}

fn text_tokens(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect()
}

fn lexical_score(query: &str, candidate: &str) -> i32 {
    let query_tokens = text_tokens(query);
    let candidate_tokens = text_tokens(candidate);
    query_tokens
        .iter()
        .filter(|token| candidate_tokens.iter().any(|value| value == *token || value.contains(token.as_str()) || token.contains(value.as_str())))
        .count() as i32
}

fn looks_like_style_lora(value: &str) -> bool {
    let text = value.to_lowercase();
    [
        "style", "anime", "illustration", "lineart", "line art", "watercolor",
        "oil", "cinematic", "render", "aesthetic", "artstyle", "art style",
        "painting", "sketch",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn looks_like_concept_pose_lora(value: &str) -> bool {
    let text = value.to_lowercase();
    [
        "pose", "concept", "action", "gesture", "motion", "dynamic",
        "composition", "camera", "perspective", "anatomy", "foreshorten",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn can_select_concept_pose(concept_count: usize) -> bool {
    concept_count == 0
}

fn lora_candidate_text(model: &registry::RegistryLoraCandidate) -> String {
    let description = model.description.as_deref().unwrap_or("no description");
    let tags = if model.tags.is_empty() {
        "none".to_string()
    } else {
        model.tags.join(", ")
    };

    format!(
        "ID: {}
MODEL TYPE: {}
NAME: {}
BASE MODEL: {}
TAGS: {}
SHORT DESCRIPTION: {}",
        model.id,
        model.model_type,
        model.name,
        model.base_model.as_deref().unwrap_or("unknown"),
        tags,
        description,
    )
}

fn validate_visual_setup(
    setup: &StoryVisualConfig,
    checkpoint: &registry::RegistryModelArtifact,
    styles: &[registry::RegistryModelArtifact],
) -> AppResult<()> {
    if setup.checkpoint_id.trim().is_empty() {
        return Err(AppError::Registry("a base checkpoint must be selected for the visual pipeline".into()));
    }
    if checkpoint.file_name.trim().is_empty() {
        return Err(AppError::Registry("selected checkpoint has no available file".into()));
    }
    if styles.len() > 2 {
        return Err(AppError::Registry("at most two style LoRAs can be locked for a story".into()));
    }
    for style in styles {
        if style.file_name.trim().is_empty() {
            return Err(AppError::Registry(format!("style LoRA {} has no available file", style.id)));
        }
    }
    Ok(())
}

fn validate_settings(settings: &AppSettings) -> AppResult<()> {
    if settings.llm_model.trim().is_empty() {
        return Err(AppError::Llm("LLM model cannot be empty".into()));
    }
    if !settings.temperature.is_finite() || !(0.0..=2.0).contains(&settings.temperature) {
        return Err(AppError::Llm("temperature must be between 0 and 2".into()));
    }
    if settings.web_search_max_results == 0 || settings.web_search_max_results > 12 {
        return Err(AppError::WebResearch("web search results must be between 1 and 12".into()));
    }
    if settings.web_fetch_max_chars < 2_000 || settings.web_fetch_max_chars > 12_000 {
        return Err(AppError::WebResearch("web page text limit must be between 2,000 and 12,000 characters".into()));
    }
    if settings.web_context_max_chars < 8_000 || settings.web_context_max_chars > 48_000 {
        return Err(AppError::WebResearch("web research context limit must be between 8,000 and 48,000 characters".into()));
    }
    if settings.web_research_enabled {
        if settings.web_search_url.trim().is_empty() {
            return Err(AppError::WebResearch("private web research requires a local SearXNG URL".into()));
        }
        if settings.web_proxy_url.trim().is_empty() {
            return Err(AppError::WebResearch("private web research requires a local SOCKS/Tor proxy".into()));
        }
        if settings.web_research_system_prompt.trim().is_empty() {
            return Err(AppError::WebResearch("web research extractor system prompt cannot be empty".into()));
        }
    }
    for (name, prompt) in [
        ("Story Architect", &settings.story_architect_system_prompt),
        ("Continuity Writer", &settings.continuity_writer_system_prompt),
        ("Scene Director", &settings.scene_director_system_prompt),
        ("LoRA Selector", &settings.lora_selector_system_prompt),
        ("Image Prompt Generator", &settings.image_prompt_generator_system_prompt),
    ] {
        if prompt.trim().is_empty() {
            return Err(AppError::Llm(format!("{name} system prompt cannot be empty")));
        }
    }


    let llm_url = settings.llm_base_url.trim();
    if !(llm_url.starts_with("http://") || llm_url.starts_with("https://")) {
        return Err(AppError::Llm("LLM base URL must start with http:// or https://".into()));
    }

    let comfyui_url = settings.comfyui_url.trim();
    if !comfyui_url.is_empty() && !(comfyui_url.starts_with("http://") || comfyui_url.starts_with("https://")) {
        return Err(AppError::ComfyUi("ComfyUI URL must start with http:// or https://".into()));
    }

    if !settings.comfyui_workflow_json.trim().is_empty() {
        let workflow: Value = serde_json::from_str(&settings.comfyui_workflow_json)
            .map_err(|e| AppError::ComfyUi(format!("workflow JSON is invalid: {e}")))?;
        if !workflow.is_object() {
            return Err(AppError::ComfyUi("ComfyUI workflow must be a JSON object".into()));
        }
    }

    Ok(())
}

fn replace_workflow_placeholders(value: &mut Value, replacements: &[(&str, Value)]) {
    match value {
        Value::String(text) => {
            let mut whole_replacement = None;
            for (token, replacement) in replacements {
                if text == *token {
                    whole_replacement = Some(replacement.clone());
                    break;
                }

                if let Value::String(replacement_text) = replacement {
                    *text = text.replace(token, replacement_text);
                }
            }

            if let Some(replacement) = whole_replacement {
                *value = replacement;
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_workflow_placeholders(item, replacements);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                replace_workflow_placeholders(item, replacements);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn require_story(store: &Store, id: &str) -> AppResult<Story> {
    let data = store.data.read().map_err(|e| AppError::Storage(e.to_string()))?;
    data.stories.get(id).cloned().ok_or_else(|| AppError::StoryNotFound(id.into()))
}
fn write_story(store: &Store, story: Story) -> AppResult<Story> {
    validate_story_identity(&story)?;
    store.persist_story(&story)?;
    store.data.write().map_err(|e| AppError::Storage(e.to_string()))?.stories.insert(story.id.clone(), story.clone());
    Ok(story)
}
fn build_summaries(data: &StoreData) -> Vec<StorySummary> {
    let mut result: Vec<_> = data.stories.values().map(|story| StorySummary {
        id: story.id.clone(), title: story.title.clone(), chapter_count: story.chapters.len(),
        scene_count: story.chapters.iter().map(|c| c.scenes.len()).sum(), updated_at: story.updated_at.clone(),
    }).collect();
    result.sort_by(|a,b| b.updated_at.cmp(&a.updated_at));
    result
}
#[tauri::command]
fn redacted_settings(settings: &AppSettings) -> AppSettings {
    let mut redacted = settings.clone();
    redacted.llm_api_key.clear();
    redacted
}

#[tauri::command]
fn get_app_state(store: State<'_, Store>) -> AppResult<AppStateDto> {
    let data = store.data.read().map_err(|e| AppError::Storage(e.to_string()))?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    Ok(AppStateDto {
        stories: build_summaries(&data),
        llm_configured: !settings.llm_base_url.is_empty() && !settings.llm_model.is_empty(),
        llm_api_key_configured: !settings.llm_api_key.is_empty(),
        settings: redacted_settings(&settings),
    })
}
#[tauri::command]
fn get_story(id: String, store: State<'_, Store>) -> AppResult<Story> { require_story(&store, &id) }
#[tauri::command]
fn get_settings(store: State<'_, Store>) -> AppResult<AppSettings> {
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    Ok(redacted_settings(&settings))
}
#[tauri::command]
fn save_settings(mut settings: AppSettings, store: State<'_, Store>) -> AppResult<AppSettings> {
    {
        let current = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?;
        if settings.llm_api_key.trim().is_empty() {
            settings.llm_api_key = current.llm_api_key.clone();
        }
    }
    validate_settings(&settings)?;
    *store.settings.write().map_err(|e| AppError::Storage(e.to_string()))? = settings.clone();
    store.persist_settings()?;
    Ok(redacted_settings(&settings))
}

#[tauri::command]
fn clear_llm_api_key(store: State<'_, Store>) -> AppResult<String> {
    {
        let mut settings = store.settings.write().map_err(|e| AppError::Storage(e.to_string()))?;
        settings.llm_api_key.clear();
    }
    store.persist_settings()?;
    Ok("Stored LLM API key cleared.".into())
}
#[tauri::command]
async fn create_story(
    prompt: String,
    visual_setup: StoryVisualSetup,
    store: State<'_, Store>,
    registry: State<'_, registry::RegistryState>,
) -> AppResult<Story> {
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    let client = registry.client().await?;

    let checkpoint = client.get(&visual_setup.checkpoint_id).await
        .map_err(|e| AppError::Registry(format!("failed to load checkpoint: {e}")))?;
    if checkpoint.model_type != registry_core::ModelType::Checkpoint {
        return Err(AppError::Registry("selected base model is not a checkpoint".into()));
    }
    let compatible = client
        .compatible(&checkpoint.id, Some(registry_core::ModelType::Lora))
        .await
        .map_err(|e| AppError::Registry(format!("failed to resolve compatible LoRAs: {e}")))?;
    let compatible_ids = compatible.iter().map(|model| model.id.as_str()).collect::<std::collections::HashSet<_>>();

    let mut style_loras = Vec::new();
    for id in visual_setup.style_lora_ids.iter().take(2) {
        let model = client.get(id).await
            .map_err(|e| AppError::Registry(format!("failed to load style LoRA {id}: {e}")))?;
        if model.model_type != registry_core::ModelType::Lora {
            return Err(AppError::Registry(format!("model {id} is not a LoRA")));
        }
        if !compatible_ids.contains(model.id.as_str()) {
            return Err(AppError::Registry(format!("style LoRA '{}' is not compatible with checkpoint '{}'", model.name, checkpoint.name)));
        }
        let artifact = registry.model_artifact(&model.id).await?;
        style_loras.push(VisualStyleLora {
            id: model.id,
            name: model.name,
            weight: 0.75,
            file_name: artifact.file_name,
            activation_prompts: artifact.activation_prompts,
        });
    }

    let checkpoint_artifact = registry.model_artifact(&checkpoint.id).await?;
    let visual_config = StoryVisualConfig {
        checkpoint_id: checkpoint.id.clone(),
        checkpoint_name: checkpoint.name.clone(),
        checkpoint_file_name: checkpoint_artifact.file_name.clone(),
        style_loras,
    };
    let style_artifacts = visual_config.style_loras.iter().map(|item| registry::RegistryModelArtifact {
        id: item.id.clone(),
        file_name: item.file_name.clone(),
        activation_prompts: item.activation_prompts.clone(),
    }).collect::<Vec<_>>();
    validate_visual_setup(&visual_config, &checkpoint_artifact, &style_artifacts)?;

    if settings.llm_model.trim().is_empty() { return Err(AppError::Llm("configure an LLM model in settings".into())); }

    let research_bundle = if settings.web_research_enabled {
        emit_pipeline(store.app(), "web_gateway", "started", "Ensuring the private web research gateway is running");
        privacy_gateway::ensure_started_with_settings(store.app(), &settings).await?;
        emit_pipeline(store.app(), "web_gateway", "completed", "Private web research gateway is ready");
        emit_pipeline(store.app(), "web_research", "started", "Searching the web through the private local research gateway");
        match research::research_web(store.app(), &settings, &prompt).await {
            Ok(bundle) => {
                emit_pipeline(
                    store.app(),
                    "web_research",
                    "completed",
                    format!("Extracted {} source-backed fact(s) from {} source page(s)", bundle.facts.len(), bundle.sources.len()),
                );
                bundle
            }
            Err(error) => {
                emit_pipeline(store.app(), "web_research", "error", error.to_string());
                return Err(error);
            }
        }
    } else {
        ResearchBundle::default()
    };

    let system = settings.story_architect_system_prompt.as_str();
    let schema_hint = r#"
JSON shape:
{"title":"","metadata":{"genre":[],"tags":[],"demographic":"","content_rating":"","tone":[],"source_type":"","source_title":"","inspirations":[]},"premise":"","central_conflict":"","themes":[],"world_setting":"","world_rules":[],"locations":[],"characters":[{"name":"","role":"","personality":[],"appearance":"","clothing":"","motivations":[]}],"relationships":[{"source":"","target":"","relation_type":"","description":""}],"open_threads":[],"introduction":"","chapter":{"title":"","summary":"","text":"","events":[],"continuity_updates":[],"character_state_updates":[],"relationship_updates":[],"open_threads":[]}}
"#;
    let research_context = research::story_architect_context(&research_bundle);
    let architect_prompt = format!("USER STORY REQUEST:
{}

{}

{}

Use the web-research facts only when they are supported by the cited sources. Do not invent external facts that are absent from the supplied research.
Return the JSON shape above.", prompt.trim(), research_context, schema_hint);
    emit_pipeline(store.app(), "story_architect", "started", "Generating story bible and opening chapter");
    let raw = chat(store.app(), &settings, "story_architect", system, &architect_prompt).await?;
    let parsed: InitialResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    validate_initial_response(&parsed)?;
    let story_id = Uuid::new_v4().to_string();
    let mut characters = Vec::with_capacity(parsed.characters.len());
    let mut name_to_id = HashMap::new();
    for (index, draft) in parsed.characters.into_iter().enumerate() {
        let name = draft.name.trim().to_string();
        let key = normalize_name(&name);
        if key.is_empty() {
            return Err(AppError::ModelResponse("generated character name is empty".into()));
        }
        if name_to_id.contains_key(&key) {
            return Err(AppError::ModelResponse(format!("duplicate generated character name: {}", name)));
        }

        let id = format!("char-{:03}", index + 1);
        name_to_id.insert(key, id.clone());
        characters.push(Character {
            id,
            name,
            role: draft.role,
            personality: draft.personality,
            appearance: draft.appearance,
            clothing: draft.clothing,
            motivations: draft.motivations,
            current_state: "Introduced in Chapter 1".into(),
        });
    }

    for update in parsed.chapter.character_state_updates.iter() {
        if let Some(character) = characters.iter_mut().find(|character| {
            character.id == update.character_id
                || normalize_name(&character.name) == normalize_name(&update.character_id)
        }) {
            character.current_state = update.current_state.clone();
            if let Some(clothing) = update.clothing.clone() {
                if !clothing.trim().is_empty() {
                    character.clothing = clothing;
                }
            }
        }
    }

    let mut relationships = parsed.relationships.into_iter().filter_map(|r| {
        Some(Relationship {
            source_character_id: name_to_id.get(&normalize_name(&r.source))?.clone(),
            target_character_id: name_to_id.get(&normalize_name(&r.target))?.clone(),
            relation_type: r.relation_type,
            description: r.description,
        })
    }).collect::<Vec<_>>();

    for update in parsed.chapter.relationship_updates.iter() {
        let Some(source_id) = name_to_id.get(&normalize_name(&update.source_character)).cloned() else { continue };
        let Some(target_id) = name_to_id.get(&normalize_name(&update.target_character)).cloned() else { continue };

        if let Some(existing) = relationships.iter_mut().find(|r| {
            r.source_character_id == source_id && r.target_character_id == target_id
        }) {
            existing.relation_type = update.relation_type.clone();
            existing.description = update.description.clone();
        } else {
            relationships.push(Relationship {
                source_character_id: source_id,
                target_character_id: target_id,
                relation_type: update.relation_type.clone(),
                description: update.description.clone(),
            });
        }
    }

    let chapter = Chapter {
        number: 1, title: parsed.chapter.title, summary: parsed.chapter.summary, text: parsed.chapter.text,
        user_directive: None,
        events: parsed.chapter.events,
        continuity_updates: parsed.chapter.continuity_updates.clone(),
        scenes: Vec::new(),
        created_at: now(),
    };
    let created = now();
    let story = Story {
        id: story_id, title: parsed.title, source_prompt: prompt, research: research_bundle,
        metadata: parsed.metadata, visual_config: visual_config.clone(), introduction: parsed.introduction,
        bible: StoryBible {
            premise: parsed.premise,
            central_conflict: parsed.central_conflict,
            themes: parsed.themes,
            world_setting: parsed.world_setting,
            world_rules: parsed.world_rules,
            locations: parsed.locations,
            characters,
            relationships,
            open_threads: parsed.open_threads.into_iter().chain(parsed.chapter.open_threads).collect(),
            continuity_notes: parsed.chapter.continuity_updates.clone(),
        },
        chapters: vec![chapter], created_at: created.clone(), updated_at: created,
    };
    write_story(&store, story)
}
#[tauri::command]
async fn generate_next_chapter(story_id: String, user_prompt: String, store: State<'_, Store>) -> AppResult<Story> {
    let mut story = require_story(&store, &story_id)?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    let next_number = story.chapters.len() + 1;
    let previous = story.chapters.last().map(|c| format!("Title: {}
Summary: {}
Events: {:?}
Text: {}", c.title, c.summary, c.events, c.text)).unwrap_or_default();
    let system = settings.continuity_writer_system_prompt.as_str();
    let bible = serde_json::to_string(&story.bible).map_err(|e| AppError::ModelResponse(e.to_string()))?;
    let directive = if user_prompt.trim().is_empty() { "(none — continue naturally)" } else { user_prompt.trim() };

    if settings.web_research_enabled && !user_prompt.trim().is_empty() {
        emit_pipeline(store.app(), "web_gateway", "started", "Ensuring the private web research gateway is running");
        privacy_gateway::ensure_started_with_settings(store.app(), &settings).await?;
        emit_pipeline(store.app(), "web_gateway", "completed", "Private web research gateway is ready");
        emit_pipeline(store.app(), "web_research", "started", format!("Researching the Chapter {} directive privately", next_number));
        match research::research_web(store.app(), &settings, &format!("{}: {}", story.title, user_prompt.trim())).await {
            Ok(bundle) => {
                research::merge_into(&mut story.research, bundle);
                emit_pipeline(store.app(), "web_research", "completed", format!("Added new source-backed research for Chapter {}", next_number));
            }
            Err(error) => {
                emit_pipeline(store.app(), "web_research", "error", error.to_string());
                return Err(error);
            }
        }
    }

    let schema = r#"{"title":"","summary":"","text":"","events":[],"continuity_updates":[],"new_characters":[{"name":"","role":"","personality":[],"appearance":"","clothing":"","motivations":[]}],"character_state_updates":[{"character_id":"","current_state":"","clothing":""}],"relationship_updates":[{"source_character":"","target_character":"","relation_type":"","description":""}],"open_threads":[]}"#;
    let research_context = research::story_architect_context(&story.research);
    let user = format!("CHAPTER NUMBER: {}

STORY BIBLE:
{}

PREVIOUS CHAPTER:
{}

USER DIRECTIVE:
{}

RESEARCH CONTEXT:
{}

Return this JSON shape:
{}", next_number, bible, previous, directive, research_context, schema);
    emit_pipeline(store.app(), "continuity_writer", "started", format!("Generating Chapter {}", next_number));
    let raw = chat(store.app(), &settings, "continuity_writer", system, &user).await?;
    let parsed: ChapterDraft = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    validate_chapter_draft(&parsed, next_number)?;
    for update in parsed.character_state_updates.iter() {
        if let Some(character) = story.bible.characters.iter_mut().find(|c| {
            c.id == update.character_id || normalize_name(&c.name) == normalize_name(&update.character_id)
        }) {
            character.current_state = update.current_state.clone();
            if let Some(clothing) = update.clothing.clone() { if !clothing.is_empty() { character.clothing = clothing; } }
        }
    }
    let mut name_to_id: HashMap<String, String> = story.bible.characters.iter()
        .map(|c| (normalize_name(&c.name), c.id.clone()))
        .collect();
    for draft in parsed.new_characters.iter() {
        let key = normalize_name(&draft.name);
        if key.is_empty() || name_to_id.contains_key(&key) { continue; }
        let id = format!("char-{:03}", story.bible.characters.len() + 1);
        name_to_id.insert(key, id.clone());
        story.bible.characters.push(Character {
            id,
            name: draft.name.trim().to_string(),
            role: draft.role.clone(),
            personality: draft.personality.clone(),
            appearance: draft.appearance.clone(),
            clothing: draft.clothing.clone(),
            motivations: draft.motivations.clone(),
            current_state: format!("Introduced in Chapter {}", next_number),
        });
    }
    for update in parsed.relationship_updates.iter() {
        let Some(source_id) = name_to_id.get(&normalize_name(&update.source_character)).cloned() else { continue };
        let Some(target_id) = name_to_id.get(&normalize_name(&update.target_character)).cloned() else { continue };
        if let Some(existing) = story.bible.relationships.iter_mut().find(|r| r.source_character_id == source_id && r.target_character_id == target_id) {
            existing.relation_type = update.relation_type.clone();
            existing.description = update.description.clone();
        } else {
            story.bible.relationships.push(Relationship {
                source_character_id: source_id,
                target_character_id: target_id,
                relation_type: update.relation_type.clone(),
                description: update.description.clone(),
            });
        }
    }
    if !parsed.open_threads.is_empty() { story.bible.open_threads = parsed.open_threads.clone(); }
    story.bible.continuity_notes.extend(parsed.continuity_updates.iter().cloned());
    let chapter = Chapter {
        number: next_number, title: parsed.title, summary: parsed.summary, text: parsed.text,
        user_directive: if user_prompt.trim().is_empty() { None } else { Some(user_prompt.trim().to_string()) },
        events: parsed.events, continuity_updates: parsed.continuity_updates, scenes: Vec::new(), created_at: now(),
    };
    story.chapters.push(chapter); story.updated_at = now(); write_story(&store, story)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneExtractionResult { pub chapter_number: usize, pub scenes: Vec<Scene> }

#[tauri::command]
async fn extract_scenes(story_id: String, chapter_number: usize, store: State<'_, Store>) -> AppResult<SceneExtractionResult> {
    let mut story = require_story(&store, &story_id)?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    let chapter_index = story.chapters.iter().position(|c| c.number == chapter_number).ok_or_else(|| AppError::ModelResponse("chapter not found".into()))?;
    let chapter = story.chapters[chapter_index].clone();
    let visual_characters = story.bible.characters.iter().map(|c| format!("{} [{}] — appearance: {}; clothing: {}; personality: {:?}", c.id, c.name, c.appearance, c.clothing, c.personality)).collect::<Vec<_>>().join("
");
    let system = settings.scene_director_system_prompt.as_str();
    let schema = r#"{"scenes":[{"description":"","location":"","time":"","characters":[],"action":"","composition":"","dialogue":""}]}"#;
    let user = format!("CHAPTER {}
TITLE: {}

TEXT:
{}

CANONICAL VISUAL CHARACTERS:
{}

Return:
{}", chapter.number, chapter.title, chapter.text, visual_characters, schema);
    emit_pipeline(store.app(), "scene_director", "started", format!("Extracting scenes for Chapter {}", chapter_number));
    let raw = chat(store.app(), &settings, "scene_director", system, &user).await?;
    let parsed: SceneResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    validate_scene_response(&parsed)?;
    for scene in &parsed.scenes {
        for character_name in &scene.characters {
            let known = story.bible.characters.iter().any(|character| {
                character_name.trim() == character.id
                    || normalize_name(character_name) == normalize_name(&character.name)
            });
            if !known {
                return Err(AppError::ModelResponse(format!(
                    "scene references unknown character '{}'",
                    character_name
                )));
            }
        }
    }
    let scenes = parsed.scenes.into_iter().enumerate().map(|(index, scene)| Scene {
        id: format!("{}-scene-{:03}", chapter.number, index + 1), order: index + 1, description: scene.description,
        location: scene.location, time: scene.time, characters: scene.characters, action: scene.action,
        composition: scene.composition, dialogue: scene.dialogue, positive_prompt: String::new(),
        negative_prompt: String::new(), selected_loras: Vec::new(), image_status: "not_ready".into(), image_url: None, image_path: None, image_mime: None, image_error: None, image_width: 1024, image_height: 1024, comfy_prompt_id: None,
    }).collect::<Vec<_>>();
    story.chapters[chapter_index].scenes = scenes.clone(); story.updated_at = now(); write_story(&store, story)?;
    Ok(SceneExtractionResult { chapter_number, scenes })
}
#[tauri::command]
async fn build_scene_prompt(
    story_id: String,
    chapter_number: usize,
    scene_id: String,
    store: State<'_, Store>,
    registry: State<'_, registry::RegistryState>,
) -> AppResult<Story> {
    let mut story = require_story(&store, &story_id)?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    let chapter = story.chapters.iter().find(|c| c.number == chapter_number).ok_or_else(|| AppError::ModelResponse("chapter not found".into()))?;
    let scene = chapter.scenes.iter().find(|s| s.id == scene_id).ok_or_else(|| AppError::ModelResponse("scene not found".into()))?;
    let matched_ids = story.bible.characters.iter()
        .filter(|c| scene.characters.iter().any(|name| name.trim() == c.id || normalize_name(name) == normalize_name(&c.name)))
        .map(|c| c.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let unknown_characters = scene.characters.iter()
        .filter(|name| {
            !story.bible.characters.iter().any(|c| {
                name.trim() == c.id || normalize_name(name) == normalize_name(&c.name)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if !unknown_characters.is_empty() {
        return Err(AppError::ModelResponse(format!(
            "scene references unknown characters: {}",
            unknown_characters.join(", ")
        )));
    }
    let characters = story.bible.characters.iter()
        .filter(|c| matched_ids.contains(c.id.as_str()))
        .map(|c| format!("{} — appearance: {}; clothing: {}; personality: {:?}", c.name, c.appearance, c.clothing, c.personality))
        .collect::<Vec<_>>().join("
");
    emit_pipeline(store.app(), "lora_selection", "started", "Selecting compatible dynamic LoRAs");
    let selected_loras = select_scene_loras(store.app(), &registry, &settings, &story, scene).await?;

    let style_activation_prompts = story.visual_config.style_loras.iter()
        .flat_map(|lora| lora.activation_prompts.iter().map(|prompt| format!("- {}: {}", lora.name, prompt)))
        .collect::<Vec<_>>();
    let dynamic_activation_prompts = selected_loras.iter()
        .flat_map(|lora| lora.activation_prompts.iter().map(|prompt| {
            format!(
                "- {} [{}{}]: {}",
                lora.name,
                lora.role,
                lora.character.as_ref().map(|character| format!(", {}", character)).unwrap_or_default(),
                prompt
            )
        }))
        .collect::<Vec<_>>();
    let activation_prompt_context = if style_activation_prompts.is_empty() && dynamic_activation_prompts.is_empty() {
        "No registry activation prompts were provided.".to_string()
    } else {
        format!(
            "LOCKED STYLE ACTIVATION PROMPTS:\n{}\n\nDYNAMIC LORA ACTIVATION PROMPTS:\n{}",
            if style_activation_prompts.is_empty() { "(none)".to_string() } else { style_activation_prompts.join("\n") },
            if dynamic_activation_prompts.is_empty() { "(none)".to_string() } else { dynamic_activation_prompts.join("\n") },
        )
    };

    let system = settings.image_prompt_generator_system_prompt.as_str();
    let schema = r#"{"positive_prompt":"","negative_prompt":"","image_width":1024,"image_height":1024}"#;
    let user = format!("STORY: {}
SCENE: {}
LOCATION: {}
TIME: {}
ACTION: {}
COMPOSITION: {}
DIALOGUE: {}
CHARACTERS:
{}

LOCKED STYLE:
{}

SELECTED DYNAMIC LORAS:
{}

REGISTRY ACTIVATION PROMPTS:
{}

Generate the prompts now.

Return:
{}",
        story.title,
        scene.description,
        scene.location,
        scene.time,
        scene.action,
        scene.composition,
        scene.dialogue,
        characters,
        story.visual_config.style_loras.iter().map(|l| format!("{} (weight {})", l.name, l.weight)).collect::<Vec<_>>().join(", "),
        selected_loras.iter().map(|l| format!("{} [{} / {}] (weight {})", l.name, l.role, l.character.clone().unwrap_or_default(), l.weight)).collect::<Vec<_>>().join(", "),
        activation_prompt_context,
        schema
    );
    emit_pipeline(store.app(), "image_prompt_generator", "started", format!("Generating positive/negative prompts for {}", scene.id));
    let raw = chat(store.app(), &settings, "image_prompt_generator", system, &user).await?;
    let parsed: ImagePromptResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    let (image_width, image_height) = normalize_image_size(
        if parsed.image_width == 0 { 1024 } else { parsed.image_width },
        if parsed.image_height == 0 { 1024 } else { parsed.image_height },
    )?;
    let chapter_mut = story.chapters.iter_mut().find(|c| c.number == chapter_number)
        .ok_or_else(|| AppError::ModelResponse("chapter disappeared while saving scene prompt".into()))?;
    let scene_mut = chapter_mut.scenes.iter_mut().find(|s| s.id == scene_id)
        .ok_or_else(|| AppError::ModelResponse("scene disappeared while saving scene prompt".into()))?;
    let required_activation_prompts = unique_nonempty(
        story.visual_config.style_loras.iter()
            .flat_map(|l| l.activation_prompts.clone())
            .chain(selected_loras.iter().flat_map(|l| l.activation_prompts.clone()))
            .collect::<Vec<_>>()
    );
    let mut positive_prompt = parsed.positive_prompt.trim().to_string();
    for prompt in required_activation_prompts.iter().rev() {
        if !positive_prompt.contains(prompt) {
            positive_prompt = format!("{}, {}", prompt, positive_prompt);
        }
    }

    scene_mut.selected_loras = selected_loras;
    scene_mut.positive_prompt = positive_prompt;
    scene_mut.negative_prompt = parsed.negative_prompt.trim().to_string();
    scene_mut.image_width = image_width;
    scene_mut.image_height = image_height;
    scene_mut.image_error = None;
    scene_mut.image_status = "prompt_ready".into();
    story.updated_at = now(); write_story(&store, story)
}
async fn select_scene_loras(
    app: &AppHandle,
    registry: &registry::RegistryState,
    settings: &AppSettings,
    story: &Story,
    scene: &Scene,
) -> AppResult<Vec<SceneLoraSelection>> {
    if story.visual_config.checkpoint_id.trim().is_empty() {
        return Err(AppError::Registry("this story has no base checkpoint configured".into()));
    }

    let candidates = registry.compatible_loras(&story.visual_config.checkpoint_id).await?;
    if candidates.is_empty() {
        return Err(AppError::Registry("no compatible LoRAs are registered for the selected checkpoint".into()));
    }

    let scene_query = format!(
        "{} {} {} {} {} {}",
        scene.description, scene.action, scene.composition, scene.location, scene.time,
        scene.characters.join(" ")
    );
    let character_text = story.bible.characters.iter()
        .filter(|character| scene.characters.iter().any(|name| {
            normalize_name(name) == normalize_name(&character.name) || name.trim() == character.id
        }))
        .take(2)
        .map(|character| format!(
            "{} — appearance: {}; clothing: {}; role: {}; personality: {:?}",
            character.name, character.appearance, character.clothing, character.role, character.personality
        ))
        .collect::<Vec<_>>();
    let character_query = format!("{} {}", character_text.join(" "), scene_query);

    let mut character_candidates = candidates.iter()
        .filter(|model| !looks_like_style_lora(&lora_candidate_text(model)))
        .map(|model| (lexical_score(&character_query, &lora_candidate_text(model)), model))
        .collect::<Vec<_>>();
    character_candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    character_candidates.truncate(28);

    let mut concept_candidates = candidates.iter()
        .filter(|model| !looks_like_style_lora(&lora_candidate_text(model)))
        .map(|model| {
            let text = lora_candidate_text(model);
            let bonus = if looks_like_concept_pose_lora(&text) { 5 } else { 0 };
            (lexical_score(&format!("{} pose action concept", scene_query), &text) + bonus, model)
        })
        .collect::<Vec<_>>();
    concept_candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    concept_candidates.truncate(22);

    let mut candidate_lines = Vec::new();
    candidate_lines.push("CHARACTER CANDIDATES:".to_string());
    for (_, model) in character_candidates {
        candidate_lines.push(lora_candidate_text(model));
    }
    candidate_lines.push("CONCEPT/POSE CANDIDATES:".to_string());
    for (_, model) in concept_candidates {
        candidate_lines.push(lora_candidate_text(model));
    }

    let scene_characters = if character_text.is_empty() {
        "(no known character in this scene)".to_string()
    } else {
        character_text.join("
")
    };

    let system = settings.lora_selector_system_prompt.as_str();
    let schema = r#"{"selections":[{"id":"","role":"character|concept_pose","character":"","weight":0.75,"reason":""}]}"#;
    let user = format!(
        "SCENE: {}
ACTION: {}
COMPOSITION: {}
CHARACTERS IN SCENE:
{}

CANDIDATES:
{}

For each selected LoRA, the reason should reference the actual registry metadata that made it relevant, such as a tag or phrase from the short description.
Do not claim capabilities that are not supported by the supplied metadata.

Return:
{}",
        scene.description, scene.action, scene.composition, scene_characters,
        candidate_lines.join("
"), schema
    );
    let raw = chat(app, settings, "lora_selector", system, &user).await?;
    let parsed: SceneLoraSelectorResponse = serde_json::from_str(clean_json(&raw))
        .map_err(|e| AppError::ModelResponse(format!("LoRA selector output was invalid: {}; raw output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;

    let candidate_map = candidates.iter().map(|m| (m.id.clone(), m)).collect::<HashMap<_, _>>();
    let allowed_characters = story.bible.characters.iter()
        .filter(|character| {
            scene.characters.iter().any(|name| {
                normalize_name(name) == normalize_name(&character.name) || name.trim() == character.id
            })
        })
        .take(2)
        .map(|character| (character.id.clone(), normalize_name(&character.name), character.name.clone()))
        .collect::<Vec<_>>();

    let mut result = Vec::new();
    let mut used = std::collections::HashSet::new();
    let mut used_characters = std::collections::HashSet::new();
    let mut character_count = 0usize;
    let mut concept_count = 0usize;

    for draft in parsed.selections {
        let Some(model) = candidate_map.get(&draft.id) else {
            return Err(AppError::ModelResponse(format!("LoRA selector returned unknown model ID '{}'", draft.id)));
        };
        if !used.insert(model.id.clone()) {
            continue;
        }
        let role = draft.role.trim().to_lowercase();
        if role != "character" && role != "concept_pose" {
            continue;
        }
        let canonical_character = if role == "character" {
            let Some(requested_character) = draft.character.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
                continue;
            };
            let Some((_character_id, character_key, character_name)) = allowed_characters.iter().find(|entry| {
                requested_character == entry.0.as_str()
                    || normalize_name(requested_character) == entry.1
            }) else {
                continue;
            };
            if !used_characters.insert(character_key.clone()) {
                continue;
            }
            character_count += 1;
            Some(character_name.clone())
        } else {
            if !can_select_concept_pose(concept_count) {
                continue;
            }
            concept_count += 1;
            None
        };

        if role == "character" && character_count > scene.characters.len().min(2) {
            continue;
        }

        let artifact = registry.model_artifact(&model.id).await?;
        let weight = if role == "character" {
            draft.weight.clamp(0.55, 1.0)
        } else {
            draft.weight.clamp(0.25, 0.85)
        };

        result.push(SceneLoraSelection {
            id: model.id.clone(),
            name: model.name.clone(),
            role,
            character: canonical_character,
            weight,
            file_name: artifact.file_name,
            activation_prompts: artifact.activation_prompts,
            reason: draft.reason.trim().to_string(),
        });
    }

    emit_pipeline(app, "lora_selection", "completed", format!("Selected {} dynamic LoRA(s)", result.len()));
    Ok(result)
}

#[tauri::command]
fn build_comfyui_workflow(
    workflow: Value,
    lora_stack: Vec<workflow_builder::WorkflowLoraInput>,
    checkpoint_node: Option<String>,
    image_width: u32,
    image_height: u32,
) -> AppResult<workflow_builder::WorkflowBuildResult> {
    workflow_builder::build_workflow(workflow_builder::WorkflowBuildRequest {
        workflow,
        lora_stack,
        checkpoint_node,
        image_width,
        image_height,
    })
}

#[tauri::command]
async fn queue_scene_image(story_id: String, chapter_number: usize, scene_id: String, store: State<'_, Store>) -> AppResult<Story> {
    let mut story = require_story(&store, &story_id)?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    if settings.comfyui_url.trim().is_empty() {
        return Err(AppError::ComfyUi("configure the ComfyUI URL in settings".into()));
    }
    if settings.comfyui_workflow_json.trim().is_empty() {
        return Err(AppError::ComfyUi("configure a ComfyUI API workflow template in settings".into()));
    }
    let chapter = story.chapters.iter().find(|c| c.number == chapter_number).ok_or_else(|| AppError::ComfyUi("chapter not found".into()))?;
    let scene = chapter.scenes.iter().find(|s| s.id == scene_id).ok_or_else(|| AppError::ComfyUi("scene not found".into()))?;
    if scene.positive_prompt.trim().is_empty() {
        return Err(AppError::ComfyUi("build the scene prompt before queuing the image".into()));
    }
    let image_width = scene.image_width;
    let image_height = scene.image_height;
    let mut workflow: Value = serde_json::from_str(&settings.comfyui_workflow_json)
        .map_err(|e| AppError::ComfyUi(format!("workflow JSON is invalid: {e}")))?;
    let seed = (Uuid::new_v4().as_u128() & u64::MAX as u128) as u64;
    let replacements = vec![
        ("{{POSITIVE_PROMPT}}", Value::String(scene.positive_prompt.clone())),
        ("{{NEGATIVE_PROMPT}}", Value::String(scene.negative_prompt.clone())),
        ("{{SEED}}", Value::Number(serde_json::Number::from(seed))),
        ("{{STORY_ID}}", Value::String(story.id.clone())),
        ("{{SCENE_ID}}", Value::String(scene.id.clone())),
        ("{{CHECKPOINT}}", Value::String(story.visual_config.checkpoint_file_name.clone())),
    ];
    let replacement_refs = replacements.iter().map(|(token, value)| (*token, value.clone())).collect::<Vec<_>>();
    replace_workflow_placeholders(&mut workflow, &replacement_refs);

    let character_loras = scene
        .selected_loras
        .iter()
        .filter(|l| l.role == "character")
        .take(2)
        .collect::<Vec<_>>();
    let concept_lora = scene.selected_loras.iter().find(|l| l.role == "concept_pose");

    let mut lora_stack = Vec::<workflow_builder::WorkflowLoraInput>::new();
    for lora in &story.visual_config.style_loras {
        lora_stack.push(workflow_builder::WorkflowLoraInput {
            file_name: lora.file_name.clone(),
            weight: lora.weight,
            clip_weight: None,
        });
    }
    for lora in character_loras {
        lora_stack.push(workflow_builder::WorkflowLoraInput {
            file_name: lora.file_name.clone(),
            weight: lora.weight,
            clip_weight: None,
        });
    }
    if let Some(lora) = concept_lora {
        lora_stack.push(workflow_builder::WorkflowLoraInput {
            file_name: lora.file_name.clone(),
            weight: lora.weight,
            clip_weight: None,
        });
    }

    emit_pipeline(store.app(), "workflow_builder", "started", format!("Building ComfyUI graph with {} LoRA(s)", lora_stack.len()));
    // Tool boundary: the selector has finished. From here on the workflow builder
    // deterministically creates one LoraLoader per selected LoRA and chains
    // MODEL + CLIP through the complete ordered stack.
    let built = workflow_builder::build_workflow(workflow_builder::WorkflowBuildRequest {
        workflow,
        lora_stack,
        checkpoint_node: None,
        image_width,
        image_height,
    })?;
    let workflow = built.workflow;
    emit_pipeline(store.app(), "workflow_builder", "completed", format!("Created {} LoRA loader node(s)", built.lora_node_ids.len()));
    let comfyui_url = settings.comfyui_url.trim();
    if !(comfyui_url.starts_with("http://") || comfyui_url.starts_with("https://")) {
        return Err(AppError::ComfyUi("ComfyUI URL must start with http:// or https://".into()));
    }
    let url = format!("{}/prompt", comfyui_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::ComfyUi(format!("failed to create HTTP client: {e}")))?;
    emit_pipeline(store.app(), "comfyui", "started", "Submitting completed workflow to ComfyUI /prompt");
    let client_id = format!("raphael-story-{}-{}", story.id, Uuid::new_v4());
    let requested_prompt_id = Uuid::new_v4().to_string();
    let response = client.post(url).json(&json!({
        "prompt": workflow,
        "client_id": client_id,
        "prompt_id": requested_prompt_id,
    })).send().await.map_err(|e| AppError::ComfyUi(e.to_string()))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| AppError::ComfyUi(e.to_string()))?;
    let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({ "error": body }));
    if !status.is_success() {
        return Err(AppError::ComfyUi(
            value.get("error").and_then(Value::as_str).unwrap_or("ComfyUI rejected the workflow").to_string()
        ));
    }
    let prompt_id = value.get("prompt_id").and_then(Value::as_str).unwrap_or(&requested_prompt_id).to_string();
    let chapter_mut = story.chapters.iter_mut().find(|c| c.number == chapter_number)
        .ok_or_else(|| AppError::ComfyUi("chapter disappeared while updating image status".into()))?;
    let scene_mut = chapter_mut.scenes.iter_mut().find(|s| s.id == scene_id)
        .ok_or_else(|| AppError::ComfyUi("scene disappeared while updating image status".into()))?;
    emit_pipeline(store.app(), "comfyui", "running", "ComfyUI accepted the workflow; monitoring generation");
    scene_mut.comfy_prompt_id = Some(prompt_id.clone());
    scene_mut.image_status = "queued".into();
    scene_mut.image_error = None;
    scene_mut.image_path = None;
    scene_mut.image_mime = None;
    story.updated_at = now();
    let queued = write_story(&store, story)?;
    comfyui::spawn_generation_monitor(
        store.app().clone(),
        settings.comfyui_url.clone(),
        queued.id.clone(),
        chapter_number,
        scene_id.clone(),
        client_id,
        prompt_id,
    );
    Ok(queued)
}

#[tauri::command]
fn get_scene_image(
    story_id: String,
    chapter_number: usize,
    scene_id: String,
    store: State<'_, Store>,
) -> AppResult<Option<String>> {
    let story = require_story(&store, &story_id)?;
    let scene = story
        .chapters
        .iter()
        .find(|chapter| chapter.number == chapter_number)
        .and_then(|chapter| chapter.scenes.iter().find(|scene| scene.id == scene_id))
        .ok_or_else(|| AppError::ComfyUi("scene not found".into()))?;

    let Some(relative_path) = scene.image_path.as_deref() else {
        return Ok(None);
    };

    let generated_root = store.root.join("generated");
    let image_path = store.root.join(relative_path);
    let canonical_root = generated_root
        .canonicalize()
        .map_err(|e| AppError::ComfyUi(format!("generated image directory unavailable: {e}")))?;
    let canonical_image = image_path
        .canonicalize()
        .map_err(|e| AppError::ComfyUi(format!("generated image unavailable: {e}")))?;
    if !canonical_image.starts_with(&canonical_root) {
        return Err(AppError::ComfyUi("generated image path escaped the application data directory".into()));
    }

    let bytes = fs::read(&canonical_image).map_err(|e| AppError::ComfyUi(e.to_string()))?;
    let mime = scene.image_mime.as_deref().unwrap_or("image/png");
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(format!("data:{mime};base64,{encoded}")))
}

fn model_manager_app_data_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(value) = env::var_os("RAPHAEL_MODEL_MANAGER_APP_DATA_DIR") {
        candidates.push(PathBuf::from(value));
    }

    if let Some(value) = env::var_os("APPDATA") {
        candidates.push(PathBuf::from(value).join("com.raphael.modelmanager"));
    }

    if let Some(value) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(value).join("com.raphael.modelmanager"));
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        candidates.push(base_dirs.data_dir().join("com.raphael.modelmanager"));
    }

    candidates
}

fn model_manager_cache_path(app_data: &Path, connection: &rusqlite::Connection) -> PathBuf {
    connection
        .query_row(
            "SELECT value FROM settings WHERE key='cache_location'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| app_data.join("cache"))
}

fn cached_model_thumbnail(registry_model_id: &str) -> AppResult<Option<String>> {
    let registry_model_id = registry_model_id.trim();
    if registry_model_id.is_empty() {
        return Ok(None);
    }

    for app_data in model_manager_app_data_candidates() {
        let db_path = app_data.join("raphael.db");
        if !db_path.is_file() {
            continue;
        }

        let connection = match rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(connection) => connection,
            Err(_) => continue,
        };

        let cached_path = match connection.query_row(
            "SELECT COALESCE(NULLIF(thumbnail_path, ''), NULLIF(cover_path, ''))
             FROM models
             WHERE registry_model_id = ?1
             LIMIT 1",
            [registry_model_id],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(_) => continue,
        };

        let Some(cached_path) = cached_path else {
            continue;
        };

        let cache_root = model_manager_cache_path(&app_data, &connection);
        let requested = PathBuf::from(&cached_path);
        let requested = if requested.is_absolute() {
            requested
        } else {
            cache_root.join(requested)
        };

        let canonical_root = cache_root
            .canonicalize()
            .unwrap_or_else(|_| cache_root.clone());
        let canonical_path = match requested.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };

        if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
            continue;
        }

        let metadata = fs::metadata(&canonical_path)
            .map_err(|error| AppError::Storage(format!("failed to inspect cached model thumbnail: {error}")))?;
        if metadata.len() > 8 * 1024 * 1024 {
            continue;
        }

        let mime = match canonical_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "avif" => "image/avif",
            _ => continue,
        };

        let bytes = fs::read(&canonical_path)
            .map_err(|error| AppError::Storage(format!("failed to read cached model thumbnail: {error}")))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Ok(Some(format!("data:{mime};base64,{encoded}")));
    }

    Ok(None)
}

#[tauri::command]
fn get_registry_model_thumbnails(
    model_ids: Vec<String>,
) -> AppResult<HashMap<String, String>> {
    let mut thumbnails = HashMap::new();

    for model_id in model_ids.into_iter().filter(|value| !value.trim().is_empty()).take(500) {
        if let Some(thumbnail) = cached_model_thumbnail(&model_id)? {
            thumbnails.insert(model_id, thumbnail);
        }
    }

    Ok(thumbnails)
}

#[tauri::command]
async fn get_service_status(
    store: State<'_, Store>,
    registry: State<'_, registry::RegistryState>,
) -> AppResult<service_status::ServiceStatusBoard> {
    let settings = match store.settings.read() {
        Ok(value) => value.clone(),
        Err(_) => AppSettings::default(),
    };

    let board = service_status::probe(&registry, &settings).await;

    let detected_comfy_url = matches!(board.comfyui.status, service_status::ServiceHealthStatus::Online)
        .then(|| board.comfyui.url.clone())
        .filter(|url| url != &settings.comfyui_url);

    if let Some(url) = detected_comfy_url {
        {
            let mut current = store.settings.write().map_err(|e| AppError::Storage(e.to_string()))?;
            current.comfyui_url = url;
        }
        store.persist_settings()?;
    }

    Ok(board)
}

#[tauri::command]
async fn test_private_web_research(store: State<'_, Store>) -> AppResult<String> {
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    privacy_gateway::ensure_started_with_settings(store.app(), &settings).await?;
    Ok("Private web research gateway is running and Tor routing is verified.".into())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let registry = registry::RegistryState::new(app.handle());
            app.manage(registry);
            let store = Store::new(app.handle()).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })?;
            let web_settings = store
                .settings
                .read()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
                .clone();
            let comfyui_url = web_settings.comfyui_url.clone();
            app.manage(store);

            if web_settings.web_research_enabled {
                let handle = app.handle().clone();
                let startup_settings = web_settings.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        privacy_gateway::ensure_started_with_settings(&handle, &startup_settings).await
                    {
                        eprintln!("Raphael private web research gateway startup failed: {error}");
                    }
                });
            }

            comfyui::resume_queued_generations(app.handle(), &comfyui_url);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_app_state, get_story, get_settings, save_settings, clear_llm_api_key, create_story, generate_next_chapter, extract_scenes, build_scene_prompt, queue_scene_image, get_scene_image, build_comfyui_workflow, test_private_web_research, get_service_status, get_registry_model_thumbnails, registry::ensure_registry, registry::get_registry_status, registry::get_registry_models])
        .run(tauri::generate_context!())
        .expect("error while running Raphael Story Generator");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_json_extracts_json_from_wrapped_output() {
        let value = clean_json("Here is the result:\n{\"ok\":true}\n");
        assert_eq!(value, "{\"ok\":true}");
    }

    #[test]
    fn normalize_name_is_stable_for_matching() {
        assert_eq!(normalize_name("  Alice  "), "alice");
    }

    #[test]
    fn concept_pose_selection_allows_one_and_rejects_additional_choices() {
        assert!(can_select_concept_pose(0));
        assert!(!can_select_concept_pose(1));
    }

    #[test]
    fn workflow_placeholders_preserve_typed_values() {
        let mut workflow = json!({
            "positive": "{{POSITIVE_PROMPT}}",
            "seed": "{{SEED}}"
        });

        replace_workflow_placeholders(&mut workflow, &[
            ("{{POSITIVE_PROMPT}}", Value::String("a hero".into())),
            ("{{SEED}}", Value::Number(serde_json::Number::from(123_u64))),
        ]);

        assert_eq!(workflow["positive"], Value::String("a hero".into()));
        assert_eq!(workflow["seed"], Value::Number(serde_json::Number::from(123_u64)));
    }

    #[test]
    fn invalid_story_identity_is_rejected() {
        let story = Story {
            id: "../outside".into(),
            title: "Story".into(),
            source_prompt: String::new(),
            research: ResearchBundle::default(),
            metadata: StoryMetadata::default(),
            visual_config: StoryVisualConfig::default(),
            introduction: String::new(),
            bible: StoryBible::default(),
            chapters: Vec::new(),
            created_at: now(),
            updated_at: now(),
        };

        assert!(validate_story_identity(&story).is_err());
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let mut settings = AppSettings::default();
        settings.temperature = 2.5;
        assert!(validate_settings(&settings).is_err());

        settings.temperature = 0.8;
        settings.comfyui_workflow_json = "not json".into();
        assert!(validate_settings(&settings).is_err());
    }
}