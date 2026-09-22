use crate::{comfyui, privacy_gateway, registry::RegistryState, research, AppSettings};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceHealthStatus {
    Online,
    Offline,
    Checking,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatusDto {
    pub service: String,
    pub status: ServiceHealthStatus,
    pub url: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatusBoard {
    pub registry: ServiceStatusDto,
    pub searxng: ServiceStatusDto,
    pub comfyui: ServiceStatusDto,
}

// Live health is derived from real endpoint probes.
pub async fn probe(registry: &RegistryState, settings: &AppSettings) -> ServiceStatusBoard {
    let registry_future = registry.status();
    let searx_future = research::check_search_api(settings);
    let comfy_future = comfyui::detect_api_url(&settings.comfyui_url);
    let (registry_status, searx_result, comfy_result) =
        tokio::join!(registry_future, searx_future, comfy_future);

    let registry = ServiceStatusDto {
        service: "registry".into(),
        status: match registry_status.status {
            crate::registry::RegistryStatus::On => ServiceHealthStatus::Online,
            crate::registry::RegistryStatus::Starting => ServiceHealthStatus::Checking,
            crate::registry::RegistryStatus::Off => ServiceHealthStatus::Offline,
        },
        url: registry_status.url,
        detail: registry_status.detail,
    };

    let searxng = match searx_result {
        Ok(()) => ServiceStatusDto {
            service: "searxng".into(),
            status: ServiceHealthStatus::Online,
            url: settings.web_search_url.clone(),
            detail: Some(
                if settings.web_research_enabled {
                    "SearXNG search API responded successfully.".into()
                } else {
                    "SearXNG API is online; private research is disabled in settings.".into()
                },
            ),
        },
        Err(_) if privacy_gateway::is_starting() => ServiceStatusDto {
            service: "searxng".into(),
            status: ServiceHealthStatus::Checking,
            url: settings.web_search_url.clone(),
            detail: Some("Starting the bundled SearXNG and Tor gateway…".into()),
        },
        Err(error) => ServiceStatusDto {
            service: "searxng".into(),
            status: ServiceHealthStatus::Offline,
            url: settings.web_search_url.clone(),
            detail: privacy_gateway::last_error().or_else(|| Some(error.to_string())),
        },
    };

    let comfyui = match comfy_result {
        Ok(url) => ServiceStatusDto {
            service: "comfyui".into(),
            status: ServiceHealthStatus::Online,
            url,
            detail: Some("ComfyUI API detected successfully.".into()),
        },
        Err(error) => ServiceStatusDto {
            service: "comfyui".into(),
            status: ServiceHealthStatus::Offline,
            url: settings.comfyui_url.clone(),
            detail: Some(error.to_string()),
        },
    };

    ServiceStatusBoard {
        registry,
        searxng,
        comfyui,
    }
}
