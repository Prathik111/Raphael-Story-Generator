use crate::{comfyui, registry::RegistryState, research, AppSettings};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceHealthStatus {
    Online,
    Offline,
    Disabled,
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

pub async fn probe(registry: &RegistryState, settings: &AppSettings) -> ServiceStatusBoard {
    let registry_status = registry.status().await;
    let registry = ServiceStatusDto {
        service: "registry".into(),
        status: match registry_status.status {
            crate::registry::RegistryStatus::On => ServiceHealthStatus::Online,
            crate::registry::RegistryStatus::Starting => ServiceHealthStatus::Offline,
            crate::registry::RegistryStatus::Off => ServiceHealthStatus::Offline,
        },
        url: registry_status.url,
        detail: registry_status.detail,
    };

    let searxng = if !settings.web_research_enabled {
        ServiceStatusDto {
            service: "searxng".into(),
            status: ServiceHealthStatus::Disabled,
            url: settings.web_search_url.clone(),
            detail: Some("Private web research is disabled in settings.".into()),
        }
    } else {
        match research::check_search_api(settings).await {
            Ok(()) => ServiceStatusDto {
                service: "searxng".into(),
                status: ServiceHealthStatus::Online,
                url: settings.web_search_url.clone(),
                detail: Some("SearXNG search API responded successfully.".into()),
            },
            Err(error) => ServiceStatusDto {
                service: "searxng".into(),
                status: ServiceHealthStatus::Offline,
                url: settings.web_search_url.clone(),
                detail: Some(error.to_string()),
            },
        }
    };

    let comfyui = match comfyui::check_api(&settings.comfyui_url).await {
        Ok(()) => ServiceStatusDto {
            service: "comfyui".into(),
            status: ServiceHealthStatus::Online,
            url: settings.comfyui_url.clone(),
            detail: Some("ComfyUI API responded successfully.".into()),
        },
        Err(error) => ServiceStatusDto {
            service: "comfyui".into(),
            status: ServiceHealthStatus::Offline,
            url: settings.comfyui_url.clone(),
            detail: Some(error.to_string()),
        },
    };

    ServiceStatusBoard { registry, searxng, comfyui }
}
