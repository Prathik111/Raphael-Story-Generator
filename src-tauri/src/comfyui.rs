use crate::{emit_pipeline, require_story, write_story, AppError, AppResult, Store};
use futures_util::StreamExt;
use reqwest::Client;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const HISTORY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ComfyGenerationEvent {
    pub story_id: String,
    pub chapter_number: usize,
    pub scene_id: String,
    pub prompt_id: String,
    pub status: String,
    pub progress: Option<f32>,
    pub current_node: Option<String>,
    pub current_step: Option<u64>,
    pub total_steps: Option<u64>,
    pub queue_remaining: Option<u64>,
    pub image_url: Option<String>,
    pub error: Option<String>,
    pub message: String,
}

fn emit_progress(app: &AppHandle, event: ComfyGenerationEvent) {
    let _ = app.emit("raphael:comfyui", event);
}

fn emit(app: &AppHandle, story_id: &str, chapter_number: usize, scene_id: &str, prompt_id: &str, status: &str, progress: Option<f32>, node: Option<String>, step: Option<u64>, total: Option<u64>, queue_remaining: Option<u64>, image_url: Option<String>, error: Option<String>, message: impl Into<String>) {
    emit_progress(
        app,
        ComfyGenerationEvent {
            story_id: story_id.to_string(),
            chapter_number,
            scene_id: scene_id.to_string(),
            prompt_id: prompt_id.to_string(),
            status: status.to_string(),
            progress,
            current_node: node,
            current_step: step,
            total_steps: total,
            queue_remaining,
            image_url,
            error,
            message: message.into(),
        },
    );
}

pub async fn check_api(raw: &str) -> AppResult<()> {
    let base = base_url(raw)?;
    let client = http_client()?;
    let url = base.join("system_stats").map_err(|error| AppError::ComfyUi(format!("invalid ComfyUI system stats URL: {error}")))?;
    let response = client.get(url).send().await.map_err(|error| AppError::ComfyUi(format!("ComfyUI health check failed: {error}")))?;
    let status = response.status();
    let body = response.text().await.map_err(|error| AppError::ComfyUi(format!("failed to read ComfyUI health response: {error}")))?;
    if !status.is_success() {
        return Err(AppError::ComfyUi(format!("ComfyUI /system_stats returned HTTP {}: {}", status, body.chars().take(300).collect::<String>())));
    }
    let value: Value = serde_json::from_str(&body).map_err(|error| AppError::ComfyUi(format!("ComfyUI health response was not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(AppError::ComfyUi("ComfyUI /system_stats did not return a JSON object".into()));
    }
    Ok(())
}
fn http_client() -> AppResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| AppError::ComfyUi(format!("failed to create ComfyUI client: {e}")))
}

fn base_url(raw: &str) -> AppResult<Url> {
    let mut url = Url::parse(raw.trim())
        .map_err(|e| AppError::ComfyUi(format!("invalid ComfyUI URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::ComfyUi("ComfyUI URL must start with http:// or https://".into()));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url)
}

fn ws_url(raw: &str, client_id: &str) -> AppResult<Url> {
    let mut url = base_url(raw)?;
    let scheme = if url.scheme() == "http" { "ws" } else { "wss" };
    url.set_scheme(scheme)
        .map_err(|_| AppError::ComfyUi("failed to convert ComfyUI URL to WebSocket URL".into()))?;
    url.set_path("/ws");
    url.query_pairs_mut().append_pair("clientId", client_id);
    Ok(url)
}

fn resumable_prompt_id(scene: &crate::Scene) -> Option<String> {
    (scene.image_status == "queued")
        .then(|| scene.comfy_prompt_id.clone())
        .flatten()
}

async fn fetch_history(client: &Client, base: &Url, prompt_id: &str) -> AppResult<Option<Value>> {
    let url = base.join(&format!("history/{prompt_id}"))
        .map_err(|e| AppError::ComfyUi(format!("invalid ComfyUI history URL: {e}")))?;
    let response = client.get(url).send().await
        .map_err(|e| AppError::ComfyUi(format!("failed to poll ComfyUI history: {e}")))?;

    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AppError::ComfyUi(format!("ComfyUI history returned HTTP {}", response.status())));
    }

    let value = response.json::<Value>().await
        .map_err(|e| AppError::ComfyUi(format!("invalid ComfyUI history response: {e}")))?;
    Ok(value.get(prompt_id).cloned())
}

async fn fetch_queue_remaining(client: &Client, base: &Url) -> Option<u64> {
    let value = client.get(base.join("queue").ok()?).send().await.ok()?.json::<Value>().await.ok()?;
    let running = value.get("queue_running").and_then(Value::as_array).map_or(0, |items| items.len() as u64);
    let pending = value.get("queue_pending").and_then(Value::as_array).map_or(0, |items| items.len() as u64);
    Some(running + pending)
}

fn history_error(history: &Value) -> Option<String> {
    let status = history.get("status")?;
    let status_str = status.get("status_str").and_then(Value::as_str).unwrap_or_default();
    if matches!(status_str.to_ascii_lowercase().as_str(), "error" | "execution_error" | "interrupted" | "execution_interrupted") {
        return Some(status.get("messages").map(|value| value.to_string()).unwrap_or_else(|| format!("ComfyUI execution ended with status '{status_str}'")));
    }
    None
}

#[derive(Clone)]
struct ImageOutput {
    filename: String,
    subfolder: String,
    folder_type: String,
}

fn image_outputs(history: &Value) -> Vec<ImageOutput> {
    let Some(outputs) = history.get("outputs").and_then(Value::as_object) else { return Vec::new(); };
    outputs.values().filter_map(Value::as_object)
        .flat_map(|node| node.get("images"))
        .filter_map(Value::as_array)
        .flat_map(|images| images.iter())
        .filter_map(|image| Some(ImageOutput {
            filename: image.get("filename")?.as_str()?.to_string(),
            subfolder: image.get("subfolder").and_then(Value::as_str).unwrap_or_default().to_string(),
            folder_type: image.get("type").and_then(Value::as_str).unwrap_or("output").to_string(),
        }))
        .collect()
}

fn safe_scene_file_name(story_id: &str, chapter_number: usize, scene_id: &str) -> String {
    let safe_scene = scene_id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' })
        .collect::<String>();
    format!("generated/{story_id}/chapter-{chapter_number}-{safe_scene}.image")
}

fn detect_mime(content_type: Option<&str>, filename: &str) -> String {
    if let Some(mime) = content_type {
        let lower = mime.to_ascii_lowercase();
        if lower.contains("jpeg") { return "image/jpeg".into(); }
        if lower.contains("webp") { return "image/webp".into(); }
        if lower.contains("gif") { return "image/gif".into(); }
    }
    match Path::new(filename).extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg".into(),
        "webp" => "image/webp".into(),
        "gif" => "image/gif".into(),
        _ => "image/png".into(),
    }
}

async fn download_image(client: &Client, base: &Url, output: &ImageOutput) -> AppResult<(Vec<u8>, String, String)> {
    let mut url = base.join("view").map_err(|e| AppError::ComfyUi(format!("invalid ComfyUI image URL: {e}")))?;
    url.query_pairs_mut()
        .append_pair("filename", &output.filename)
        .append_pair("subfolder", &output.subfolder)
        .append_pair("type", &output.folder_type);

    let response = client.get(url.clone()).send().await
        .map_err(|e| AppError::ComfyUi(format!("failed to download generated image: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::ComfyUi(format!("ComfyUI image endpoint returned HTTP {}", response.status())));
    }
    if response.content_length().is_some_and(|length| length > MAX_IMAGE_BYTES) {
        return Err(AppError::ComfyUi("generated image exceeds the 25 MB safety limit".into()));
    }

    let mime = detect_mime(response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()), &output.filename);
    let bytes = response.bytes().await.map_err(|e| AppError::ComfyUi(format!("failed to read generated image: {e}")))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(AppError::ComfyUi("generated image exceeded the 25 MB safety limit".into()));
    }
    Ok((bytes.to_vec(), mime, url.to_string()))
}

fn save_image(app: &AppHandle, story_id: &str, chapter_number: usize, scene_id: &str, bytes: &[u8]) -> AppResult<String> {
    let store = app.state::<Store>();
    let relative = safe_scene_file_name(story_id, chapter_number, scene_id);
    let absolute = store.root.join(&relative);
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::ComfyUi(format!("failed to create image directory: {e}")))?;
    }
    let temporary = PathBuf::from(format!("{}.tmp", absolute.display()));
    fs::write(&temporary, bytes).map_err(|e| AppError::ComfyUi(format!("failed to save generated image: {e}")))?;
    if absolute.exists() {
        let _ = fs::remove_file(&absolute);
    }
    fs::rename(&temporary, &absolute).map_err(|e| AppError::ComfyUi(format!("failed to finalize generated image: {e}")))?;
    Ok(relative)
}

fn set_scene_status(app: &AppHandle, story_id: &str, chapter_number: usize, scene_id: &str, status: &str, error: Option<String>) -> AppResult<()> {
    let store = app.state::<Store>();
    let mut story = require_story(&store, story_id)?;
    let chapter = story.chapters.iter_mut().find(|chapter| chapter.number == chapter_number).ok_or_else(|| AppError::ComfyUi("chapter disappeared while tracking generation".into()))?;
    let scene = chapter.scenes.iter_mut().find(|scene| scene.id == scene_id).ok_or_else(|| AppError::ComfyUi("scene disappeared while tracking generation".into()))?;
    scene.image_status = status.into();
    scene.image_error = error;
    story.updated_at = crate::now();
    write_story(&store, story)?;
    Ok(())
}

fn set_scene_generated(app: &AppHandle, story_id: &str, chapter_number: usize, scene_id: &str, image_path: String, image_mime: String, image_url: String) -> AppResult<()> {
    let store = app.state::<Store>();
    let mut story = require_story(&store, story_id)?;
    let chapter = story.chapters.iter_mut().find(|chapter| chapter.number == chapter_number).ok_or_else(|| AppError::ComfyUi("chapter disappeared while saving generated image".into()))?;
    let scene = chapter.scenes.iter_mut().find(|scene| scene.id == scene_id).ok_or_else(|| AppError::ComfyUi("scene disappeared while saving generated image".into()))?;
    scene.image_status = "generated".into();
    scene.image_error = None;
    scene.image_path = Some(image_path);
    scene.image_mime = Some(image_mime);
    scene.image_url = Some(image_url);
    story.updated_at = crate::now();
    write_story(&store, story)?;
    Ok(())
}

async fn finalize_from_history(app: &AppHandle, client: &Client, base: &Url, story_id: &str, chapter_number: usize, scene_id: &str, prompt_id: &str, history: &Value) -> AppResult<bool> {
    if let Some(error) = history_error(history) {
        set_scene_status(app, story_id, chapter_number, scene_id, "failed", Some(error.clone()))?;
        emit(app, story_id, chapter_number, scene_id, prompt_id, "failed", None, None, None, None, None, None, Some(error.clone()), "ComfyUI execution failed");
        emit_pipeline(app, "comfyui", "error", format!("Scene {scene_id} failed: {error}"));
        return Ok(true);
    }

    let outputs = image_outputs(history);
    if outputs.is_empty() {
        return Ok(false);
    }

    let output = outputs.first().ok_or_else(|| AppError::ComfyUi("ComfyUI returned no image output".into()))?;
    let (bytes, mime, image_url) = download_image(client, base, output).await?;
    let relative = save_image(app, story_id, chapter_number, scene_id, &bytes)?;
    set_scene_generated(app, story_id, chapter_number, scene_id, relative, mime, image_url.clone())?;
    emit(app, story_id, chapter_number, scene_id, prompt_id, "completed", Some(100.0), None, None, None, None, Some(image_url), None, "Image generation completed");
    emit_pipeline(app, "comfyui", "completed", format!("Generated image for scene {scene_id}"));
    Ok(true)
}

async fn monitor_generation(app: AppHandle, base_url_raw: String, story_id: String, chapter_number: usize, scene_id: String, client_id: String, prompt_id: String) -> AppResult<()> {
    let base = base_url(&base_url_raw)?;
    let client = http_client()?;

    set_scene_status(&app, &story_id, chapter_number, &scene_id, "running", None)?;
    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", Some(0.0), None, None, None, None, None, None, "ComfyUI is executing the workflow");

    let socket_url = ws_url(&base_url_raw, &client_id)?;
    let socket_result = timeout(CONNECT_TIMEOUT, connect_async(socket_url.to_string())).await;
    let mut socket = match socket_result {
        Ok(Ok((socket, _))) => Some(socket),
        _ => {
            emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", None, None, None, None, None, None, None, "Live WebSocket progress unavailable; monitoring ComfyUI history");
            None
        }
    };

    let deadline = Instant::now() + GENERATION_TIMEOUT;
    let mut next_poll = Instant::now();

    while Instant::now() < deadline {
        if let Some(ws) = socket.as_mut() {
            tokio::select! {
                message = ws.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let text = text.to_string();
                            let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };
                            let data = value.get("data").cloned().unwrap_or(Value::Null);
                            let event_prompt_id = data.get("prompt_id").and_then(Value::as_str).unwrap_or_default();
                            if !event_prompt_id.is_empty() && event_prompt_id != prompt_id { continue; }

                            match value.get("type").and_then(Value::as_str).unwrap_or_default() {
                                "status" => {
                                    let remaining = data.get("status").and_then(|v| v.get("exec_info")).and_then(|v| v.get("queue_remaining")).and_then(Value::as_u64);
                                    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", None, None, None, None, remaining, None, None, "ComfyUI queue status");
                                }
                                "execution_start" => {
                                    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", Some(0.0), None, None, None, None, None, None, "Workflow execution started");
                                }
                                "executing" => {
                                    let node = data.get("node").and_then(Value::as_str).map(str::to_string);
                                    if node.is_none() {
                                        if let Some(history) = fetch_history(&client, &base, &prompt_id).await? {
                                            if finalize_from_history(&app, &client, &base, &story_id, chapter_number, &scene_id, &prompt_id, &history).await? { return Ok(()); }
                                        }
                                    } else {
                                        emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", None, node, None, None, None, None, None, "Executing ComfyUI node");
                                    }
                                }
                                "progress" => {
                                    let value = data.get("value").and_then(Value::as_u64).unwrap_or(0);
                                    let max = data.get("max").and_then(Value::as_u64).unwrap_or(0);
                                    let progress = (max > 0).then(|| (value as f32 / max as f32) * 100.0);
                                    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", progress, data.get("node").and_then(Value::as_str).map(str::to_string), Some(value), Some(max), None, None, None, "Sampling image");
                                }
                                "execution_error" | "execution_interrupted" => {
                                    let error = data.get("exception_message").or_else(|| data.get("message")).and_then(Value::as_str).unwrap_or("ComfyUI execution failed").to_string();
                                    set_scene_status(&app, &story_id, chapter_number, &scene_id, "failed", Some(error.clone()))?;
                                    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "failed", None, None, None, None, None, None, Some(error.clone()), "ComfyUI execution failed");
                                    emit_pipeline(&app, "comfyui", "error", format!("Scene {scene_id} failed: {error}"));
                                    return Ok(());
                                }
                                "execution_success" => {
                                    if let Some(history) = fetch_history(&client, &base, &prompt_id).await? {
                                        if finalize_from_history(&app, &client, &base, &story_id, chapter_number, &scene_id, &prompt_id, &history).await? { return Ok(()); }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => { socket = None; }
                        Some(Ok(_)) => {}
                        Some(Err(_)) => { socket = None; }
                    }
                }
                _ = sleep(Duration::from_millis(250)) => {}
            }
        } else {
            sleep(Duration::from_millis(500)).await;
        }

        if Instant::now() >= next_poll {
            if let Some(history) = fetch_history(&client, &base, &prompt_id).await? {
                if finalize_from_history(&app, &client, &base, &story_id, chapter_number, &scene_id, &prompt_id, &history).await? { return Ok(()); }
            }
            let remaining = fetch_queue_remaining(&client, &base).await;
            emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "running", None, None, None, None, remaining, None, None, if socket.is_some() { "Waiting for ComfyUI progress events" } else { "Polling ComfyUI generation status" });
            next_poll = Instant::now() + HISTORY_POLL_INTERVAL;
        }
    }

    let error = "ComfyUI generation exceeded the 60 minute monitoring timeout".to_string();
    set_scene_status(&app, &story_id, chapter_number, &scene_id, "failed", Some(error.clone()))?;
    emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "failed", None, None, None, None, None, None, Some(error.clone()), "ComfyUI generation timed out");
    emit_pipeline(&app, "comfyui", "error", format!("Scene {scene_id} exceeded the ComfyUI monitoring timeout"));
    Ok(())
}

pub fn spawn_generation_monitor(app: AppHandle, base_url: String, story_id: String, chapter_number: usize, scene_id: String, client_id: String, prompt_id: String) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = monitor_generation(app.clone(), base_url, story_id.clone(), chapter_number, scene_id.clone(), client_id, prompt_id.clone()).await {
            let message = error.to_string();
            let _ = set_scene_status(&app, &story_id, chapter_number, &scene_id, "failed", Some(message.clone()));
            emit(&app, &story_id, chapter_number, &scene_id, &prompt_id, "failed", None, None, None, None, None, None, Some(message.clone()), "ComfyUI generation failed");
            emit_pipeline(&app, "comfyui", "error", message);
        }
    });
}

pub fn resume_queued_generations(app: &AppHandle, base_url: &str) {
    let store = app.state::<Store>();
    let jobs = store.data.read().ok().map(|data| {
        data.stories.values().flat_map(|story| story.chapters.iter().flat_map(|chapter| chapter.scenes.iter().filter_map(|scene| {
            resumable_prompt_id(scene).map(|prompt_id| (story.id.clone(), chapter.number, scene.id.clone(), prompt_id))
        })).collect::<Vec<_>>()()
    }).unwrap_or_default();

    for (story_id, chapter_number, scene_id, prompt_id) in jobs {
        spawn_generation_monitor(
            app.clone(),
            base_url.to_string(),
            story_id,
            chapter_number,
            scene_id,
            format!("raphael-resume-{}", uuid::Uuid::new_v4()),
            prompt_id,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_scene_names_do_not_allow_path_separators() {
        assert_eq!(
            safe_scene_file_name("550e8400-e29b-41d4-a716-446655440000", 1, "../../secret"),
            "generated/550e8400-e29b-41d4-a716-446655440000/chapter-1-______secret.image"
        );
    }

    #[test]
    fn ws_urls_follow_http_scheme() {
        let url = ws_url("http://127.0.0.1:8188", "client").unwrap();
        assert_eq!(url.as_str(), "ws://127.0.0.1:8188/ws?clientId=client");
    }

    #[test]
    fn queued_scene_is_resumable_only_with_a_prompt_id() {
        let mut scene = crate::Scene::default();
        scene.image_status = "queued".into();

        assert_eq!(resumable_prompt_id(&scene), None);

        scene.comfy_prompt_id = Some("prompt-123".into());
        assert_eq!(resumable_prompt_id(&scene).as_deref(), Some("prompt-123"));

        scene.image_status = "generated".into();
        assert_eq!(resumable_prompt_id(&scene), None);
    }
}
