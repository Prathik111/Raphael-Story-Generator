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
use std::{collections::HashMap, env, fs, path::PathBuf, sync::RwLock};
use keyring::Entry;
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
Every scene field description, location, time, action, composition, and dialogue must be a string. characters must be an array of string character names exactly matching the canonical character names. Do not return objects or nested structures for any scene field; write structured visual details as natural-language strings.
Return ONLY valid JSON."#;

const DEFAULT_LORA_SELECTOR_SYSTEM_PROMPT: &str = r#"You are Raphael Character LoRA Selector. Select existing character-specific LoRAs for the named characters in one image scene.

The candidate metadata comes directly from the Raphael Model Registry. Treat these fields as authoritative:
- MODEL TYPE
- BASE MODEL
- TAGS
- SHORT DESCRIPTION

Only select models whose Registry tags explicitly identify them as character LoRAs and whose name, tags, or description match a character present in this scene. Never substitute another character's LoRA because it has a similar art style or genre. Never select style, concept, pose, background, or general-purpose LoRAs; locked style LoRAs are handled separately.

Select at most one LoRA for each canonical scene character and at most two character LoRAs per scene. Use only candidate IDs supplied in the prompt. If no candidate clearly matches a character, omit that character instead of guessing. Explain the matching Registry metadata in the reason field.

Return ONLY valid JSON using the requested schema."#;

const KEYRING_SERVICE: &str = "com.raphael.storygenerator";
const KEYRING_USER: &str = "llm-api-key";

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
- Never choose or change output dimensions. The user's fixed image size is supplied separately and applied uniformly by the application.
- Return only positive_prompt and negative_prompt; image dimensions are application settings, not model-generated output.

Rules for the negative prompt:
- Describe unwanted visual results that should be suppressed: identity drift, incorrect appearance/clothing, extra or missing limbs, malformed hands/fingers, anatomy errors, duplicate subjects, bad proportions, deformed faces, blur, low detail, noise, compression artifacts, text, watermark, logo, signature, UI elements, cropped subjects, and scene contradictions.
- Keep it as a concise comma-separated list.
- Never put LoRA activation prompts or positive scene facts into the negative prompt.
- Do not use the negative prompt to introduce a different style.

Output rules:
- Return ONLY valid JSON matching the exact schema.
- Do not wrap the JSON in Markdown fences.
- Do not add commentary before or after the JSON."#;

fn default_image_width() -> u32 { 1920 }
fn default_image_height() -> u32 { 1080 }

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
    #[serde(default = "default_image_width")]
    pub image_width: u32,
    #[serde(default = "default_image_height")]
    pub image_height: u32,
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
            image_width: default_image_width(),
            image_height: default_image_height(),
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
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    pub activation_prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StoryVisualConfig {
    pub checkpoint_id: String,
    pub checkpoint_name: String,
    pub checkpoint_file_name: String,
    #[serde(default)]
    pub checkpoint_file_path: String,
    #[serde(default)]
    pub checkpoint_relative_path: Option<String>,
    #[serde(default)]
    pub checkpoint_version_id: Option<String>,
    #[serde(default)]
    pub checkpoint_file_id: Option<String>,
    #[serde(default)]
    pub checkpoint_sha256: Option<String>,
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
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
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
    #[serde(default = "default_image_width")]
    pub image_width: u32,
    #[serde(default = "default_image_height")]
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
    #[serde(default)]
    pub revision: u64,
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

fn json_value_as_string(value: &Value) -> Value {
    match value {
        Value::String(_) => value.clone(),
        Value::Null => Value::String(String::new()),
        _ => Value::String(serde_json::to_string(value).unwrap_or_else(|_| value.to_string())),
    }
}

fn normalize_story_architect_json(value: &mut Value) {
    let character_names = value
        .get("characters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|character| character.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|name| !name.trim().is_empty())
        .collect::<Vec<_>>();
    normalize_story_architect_value(value, &character_names);
}

fn normalize_story_architect_value(value: &mut Value, character_names: &[String]) {
    const STRING_KEYS: &[&str] = &[
        "title",
        "demographic",
        "content_rating",
        "source_type",
        "source_title",
        "premise",
        "central_conflict",
        "world_setting",
        "name",
        "role",
        "appearance",
        "clothing",
        "relation_type",
        "description",
        "source",
        "target",
        "introduction",
        "summary",
        "text",
        "character_id",
        "current_state",
        "source_character",
        "target_character",
    ];

    const STRING_ARRAY_KEYS: &[&str] = &[
        "genre",
        "tags",
        "tone",
        "inspirations",
        "themes",
        "world_rules",
        "locations",
        "personality",
        "motivations",
        "events",
        "continuity_updates",
        "open_threads",
    ];

    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if STRING_KEYS.iter().any(|candidate| *candidate == key) {
                    *child = json_value_as_string(child);
                } else if STRING_ARRAY_KEYS.iter().any(|candidate| *candidate == key) {
                    if let Value::Array(items) = child {
                        for item in items.iter_mut() {
                            *item = json_value_as_string(item);
                        }
                    } else {
                        *child = Value::Array(vec![json_value_as_string(child)]);
                    }
                } else if ["character_state_updates", "relationship_updates", "relationships", "characters"]
                    .iter()
                    .any(|candidate| *candidate == key)
                {
                    normalize_story_architect_records(child, key, character_names);
                } else {
                    normalize_story_architect_value(child, character_names);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_story_architect_value(item, character_names);
            }
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

fn normalize_story_architect_records(value: &mut Value, key: &str, character_names: &[String]) {
    let original = std::mem::replace(value, Value::Null);
    let items = match original {
        Value::Array(items) => items,
        Value::String(text) => vec![Value::String(text)],
        Value::Null => Vec::new(),
        other => vec![other],
    };

    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let mut item = match item {
            Value::String(text) if key == "character_state_updates" => {
                let lower_text = text.to_lowercase();
                let character_id = character_names
                    .iter()
                    .find(|name| lower_text.contains(&name.to_lowercase()))
                    .cloned()
                    .or_else(|| (character_names.len() == 1).then(|| character_names[0].clone()))
                    .unwrap_or_default();
                json!({
                    "character_id": character_id,
                    "current_state": text,
                    "clothing": null
                })
            }
            Value::String(text) if key == "relationship_updates" => json!({
                "source_character": "",
                "target_character": "",
                "relation_type": "",
                "description": text
            }),
            Value::String(text) if key == "relationships" => json!({
                "source": "",
                "target": "",
                "relation_type": "",
                "description": text
            }),
            Value::String(text) if key == "characters" => json!({"name": text}),
            other => other,
        };

        if let Some(object) = item.as_object_mut() {
            let required_string_fields: &[&str] = match key {
                "character_state_updates" => &["character_id", "current_state"],
                "relationship_updates" => &["source_character", "target_character", "relation_type", "description"],
                "relationships" => &["source", "target", "relation_type", "description"],
                "characters" => &["name"],
                _ => &[],
            };
            for field in required_string_fields {
                object.entry((*field).to_string()).or_insert_with(|| Value::String(String::new()));
            }
            if key == "character_state_updates" {
                object.entry("clothing".to_string()).or_insert(Value::Null);
            }
        }

        normalize_story_architect_value(&mut item, character_names);
        normalized.push(item);
    }

    *value = Value::Array(normalized);
}

fn parse_story_architect_response(raw: &str) -> Result<InitialResponse, String> {
    let cleaned = clean_json(raw);

    match serde_json::from_str::<InitialResponse>(cleaned) {
        Ok(parsed) => return Ok(parsed),
        Err(strict_error) => {
            let mut value = serde_json::from_str::<Value>(cleaned)
                .map_err(|json_error| format!("invalid JSON: {json_error}; strict schema error: {strict_error}"))?;

            normalize_story_architect_json(&mut value);

            serde_json::from_value::<InitialResponse>(value).map_err(|tolerant_error| {
                format!("strict schema error: {strict_error}; compatibility normalization parse also failed: {tolerant_error}")
            })
        }
    }
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
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SceneDraft {
    description: String,
    location: String,
    time: String,
    characters: Vec<String>,
    action: String,
    composition: String,
    dialogue: String,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SceneResponse { scenes: Vec<SceneDraft> }

fn scene_value_as_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(items) => items
            .iter()
            .map(scene_value_as_string)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        // Scene descriptions occasionally arrive as structured objects. Prefer a
        // readable text-like field; preserve the whole object if none is available.
        Value::Object(map) => {
            for key in ["text", "description", "value", "name", "location", "time", "period", "action", "details", "summary"] {
                if let Some(child) = map.get(key) {
                    let text = scene_value_as_string(child);
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
        Value::Number(_) | Value::Bool(_) => value.to_string(),
    }
}

fn scene_character_as_string(value: &Value) -> String {
    if let Value::Object(map) = value {
        for key in ["name", "character", "character_name", "id", "text", "value"] {
            if let Some(name) = map.get(key).and_then(Value::as_str) {
                if !name.trim().is_empty() {
                    return name.trim().to_string();
                }
            }
        }
    }
    scene_value_as_string(value)
}

fn normalize_scene_response_json(value: &mut Value) {
    let Some(root) = value.as_object_mut() else { return };
    let Some(scenes_value) = root.get_mut("scenes") else { return };

    let original_scenes = std::mem::replace(scenes_value, Value::Null);
    *scenes_value = match original_scenes {
        Value::Array(items) => Value::Array(items),
        Value::Object(_) => Value::Array(vec![original_scenes]),
        Value::Null => Value::Array(Vec::new()),
        other => Value::Array(vec![other]),
    };

    let Some(scenes) = scenes_value.as_array_mut() else { return };
    for scene in scenes {
        if !scene.is_object() {
            *scene = json!({"description": scene_value_as_string(scene)});
        }
        let Some(fields) = scene.as_object_mut() else { continue };
        for key in ["description", "location", "time", "action", "composition", "dialogue"] {
            if let Some(value) = fields.get_mut(key) {
                *value = Value::String(scene_value_as_string(value));
            }
        }

        let character_value = fields.remove("characters");
        let names = match character_value {
            Some(Value::Array(items)) => items
                .iter()
                .map(scene_character_as_string)
                .map(Value::String)
                .collect(),
            Some(Value::Null) | None => Vec::new(),
            Some(Value::Object(map))
                if map.contains_key("name")
                    || map.contains_key("character")
                    || map.contains_key("character_name")
                    || map.contains_key("id") =>
            {
                vec![Value::String(scene_character_as_string(&Value::Object(map)))]
            }
            Some(Value::Object(map)) => map.keys().map(|name| Value::String(name.clone())).collect(),
            Some(Value::String(text)) if text.trim().is_empty() => Vec::new(),
            Some(other) => vec![Value::String(scene_character_as_string(&other))],
        };
        fields.insert("characters".into(), Value::Array(names));
    }
}

fn parse_scene_response(raw: &str) -> Result<SceneResponse, String> {
    let cleaned = clean_json(raw);
    match serde_json::from_str::<SceneResponse>(cleaned) {
        Ok(parsed) => Ok(parsed),
        Err(strict_error) => {
            let mut value = serde_json::from_str::<Value>(cleaned)
                .map_err(|json_error| format!("invalid JSON: {json_error}; strict schema error: {strict_error}"))?;
            normalize_scene_response_json(&mut value);
            serde_json::from_value::<SceneResponse>(value).map_err(|tolerant_error| {
                format!("strict schema error: {strict_error}; scene compatibility normalization also failed: {tolerant_error}")
            })
        }
    }
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ImagePromptResponse {
    positive_prompt: String,
    negative_prompt: String,
}

fn validate_image_size(width: u32, height: u32) -> AppResult<(u32, u32)> {
    for (label, value) in [("width", width), ("height", height)] {
        if !(512..=4096).contains(&value) || value % 8 != 0 {
            return Err(AppError::ComfyUi(format!(
                "image {label} must be between 512 and 4096 and divisible by 8; received {value}"
            )));
        }
    }
    Ok((width, height))
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
        let mut settings = if settings_path.exists() {
            load_json::<AppSettings>(&settings_path).map_err(|e| {
                AppError::Storage(format!("failed to load settings.json: {e}"))
            })?
        } else {
            AppSettings::default()
        };

        if !settings.llm_api_key.trim().is_empty() {
            let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
                .map_err(|e| AppError::Storage(format!("failed to access OS credential store: {e}")))?;
            entry.set_password(settings.llm_api_key.trim())
                .map_err(|e| AppError::Storage(format!("failed to migrate stored LLM API key to OS credential store: {e}")))?;
            settings.llm_api_key.clear();
            save_json(&settings_path, &settings)?;
        } else if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            match entry.get_password() {
                Ok(secret) => settings.llm_api_key = secret,
                Err(keyring::Error::NoEntry) => {}
                Err(error) => return Err(AppError::Storage(format!("failed to load LLM API key from OS credential store: {error}"))),
            }
        }

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
            let mut story = story;
            if story.revision == 0 {
                story.revision = 1;
            }
            data.stories.insert(story.id.clone(), story);
        }

        Ok(Self { app: app.clone(), root, data: RwLock::new(data), settings: RwLock::new(settings) })
    }
    fn persist_settings(&self) -> AppResult<()> {
        let settings = self.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
        let mut persisted = settings;
        persisted.llm_api_key.clear();
        save_json(&self.root.join("settings.json"), &persisted)
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
    thinking_delta: Option<String>,
    thinking: Option<String>,
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
        thinking_delta: None,
        thinking: None,
        delta: None,
        response,
        error: Some(message),
    });
}

fn stream_value_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }

    if let Some(object) = value.as_object() {
        for key in ["text", "content", "value"] {
            if let Some(child) = object.get(key) {
                if let Some(text) = stream_value_text(child) {
                    return Some(text);
                }
            }
        }
    }

    if let Some(parts) = value.as_array() {
        let text = parts
            .iter()
            .filter_map(stream_value_text)
            .collect::<String>();

        if !text.is_empty() {
            return Some(text);
        }
    }

    None
}

fn extract_stream_content(value: &Value) -> Option<String> {
    let candidates = [
        value
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content")),
        value
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content")),
        value
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|choice| choice.get("text")),
        value.get("message").and_then(|message| message.get("content")),
        value.get("output_text"),
        value.get("response"),
        value.get("content"),
        value.get("text"),
    ];

    candidates.into_iter().flatten().find_map(stream_value_text)
}

fn extract_stream_reasoning(value: &Value) -> Option<&str> {
    value
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|choice| {
            choice
                .get("delta")
                .and_then(|delta| {
                    delta
                        .get("reasoning")
                        .and_then(Value::as_str)
                        .or_else(|| delta.get("reasoning_content").and_then(Value::as_str))
                        .or_else(|| delta.get("thinking").and_then(Value::as_str))
                })
                .or_else(|| {
                    choice
                        .get("message")
                        .and_then(|message| {
                            message
                                .get("reasoning")
                                .and_then(Value::as_str)
                                .or_else(|| message.get("reasoning_content").and_then(Value::as_str))
                                .or_else(|| message.get("thinking").and_then(Value::as_str))
                        })
                })
        })
        .or_else(|| value.get("reasoning").and_then(Value::as_str))
        .or_else(|| value.get("reasoning_content").and_then(Value::as_str))
        .or_else(|| value.get("thinking").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
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
        thinking_delta: None,
        thinking: Some(String::new()),
        delta: None,
        response: Some(String::new()),
        error: None,
    });

    let mut body = json!({
        "model": settings.llm_model.trim(),
        "temperature": settings.temperature,
        "stream": true,
        "messages": [
            {"role":"system","content":system},
            {"role":"user","content":user}
        ]
    });

    // Story-generation stages require the structured answer in the content field.
    // With thinking enabled, some Ollama reasoning models (including Qwen3.5)
    // can return empty content or spend the output budget in the reasoning field.
    // On Ollama's OpenAI-compatible API, "none" disables thinking and keeps the
    // JSON response in the normal content field for reliable parsing.
    let lower_base = base.to_ascii_lowercase();
    if lower_base.contains("11434") || lower_base.contains("ollama") {
        body["reasoning_effort"] = json!("none");
        body["response_format"] = json!({"type": "json_object"});
        body["max_tokens"] = json!(8192);
    }

    let endpoint = url.clone();
    let mut req = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        // Local/OpenAI-compatible servers can expose malformed or partial
        // compressed streams. Request identity encoding so the byte stream
        // reaches Raphael exactly as the provider emitted it.
        .header("Accept-Encoding", "identity")
        .json(&body);

    if !settings.llm_api_key.trim().is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {}", settings.llm_api_key.trim()));
    }

    let response = match req.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            emit_pipeline(app, stage, "error", message.clone());
            emit_llm_error(
                app,
                &generation_id,
                stage,
                settings.llm_model.trim(),
                None,
                message.clone(),
            );
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
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "request rejected".into());
        let value: Value = serde_json::from_str(&body)
            .unwrap_or_else(|_| json!({"error": body}));
        let message = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request rejected")
            .to_string();

        emit_pipeline(app, stage, "error", message.clone());
        emit_llm_error(
            app,
            &generation_id,
            stage,
            settings.llm_model.trim(),
            None,
            message.clone(),
        );
        return Err(AppError::Llm(message));
    }

    let token_generation_id = generation_id.clone();
    let mut full_response = String::new();
    let mut reasoning_response = String::new();
    let mut buffer = Vec::<u8>::new();
    let mut parsed_any = false;
    let mut saw_sse = false;
    let mut stream_error: Option<String> = None;

    let mut consume_payload = |payload: &str| {
        let data = payload.trim();

        if data.is_empty() || data == "[DONE]" {
            return;
        }

        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };

        if let Some(reasoning) = extract_stream_reasoning(&value) {
            reasoning_response.push_str(reasoning);

            emit_llm(app, LlmGenerationEvent {
                generation_id: token_generation_id.clone(),
                stage: stage.into(),
                status: "token".into(),
                model: settings.llm_model.trim().into(),
                system_prompt: None,
                user_prompt: None,
                thinking_delta: Some(reasoning.to_string()),
                thinking: Some(reasoning_response.clone()),
                delta: None,
                response: None,
                error: None,
            });
        }

        if let Some(text) = extract_stream_content(&value) {
            parsed_any = true;
            full_response.push_str(&text);

            emit_llm(app, LlmGenerationEvent {
                generation_id: token_generation_id.clone(),
                stage: stage.into(),
                status: "token".into(),
                model: settings.llm_model.trim().into(),
                system_prompt: None,
                user_prompt: None,
                thinking_delta: None,
                thinking: Some(reasoning_response.clone()),
                delta: Some(text),
                response: None,
                error: None,
            });
        }
    };

    // The request explicitly asks for streaming. Do not trust Content-Type alone:
    // several OpenAI-compatible servers send SSE/NDJSON with a generic JSON type.
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                stream_error = Some(error.to_string());
                // Do not immediately discard a response that already contains
                // usable JSON. Some providers close an otherwise valid SSE
                // stream with a transport/framing error.
                break;
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
                // Support NDJSON providers and newline-delimited JSON bodies.
                consume_payload(line);
            }
        }
    }

    // Some providers send one complete JSON object without a trailing newline.
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

    if !parsed_any && !reasoning_response.trim().is_empty() {
        // Some reasoning providers put the complete structured answer in the
        // reasoning field. Preserve only the structured object rather than
        // surfacing the provider's private chain-of-thought.
        if let (Some(start), Some(end)) = (
            reasoning_response.find('{'),
            reasoning_response.rfind('}'),
        ) {
            if start < end {
                full_response = reasoning_response[start..=end].to_string();
                parsed_any = true;
            }
        }
    }

    let complete_json = parsed_any
        && serde_json::from_str::<Value>(clean_json(&full_response)).is_ok();

    if let Some(error) = stream_error.as_deref() {
        if !complete_json {
            // One safe recovery attempt: ask the same provider for a normal,
            // non-streaming response. This is useful when only the SSE/body
            // transport failed; it does not retry successful or complete streams.
            let mut retry_body = body.clone();
            retry_body["stream"] = json!(false);

            let mut retry_request = client
                .post(endpoint.clone())
                .header(CONTENT_TYPE, "application/json")
                .header("Accept-Encoding", "identity")
                .json(&retry_body);

            if !settings.llm_api_key.trim().is_empty() {
                retry_request = retry_request
                    .header(AUTHORIZATION, format!("Bearer {}", settings.llm_api_key.trim()));
            }

            match retry_request.send().await {
                Ok(retry_response) if retry_response.status().is_success() => {
                    let retry_content_type = retry_response
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_ascii_lowercase();

                    match retry_response.json::<Value>().await {
                        Ok(value) => {
                            if let Some(reasoning) = extract_stream_reasoning(&value) {
                                reasoning_response.push_str(reasoning);
                            }

                            if let Some(text) = extract_stream_content(&value) {
                                full_response = text;
                                if serde_json::from_str::<Value>(clean_json(&full_response)).is_ok() {
                                    emit_pipeline(
                                        app,
                                        stage,
                                        "completed",
                                        format!("Recovered the LLM response with a non-streaming retry after the SSE transport failed: {error}"),
                                    );
                                    emit_llm(app, LlmGenerationEvent {
                                        generation_id,
                                        stage: stage.into(),
                                        status: "completed".into(),
                                        model: settings.llm_model.trim().into(),
                                        system_prompt: None,
                                        user_prompt: None,
                                        thinking_delta: None,
                                        thinking: Some(reasoning_response),
                                        delta: None,
                                        response: Some(full_response.clone()),
                                        error: None,
                                    });
                                    return Ok(full_response);
                                }
                            }

                            let message = format!(
                                "LLM stream transport failed before a complete JSON response was received ({error}); non-streaming retry returned no usable JSON (content-type: '{}')",
                                if retry_content_type.is_empty() { "unknown" } else { retry_content_type.as_str() }
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
                        Err(retry_error) => {
                            let message = format!(
                                "LLM stream transport failed before a complete JSON response was received ({error}); non-streaming retry failed to decode the response body: {retry_error}"
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
                    }
                }
                Ok(retry_response) => {
                    let status = retry_response.status();
                    let body_text = retry_response.text().await.unwrap_or_default();
                    let message = format!(
                        "LLM stream transport failed before a complete JSON response was received ({error}); non-streaming retry returned HTTP {}: {}",
                        status,
                        body_text.chars().take(300).collect::<String>()
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
                Err(retry_error) => {
                    let message = format!(
                        "LLM stream transport failed before a complete JSON response was received ({error}); non-streaming retry request failed: {retry_error}"
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
            }
        }

    }

    if !parsed_any {
        let format = if saw_sse {
            "SSE"
        } else if content_type.contains("ndjson") {
            "NDJSON"
        } else {
            "JSON/stream"
        };

        let message = format!(
            "LLM returned a {} response, but no usable content was found (content-type: '{}'). The provider may be returning only reasoning or an unsupported response shape.",
            format,
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
        thinking_delta: None,
        thinking: Some(reasoning_response.clone()),
        delta: None,
        response: Some(full_response.clone()),
        error: None,
    });

    Ok(full_response)
}

fn clean_json(raw: &str) -> &str {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```JSON")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if serde_json::from_str::<Value>(trimmed).is_ok() {
            return trimmed;
        }
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut start_index = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in chars.iter().enumerate() {
        if start_index.is_none() {
            if *ch == '{' {
                start_index = Some(index);
                depth = 1;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        if in_string {
            if *ch == '\\' {
                escaped = true;
            } else if *ch == '"' {
                in_string = false;
            }
            continue;
        }

        match *ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let start = start_index.unwrap_or(index);
                    let candidate: String = chars[start..=index].iter().collect();
                    if serde_json::from_str::<Value>(&candidate).is_ok() {
                        let offset = trimmed.find(&candidate).unwrap_or(0);
                        return &trimmed[offset..offset + candidate.len()];
                    }
                    start_index = None;
                }
            }
            _ => {}
        }
    }

    trimmed
}

fn merge_missing_chapter_fields(
    chapter: &mut ChapterDraft,
    repair: &Value,
) -> Result<Vec<&'static str>, String> {
    let fields = repair.get("chapter").filter(|value| value.is_object()).unwrap_or(repair);
    let mut repaired = Vec::new();

    if chapter.title.trim().is_empty() {
        let replacement = fields.get("title").and_then(Value::as_str).unwrap_or_default().trim();
        if !replacement.is_empty() {
            chapter.title = replacement.to_string();
            repaired.push("title");
        }
    }
    if chapter.summary.trim().is_empty() {
        let replacement = fields.get("summary").and_then(Value::as_str).unwrap_or_default().trim();
        if !replacement.is_empty() {
            chapter.summary = replacement.to_string();
            repaired.push("summary");
        }
    }
    if chapter.text.trim().is_empty() {
        let replacement = fields.get("text").and_then(Value::as_str).unwrap_or_default().trim();
        if !replacement.is_empty() {
            chapter.text = replacement.to_string();
            repaired.push("text");
        }
    }

    if chapter.title.trim().is_empty() || chapter.text.trim().is_empty() {
        return Err("repair response did not provide a non-empty chapter title and text".into());
    }
    if chapter.summary.trim().is_empty() {
        return Err("repair response did not provide a non-empty chapter summary".into());
    }
    Ok(repaired)
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

fn is_character_lora(model: &registry::RegistryLoraCandidate) -> bool {
    // Keep category semantics aligned with Raphael Image Generator's Registry
    // discovery: only Registry-tagged character LoRAs enter the character pool.
    model.tags.iter().any(|tag| {
        tag.trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>() == "character"
    })
}

fn character_lora_match_score(
    character_name: &str,
    model: &registry::RegistryLoraCandidate,
) -> i32 {
    let metadata = format!(
        "{} {} {}",
        model.name,
        model.tags.join(" "),
        model.description.as_deref().unwrap_or_default()
    );
    let stop_words = [
        "the", "and", "for", "with", "from", "into", "character", "characters",
        "lora", "anime", "official", "model", "version",
    ];
    let name_tokens = text_tokens(character_name)
        .into_iter()
        .filter(|token| !stop_words.contains(&token.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let metadata_tokens = text_tokens(&metadata)
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

    name_tokens.intersection(&metadata_tokens).count() as i32
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
    validate_image_size(settings.image_width, settings.image_height)?;
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

fn comfy_relative_model_name(path: &str, relative_path: Option<&str>, folder: &str, fallback: &str) -> String {
    let source = if !path.trim().is_empty() { path } else { relative_path.unwrap_or_default() };
    if source.trim().is_empty() {
        return fallback.to_string();
    }

    let normalized = source.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let mut relative = if let Some(index) = lower.find("/models/") {
        Some(normalized[index + "/models/".len()..].to_string())
    } else {
        lower.starts_with("models/").then(|| normalized["models/".len()..].to_string())
    };

    if let Some(value) = relative.as_mut() {
        let value_lower = value.to_ascii_lowercase();
        let folder_prefix = format!("{folder}/");
        if value_lower.starts_with(&folder_prefix) {
            *value = value[folder_prefix.len()..].to_string();
        } else if folder.eq_ignore_ascii_case("unet") && value_lower.starts_with("diffusion_models/") {
            *value = value["diffusion_models/".len()..].to_string();
        }
        if !value.is_empty() {
            return value.replace('/', "\\");
        }
    }

    let marker = format!("/{folder}/");
    if let Some(index) = lower.find(&marker) {
        return normalized[index + marker.len()..].replace('/', "\\");
    }
    let folder_prefix = format!("{folder}/");
    if lower.starts_with(&folder_prefix) {
        return normalized[folder_prefix.len()..].replace('/', "\\");
    }
    if folder.eq_ignore_ascii_case("unet") && lower.starts_with("diffusion_models/") {
        return normalized["diffusion_models/".len()..].replace('/', "\\");
    }
    fallback.to_string()
}

fn checkpoint_uses_unet_loader(path: &str, relative_path: Option<&str>) -> bool {
    let combined = format!("{} {}", path, relative_path.unwrap_or_default())
        .replace('\\', "/")
        .to_ascii_lowercase();
    combined.contains("models/unet/")
        || combined.contains("models/diffusion_models/")
        || combined.contains("/unet/")
        || combined.contains("/diffusion_models/")
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
fn write_story(store: &Store, mut story: Story) -> AppResult<Story> {
    validate_story_identity(&story)?;
    let mut data = store.data.write().map_err(|e| AppError::Storage(e.to_string()))?;
    if let Some(current) = data.stories.get(&story.id) {
        if current.revision != story.revision {
            return Err(AppError::Storage(format!(
                "story '{}' changed while this operation was running; refusing to overwrite newer state",
                story.id
            )));
        }
        story.revision = current.revision.saturating_add(1);
    } else {
        story.revision = 1;
    }
    store.persist_story(&story)?;
    data.stories.insert(story.id.clone(), story.clone());
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
fn llm_keyring_entry() -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| AppError::Storage(format!("failed to access OS credential store: {error}")))
}

fn save_llm_api_key(secret: &str) -> AppResult<()> {
    let entry = llm_keyring_entry()?;
    if secret.trim().is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(AppError::Storage(format!("failed to clear LLM API key: {error}"))),
        }
    } else {
        entry.set_password(secret.trim())
            .map_err(|error| AppError::Storage(format!("failed to store LLM API key in OS credential store: {error}")))
    }
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
    let current_key = store.settings.read()
        .map_err(|e| AppError::Storage(e.to_string()))?
        .llm_api_key.clone();
    if settings.llm_api_key.trim().is_empty() {
        settings.llm_api_key = current_key;
    } else {
        save_llm_api_key(&settings.llm_api_key)?;
    }
    validate_settings(&settings)?;
    *store.settings.write().map_err(|e| AppError::Storage(e.to_string()))? = settings.clone();
    let mut persisted = settings.clone();
    persisted.llm_api_key.clear();
    save_json(&store.root.join("settings.json"), &persisted)?;
    Ok(redacted_settings(&settings))
}

#[tauri::command]
fn clear_llm_api_key(store: State<'_, Store>) -> AppResult<String> {
    save_llm_api_key("")?;
    let mut settings = store.settings.write().map_err(|e| AppError::Storage(e.to_string()))?;
    settings.llm_api_key.clear();
    let mut persisted = settings.clone();
    persisted.llm_api_key.clear();
    save_json(&store.root.join("settings.json"), &persisted)?;
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
    let compatible = registry
        .compatible_models(&checkpoint.id, Some(registry_core::ModelType::Lora))
        .await?;
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
            file_path: artifact.path,
            relative_path: artifact.relative_path,
            version_id: artifact.version_id.clone(),
            file_id: Some(artifact.file_id.clone()),
            sha256: artifact.sha256.clone(),
            activation_prompts: artifact.activation_prompts,
        });
    }

    let checkpoint_artifact = registry.model_artifact(&checkpoint.id).await?;
    let visual_config = StoryVisualConfig {
        checkpoint_id: checkpoint.id.clone(),
        checkpoint_name: checkpoint.name.clone(),
        checkpoint_file_name: checkpoint_artifact.file_name.clone(),
        checkpoint_file_path: checkpoint_artifact.path.clone(),
        checkpoint_relative_path: checkpoint_artifact.relative_path.clone(),
        checkpoint_version_id: checkpoint_artifact.version_id.clone(),
        checkpoint_file_id: Some(checkpoint_artifact.file_id.clone()),
        checkpoint_sha256: checkpoint_artifact.sha256.clone(),
        style_loras,
    };
    let style_artifacts = visual_config.style_loras.iter().map(|item| registry::RegistryModelArtifact {
        id: item.id.clone(),
        version_id: item.version_id.clone(),
        file_id: item.file_id.clone().unwrap_or_default(),
        file_name: item.file_name.clone(),
        path: item.file_path.clone(),
        relative_path: item.relative_path.clone(),
        sha256: item.sha256.clone(),
        activation_prompts: item.activation_prompts.clone(),
    }).collect::<Vec<_>>();
    validate_visual_setup(&visual_config, &checkpoint_artifact, &style_artifacts)?;

    if settings.llm_model.trim().is_empty() { return Err(AppError::Llm("configure an LLM model in settings".into())); }

    let research_query = research::research_query_from_prompt(&prompt);
    let research_bundle = if settings.web_research_enabled {
        if let Some(query) = research_query.as_deref() {
            emit_pipeline(store.app(), "web_research", "started", format!("Research requested for explicit story subject(s): {query}"));
            emit_pipeline(store.app(), "web_gateway", "started", "Ensuring the private web research gateway is running");
            privacy_gateway::ensure_started_with_settings(store.app(), &settings).await?;
            emit_pipeline(store.app(), "web_gateway", "completed", "Private web research gateway is ready");
            match research::research_web(store.app(), &settings, query).await {
                Ok(bundle) => {
                    emit_pipeline(
                        store.app(),
                        "web_research",
                        "completed",
                        format!("Extracted {} source-backed fact(s) from {} usable source(s)", bundle.facts.len(), bundle.sources.len()),
                    );
                    bundle
                }
                Err(error) => {
                    emit_pipeline(store.app(), "web_research", "error", error.to_string());
                    return Err(error);
                }
            }
        } else {
            emit_pipeline(store.app(), "web_research", "skipped", "No explicit external character, series, world, or other research subject was found in the prompt; generating without web research");
            ResearchBundle::default()
        }
    } else {
        ResearchBundle::default()
    };

    let system = settings.story_architect_system_prompt.as_str();
    let schema_hint = r#"
JSON shape:
{"title":"","metadata":{"genre":[],"tags":[],"demographic":"","content_rating":"","tone":[],"source_type":"","source_title":"","inspirations":[]},"premise":"","central_conflict":"","themes":[],"world_setting":"","world_rules":[],"locations":[],"characters":[{"name":"","role":"","personality":[],"appearance":"","clothing":"","motivations":[]}],"relationships":[{"source":"","target":"","relation_type":"","description":""}],"open_threads":[],"introduction":"","chapter":{"title":"","summary":"","text":"","events":[],"continuity_updates":[],"character_state_updates":[{"character_id":"","current_state":"","clothing":null}],"relationship_updates":[{"source_character":"","target_character":"","relation_type":"","description":""}],"open_threads":[]}}
"#;
    let research_context = research::story_architect_context(&research_bundle);
    let architect_prompt = format!("USER STORY REQUEST:
{}

{}

{}

Use the web-research facts only when they are supported by the cited sources. Do not invent external facts that are absent from the supplied research.
Every empty string in the example JSON is a placeholder, not a value to copy. Populate required string fields with meaningful content. In particular, introduction must be a non-empty, engaging 2–4 sentence introduction that sets up the story before Chapter 1. Chapter title, summary, and text must also be non-empty. Chapter 1 must be an actual complete chapter with a beginning, development, and ending hook; aim for roughly 500–800 words of narrative text, not a summary or outline. Every scalar string field must be a JSON string; list-valued fields must be JSON arrays. For chapter.character_state_updates, every item must be an object with character_id (an exact existing character name or ID), current_state (a string), and optional clothing (a string or null); never put plain strings in this array, and leave it empty when no state changes are needed. For chapter.relationship_updates, each item must be an object with source_character, target_character, relation_type, and description string fields; leave it empty when there are no changes. Relationship entries must use the same object shape shown above, never plain strings.
Return the JSON shape above.", prompt.trim(), research_context, schema_hint);
    emit_pipeline(store.app(), "story_architect", "started", "Generating story bible and opening chapter");
    let raw = chat(store.app(), &settings, "story_architect", system, &architect_prompt).await?;
    let mut parsed: InitialResponse = parse_story_architect_response(&raw)
        .map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;

    // Smaller local models may copy the empty introduction placeholder from the schema.
    // Repair only that field instead of throwing away the otherwise valid Story Bible.
    if parsed.introduction.trim().is_empty() {
        emit_pipeline(
            store.app(),
            "story_introduction_repair",
            "started",
            "Story Architect returned an empty introduction; generating the missing field",
        );
        let character_names = parsed.characters.iter()
            .map(|character| character.name.trim())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let chapter_excerpt = parsed.chapter.text.chars().take(3000).collect::<String>();
        let repair_system = "You are Raphael Story Introduction Repairer. Write a concise, engaging introduction for an existing story. Return only valid JSON with one non-empty string field named introduction. Do not return an empty string, a chapter, or commentary.";
        let repair_prompt = format!(
            "Write a 2–4 sentence introduction that appears before Chapter 1. Set up the premise and central conflict, preserve canon, and do not simply repeat the chapter text.\n\nUSER STORY REQUEST:\n{}\n\nTITLE:\n{}\n\nPREMISE:\n{}\n\nCENTRAL CONFLICT:\n{}\n\nWORLD SETTING:\n{}\n\nCHARACTERS:\n{}\n\nCHAPTER 1 TITLE:\n{}\n\nCHAPTER 1 SUMMARY:\n{}\n\nCHAPTER 1 OPENING FOR CONTEXT:\n{}\n\nReturn exactly this JSON shape with meaningful content: {{\"introduction\":\"Your introduction here\"}}",
            prompt.trim(),
            parsed.title,
            parsed.premise,
            parsed.central_conflict,
            parsed.world_setting,
            if character_names.is_empty() { "(not specified)" } else { &character_names },
            parsed.chapter.title,
            parsed.chapter.summary,
            chapter_excerpt,
        );
        let repaired_raw = chat(
            store.app(),
            &settings,
            "story_introduction_repair",
            repair_system,
            &repair_prompt,
        ).await?;
        let repaired_value: Value = serde_json::from_str(clean_json(&repaired_raw))
            .map_err(|error| AppError::ModelResponse(format!(
                "story introduction repair returned invalid JSON: {}; raw output starts with: {}",
                error,
                &repaired_raw.chars().take(200).collect::<String>()
            )))?;
        let repaired_introduction = repaired_value.get("introduction")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if repaired_introduction.is_empty() {
            emit_pipeline(
                store.app(),
                "story_introduction_repair",
                "error",
                "The repair response did not contain a non-empty introduction",
            );
            return Err(AppError::ModelResponse(
                "generated story introduction was empty; automatic repair returned no non-empty introduction".into()
            ));
        }
        parsed.introduction = repaired_introduction.to_string();
        emit_pipeline(
            store.app(),
            "story_introduction_repair",
            "completed",
            "Filled the missing story introduction without regenerating the Story Bible",
        );
    }
    // Compact local models can still copy empty chapter placeholders from the schema.
    // Repair only the missing Chapter 1 fields, keeping all other generated canon intact.
    if parsed.chapter.title.trim().is_empty()
        || parsed.chapter.summary.trim().is_empty()
        || parsed.chapter.text.trim().is_empty()
    {
        emit_pipeline(
            store.app(),
            "chapter_one_repair",
            "started",
            "Story Architect returned an incomplete Chapter 1; generating the missing fields",
        );

        let character_context = parsed.characters.iter()
            .map(|character| format!(
                "{} — role: {}; appearance: {}; clothing: {}",
                character.name, character.role, character.appearance, character.clothing
            ))
            .collect::<Vec<_>>()
            .join("\n");
        let repair_system = "You are Raphael Chapter One Repairer. Complete missing fields for Chapter 1 of an existing story. Preserve all supplied canon. Return only valid JSON with non-empty string fields title, summary, and text. The text must be a complete narrative chapter, not an outline.";
        let repair_prompt = format!(
            "Complete the missing Chapter 1 fields for this story. Use approximately 500–800 words for text, with a coherent beginning, development, and ending hook. Do not return empty strings. Return JSON with exactly these fields: title, summary, text. Fill each field below only if it is marked MISSING, but always return all three fields.\n\nUSER STORY REQUEST:\n{}\n\nSTORY TITLE:\n{}\n\nPREMISE:\n{}\n\nCENTRAL CONFLICT:\n{}\n\nWORLD SETTING:\n{}\n\nCHARACTERS:\n{}\n\nINTRODUCTION:\n{}\n\nEXISTING CHAPTER TITLE: {}\nEXISTING CHAPTER SUMMARY: {}\nEXISTING CHAPTER TEXT (context only):\n{}\n\nRequired JSON shape: {{\"title\":\"non-empty chapter title\",\"summary\":\"non-empty summary\",\"text\":\"complete chapter narrative\"}}",
            prompt.trim(),
            parsed.title,
            parsed.premise,
            parsed.central_conflict,
            parsed.world_setting,
            if character_context.is_empty() { "(no characters supplied)" } else { &character_context },
            parsed.introduction,
            if parsed.chapter.title.trim().is_empty() { "MISSING" } else { parsed.chapter.title.as_str() },
            if parsed.chapter.summary.trim().is_empty() { "MISSING" } else { parsed.chapter.summary.as_str() },
            parsed.chapter.text.chars().take(2500).collect::<String>(),
        );

        let repaired_raw = chat(
            store.app(),
            &settings,
            "chapter_one_repair",
            repair_system,
            &repair_prompt,
        ).await?;
        let repaired_value: Value = serde_json::from_str(clean_json(&repaired_raw))
            .map_err(|error| AppError::ModelResponse(format!(
                "Chapter 1 repair returned invalid JSON: {}; raw output starts with: {}",
                error,
                &repaired_raw.chars().take(200).collect::<String>()
            )))?;
        let repaired_fields = merge_missing_chapter_fields(&mut parsed.chapter, &repaired_value)
            .map_err(|error| AppError::ModelResponse(format!("Chapter 1 repair failed: {error}")))?;
        emit_pipeline(
            store.app(),
            "chapter_one_repair",
            "completed",
            format!("Repaired missing Chapter 1 field(s): {}", repaired_fields.join(", ")),
        );
    }

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
        revision: 0,
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

    let directive_research_query = research::research_query_from_prompt(user_prompt.as_str());
    if settings.web_research_enabled {
        if let Some(query) = directive_research_query.as_deref() {
            emit_pipeline(store.app(), "web_research", "started", format!("Research requested for Chapter {} subject(s): {}", next_number, query));
            emit_pipeline(store.app(), "web_gateway", "started", "Ensuring the private web research gateway is running");
            privacy_gateway::ensure_started_with_settings(store.app(), &settings).await?;
            emit_pipeline(store.app(), "web_gateway", "completed", "Private web research gateway is ready");
            match research::research_web(store.app(), &settings, query).await {
                Ok(bundle) => {
                    research::merge_into(&mut story.research, bundle);
                    emit_pipeline(store.app(), "web_research", "completed", format!("Added new source-backed research for Chapter {}", next_number));
                }
                Err(error) => {
                    emit_pipeline(store.app(), "web_research", "error", error.to_string());
                    return Err(error);
                }
            }
        } else {
            emit_pipeline(store.app(), "web_research", "skipped", format!("No explicit research subject was found in the Chapter {} directive; continuing without web research", next_number));
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
    story = require_story(&store, &story_id)?;
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
    let (image_width, image_height) = validate_image_size(settings.image_width, settings.image_height)?;
    let chapter_index = story.chapters.iter().position(|c| c.number == chapter_number).ok_or_else(|| AppError::ModelResponse("chapter not found".into()))?;
    let chapter = story.chapters[chapter_index].clone();
    let visual_characters = story.bible.characters.iter().map(|c| format!("{} [{}] — appearance: {}; clothing: {}; personality: {:?}", c.id, c.name, c.appearance, c.clothing, c.personality)).collect::<Vec<_>>().join("
");
    let system = settings.scene_director_system_prompt.as_str();
    let schema = r#"{"scenes":[{"description":"single coherent visual beat","location":"specific location as a string","time":"time of day as a string","characters":["Exact canonical character name"],"action":"visible action as a string","composition":"camera and framing as a string","dialogue":"dialogue as a string or empty string"}]}"#;
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
    let parsed: SceneResponse = parse_scene_response(&raw).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
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
        negative_prompt: String::new(), selected_loras: Vec::new(), image_status: "not_ready".into(), image_url: None, image_path: None, image_mime: None, image_error: None, image_width, image_height, comfy_prompt_id: None,
    }).collect::<Vec<_>>();
    story = require_story(&store, &story_id)?;
    let chapter_index = story.chapters.iter().position(|c| c.number == chapter_number).ok_or_else(|| AppError::ModelResponse("chapter disappeared while saving extracted scenes".into()))?;
    story.chapters[chapter_index].scenes = scenes.clone();
    story.updated_at = now();
    write_story(&store, story)?;
    Ok(SceneExtractionResult { chapter_number, scenes })
}
fn compose_positive_prompt(
    activation_prompts: &[String],
    canonical_character_visuals: &str,
    generated_scene_prompt: &str,
) -> String {
    let activations = unique_nonempty(activation_prompts.iter().cloned());
    let mut scene_prompt = generated_scene_prompt.trim().to_string();
    for activation in &activations {
        scene_prompt = scene_prompt.replace(activation, "");
    }
    scene_prompt = scene_prompt
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");

    let mut parts = activations;
    if !canonical_character_visuals.trim().is_empty() {
        parts.push(format!(
            "canonical character design lock: {}",
            canonical_character_visuals.trim()
        ));
    }
    if !scene_prompt.is_empty() {
        parts.push(scene_prompt);
    }
    unique_nonempty(parts).join(", ")
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
    let (image_width, image_height) = validate_image_size(settings.image_width, settings.image_height)?;
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
        .map(|c| format!("{} — canonical appearance: {}; canonical clothing/outfit: {}; personality: {:?}", c.name, c.appearance, c.clothing, c.personality))
        .collect::<Vec<_>>().join("\n");
    let canonical_character_visuals = scene.characters.iter()
        .filter_map(|name| story.bible.characters.iter().find(|c| {
            name.trim() == c.id || normalize_name(name) == normalize_name(&c.name)
        }))
        .map(|character| format!(
            "{} — appearance: {}; exact canonical outfit: {}",
            character.name,
            if character.appearance.trim().is_empty() { "not specified" } else { character.appearance.trim() },
            if character.clothing.trim().is_empty() { "not specified" } else { character.clothing.trim() },
        ))
        .collect::<Vec<_>>();
    let canonical_character_visuals = unique_nonempty(canonical_character_visuals).join("; ");
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
    let schema = r#"{"positive_prompt":"","negative_prompt":""}"#;
    let user = format!("STORY: {}
FIXED IMAGE SIZE: {}x{}
Use this exact resolution for this scene and every other scene. Do not choose, infer, or return another size; the application applies the configured dimensions deterministically.
SCENE: {}
LOCATION: {}
TIME: {}
ACTION: {}
COMPOSITION: {}
DIALOGUE: {}

CANONICAL CHARACTER DESIGN — SOURCE OF TRUTH:
{}
Every listed character's appearance and outfit must remain exactly consistent with this reference. Do not change hair, eye color, age, body type, signature features, clothing type, clothing colors, accessories, or outfit layers. Raphael will inject this canonical reference into the final prompt verbatim. The generated positive_prompt should focus on scene-specific pose, action, expression, framing, lighting and environment, and must not invent or contradict character design.

CHARACTERS (same canon, additional context):
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
        image_width,
        image_height,
        scene.description,
        scene.location,
        scene.time,
        scene.action,
        scene.composition,
        scene.dialogue,
        canonical_character_visuals,
        characters,
        story.visual_config.style_loras.iter().map(|l| format!("{} (weight {})", l.name, l.weight)).collect::<Vec<_>>().join(", "),
        selected_loras.iter().map(|l| format!("{} [{} / {}] (weight {})", l.name, l.role, l.character.clone().unwrap_or_default(), l.weight)).collect::<Vec<_>>().join(", "),
        activation_prompt_context,
        schema
    );
    emit_pipeline(store.app(), "image_prompt_generator", "started", format!("Generating positive/negative prompts for {}", scene.id));
    let raw = chat(store.app(), &settings, "image_prompt_generator", system, &user).await?;
    let parsed: ImagePromptResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    story = require_story(&store, &story_id)?;
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
    let positive_prompt = compose_positive_prompt(
        &required_activation_prompts,
        &canonical_character_visuals,
        &parsed.positive_prompt,
    );

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

    let characters = story.bible.characters.iter()
        .filter(|character| scene.characters.iter().any(|name| {
            normalize_name(name) == normalize_name(&character.name) || name.trim() == character.id
        }))
        .take(2)
        .collect::<Vec<_>>();
    if characters.is_empty() {
        emit_pipeline(app, "lora_selection", "completed", "No canonical character is present in this scene; no character LoRA is needed");
        return Ok(Vec::new());
    }

    emit_pipeline(
        app,
        "character_lora_catalog",
        "started",
        format!("Searching the Registry for character LoRAs matching {}", characters.iter().map(|character| character.name.as_str()).collect::<Vec<_>>().join(", ")),
    );
    let candidates = registry.compatible_loras(&story.visual_config.checkpoint_id).await?;

    // Match only compatible, explicitly character-tagged Registry entries to
    // the canonical character names. Generic style, pose and concept LoRAs
    // never enter this scene's dynamic selection pool.
    let mut character_candidates = candidates.iter()
        .filter(|model| is_character_lora(model))
        .map(|model| {
            let score = characters.iter()
                .map(|character| character_lora_match_score(&character.name, model))
                .max()
                .unwrap_or(0);
            (score, model)
        })
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    character_candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    character_candidates.truncate(28);

    if character_candidates.is_empty() {
        emit_pipeline(
            app,
            "character_lora_catalog",
            "completed",
            format!(
                "No compatible character-tagged LoRA matched {}; the scene will use its canonical character design without a character LoRA",
                characters.iter().map(|character| character.name.as_str()).collect::<Vec<_>>().join(", ")
            ),
        );
        emit_pipeline(app, "lora_selection", "completed", "Selected 0 dynamic character LoRAs");
        return Ok(Vec::new());
    }

    let character_text = characters.iter()
        .map(|character| format!(
            "{} — appearance: {}; clothing: {}; role: {}; personality: {:?}",
            character.name, character.appearance, character.clothing, character.role, character.personality
        ))
        .collect::<Vec<_>>();
    let candidate_lines = character_candidates.iter()
        .map(|(score, model)| format!(
            "{}\nCANONICAL CHARACTER NAME MATCH SCORE: {}",
            lora_candidate_text(model), score
        ))
        .collect::<Vec<_>>();

    emit_pipeline(
        app,
        "character_lora_catalog",
        "completed",
        format!("Found {} compatible character LoRA candidate(s) matching this scene", character_candidates.len()),
    );

    let system = settings.lora_selector_system_prompt.as_str();
    let schema = r#"{"selections":[{"id":"","role":"character","character":"exact canonical character name","weight":0.85,"reason":"matching Registry name/tag/description"}]}"#;
    let user = format!(
        "SCENE: {}\nACTION: {}\nCOMPOSITION: {}\n\nCANONICAL CHARACTERS IN THIS SCENE:\n{}\n\nMATCHED CHARACTER-ONLY REGISTRY CANDIDATES:\n{}\n\nChoose only a LoRA whose Registry metadata identifies the same character. Use only supplied candidate IDs and exact canonical character names. Select at most one LoRA per character, at most two total. Do not choose generic style, pose, concept, setting, or background LoRAs. If none is an unambiguous match, return an empty selections array. The reason must quote or name the relevant Registry metadata.\n\nReturn:\n{}",
        scene.description,
        scene.action,
        scene.composition,
        character_text.join("\n"),
        candidate_lines.join("\n\n"),
        schema
    );
    let raw = chat(app, settings, "lora_selector", system, &user).await?;
    let parsed: SceneLoraSelectorResponse = serde_json::from_str(clean_json(&raw))
        .map_err(|e| AppError::ModelResponse(format!("LoRA selector output was invalid: {}; raw output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;

    let candidate_map = character_candidates.iter()
        .map(|(_, model)| (model.id.clone(), *model))
        .collect::<HashMap<_, _>>();
    let allowed_characters = characters.iter()
        .map(|character| (character.id.clone(), normalize_name(&character.name), character.name.clone()))
        .collect::<Vec<_>>();

    let mut result = Vec::new();
    let mut used_models = std::collections::HashSet::new();
    let mut used_characters = std::collections::HashSet::new();

    // First honor the LLM's ranked choice, but validate every ID, character
    // assignment and name/metadata match before a model can reach ComfyUI.
    for draft in parsed.selections {
        let Some(model) = candidate_map.get(&draft.id) else {
            continue;
        };
        if draft.role.trim().to_ascii_lowercase() != "character" {
            continue;
        }
        let Some(requested_character) = draft.character.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        let Some((_character_id, character_key, character_name)) = allowed_characters.iter().find(|entry| {
            requested_character == entry.0.as_str()
                || normalize_name(requested_character) == entry.1
        }) else {
            continue;
        };
        if character_lora_match_score(character_name, model) <= 0
            || !used_models.insert(model.id.clone())
            || !used_characters.insert(character_key.clone())
        {
            continue;
        }

        let artifact = registry.model_artifact(&model.id).await?;
        result.push(SceneLoraSelection {
            id: model.id.clone(),
            name: model.name.clone(),
            role: "character".into(),
            character: Some(character_name.clone()),
            weight: draft.weight.clamp(0.55, 1.0),
            file_name: artifact.file_name,
            file_path: artifact.path,
            relative_path: artifact.relative_path,
            version_id: artifact.version_id.clone(),
            file_id: Some(artifact.file_id.clone()),
            sha256: artifact.sha256.clone(),
            activation_prompts: artifact.activation_prompts,
            reason: if draft.reason.trim().is_empty() {
                "Matched Registry character metadata to the canonical character name".into()
            } else {
                draft.reason.trim().to_string()
            },
        });
    }

    // Deterministic fallback: use the strongest name/tag/description match for
    // any scene character the small selector model omitted. Never guess when
    // the Registry has no character-specific match.
    for (_character_id, character_key, character_name) in &allowed_characters {
        if used_characters.contains(character_key) {
            continue;
        }
        let best = character_candidates.iter()
            .filter(|(_, model)| !used_models.contains(&model.id))
            .filter_map(|(_, model)| {
                let score = character_lora_match_score(character_name, model);
                (score > 0).then_some((score, *model))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.name.cmp(&a.1.name)));
        let Some((_score, model)) = best else {
            continue;
        };
        let artifact = registry.model_artifact(&model.id).await?;
        result.push(SceneLoraSelection {
            id: model.id.clone(),
            name: model.name.clone(),
            role: "character".into(),
            character: Some(character_name.clone()),
            weight: 0.85,
            file_name: artifact.file_name,
            file_path: artifact.path,
            relative_path: artifact.relative_path,
            version_id: artifact.version_id.clone(),
            file_id: Some(artifact.file_id.clone()),
            sha256: artifact.sha256.clone(),
            activation_prompts: artifact.activation_prompts,
            reason: "Deterministic Registry name/tag/description match to canonical character".into(),
        });
        used_models.insert(model.id.clone());
        used_characters.insert(character_key.clone());
    }

    emit_pipeline(app, "lora_selection", "completed", format!("Selected {} dynamic character LoRA(s)", result.len()));
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
    let chapter = story.chapters.iter().find(|c| c.number == chapter_number).ok_or_else(|| AppError::ComfyUi("chapter not found".into()))?;
    let scene = chapter.scenes.iter().find(|s| s.id == scene_id).ok_or_else(|| AppError::ComfyUi("scene not found".into()))?;
    if scene.positive_prompt.trim().is_empty() {
        return Err(AppError::ComfyUi("build the scene prompt before queuing the image".into()));
    }

    let (image_width, image_height) = validate_image_size(settings.image_width, settings.image_height)?;
    let positive_prompt = scene.positive_prompt.clone();
    let negative_prompt = scene.negative_prompt.clone();
    let seed = (Uuid::new_v4().as_u128() & u64::MAX as u128) as u64;

    let use_unet_loader = checkpoint_uses_unet_loader(
        &story.visual_config.checkpoint_file_path,
        story.visual_config.checkpoint_relative_path.as_deref(),
    );
    let checkpoint_name = comfy_relative_model_name(
        &story.visual_config.checkpoint_file_path,
        story.visual_config.checkpoint_relative_path.as_deref(),
        if use_unet_loader { "unet" } else { "checkpoints" },
        &story.visual_config.checkpoint_file_name,
    );

    // Image Generator-compatible model order: locked style first, then the
    // character-matched LoRAs selected for this scene. No generic dynamic LoRAs.
    let character_loras = scene
        .selected_loras
        .iter()
        .filter(|lora| lora.role == "character")
        .take(2)
        .collect::<Vec<_>>();

    let mut lora_stack = Vec::<workflow_builder::WorkflowLoraInput>::new();
    for lora in &story.visual_config.style_loras {
        let file_name = comfy_relative_model_name(
            &lora.file_path,
            lora.relative_path.as_deref(),
            "loras",
            &lora.file_name,
        );
        lora_stack.push(workflow_builder::WorkflowLoraInput {
            file_name,
            weight: lora.weight,
            clip_weight: None,
        });
    }
    for lora in character_loras {
        let file_name = comfy_relative_model_name(
            &lora.file_path,
            lora.relative_path.as_deref(),
            "loras",
            &lora.file_name,
        );
        lora_stack.push(workflow_builder::WorkflowLoraInput {
            file_name,
            weight: lora.weight,
            clip_weight: None,
        });
    }

    let custom_workflow = !settings.comfyui_workflow_json.trim().is_empty();
    let built = if custom_workflow {
        let mut workflow: Value = serde_json::from_str(&settings.comfyui_workflow_json)
            .map_err(|e| AppError::ComfyUi(format!("workflow JSON is invalid: {e}")))?;
        let replacements = vec![
            ("{{POSITIVE_PROMPT}}", Value::String(positive_prompt.clone())),
            ("{{NEGATIVE_PROMPT}}", Value::String(negative_prompt.clone())),
            ("{{SEED}}", Value::Number(serde_json::Number::from(seed))),
            ("{{STORY_ID}}", Value::String(story.id.clone())),
            ("{{SCENE_ID}}", Value::String(scene.id.clone())),
            ("{{CHECKPOINT}}", Value::String(checkpoint_name.clone())),
        ];
        let replacement_refs = replacements.iter().map(|(token, value)| (*token, value.clone())).collect::<Vec<_>>();
        replace_workflow_placeholders(&mut workflow, &replacement_refs);
        emit_pipeline(store.app(), "workflow_builder", "started", format!("Building configured ComfyUI workflow with {} LoRA(s)", lora_stack.len()));
        workflow_builder::build_workflow(workflow_builder::WorkflowBuildRequest {
            workflow,
            lora_stack,
            checkpoint_node: None,
            image_width,
            image_height,
        })?
    } else {
        emit_pipeline(store.app(), "workflow_builder", "started", format!("Building Raphael Image Generator workflow with {} LoRA(s)", lora_stack.len()));
        workflow_builder::build_raphael_workflow(workflow_builder::RaphaelWorkflowRequest {
            checkpoint_name,
            use_unet_loader,
            positive_prompt,
            negative_prompt,
            seed,
            image_width,
            image_height,
            lora_stack,
        })?
    };
    let workflow = built.workflow;
    emit_pipeline(
        store.app(),
        "workflow_builder",
        "completed",
        format!("Created {} LoRA loader node(s){}", built.lora_node_ids.len(), if custom_workflow { " in the configured workflow" } else { " in the built-in Image Generator workflow" }),
    );
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
    story = require_story(&store, &story_id)?;
    let chapter_mut = story.chapters.iter_mut().find(|c| c.number == chapter_number)
        .ok_or_else(|| AppError::ComfyUi("chapter disappeared while updating image status".into()))?;
    let scene_mut = chapter_mut.scenes.iter_mut().find(|s| s.id == scene_id)
        .ok_or_else(|| AppError::ComfyUi("scene disappeared while updating image status".into()))?;
    emit_pipeline(store.app(), "comfyui", "running", "ComfyUI accepted the workflow; monitoring generation");
    scene_mut.comfy_prompt_id = Some(prompt_id.clone());
    scene_mut.image_width = image_width;
    scene_mut.image_height = image_height;
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

#[tauri::command]
async fn get_registry_model_thumbnails(
    model_ids: Vec<String>,
    registry: State<'_, registry::RegistryState>,
) -> AppResult<HashMap<String, String>> {
    let client = registry.client().await?;
    let mut thumbnails = HashMap::new();
    for model_id in model_ids.into_iter().filter(|value| !value.trim().is_empty()).take(500) {
        let assets = client.assets(&model_id).await
            .map_err(|error| AppError::Registry(format!("failed to load assets for model {model_id}: {error}")))?;
        let asset = ["thumbnail", "cover", "preview"].iter()
            .find_map(|kind| assets.iter().find(|asset| asset.kind.to_string() == *kind));
        let Some(asset) = asset else { continue; };
        let (content_type, bytes) = client.asset_content(&model_id, &asset.id).await
            .map_err(|error| AppError::Registry(format!("failed to load image asset for model {model_id}: {error}")))?;
        if bytes.len() > 8 * 1024 * 1024 { continue; }
        let mime = match content_type.to_ascii_lowercase().as_str() {
            value if value.contains("jpeg") => "image/jpeg",
            value if value.contains("png") => "image/png",
            value if value.contains("webp") => "image/webp",
            value if value.contains("gif") => "image/gif",
            value if value.contains("avif") => "image/avif",
            _ => continue,
        };
        thumbnails.insert(model_id, format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)));
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
    research::check_search_api(&settings).await?;
    Ok("Private web research gateway is running, Tor routing is verified, and SearXNG returned usable search results.".into())
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
                tauri::async_runtime::spawn(async move {
                    loop {
                        let current_settings = {
                            let store = handle.state::<Store>();
                            let settings_result = store.settings.read().map(|settings| settings.clone());
                            match settings_result {
                                Ok(settings) => settings,
                                Err(error) => {
                                    eprintln!("Raphael could not read research settings during gateway startup: {error}");
                                    break;
                                }
                            }
                        };

                        if !current_settings.web_research_enabled {
                            break;
                        }

                        match privacy_gateway::ensure_started_with_settings(&handle, &current_settings).await {
                            Ok(()) => break,
                            Err(error) => {
                                eprintln!(
                                    "Raphael private web research gateway startup failed; retrying in 15 seconds: {error}"
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            }
                        }
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
    fn chapter_repair_fills_only_missing_fields() {
        let mut chapter = ChapterDraft {
            title: "Already valid".into(),
            summary: String::new(),
            text: String::new(),
            ..ChapterDraft::default()
        };
        let repair = json!({
            "title": "Should not replace existing title",
            "summary": "A complete summary.",
            "text": "A complete chapter."
        });

        let repaired = merge_missing_chapter_fields(&mut chapter, &repair).unwrap();
        assert_eq!(chapter.title, "Already valid");
        assert_eq!(chapter.summary, "A complete summary.");
        assert_eq!(chapter.text, "A complete chapter.");
        assert_eq!(repaired, vec!["summary", "text"]);
    }

    #[test]
    fn chapter_repair_accepts_nested_chapter_shape() {
        let mut chapter = ChapterDraft::default();
        let repair = json!({
            "chapter": {
                "title": "Chapter One",
                "summary": "The hero begins the journey.",
                "text": "A full narrative."
            }
        });

        assert!(merge_missing_chapter_fields(&mut chapter, &repair).is_ok());
        assert_eq!(chapter.title, "Chapter One");
        assert_eq!(chapter.summary, "The hero begins the journey.");
        assert_eq!(chapter.text, "A full narrative.");
    }

    #[test]
    fn chapter_repair_rejects_still_empty_required_fields() {
        let mut chapter = ChapterDraft::default();
        let repair = json!({"title": "", "summary": "", "text": ""});
        assert!(merge_missing_chapter_fields(&mut chapter, &repair).is_err());
    }

    #[test]
    fn research_subject_extractor_skips_random_story_requests() {
        assert!(research::research_query_from_prompt("Give me a random story").is_none());
        assert!(research::research_query_from_prompt("Surprise me with an original story").is_none());
    }

    #[test]
    fn research_subject_extractor_keeps_explicit_subjects_only() {
        assert_eq!(
            research::research_query_from_prompt("Write a story about Naruto Uzumaki"),
            Some("Naruto Uzumaki".into())
        );
    }

    #[test]
    fn stream_content_parser_handles_openai_delta_and_ollama_message_shapes() {
        let openai = json!({"choices":[{"delta":{"content":"{\"title\":\"X\"}"}}]});
        assert_eq!(
            extract_stream_content(&openai).as_deref(),
            Some("{\"title\":\"X\"}")
        );

        let ollama = json!({"message":{"content":"{\"title\":\"X\"}"}});
        assert_eq!(
            extract_stream_content(&ollama).as_deref(),
            Some("{\"title\":\"X\"}")
        );
    }

    #[test]
    fn stream_content_parser_handles_array_object_and_top_level_shapes() {
        let parts = json!({
            "choices":[{"delta":{"content":[{"type":"text","text":"hello "},{"type":"text","text":"world"}]}}]
        });
        assert_eq!(extract_stream_content(&parts).as_deref(), Some("hello world"));

        let object = json!({
            "choices":[{"delta":{"content":{"type":"text","text":"object content"}}}]
        });
        assert_eq!(extract_stream_content(&object).as_deref(), Some("object content"));

        let response = json!({"response":"final json"});
        assert_eq!(extract_stream_content(&response).as_deref(), Some("final json"));
    }

    #[test]
    fn stream_reasoning_parser_handles_nested_and_top_level_shapes() {
        let nested = json!({"choices":[{"delta":{"reasoning":"thinking..."}}]});
        assert_eq!(extract_stream_reasoning(&nested), Some("thinking..."));

        let top_level = json!({"thinking":"still thinking"});
        assert_eq!(extract_stream_reasoning(&top_level), Some("still thinking"));
    }

    #[test]
    fn scene_parser_normalizes_object_valued_scene_fields() {
        let raw = r#"{
            "scenes": [{
                "description": "An atmospheric view of the wastes.",
                "location": {"name": "Ashen Wastes", "region": "Starfall Zone ruins"},
                "time": {"period": "Dusk", "weather": "perpetual twilight"},
                "characters": [{"name": "Aetheria Vane"}, "Milo"],
                "action": {"text": "Aetheria scans the horizon."},
                "composition": {"framing": "wide shot", "camera": "low angle"},
                "dialogue": {"text": "We are not alone."}
            }]
        }"#;

        let parsed = parse_scene_response(raw)
            .expect("map-valued scene details should be normalized to strings");
        assert_eq!(parsed.scenes.len(), 1);
        assert_eq!(parsed.scenes[0].location, "Ashen Wastes");
        assert_eq!(parsed.scenes[0].time, "Dusk");
        assert_eq!(parsed.scenes[0].action, "Aetheria scans the horizon.");
        assert_eq!(parsed.scenes[0].composition, r#"{"camera":"low angle","framing":"wide shot"}"#);
        assert_eq!(parsed.scenes[0].dialogue, "We are not alone.");
        assert_eq!(parsed.scenes[0].characters, vec!["Aetheria Vane", "Milo"]);
    }

    #[test]
    fn positive_prompt_injects_canonical_character_appearance_and_outfit() {
        let activations = vec!["aetheria_trigger".to_string(), "clockwork_style".to_string()];
        let visuals = "Aetheria Vane — appearance: silver hair, blue eyes; exact canonical outfit: navy coat, brass buttons";
        let generated = "wide cinematic shot, aetheria_trigger, dramatic twilight lighting";
        let result = compose_positive_prompt(&activations, visuals, generated);
        assert!(result.starts_with("aetheria_trigger, clockwork_style, canonical character design lock:"));
        assert!(result.contains("silver hair, blue eyes"));
        assert!(result.contains("navy coat, brass buttons"));
        assert!(result.contains("wide cinematic shot"));
        assert!(result.contains("dramatic twilight lighting"));
        assert_eq!(result.matches("aetheria_trigger").count(), 1);
        assert!(!result.contains(", ,"));
    }

    #[test]
    fn scene_parser_accepts_single_scene_object_and_scalar_characters() {
        let raw = r#"{
            "scenes": {
                "description": "A lone character crosses a bridge.",
                "location": "Iron Bridge",
                "time": "Night",
                "characters": "Aetheria Vane",
                "action": "Walking",
                "composition": "Wide shot",
                "dialogue": ""
            }
        }"#;
        let parsed = parse_scene_response(raw).expect("single scene and scalar character should normalize");
        assert_eq!(parsed.scenes.len(), 1);
        assert_eq!(parsed.scenes[0].characters, vec!["Aetheria Vane"]);
    }

    #[test]
    fn story_architect_parser_normalizes_sequence_string_fields_and_string_state_updates() {
        let raw = r#"{
            "title":"Test",
            "metadata":{"genre":["Fantasy"],"tags":["Magic"],"demographic":"Young Adult","content_rating":"Teen","tone":["Atmospheric"],"source_type":"","source_title":"","inspirations":[]},
            "premise":"A test premise",
            "central_conflict":"A test conflict",
            "themes":["identity"],
            "world_setting":"A test world",
            "world_rules":["magic"],
            "locations":["Tower"],
            "characters":[{
                "name":"Aetheria Vane",
                "role":["Inventor","Heir"],
                "personality":["curious"],
                "appearance":"silver-haired",
                "clothing":"coat",
                "motivations":["find the truth"]
            }],
            "relationships":[],
            "open_threads":[],
            "introduction":"Intro",
            "chapter":{
                "title":"Chapter 1",
                "summary":"Summary",
                "text":"Text",
                "events":[],
                "continuity_updates":[],
                "character_state_updates":["Aetheria Vane has fragmented memories of her mother, creating emotional confusion"],
                "relationship_updates":[],
                "open_threads":[]
            }
        }"#;

        let parsed = parse_story_architect_response(raw)
            .expect("schema normalization should recover scalar and state-update shape mismatches");
        assert_eq!(parsed.characters[0].role, r#"["Inventor","Heir"]"#);
        assert_eq!(parsed.chapter.character_state_updates.len(), 1);
        assert_eq!(parsed.chapter.character_state_updates[0].character_id, "Aetheria Vane");
        assert_eq!(
            parsed.chapter.character_state_updates[0].current_state,
            "Aetheria Vane has fragmented memories of her mother, creating emotional confusion"
        );
    }

    #[test]
    fn story_architect_parser_normalizes_string_relationship_updates() {
        let raw = r#"{
            "title":"Test",
            "metadata":{"genre":[],"tags":[],"demographic":"General Audience","content_rating":"PG","tone":[],"source_type":"","source_title":"","inspirations":[]},
            "premise":"Premise","central_conflict":"Conflict","themes":[],"world_setting":"World","world_rules":[],"locations":[],
            "characters":[{"name":"Hero","role":"lead","personality":[],"appearance":"tall","clothing":"coat","motivations":[]}],
            "relationships":["The hero trusts the guide."],"open_threads":[],"introduction":"Intro",
            "chapter":{"title":"Chapter 1","summary":"Summary","text":"Text","events":[],"continuity_updates":[],"character_state_updates":[],"relationship_updates":["The hero now trusts the guide."],"open_threads":[]}
        }"#;

        let parsed = parse_story_architect_response(raw)
            .expect("plain-language relationship entries should not crash schema parsing");
        assert_eq!(parsed.relationships.len(), 1);
        assert_eq!(parsed.relationships[0].description, "The hero trusts the guide.");
        assert_eq!(parsed.chapter.relationship_updates.len(), 1);
        assert_eq!(
            parsed.chapter.relationship_updates[0].description,
            "The hero now trusts the guide."
        );
    }

    #[test]
    fn story_architect_parser_accepts_numeric_values_for_string_fields() {
        let raw = r#"{
            "title":"Test",
            "metadata":{"genre":["Fantasy"],"tags":["Magic"],"demographic":"General Audience","content_rating":"PG-13","tone":["Epic"],"source_type":"","source_title":"","inspirations":[]},
            "premise":"A test premise",
            "central_conflict":"A test conflict",
            "themes":["identity"],
            "world_setting":"A test world",
            "world_rules":["magic"],
            "locations":["Tower"],
            "characters":[{
                "name":"Hero",
                "role":1.5,
                "personality":["calm"],
                "appearance":"tall",
                "clothing":"coat",
                "motivations":["survive"]
            }],
            "relationships":[],
            "open_threads":[],
            "introduction":"Intro",
            "chapter":{
                "title":"Chapter 1",
                "summary":"Summary",
                "text":"Text",
                "events":[],
                "continuity_updates":[],
                "character_state_updates":[],
                "relationship_updates":[],
                "open_threads":[]
            }
        }"#;

        let parsed = parse_story_architect_response(raw).expect("numeric string compatibility should parse");
        assert_eq!(parsed.characters[0].role, "1.5");
    }

    #[test]
    fn story_architect_parser_stringifies_object_values_only_on_string_fields() {
        let raw = r#"{
            "title":"Test",
            "metadata":{"genre":["Fantasy"],"tags":["Magic"],"demographic":"General Audience","content_rating":"PG-13","tone":["Epic"],"source_type":"","source_title":"","inspirations":[]},
            "premise":"A test premise",
            "central_conflict":"A test conflict",
            "themes":["identity"],
            "world_setting":"A test world",
            "world_rules":["magic"],
            "locations":["Tower"],
            "characters":[{
                "name":"Hero",
                "role":"Mage",
                "personality":["calm"],
                "appearance":{"height":"tall","hair":"black"},
                "clothing":"coat",
                "motivations":["survive"]
            }],
            "relationships":[],
            "open_threads":[],
            "introduction":"Intro",
            "chapter":{
                "title":"Chapter 1",
                "summary":"Summary",
                "text":"Text",
                "events":[],
                "continuity_updates":[],
                "character_state_updates":[],
                "relationship_updates":[]
            }
        }"#;

        let parsed = parse_story_architect_response(raw).expect("object-valued string field should normalize");
        let appearance: Value =
            serde_json::from_str(&parsed.characters[0].appearance).expect("appearance should contain JSON text");
        assert_eq!(appearance["height"], "tall");
        assert_eq!(appearance["hair"], "black");
    }

    #[test]
    fn clean_json_extracts_json_from_wrapped_output() {
        let value = clean_json("Here is the result:\n{\"ok\":true}\n");
        assert_eq!(value, "{\"ok\":true}");
    }

    #[test]
    fn clean_json_ignores_model_prose_after_valid_json() {
        let value = clean_json("{\"ok\":true} I should double-check the schema before continuing.");
        assert_eq!(value, "{\"ok\":true}");
    }

    #[test]
    fn clean_json_ignores_braces_inside_json_strings() {
        let value = clean_json("Wrapper: {\"text\":\"a { b } c\",\"ok\":true} trailing prose");
        assert_eq!(value, "{\"text\":\"a { b } c\",\"ok\":true}");
    }

    #[test]
    fn comfy_model_names_are_relative_to_the_correct_models_directory() {
        assert_eq!(
            comfy_relative_model_name(
                r"D:\ComfyUI\models\diffusion_models\anima\model.safetensors",
                None,
                "unet",
                "fallback.safetensors",
            ),
            r"anima\model.safetensors"
        );
        assert_eq!(
            comfy_relative_model_name(
                "",
                Some("models/loras/Anima/Character.safetensors"),
                "loras",
                "fallback.safetensors",
            ),
            r"Anima\Character.safetensors"
        );
        assert!(checkpoint_uses_unet_loader(
            r"D:\ComfyUI\models\diffusion_models\anima\model.safetensors",
            None,
        ));
        assert!(!checkpoint_uses_unet_loader(
            r"D:\ComfyUI\models\checkpoints\model.safetensors",
            None,
        ));
    }

    #[test]
    fn normalize_name_is_stable_for_matching() {
        assert_eq!(normalize_name("  Alice  "), "alice");
    }

    #[test]
    fn character_lora_filter_requires_registry_character_tag() {
        let character = registry::RegistryLoraCandidate {
            id: "lora_naruto".into(),
            name: "Naruto Uzumaki".into(),
            model_type: "lora".into(),
            description: Some("Naruto character identity".into()),
            base_model: Some("SDXL".into()),
            tags: vec!["character".into(), "anime".into()],
        };
        let style = registry::RegistryLoraCandidate {
            id: "lora_anime_style".into(),
            name: "Anime Style".into(),
            model_type: "lora".into(),
            description: Some("General anime rendering style".into()),
            base_model: Some("SDXL".into()),
            tags: vec!["style".into(), "anime".into()],
        };
        assert!(is_character_lora(&character));
        assert!(!is_character_lora(&style));
        assert!(character_lora_match_score("Naruto Uzumaki", &character) >= 2);
        assert_eq!(character_lora_match_score("Sasuke Uchiha", &character), 0);
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
            revision: 0,
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
    fn image_size_defaults_to_1920_by_1080_and_accepts_eight_pixel_steps() {
        let settings = AppSettings::default();
        assert_eq!(settings.image_width, 1920);
        assert_eq!(settings.image_height, 1080);
        assert_eq!(validate_image_size(1920, 1080).unwrap(), (1920, 1080));
        // 864 caused the previous workflow validation error because it is not
        // divisible by 64, though it is a valid multiple of 8 for the latent size.
        assert_eq!(validate_image_size(864, 1536).unwrap(), (864, 1536));
    }

    #[test]
    fn image_size_rejects_invalid_latent_dimensions() {
        assert!(validate_image_size(1919, 1080).is_err());
        assert!(validate_image_size(1920, 1079).is_err());
        assert!(validate_image_size(256, 1080).is_err());
        assert!(validate_image_size(1920, 5000).is_err());
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