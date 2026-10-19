use crate::{comfyui, privacy_gateway, registry::RegistryState, research, AppSettings};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::{net::IpAddr, time::Duration};
use url::Url;

const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const OLLAMA_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const OLLAMA_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

fn normalize_ollama_base(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let mut url = Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    let host = url.host_str()?.trim_matches(['[', ']']);
    if host == "0.0.0.0" {
        url.set_host(Some("127.0.0.1")).ok()?;
    } else if host == "::" {
        url.set_host(Some("::1")).ok()?;
    } else if host != "localhost"
        && !host.parse::<IpAddr>().map(|address| address.is_loopback()).unwrap_or(false)
    {
        // Service health only probes local Ollama endpoints; do not turn an
        // environment variable or a remote LLM URL into an arbitrary network probe.
        return None;
    }

    if url.port().is_none() {
        url.set_port(Some(11434)).ok()?;
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Some(url)
}

fn ollama_candidates(settings: &AppSettings) -> Vec<Url> {
    let raw_candidates = [
        std::env::var("RAPHAEL_OLLAMA_URL").ok(),
        std::env::var("OLLAMA_HOST").ok(),
        Some(settings.llm_base_url.clone()),
        Some(DEFAULT_OLLAMA_URL.to_string()),
    ];
    let mut candidates = Vec::new();

    for base in raw_candidates.into_iter().flatten()
        .filter_map(|raw| normalize_ollama_base(&raw))
    {
        if !candidates.iter().any(|existing: &Url| existing == &base) {
            candidates.push(base);
        }
    }

    candidates
}

async fn detect_ollama(settings: &AppSettings) -> Result<String, String> {
    let candidates = ollama_candidates(settings);
    let client = Client::builder()
        .connect_timeout(OLLAMA_CONNECT_TIMEOUT)
        .timeout(OLLAMA_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to create Ollama health client: {error}"))?;

    let mut errors = Vec::new();
    for base in &candidates {
        let endpoint = base.join("api/version")
            .map_err(|error| format!("invalid Ollama health URL: {error}"))?;
        match client.get(endpoint.clone()).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(value) if value.get("version").and_then(Value::as_str)
                        .is_some_and(|version| !version.trim().is_empty()) =>
                    {
                        return Ok(base.as_str().trim_end_matches('/').to_string());
                    }
                    Ok(_) => errors.push(format!("{endpoint} did not return an Ollama version")),
                    Err(error) => errors.push(format!("{endpoint} returned invalid JSON: {error}")),
                }
            }
            Ok(response) => errors.push(format!("{endpoint} returned HTTP {}", response.status())),
            Err(error) => errors.push(format!("{endpoint}: {error}")),
        }
    }

    Err(format!(
        "Ollama API was not detected on local endpoints. {}",
        errors.join(" | ")
    ))
}

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
    pub ollama: ServiceStatusDto,
}

// Live health is derived from real endpoint probes.
pub async fn probe(registry: &RegistryState, settings: &AppSettings) -> ServiceStatusBoard {
    let registry_future = registry.status();
    let searx_future = research::check_search_gateway(settings);
    let comfy_future = comfyui::detect_api_url(&settings.comfyui_url);
    let ollama_future = detect_ollama(settings);
    let (registry_status, searx_result, comfy_result, ollama_result) =
        tokio::join!(registry_future, searx_future, comfy_future, ollama_future);

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
                    "SearXNG local gateway is reachable; engine availability is checked when research actually runs.".into()
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

    let ollama = match ollama_result {
        Ok(url) => ServiceStatusDto {
            service: "ollama".into(),
            status: ServiceHealthStatus::Online,
            url,
            detail: Some("Ollama API detected successfully via /api/version.".into()),
        },
        Err(error) => ServiceStatusDto {
            service: "ollama".into(),
            status: ServiceHealthStatus::Offline,
            url: ollama_candidates(settings)
                .first()
                .map(|url| url.as_str().trim_end_matches('/').to_string())
                .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string()),
            detail: Some(error),
        },
    };

    ServiceStatusBoard {
        registry,
        searxng,
        comfyui,
        ollama,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_health_normalizes_wildcard_bind_to_loopback() {
        let base = normalize_ollama_base("http://0.0.0.0:11434/v1")
            .expect("wildcard Ollama bind should map to local loopback for probing");
        assert_eq!(base.host_str(), Some("127.0.0.1"));
        assert_eq!(base.port(), Some(11434));
        assert_eq!(base.join("api/version").unwrap().path(), "/api/version");
    }

    #[test]
    fn ollama_health_defaults_a_missing_port_to_11434() {
        let base = normalize_ollama_base("localhost")
            .expect("localhost should be a valid Ollama endpoint");
        assert_eq!(base.host_str(), Some("localhost"));
        assert_eq!(base.port(), Some(11434));
    }

    #[test]
    fn ollama_health_refuses_nonlocal_endpoints() {
        assert!(normalize_ollama_base("https://api.example.com:11434").is_none());
        assert!(normalize_ollama_base("socks5h://127.0.0.1:9050").is_none());
    }
}
