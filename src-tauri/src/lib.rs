use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, path::PathBuf, sync::RwLock};
use tauri::{AppHandle, Manager, State};
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
}
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {
        serializer.serialize_str(&self.to_string())
    }
}
type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_api_key: String,
    pub temperature: f32,
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
    pub image_status: String,
    pub image_url: Option<String>,
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
    pub metadata: StoryMetadata,
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
pub struct AppStateDto {
    pub stories: Vec<StorySummary>,
    pub settings: AppSettings,
    pub llm_configured: bool,
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
#[derive(Debug, Deserialize)]
struct ImagePromptResponse { positive_prompt: String, negative_prompt: String }
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoreData { stories: HashMap<String, Story> }

struct Store {
    root: PathBuf,
    data: RwLock<StoreData>,
    settings: RwLock<AppSettings>,
}
impl Store {
    fn new(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app.path().app_data_dir().map_err(|e| AppError::Storage(e.to_string()))?;
        let root = data_dir.join("story-generator");
        fs::create_dir_all(root.join("stories")).map_err(|e| AppError::Storage(e.to_string()))?;
        let settings = load_json::<AppSettings>(&root.join("settings.json")).unwrap_or_default();
        let mut data = StoreData::default();
        if let Ok(entries) = fs::read_dir(root.join("stories")) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|x| x.to_str()) != Some("json") { continue; }
                if let Ok(story) = load_json::<Story>(&entry.path()) { data.stories.insert(story.id.clone(), story); }
            }
        }
        Ok(Self { root, data: RwLock::new(data), settings: RwLock::new(settings) })
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
    let text = fs::read_to_string(path).map_err(|e| AppError::Storage(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| AppError::Storage(e.to_string()))
}
fn save_json<T: Serialize>(path: &PathBuf, value: &T) -> AppResult<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| AppError::Storage(e.to_string()))?;
    fs::write(path, text).map_err(|e| AppError::Storage(e.to_string()))
}
fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("unix:{}", seconds)
}

fn normalize_name(value: &str) -> String {
    value.trim().to_lowercase()
}

fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::Llm(format!("failed to create HTTP client: {e}")))
}

async fn chat(settings: &AppSettings, system: &str, user: &str) -> AppResult<String> {
    validate_settings(settings)?;
    let base = settings.llm_base_url.trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") };
    let client = http_client()?;
    let body = json!({
        "model": settings.llm_model,
        "temperature": settings.temperature,
        "messages": [
            {"role":"system","content":system},
            {"role":"user","content":user}
        ]
    });
    let mut req = client.post(url).header(CONTENT_TYPE, "application/json").json(&body);
    if !settings.llm_api_key.trim().is_empty() { req = req.header(AUTHORIZATION, format!("Bearer {}", settings.llm_api_key.trim())); }
    let response = req.send().await.map_err(|e| AppError::Llm(e.to_string()))?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|e| AppError::Llm(e.to_string()))?;
    if !status.is_success() { return Err(AppError::Llm(value.get("error").and_then(Value::as_str).unwrap_or("request rejected").to_string())); }
    value.get("choices").and_then(|v| v.get(0)).and_then(|v| v.get("message")).and_then(|v| v.get("content")).and_then(Value::as_str).map(|s| s.to_string()).ok_or_else(|| AppError::ModelResponse("missing choices[0].message.content".into()))
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

fn validate_chapter_draft(parsed: &ChapterDraft, chapter_number: usize) -> AppResult<()> {
    if parsed.title.trim().is_empty() || parsed.text.trim().is_empty() {
        return Err(AppError::ModelResponse(format!(
            "generated Chapter {} is incomplete",
            chapter_number
        )));
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
fn get_app_state(store: State<'_, Store>) -> AppResult<AppStateDto> {
    let data = store.data.read().map_err(|e| AppError::Storage(e.to_string()))?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    Ok(AppStateDto { stories: build_summaries(&data), llm_configured: !settings.llm_base_url.is_empty() && !settings.llm_model.is_empty(), settings })
}
#[tauri::command]
fn get_story(id: String, store: State<'_, Store>) -> AppResult<Story> { require_story(&store, &id) }
#[tauri::command]
fn get_settings(store: State<'_, Store>) -> AppResult<AppSettings> {
    Ok(store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone())
}
#[tauri::command]
fn save_settings(settings: AppSettings, store: State<'_, Store>) -> AppResult<AppSettings> {
    validate_settings(&settings)?;
    *store.settings.write().map_err(|e| AppError::Storage(e.to_string()))? = settings.clone();
    store.persist_settings()?;
    Ok(settings)
}
#[tauri::command]
async fn create_story(prompt: String, store: State<'_, Store>) -> AppResult<Story> {
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    if settings.llm_model.trim().is_empty() { return Err(AppError::Llm("configure an LLM model in settings".into())); }
    let system = r#"
You are Raphael Story Architect. Convert a user's natural-language story request into a structured story bible and opening chapter.
Return ONLY valid JSON matching the requested schema. Do not wrap it in markdown.
Extract explicit facts faithfully. You may invent missing details, but make them internally consistent and suitable for future visual generation.
The story must include metadata: anime/show source information if present, genre, tags, demographic, content rating, tone. Characters require stable visual details: personality, appearance, clothing, motivations.
Relationships must use character names from the characters array. Chapter 1 must establish canon without contradicting the request.
"#;
    let schema_hint = r#"
JSON shape:
{"title":"","metadata":{"genre":[],"tags":[],"demographic":"","content_rating":"","tone":[],"source_type":"","source_title":"","inspirations":[]},"premise":"","central_conflict":"","themes":[],"world_setting":"","world_rules":[],"locations":[],"characters":[{"name":"","role":"","personality":[],"appearance":"","clothing":"","motivations":[]}],"relationships":[{"source":"","target":"","relation_type":"","description":""}],"open_threads":[],"introduction":"","chapter":{"title":"","summary":"","text":"","events":[],"continuity_updates":[],"character_state_updates":[],"relationship_updates":[],"open_threads":[]}}
"#;
    let raw = chat(&settings, system, &format!("User story request:
{}

{}", prompt.trim(), schema_hint)).await?;
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
        user_directive: None, events: parsed.chapter.events, continuity_updates: parsed.chapter.continuity_updates,
        scenes: Vec::new(), created_at: now(),
    };
    let created = now();
    let story = Story {
        id: story_id, title: parsed.title, source_prompt: prompt,
        metadata: parsed.metadata, introduction: parsed.introduction,
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
    let system = r#"
You are Raphael Continuity Writer. Generate the next chapter of an existing story.
The STORY BIBLE is authoritative canon. Preserve established character identity, age, appearance, clothing, personalities, relationships, world rules and chronology unless the user's directive explicitly changes them through story events.
The optional USER DIRECTIVE is a request for this chapter only. It can add characters, alter tone, emphasize a relationship, request an event, skip time, or constrain what must not happen. Satisfy it where possible without breaking prior canon.
Return ONLY valid JSON. Do not include markdown.
"#;
    let bible = serde_json::to_string(&story.bible).map_err(|e| AppError::ModelResponse(e.to_string()))?;
    let directive = if user_prompt.trim().is_empty() { "(none — continue naturally)" } else { user_prompt.trim() };
    let schema = r#"{"title":"","summary":"","text":"","events":[],"continuity_updates":[],"new_characters":[{"name":"","role":"","personality":[],"appearance":"","clothing":"","motivations":[]}],"character_state_updates":[{"character_id":"","current_state":"","clothing":""}],"relationship_updates":[{"source_character":"","target_character":"","relation_type":"","description":""}],"open_threads":[]}"#;
    let user = format!("CHAPTER NUMBER: {}

STORY BIBLE:
{}

PREVIOUS CHAPTER:
{}

USER DIRECTIVE:
{}

Return this JSON shape:
{}", next_number, bible, previous, directive, schema);
    let raw = chat(&settings, system, &user).await?;
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
    let system = r#"
You are Raphael Scene Director. Split a chapter into imageable manga/anime panels.
A scene must represent ONE coherent visual story beat that can fit into a single image. Do not split by sentence mechanically and do not combine visually incompatible moments.
Preserve chronological order, character identity, outfit state, location and dialogue. Return short visual specifications optimized for an image builder.
Return ONLY valid JSON.
"#;
    let schema = r#"{"scenes":[{"description":"","location":"","time":"","characters":[],"action":"","composition":"","dialogue":""}]}"#;
    let user = format!("CHAPTER {}
TITLE: {}

TEXT:
{}

CANONICAL VISUAL CHARACTERS:
{}

Return:
{}", chapter.number, chapter.title, chapter.text, visual_characters, schema);
    let raw = chat(&settings, system, &user).await?;
    let parsed: SceneResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    let scenes = parsed.scenes.into_iter().enumerate().map(|(index, scene)| Scene {
        id: format!("{}-scene-{:03}", chapter.number, index + 1), order: index + 1, description: scene.description,
        location: scene.location, time: scene.time, characters: scene.characters, action: scene.action,
        composition: scene.composition, dialogue: scene.dialogue, positive_prompt: String::new(),
        negative_prompt: String::new(), image_status: "not_ready".into(), image_url: None, comfy_prompt_id: None,
    }).collect::<Vec<_>>();
    story.chapters[chapter_index].scenes = scenes.clone(); story.updated_at = now(); write_story(&store, story)?;
    Ok(SceneExtractionResult { chapter_number, scenes })
}
#[tauri::command]
async fn build_scene_prompt(story_id: String, chapter_number: usize, scene_id: String, store: State<'_, Store>) -> AppResult<Story> {
    let mut story = require_story(&store, &story_id)?;
    let settings = store.settings.read().map_err(|e| AppError::Storage(e.to_string()))?.clone();
    let chapter = story.chapters.iter().find(|c| c.number == chapter_number).ok_or_else(|| AppError::ModelResponse("chapter not found".into()))?;
    let scene = chapter.scenes.iter().find(|s| s.id == scene_id).ok_or_else(|| AppError::ModelResponse("scene not found".into()))?;
    let characters = story.bible.characters.iter().filter(|c| {
        scene.characters.iter().any(|name| name.trim() == c.id || normalize_name(name) == normalize_name(&c.name))
    }).map(|c| format!("{} — appearance: {}; clothing: {}; personality: {:?}", c.name, c.appearance, c.clothing, c.personality)).collect::<Vec<_>>().join("
");
    let system = r#"
You are Raphael Image Builder prompt director. Convert one scene into a positive and negative image-generation prompt suitable for an anime/manga diffusion workflow.
Positive prompt should describe composition, camera, subjects, appearance, clothing, action, setting, lighting, mood and clean visual style. Negative prompt should suppress identity drift, extra limbs, malformed hands, text artifacts, low quality and scene contradictions.
Do not invent a different character appearance. Return ONLY valid JSON.
"#;
    let schema = r#"{"positive_prompt":"","negative_prompt":""}"#;
    let user = format!("STORY: {}
SCENE: {}
LOCATION: {}
TIME: {}
ACTION: {}
COMPOSITION: {}
DIALOGUE: {}
CHARACTERS:
{}

Return:
{}", story.title, scene.description, scene.location, scene.time, scene.action, scene.composition, scene.dialogue, characters, schema);
    let raw = chat(&settings, system, &user).await?;
    let parsed: ImagePromptResponse = serde_json::from_str(clean_json(&raw)).map_err(|e| AppError::ModelResponse(format!("{}; raw model output starts with: {}", e, &raw.chars().take(300).collect::<String>())))?;
    let chapter_mut = story.chapters.iter_mut().find(|c| c.number == chapter_number).unwrap();
    let scene_mut = chapter_mut.scenes.iter_mut().find(|s| s.id == scene_id).unwrap();
    scene_mut.positive_prompt = parsed.positive_prompt;
    scene_mut.negative_prompt = parsed.negative_prompt;
    scene_mut.image_status = "prompt_ready".into();
    story.updated_at = now(); write_story(&store, story)
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
    let mut workflow: Value = serde_json::from_str(&settings.comfyui_workflow_json)
        .map_err(|e| AppError::ComfyUi(format!("workflow JSON is invalid: {e}")))?;
    let seed = (Uuid::new_v4().as_u128() & u64::MAX as u128) as u64;
    replace_workflow_placeholders(&mut workflow, &[
        ("{{POSITIVE_PROMPT}}", Value::String(scene.positive_prompt.clone())),
        ("{{NEGATIVE_PROMPT}}", Value::String(scene.negative_prompt.clone())),
        ("{{SEED}}", Value::Number(serde_json::Number::from(seed))),
        ("{{STORY_ID}}", Value::String(story.id.clone())),
        ("{{SCENE_ID}}", Value::String(scene.id.clone())),
    ]);
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
    let response = client.post(url).json(&json!({
        "prompt": workflow,
        "client_id": format!("raphael-story-{}", story.id),
    })).send().await.map_err(|e| AppError::ComfyUi(e.to_string()))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| AppError::ComfyUi(e.to_string()))?;
    let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({ "error": body }));
    if !status.is_success() {
        return Err(AppError::ComfyUi(
            value.get("error").and_then(Value::as_str).unwrap_or("ComfyUI rejected the workflow").to_string()
        ));
    }
    let prompt_id = value.get("prompt_id").and_then(Value::as_str).ok_or_else(|| AppError::ComfyUi("ComfyUI did not return a prompt_id".into()))?.to_string();
    let chapter_mut = story.chapters.iter_mut().find(|c| c.number == chapter_number).unwrap();
    let scene_mut = chapter_mut.scenes.iter_mut().find(|s| s.id == scene_id).unwrap();
    scene_mut.comfy_prompt_id = Some(prompt_id);
    scene_mut.image_status = "queued".into();
    story.updated_at = now();
    write_story(&store, story)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let store = Store::new(app.handle()).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })?;
            app.manage(store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_app_state, get_story, get_settings, save_settings, create_story, generate_next_chapter, extract_scenes, build_scene_prompt, queue_scene_image])
        .run(tauri::generate_context!())
        .expect("error while running Raphael Story Generator");
}
