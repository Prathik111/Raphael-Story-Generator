use crate::{research, AppError, AppResult};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

const PROJECT_NAME: &str = "raphael-private-search";
const DEFAULT_SEARCH_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_PROXY_URL: &str = "socks5h://127.0.0.1:9050";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const SECRET_FILE: &str = ".searxng-secret";

fn compose_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let packaged = app
        .path()
        .resource_dir()
        .map_err(|error| AppError::WebResearch(format!("failed to locate app resources: {error}")))?
        .join("privacy-search");

    if packaged.is_dir() {
        return Ok(packaged);
    }

    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../privacy-search");

    if dev.is_dir() {
        return Ok(dev);
    }

    Err(AppError::WebResearch(
        "private web research gateway resources were not found".into(),
    ))
}

fn runtime_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let data = app
        .path()
        .app_local_data_dir()
        .map_err(|error| AppError::WebResearch(format!("failed to locate app local data: {error}")))?;
    let dir = data.join("privacy-search");
    fs::create_dir_all(&dir)
        .map_err(|error| AppError::WebResearch(format!("failed to create private search runtime directory: {error}")))?;
    Ok(dir)
}

fn ensure_secret(runtime: &Path) -> AppResult<String> {
    let path = runtime.join(SECRET_FILE);

    if path.is_file() {
        let secret = fs::read_to_string(&path)
            .map_err(|error| AppError::WebResearch(format!("failed to read private search secret: {error}")))?;
        let secret = secret.trim();
        if !secret.is_empty() {
            return Ok(secret.to_string());
        }
    }

    let secret = Uuid::new_v4().simple().to_string();
    fs::write(&path, format!("{secret}\n"))
        .map_err(|error| AppError::WebResearch(format!("failed to create private search secret: {error}")))?;
    Ok(secret)
}

fn configure_searxng_secret(runtime: &Path, secret: &str) -> AppResult<()> {
    let path = runtime.join("searxng").join("settings.yml");
    let content = fs::read_to_string(&path)
        .map_err(|error| AppError::WebResearch(format!("failed to read SearXNG settings: {error}")))?;
    let configured = content.replace("__RAPHAEL_SECRET__", secret);
    fs::write(&path, configured)
        .map_err(|error| AppError::WebResearch(format!("failed to configure SearXNG secret: {error}")))?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> AppResult<()> {
    fs::create_dir_all(target)
        .map_err(|error| AppError::WebResearch(format!("failed to create {}: {error}", target.display())))?;

    let entries = fs::read_dir(source)
        .map_err(|error| AppError::WebResearch(format!("failed to read {}: {error}", source.display())))?;

    for entry in entries {
        let entry = entry
            .map_err(|error| AppError::WebResearch(format!("failed to read private search resource: {error}")))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());

        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)
                .map_err(|error| AppError::WebResearch(format!(
                    "failed to install private search resource {}: {error}",
                    source_path.display()
                )))?;
        }
    }

    Ok(())
}

fn install_runtime_files(app: &AppHandle) -> AppResult<PathBuf> {
    let source = compose_dir(app)?;
    let target = runtime_dir(app)?;

    copy_dir_recursive(&source, &target)?;

    let secret = ensure_secret(&target)?;
    configure_searxng_secret(&target, &secret)?;
    Ok(target)
}

async fn docker_compose_up(dir: &Path) -> AppResult<()> {
    let dir = dir.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let output = Command::new("docker")
            .args(["compose", "--project-name", PROJECT_NAME, "up", "-d", "--build"])
            .current_dir(&dir)
            .output()
            .map_err(|error| format!("Docker is not available: {error}"))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Err(format!(
                "Docker Compose failed ({}): {}",
                output.status,
                if stderr.is_empty() { stdout } else { stderr }
            ))
        }
    })
    .await
    .map_err(|error| AppError::WebResearch(format!("Docker startup task failed: {error}")))?
    .map_err(AppError::WebResearch)?;

    Ok(())
}

pub async fn ensure_started_with_settings(app: &AppHandle, settings: &crate::AppSettings) -> AppResult<()> {
    if research::check_private_search(settings).await.is_ok() {
        return Ok(());
    }

    if settings.web_search_url.trim() != DEFAULT_SEARCH_URL
        || settings.web_proxy_url.trim() != DEFAULT_PROXY_URL
    {
        return Err(AppError::WebResearch(
            "automatic gateway startup only supports Raphael's bundled local SearXNG/Tor endpoints; the configured custom endpoints are not running".into(),
        ));
    }


    let runtime = install_runtime_files(app)?;
    docker_compose_up(&runtime).await?;

    timeout(STARTUP_TIMEOUT, async {
        loop {
            if research::check_private_search(settings).await.is_ok() {
                return Ok(());
            }
            sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .map_err(|_| {
        AppError::WebResearch(format!(
            "private web research gateway did not become ready within {} seconds",
            STARTUP_TIMEOUT.as_secs()
        ))
    })?
}
