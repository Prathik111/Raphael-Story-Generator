use crate::{AppError, AppResult};
use directories::ProjectDirs;
use registry_client::RegistryClient;
use registry_core::{ModelSearch, ModelType};
use serde::Serialize;
use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::time::{sleep, timeout};

const DEFAULT_PORT: u16 = 43217;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryStatus {
    Off,
    Starting,
    On,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryStatusDto {
    pub status: RegistryStatus,
    pub url: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryModelDto {
    pub id: String,
    pub name: String,
    pub model_type: String,
    pub base_model: Option<String>,
    pub creator: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone)]
pub struct RegistryModelArtifact {
    pub id: String,
    pub version_id: Option<String>,
    pub file_id: String,
    pub file_name: String,
    pub path: String,
    pub relative_path: Option<String>,
    pub sha256: Option<String>,
    pub activation_prompts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryLoraCandidate {
    pub id: String,
    pub name: String,
    pub model_type: String,
    pub description: Option<String>,
    pub base_model: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryCatalogDto {
    pub checkpoints: Vec<RegistryModelDto>,
    pub checkpoint_total: i64,
    pub loras: Vec<RegistryModelDto>,
    pub lora_total: i64,
}

#[derive(Clone)]
pub struct RegistryState {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    base_url: String,
    token_path: PathBuf,
    executable: Option<PathBuf>,
    #[allow(dead_code)]
    source_manifest: Option<PathBuf>,
    remote_configured: bool,
    http: reqwest::Client,
    starting: AtomicBool,
    last_error: RwLock<Option<String>>,
}

fn add_token_candidate(
    candidates: &mut Vec<(String, String, Option<PathBuf>)>,
    token: String,
    source: String,
    path: Option<PathBuf>,
) {
    let token = token.trim().to_string();
    if token.is_empty() || candidates.iter().any(|(existing, _, _)| existing == &token) {
        return;
    }
    candidates.push((token, source, path));
}

fn add_token_file_candidate(
    candidates: &mut Vec<(String, String, Option<PathBuf>)>,
    path: PathBuf,
) {
    if let Ok(contents) = std::fs::read_to_string(&path) {
        let source = format!("token file {}", path.display());
        add_token_candidate(candidates, contents, source, Some(path));
    }
}

impl RegistryState {
    pub fn new(app: &AppHandle) -> Self {
        let port = env::var("RAPHAEL_REGISTRY_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let data_dir = registry_data_dir();
        let token_path = env::var_os("RAPHAEL_REGISTRY_TOKEN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("registry.token"));
        // Match Raphael Image Generator: an explicit URL means the Registry is
        // managed elsewhere, so never launch a second local Registry instance.
        let configured_url = env::var("RAPHAEL_REGISTRY_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        let remote_configured = configured_url.is_some();
        let base_url = configured_url
            .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
        let (executable, source_manifest) = if remote_configured {
            (None, None)
        } else {
            locate_registry(app)
        };

        Self {
            inner: Arc::new(RegistryInner {
                base_url,
                token_path,
                executable,
                source_manifest,
                remote_configured,
                http: reqwest::Client::builder()
                    .connect_timeout(Duration::from_millis(800))
                    .timeout(Duration::from_secs(2))
                    .build()
                    .expect("failed to create Registry health client"),
                starting: AtomicBool::new(false),
                last_error: RwLock::new(None),
            }),
        }
    }

    fn token_candidates(&self) -> Vec<(String, String, Option<PathBuf>)> {
        let mut candidates = Vec::new();

        // Do not send tokens discovered from local Raphael installations to an
        // explicitly configured remote Registry.
        if self.inner.remote_configured {
            if let Ok(value) = env::var("RAPHAEL_REGISTRY_AUTH_TOKEN") {
                add_token_candidate(&mut candidates, value, "RAPHAEL_REGISTRY_AUTH_TOKEN".into(), None);
            }
            add_token_file_candidate(&mut candidates, self.inner.token_path.clone());
            return candidates;
        }

        // For a local Registry, the persisted token file is authoritative. The
        // environment may contain a stale token inherited from an old launch.
        add_token_file_candidate(&mut candidates, self.inner.token_path.clone());
        // Older Registry builds used Roaming AppData, while current Raphael
        // clients use Local AppData. Probe both known local locations so the
        // correct token can be recovered without deleting model data.
        #[cfg(windows)]
        for variable in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(root) = env::var_os(variable) {
                let data_dir = PathBuf::from(root)
                    .join("Raphael")
                    .join("ModelRegistry")
                    .join("data");
                add_token_file_candidate(&mut candidates, data_dir.join("registry.token"));

                // The active Registry writes its token file path to registry.lock.
                if let Ok(contents) = std::fs::read_to_string(data_dir.join("registry.lock")) {
                    if let Ok(info) = serde_json::from_str::<serde_json::Value>(&contents) {
                        if let Some(path) = info.get("token_file").and_then(|value| value.as_str()) {
                            add_token_file_candidate(&mut candidates, PathBuf::from(path));
                        }
                    }
                }
            }
        }

        // Environment variables can linger across upgrades, so only use the
        // environment token after all known on-disk local token files.
        if let Ok(value) = env::var("RAPHAEL_REGISTRY_AUTH_TOKEN") {
            add_token_candidate(&mut candidates, value, "RAPHAEL_REGISTRY_AUTH_TOKEN".into(), None);
        }

        candidates
    }

    fn read_token(&self) -> Result<String, String> {
        self.token_candidates()
            .into_iter()
            .next()
            .map(|(token, _, _)| token)
            .ok_or_else(|| format!(
                "could not read a non-empty Raphael Model Registry token from {} or its configured environment variable",
                self.inner.token_path.display()
            ))
    }

    async fn authorized_token(&self) -> Result<String, String> {
        let candidates = self.token_candidates();
        if candidates.is_empty() {
            return Err(format!(
                "No Registry token was found. Checked {} and RAPHAEL_REGISTRY_AUTH_TOKEN.",
                self.inner.token_path.display()
            ));
        }

        let status_url = format!("{}/api/v1/status", self.inner.base_url);
        let mut checked_sources = Vec::new();
        let mut transport_error = None;

        for (token, source, _) in &candidates {
            let response = self.inner.http
                .get(&status_url)
                .bearer_auth(token.trim())
                .header("x-raphael-actor", "story-generator")
                .send()
                .await;

            match response {
                Ok(response)
                    if response.status() == reqwest::StatusCode::UNAUTHORIZED
                        || response.status() == reqwest::StatusCode::FORBIDDEN =>
                {
                    checked_sources.push(source.clone());
                }
                Ok(_response) => {
                    // Align the current local token file with whichever credential
                    // the already-running Registry actually accepts.
                    if !self.inner.remote_configured {
                        let current = std::fs::read_to_string(&self.inner.token_path)
                            .unwrap_or_default();
                        if current.trim() != token {
                            if let Some(parent) = self.inner.token_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(
                                &self.inner.token_path,
                                format!("{token}\n"),
                            );
                        }
                    }
                    return Ok(token.clone());
                }
                Err(error) => {
                    transport_error = Some(error.to_string());
                    checked_sources.push(source.clone());
                }
            }
        }

        if let Some(error) = transport_error {
            return Err(format!(
                "Could not verify Registry authentication at {}. Checked token sources: {}. Last connection error: {}",
                self.inner.base_url,
                checked_sources.join(", "),
                error
            ));
        }

        Err(format!(
            "Registry at {} is responding but rejected every available bearer token. Checked: {}. The Registry and Raphael still have different tokens; close Raphael Model Manager and other Registry clients, restart the Registry, then reopen Story Generator. No token values were logged.",
            self.inner.base_url,
            checked_sources.join(", ")
        ))
    }

    async fn health(&self) -> bool {
        self.inner
            .http
            .get(format!("{}/health", self.inner.base_url))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    fn set_error(&self, value: Option<String>) {
        if let Ok(mut error) = self.inner.last_error.write() {
            *error = value;
        }
    }

    fn error(&self) -> Option<String> {
        self.inner.last_error.read().ok().and_then(|value| value.clone())
    }

    pub async fn status(&self) -> RegistryStatusDto {
        let status = if self.inner.starting.load(Ordering::Acquire) {
            RegistryStatus::Starting
        } else if self.health().await {
            RegistryStatus::On
        } else {
            RegistryStatus::Off
        };

        RegistryStatusDto {
            status,
            url: self.inner.base_url.clone(),
            detail: match status {
                RegistryStatus::On => None,
                RegistryStatus::Starting => Some("Starting Raphael Model Registry…".into()),
                RegistryStatus::Off => self.error().or_else(|| Some("Registry is not running".into())),
            },
        }
    }

    async fn ensure_running(&self) -> RegistryStatusDto {
        if self.health().await {
            self.set_error(None);
            return self.status().await;
        }

        if self.inner.remote_configured {
            self.set_error(Some(format!(
                "Configured remote Registry at {} is unavailable; no local Registry was started",
                self.inner.base_url
            )));
            return self.status().await;
        }

        if self
            .inner
            .starting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let _ = timeout(STARTUP_TIMEOUT, async {
                while !self.health().await {
                    sleep(Duration::from_millis(150)).await;
                }
            })
            .await;
            return self.status().await;
        }

        self.set_error(None);
        let start_result = self.spawn_registry().await;
        if let Err(error) = start_result {
            self.set_error(Some(error));
            self.inner.starting.store(false, Ordering::Release);
            return self.status().await;
        }

        let became_ready = timeout(STARTUP_TIMEOUT, async {
            loop {
                if self.health().await {
                    break true;
                }
                sleep(Duration::from_millis(150)).await;
            }
        })
        .await
        .unwrap_or(false);

        self.inner.starting.store(false, Ordering::Release);

        if became_ready {
            self.set_error(None);
        } else {
            self.set_error(Some(format!(
                "Registry process started, but {} did not respond to /health within {} seconds",
                self.inner.base_url,
                STARTUP_TIMEOUT.as_secs()
            )));
        }

        self.status().await
    }

    fn configure_spawned_registry(&self, command: &mut std::process::Command) {
        // Keep the server's token/database directory aligned with the token
        // used by this client. Without this, the Registry's Windows default
        // may resolve to a different AppData directory and every API request
        // can fail with HTTP 401 even when both processes are otherwise healthy.
        if let Some(data_dir) = self.inner.token_path.parent() {
            command.env("RAPHAEL_REGISTRY_DATA_DIR", data_dir);
        }
        if let Ok(token) = self.read_token() {
            command.env("RAPHAEL_REGISTRY_AUTH_TOKEN", token);
        }
    }

    async fn spawn_registry(&self) -> Result<(), String> {
        if let Some(executable) = &self.inner.executable {
            let mut command = std::process::Command::new(executable);
            command.arg("server");
            self.configure_spawned_registry(&mut command);
            spawn_hidden(command)
                .map_err(|error| format!("failed to start {}: {error}", executable.display()))?;
            return Ok(());
        }

        #[cfg(debug_assertions)]
        if let Some(manifest) = &self.inner.source_manifest {
            let mut command = std::process::Command::new("cargo");
            command.args([
                "run",
                "--manifest-path",
                &manifest.to_string_lossy(),
                "-p",
                "registry-server",
                "--",
                "server",
            ]);
            self.configure_spawned_registry(&mut command);
            spawn_hidden(command)
                .map_err(|error| format!("failed to start Registry from Cargo: {error}"))?;
            return Ok(());
        }

        Err(
            "Raphael Model Registry executable was not found. Build the Registry or set RAPHAEL_REGISTRY_EXECUTABLE."
                .into(),
        )
    }

    pub async fn client(&self) -> AppResult<RegistryClient> {
        if !self.health().await {
            let status = self.ensure_running().await;
            if !matches!(status.status, RegistryStatus::On) {
                return Err(AppError::Registry(
                    status
                        .detail
                        .unwrap_or_else(|| "Registry is unavailable".into()),
                ));
            }
        }

        let token = self.authorized_token().await
            .map_err(|error| AppError::Registry(format!("Registry authentication is unavailable: {error}")))?;
        RegistryClient::new(self.inner.base_url.clone(), token)
            .map_err(|error| AppError::Registry(format!("Registry client could not be initialized: {error}")))
    }

    async fn catalog(&self) -> AppResult<RegistryCatalogDto> {
        let client = self.client().await?;
        let checkpoint_result = client
            .search(ModelSearch {
                model_type: Some(ModelType::Checkpoint),
                limit: 200,
                ..Default::default()
            })
            .await
            .map_err(|error| AppError::Registry(format!("failed to load checkpoints: {error}")))?;
        let lora_result = client
            .search(ModelSearch {
                model_type: Some(ModelType::Lora),
                limit: 200,
                ..Default::default()
            })
            .await
            .map_err(|error| AppError::Registry(format!("failed to load LoRAs: {error}")))?;

        Ok(RegistryCatalogDto {
            checkpoint_total: checkpoint_result.total,
            checkpoints: checkpoint_result.items.into_iter().map(RegistryModelDto::from_model).collect(),
            lora_total: lora_result.total,
            loras: lora_result.items.into_iter().map(RegistryModelDto::from_model).collect(),
        })
    }
}

impl RegistryState {
    pub async fn compatible_models(&self, model_id: &str, model_type: Option<ModelType>) -> AppResult<Vec<registry_core::Model>> {
        let mut url = format!("{}/api/v1/models/{}/compatibility", self.inner.base_url, model_id);
        if let Some(model_type) = model_type {
            url.push_str(&format!("?type={}", model_type));
        }

        let token = self.authorized_token().await
            .map_err(|error| AppError::Registry(format!("failed to authenticate with Registry: {error}")))?;
        let response = self.inner.http
            .get(url)
            .bearer_auth(token.trim())
            .header("x-raphael-actor", "story-generator")
            .send()
            .await
            .map_err(|error| AppError::Registry(format!("failed to query Registry compatibility: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Registry(format!("Registry compatibility query returned HTTP {status}: {body}")));
        }

        let result = response
            .json::<registry_core::CompatibilityResult>()
            .await
            .map_err(|error| AppError::Registry(format!("invalid Registry compatibility response: {error}")))?;

        Ok(result.candidates)
    }

    pub async fn compatible_loras(&self, checkpoint_id: &str) -> AppResult<Vec<RegistryLoraCandidate>> {
        let client = self.client().await?;
        let models = self
            .compatible_models(checkpoint_id, Some(ModelType::Lora))
            .await?;

        let mut candidates = Vec::with_capacity(models.len());
        for model in models {
            let tags = client.tags(&model.id)
                .await
                .map_err(|error| AppError::Registry(format!("failed to load tags for {}: {error}", model.name)))?;

            candidates.push(RegistryLoraCandidate {
                id: model.id,
                name: model.name,
                model_type: model.model_type.to_string(),
                description: model.description.map(|value| value.chars().take(600).collect()),
                base_model: model.base_model,
                tags,
            });
        }

        Ok(candidates)
    }

    pub async fn model_artifact(&self, model_id: &str) -> AppResult<RegistryModelArtifact> {
        let client = self.client().await?;
        let model = client
            .get(model_id)
            .await
            .map_err(|error| AppError::Registry(format!("failed to load model {model_id}: {error}")))?;
        let files = client
            .files(model_id)
            .await
            .map_err(|error| AppError::Registry(format!("failed to load files for {}: {error}", model.name)))?;

        let versions = client
            .versions(model_id)
            .await
            .map_err(|error| AppError::Registry(format!("failed to load versions for {}: {error}", model.name)))?;
        let latest_version = versions
            .into_iter()
            .max_by_key(|version| version.updated_at);

        let file = if let Some(version) = latest_version.as_ref() {
            files
                .iter()
                .filter(|file| {
                    matches!(file.status, registry_core::FileStatus::Available)
                        && file.version_id.as_deref() == Some(version.id.as_str())
                })
                .max_by_key(|file| file.updated_at)
                .cloned()
                .or_else(|| {
                    files
                        .iter()
                        .filter(|file| {
                            matches!(file.status, registry_core::FileStatus::Available)
                                && file.version_id.is_none()
                        })
                        .max_by_key(|file| file.updated_at)
                        .cloned()
                })
        } else {
            files
                .into_iter()
                .filter(|file| matches!(file.status, registry_core::FileStatus::Available))
                .max_by_key(|file| file.updated_at)
        }
        .ok_or_else(|| AppError::Registry(format!("model '{}' has no available file matching its latest version", model.name)))?;

        let activation_prompts = latest_version
            .as_ref()
            .map(|version| version.activation_prompts.clone())
            .unwrap_or_default();

        Ok(RegistryModelArtifact {
            id: model.id,
            version_id: latest_version.as_ref().map(|version| version.id.clone()),
            file_id: file.id,
            file_name: file.filename,
            path: file.path,
            relative_path: file.relative_path,
            sha256: file.sha256,
            activation_prompts,
        })
    }
}

impl RegistryModelDto {
    fn from_model(model: registry_core::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            model_type: model.model_type.to_string(),
            base_model: model.base_model,
            creator: model.creator,
            revision: model.revision,
        }
    }
}

fn registry_data_dir() -> PathBuf {
    if let Some(path) = env::var_os("RAPHAEL_REGISTRY_DATA_DIR") {
        return PathBuf::from(path);
    }

    // Keep token discovery aligned with Raphael Image Generator and the
    // standalone Registry on Windows.
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Raphael")
            .join("ModelRegistry")
            .join("data");
    }

    ProjectDirs::from("com", "Raphael", "ModelRegistry")
        .map(|projects| projects.data_dir().join("data"))
        .unwrap_or_else(|| PathBuf::from(".raphael-model-registry").join("data"))
}

fn locate_registry(app: &AppHandle) -> (Option<PathBuf>, Option<PathBuf>) {
    let mut executables = Vec::<PathBuf>::new();
    let mut manifests = Vec::<PathBuf>::new();

    if let Some(value) = env::var_os("RAPHAEL_REGISTRY_EXECUTABLE") {
        executables.push(PathBuf::from(value));
    }

    if let Some(value) = env::var_os("RAPHAEL_REGISTRY_DIR") {
        let root = PathBuf::from(value);
        executables.extend(registry_executable_candidates(&root));
        manifests.push(root.join("Cargo.toml"));
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        executables.push(resource_dir.join("registry").join(registry_executable_name()));
        executables.push(resource_dir.join(registry_executable_name()));
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            executables.push(parent.join(registry_executable_name()));
            executables.push(parent.join("registry").join(registry_executable_name()));
        }
    }

    #[cfg(debug_assertions)]
    {
        let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../raphael-model-registry");
        executables.extend(registry_executable_candidates(&source_root));
        manifests.push(source_root.join("Cargo.toml"));
    }

    let executable = executables.into_iter().find(|path| path.is_file());
    let source_manifest = manifests.into_iter().find(|path| path.is_file());
    (executable, source_manifest)
}

fn registry_executable_candidates(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("target").join("debug").join(registry_executable_name()),
        root.join("target").join("release").join(registry_executable_name()),
    ]
}

fn registry_executable_name() -> &'static str {
    if cfg!(windows) {
        "raphael-registry.exe"
    } else {
        "raphael-registry"
    }
}

fn spawn_hidden(mut command: std::process::Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}

#[tauri::command]
pub async fn ensure_registry(state: State<'_, RegistryState>) -> AppResult<RegistryStatusDto> {
    Ok(state.ensure_running().await)
}

#[tauri::command]
pub async fn get_registry_status(state: State<'_, RegistryState>) -> AppResult<RegistryStatusDto> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn get_registry_models(state: State<'_, RegistryState>) -> AppResult<RegistryCatalogDto> {
    state.catalog().await
}
