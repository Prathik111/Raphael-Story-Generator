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
    let url = if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") };
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

    let mut req = client.post(url).header(CONTENT_TYPE, "application/json").json(&body);
    if !settings.llm_api_key.trim().is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {}", settings.llm_api_key.trim()));
    }

    let response = match req.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            emit_pipeline(app, stage, "error", message.clone());
            emit_llm(app, LlmGenerationEvent {
                generation_id: generation_id.clone(), stage: stage.into(), status: "error".into(),
                model: settings.llm_model.trim().into(), system_prompt: None, user_prompt: None,
                delta: None, response: None, error: Some(message.clone()),
            });
            return Err(AppError::Llm(message));
        }
    };

    let status = response.status();
    let content_type = response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_else(|_| "request rejected".into());
        let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({"error": body}));
        let message = value.get("error").and_then(Value::as_str).unwrap_or("request rejected").to_string();
        emit_pipeline(app, stage, "error", message.clone());
        emit_llm(app, LlmGenerationEvent {
            generation_id: generation_id.clone(), stage: stage.into(), status: "error".into(),
            model: settings.llm_model.trim().into(), system_prompt: None, user_prompt: None,
            delta: None, response: None, error: Some(message.clone()),
        });
        return Err(AppError::Llm(message));
    }

    let mut full_response = String::new();

    if !content_type.contains("text/event-stream") {
        let value: Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                let message = format!("failed to parse LLM JSON response: {error}");
                emit_pipeline(app, stage, "error", message.clone());
                emit_llm_error(app, &generation_id, stage, settings.llm_model.trim(), Some(full_response.clone()), message.clone());
                return Err(AppError::Llm(message));
            }
        };
        let text = value
            .get("choices").and_then(|v| v.get(0))
            .and_then(|v| v.get("message"))
            .and_then(|v| v.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::ModelResponse("missing choices[0].message.content".into()));
        let text = match text {
            Ok(text) => text,
            Err(error) => {
                let message = error.to_string();
                emit_pipeline(app, stage, "error", message.clone());
                emit_llm_error(app, &generation_id, stage, settings.llm_model.trim(), Some(full_response.clone()), message.clone());
                return Err(error);
            }
        };
        full_response.push_str(text);
        if !text.is_empty() {
            emit_llm(app, LlmGenerationEvent {
                generation_id: generation_id.clone(), stage: stage.into(), status: "token".into(),
                model: settings.llm_model.trim().into(), system_prompt: None, user_prompt: None,
                delta: Some(text.into()), response: None, error: None,
            });
        }
    } else {
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::<u8>::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    let message = error.to_string();
                    emit_llm(app, LlmGenerationEvent {
                        generation_id: generation_id.clone(), stage: stage.into(), status: "error".into(),
                        model: settings.llm_model.trim().into(), system_prompt: None, user_prompt: None,
                        delta: None, response: Some(full_response.clone()), error: Some(message.clone()),
                    });
                    return Err(AppError::Llm(message));
                }
            };
            buffer.extend_from_slice(&chunk);

            while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=index).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let data = line.trim().strip_prefix("data:").map(str::trim);
                let Some(data) = data else { continue };
                if data == "[DONE]" { continue; }

                let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
                let Some(delta) = value
                    .get("choices").and_then(|v| v.get(0))
                    .and_then(|v| v.get("delta"))
                    .and_then(|v| v.get("content"))
                    .and_then(Value::as_str)
                else { continue };