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
    pub name: String,
    pub file_name: String,
    pub activation_prompts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryLoraCandidate {
    pub id: String,
    pub name: String,
    pub model_type: String,
    pub description: Option<String>,
    pub base_model: Option<String>,
    pub creator: Option<String>,
    pub tags: Vec<String>,
    pub activation_prompts: Vec<String>,
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
    source_manifest: Option<PathBuf>,
    http: reqwest::Client,
    starting: AtomicBool,
    last_error: RwLock<Option<String>>,
}

impl RegistryState {
    pub fn new(app: &AppHandle) -> Self {
        let port = env::var("RAPHAEL_REGISTRY_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let data_dir = registry_data_dir();
        let (executable, source_manifest) = locate_registry(app);

        Self {
            inner: Arc::new(RegistryInner {
                base_url: format!("http://127.0.0.1:{port}"),
                token_path: data_dir.join("registry.token"),
                executable,
                source_manifest,
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

    async fn status(&self) -> RegistryStatusDto {
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

    async fn spawn_registry(&self) -> Result<(), String> {
        if let Some(executable) = &self.inner.executable {
            let mut command = std::process::Command::new(executable);
            command.arg("server");
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

        RegistryClient::from_token_file(&self.inner.base_url, &self.inner.token_path)
            .map_err(|error| AppError::Registry(format!("Registry authentication is unavailable: {error}")))
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
    pub async fn compatible_loras(&self, checkpoint_id: &str) -> AppResult<Vec<RegistryLoraCandidate>> {
        let client = self.client().await?;
        let models = client
            .compatible(checkpoint_id, Some(ModelType::Lora))
            .await
            .map_err(|error| AppError::Registry(format!("failed to load compatible LoRAs: {error}")))?;

        let mut candidates = Vec::with_capacity(models.len());
        for model in models {
            let tags = client.tags(&model.id)
                .await
                .map_err(|error| AppError::Registry(format!("failed to load tags for {}: {error}", model.name)))?;
            let activation_prompts = client
                .versions(&model.id)
                .await
                .map_err(|error| AppError::Registry(format!("failed to load versions for {}: {error}", model.name)))?
                .into_iter()
                .max_by_key(|version| version.updated_at)
                .map(|version| version.activation_prompts)
                .unwrap_or_default();

            candidates.push(RegistryLoraCandidate {
                id: model.id,
                name: model.name,
                model_type: model.model_type.to_string(),
                description: model.description.map(|value| value.chars().take(600).collect()),
                base_model: model.base_model,
                creator: model.creator,
                tags,
                activation_prompts,
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
        let file = files
            .into_iter()
            .filter(|file| matches!(file.status, registry_core::FileStatus::Available))
            .max_by_key(|file| file.updated_at)
            .ok_or_else(|| AppError::Registry(format!("model '{}' has no available file", model.name)))?;

        let versions = client
            .versions(model_id)
            .await
            .map_err(|error| AppError::Registry(format!("failed to load versions for {}: {error}", model.name)))?;
        let activation_prompts = versions
            .into_iter()
            .max_by_key(|version| version.updated_at)
            .map(|version| version.activation_prompts)
            .unwrap_or_default();

        Ok(RegistryModelArtifact {
            id: model.id,
            name: model.name,
            file_name: file.filename,
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

    ProjectDirs::from("com", "Raphael", "ModelRegistry")
        .map(|projects| projects.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".raphael-model-registry"))
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
    Ok(state.inner.ensure_running().await)
}

#[tauri::command]
pub async fn get_registry_status(state: State<'_, RegistryState>) -> AppResult<RegistryStatusDto> {
    Ok(state.inner.status().await)
}

#[tauri::command]
pub async fn get_registry_models(state: State<'_, RegistryState>) -> AppResult<RegistryCatalogDto> {
    state.inner.catalog().await
}
