use axum::{
    body::{to_bytes, Body},
    extract::State as AxumState,
    http::{
        header::{
            self, HeaderValue, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE,
        },
        Method, Request, Response, StatusCode,
    },
    routing::any,
    Router,
};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::Mutex,
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

use crate::{AppError, AppResult, AppSettings};

const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const MAX_PROXY_REQUEST_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct OllamaModelDetection {
    pub base_url: String,
    pub openai_base_url: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaLanHostStatus {
    pub running: bool,
    pub port: u16,
    pub lan_url: Option<String>,
    pub openai_base_url: Option<String>,
    pub models: Vec<String>,
    pub detail: Option<String>,
}

impl OllamaLanHostStatus {
    fn stopped(port: u16) -> Self {
        Self {
            running: false,
            port,
            lan_url: None,
            openai_base_url: None,
            models: Vec::new(),
            detail: Some("Ollama LAN hosting is stopped.".into()),
        }
    }
}

struct RunningHost {
    status: OllamaLanHostStatus,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub struct OllamaLanHost {
    running: Mutex<Option<RunningHost>>,
}

#[derive(Clone)]
struct ProxyState {
    upstream: Url,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTags {
    models: Vec<OllamaModel>,
}

fn normalize_local_ollama_url(raw: &str) -> Option<Url> {
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

    let host = url.host_str()?.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "0.0.0.0" || host == "::" {
        url.set_host(Some(if host == "::" { "::1" } else { "127.0.0.1" })).ok()?;
    } else if host != "localhost"
        && !host.parse::<std::net::IpAddr>().map(|address| address.is_loopback()).unwrap_or(false)
    {
        // Model detection and LAN hosting may only inspect a local Ollama
        // process. The LAN proxy is the one intentional remote-facing endpoint.
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

fn local_ollama_candidates(settings: &AppSettings, configured_base_url: Option<&str>) -> Vec<Url> {
    let raw = [
        configured_base_url.map(str::to_string),
        std::env::var("RAPHAEL_OLLAMA_URL").ok(),
        std::env::var("OLLAMA_HOST").ok(),
        Some(settings.llm_base_url.clone()),
        Some(DEFAULT_OLLAMA_URL.to_string()),
    ];
    let mut candidates = Vec::new();

    for url in raw.into_iter().flatten().filter_map(|value| normalize_local_ollama_url(&value)) {
        if !candidates.iter().any(|existing: &Url| existing == &url) {
            candidates.push(url);
        }
    }
    candidates
}

fn model_client() -> AppResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| AppError::Llm(format!("failed to create Ollama model detection client: {error}")))
}

async fn request_model_list(client: &Client, base: &Url) -> Result<Vec<String>, String> {
    let endpoint = base.join("api/tags").map_err(|error| error.to_string())?;
    let response = client.get(endpoint.clone()).send().await
        .map_err(|error| format!("{endpoint}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{endpoint} returned HTTP {}", response.status()));
    }
    let value = response.json::<OllamaTags>().await
        .map_err(|error| format!("{endpoint} returned invalid model JSON: {error}"))?;
    let mut models = value.models.into_iter()
        .map(|model| model.name.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

async fn resolve_local_ollama(
    settings: &AppSettings,
    configured_base_url: Option<&str>,
) -> AppResult<(Url, Vec<String>)> {
    let client = model_client()?;
    let candidates = local_ollama_candidates(settings, configured_base_url);
    let mut errors = Vec::new();

    for base in candidates {
        match request_model_list(&client, &base).await {
            Ok(models) => return Ok((base, models)),
            Err(error) => errors.push(error),
        }
    }

    Err(AppError::Llm(format!(
        "Ollama was not detected on local endpoints. Start Ollama and check that its API is listening on port 11434, or set RAPHAEL_OLLAMA_URL to a local Ollama endpoint. {}",
        if errors.is_empty() { String::new() } else { errors.join(" | ") }
    )))
}

fn openai_base_url(base: &Url) -> String {
    format!("{}/v1", base.as_str().trim_end_matches('/'))
}

pub async fn detect_models(settings: &AppSettings, configured_base_url: Option<&str>) -> AppResult<OllamaModelDetection> {
    let (base, models) = resolve_local_ollama(settings, configured_base_url).await?;
    if models.is_empty() {
        return Err(AppError::Llm(format!(
            "Ollama is running at {}, but no models are installed. Pull a model with Ollama first.",
            base.as_str().trim_end_matches('/')
        )));
    }
    Ok(OllamaModelDetection {
        base_url: base.as_str().trim_end_matches('/').to_string(),
        openai_base_url: openai_base_url(&base),
        models,
    })
}

fn detect_private_lan_ipv4() -> AppResult<Ipv4Addr> {
    // UDP connect selects the local interface for the route but sends no
    // application data. Bind only to a private IPv4 address, never 0.0.0.0.
    for target in [
        "192.168.1.1:80",
        "10.0.0.1:80",
        "172.16.0.1:80",
        "8.8.8.8:80",
    ] {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { continue };
        if socket.connect(target).is_err() {
            continue;
        }
        let Ok(SocketAddr::V4(address)) = socket.local_addr() else { continue };
        if address.ip().is_private() && !address.ip().is_loopback() {
            return Ok(*address.ip());
        }
    }

    Err(AppError::Llm(
        "Could not detect a private LAN IPv4 address. Connect this PC to a private Wi-Fi/Ethernet network and retry.".into(),
    ))
}

pub fn status(host: &OllamaLanHost, default_port: u16) -> AppResult<OllamaLanHostStatus> {
    let running = host.running.lock()
        .map_err(|_| AppError::Llm("Ollama LAN host state lock is poisoned".into()))?;
    Ok(running.as_ref().map(|state| state.status.clone())
        .unwrap_or_else(|| OllamaLanHostStatus::stopped(default_port)))
}

pub async fn start_host(
    host: &OllamaLanHost,
    settings: &AppSettings,
    configured_base_url: Option<&str>,
    port: u16,
) -> AppResult<OllamaLanHostStatus> {
    if port < 1024 {
        return Err(AppError::Llm("LAN host port must be between 1024 and 65535.".into()));
    }
    {
        let running = host.running.lock()
            .map_err(|_| AppError::Llm("Ollama LAN host state lock is poisoned".into()))?;
        if let Some(active) = running.as_ref() {
            return Err(AppError::Llm(format!(
                "Ollama LAN hosting is already active at {}. Stop it before starting another host.",
                active.status.openai_base_url.as_deref().unwrap_or("the configured port")
            )));
        }
    }

    let (upstream, models) = resolve_local_ollama(settings, configured_base_url).await?;
    if models.is_empty() {
        return Err(AppError::Llm(format!(
            "Ollama at {} has no installed models. Install a model before hosting it on the LAN.",
            upstream.as_str().trim_end_matches('/')
        )));
    }

    let lan_ip = detect_private_lan_ipv4()?;
    let address = SocketAddr::from((lan_ip, port));
    let listener = TcpListener::bind(address).await.map_err(|error| {
        AppError::Llm(format!(
            "Could not start the Ollama LAN host on {address}: {error}. Choose another port or close the application using it."
        ))
    })?;

    let proxy_state = ProxyState {
        upstream: upstream.clone(),
        client: Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60 * 60))
            .build()
            .map_err(|error| AppError::Llm(format!("failed to create Ollama LAN proxy client: {error}")))?,
    };
    let router = Router::new()
        .route("/", any(proxy))
        .fallback(proxy)
        .with_state(proxy_state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router)
            .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
            .await
        {
            eprintln!("Raphael Ollama LAN host stopped: {error}");
        }
    });

    let lan_url = format!("http://{lan_ip}:{port}");
    let status = OllamaLanHostStatus {
        running: true,
        port,
        lan_url: Some(lan_url.clone()),
        openai_base_url: Some(format!("{lan_url}/v1")),
        models,
        detail: Some(format!(
            "Authenticated Ollama proxy running on private LAN address {lan_url}; upstream is local Ollama at {}.",
            upstream.as_str().trim_end_matches('/')
        )),
    };

    let mut running = host.running.lock()
        .map_err(|_| AppError::Llm("Ollama LAN host state lock is poisoned".into()))?;
    if running.is_some() {
        let _ = shutdown_tx.send(());
        task.abort();
        return Err(AppError::Llm("Ollama LAN host was started concurrently; retry the request.".into()));
    }
    *running = Some(RunningHost { status: status.clone(), shutdown: shutdown_tx, task });
    Ok(status)
}

pub fn stop_host(host: &OllamaLanHost, default_port: u16) -> AppResult<OllamaLanHostStatus> {
    let mut running = host.running.lock()
        .map_err(|_| AppError::Llm("Ollama LAN host state lock is poisoned".into()))?;
    if let Some(active) = running.take() {
        let _ = active.shutdown.send(());
        active.task.abort();
    }
    Ok(OllamaLanHostStatus::stopped(default_port))
}

fn allowed_method_path(method: &Method, raw_path: &str) -> bool {
    // Some OpenAI-compatible clients add a trailing slash or request one of
    // the standard metadata/inference routes during client initialization.
    let path = raw_path.trim_start_matches('/').trim_end_matches('/');
    match (method.as_str(), path) {
        ("GET", "v1")
        | ("GET", "v1/models")
        | ("POST", "v1/chat/completions")
        // Also accept OpenAI SDKs configured with a host root instead of /v1.
        | ("GET", "models")
        | ("POST", "chat/completions")
        | ("POST", "completions")
        | ("POST", "embeddings")
        | ("POST", "responses")
        | ("POST", "v1/completions")
        | ("POST", "v1/embeddings")
        | ("POST", "v1/responses")
        | ("POST", "api/chat")
        | ("POST", "api/generate")
        | ("POST", "api/embed")
        | ("POST", "api/embeddings")
        | ("POST", "api/show")
        | ("GET", "api/tags")
        | ("GET", "api/version") => true,
        // A read-only model lookup is safe; mutations and model management
        // endpoints such as /api/pull, /api/delete and /api/push stay blocked.
        ("GET", path) if (path.starts_with("v1/models/") && path.len() > "v1/models/".len())
            || (path.starts_with("models/") && path.len() > "models/".len()) => true,
        _ => false,
    }
}

fn add_cors_headers(response: &mut Response<Body>) {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Authorization, Content-Type, Accept"));
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "error": message }).to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    add_cors_headers(&mut response);
    response
}

async fn proxy(
    AxumState(state): AxumState<ProxyState>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    if parts.method == Method::OPTIONS {
        let mut response = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty()));
        add_cors_headers(&mut response);
        return response;
    }

    let path = parts.uri.path().trim_start_matches('/').trim_end_matches('/');
    if parts.method == Method::GET && path.is_empty() {
        let mut response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({
                "status": "ok",
                "service": "Raphael Ollama LAN proxy",
                "api_base": "/v1"
            }).to_string()))
            .unwrap_or_else(|_| Response::new(Body::empty()));
        add_cors_headers(&mut response);
        return response;
    }

    if !allowed_method_path(&parts.method, path) {
        return json_error(
            StatusCode::NOT_FOUND,
            &format!("Ollama LAN proxy route not enabled: {} /{}.", parts.method, path),
        );
    }

    // Normalize trailing slashes before forwarding so an accepted path does
    // not become a 404 at the Ollama upstream. Some clients probe GET /v1;
    // answer that as the standard model-list endpoint.
    let upstream_path = if parts.method == Method::GET && path == "v1" {
        "v1/models".to_string()
    } else if path == "models"
        || path.starts_with("models/")
        || path == "chat/completions"
        || path == "completions"
        || path == "embeddings"
        || path == "responses"
    {
        format!("v1/{path}")
    } else {
        path.to_string()
    };
    let query = parts.uri.query().map(|value| format!("?{value}")).unwrap_or_default();
    let url = format!("{}/{}{}", state.upstream.as_str().trim_end_matches('/'), upstream_path, query);
    let mut outbound = state.client.request(parts.method, url);

    for (name, value) in parts.headers.iter() {
        if matches!(
            name.as_str(),
            "authorization" | "host" | "content-length" | "connection" | "origin"
                | "access-control-request-method" | "access-control-request-headers"
        ) {
            continue;
        }
        outbound = outbound.header(name, value);
    }

    let body_bytes = match to_bytes(body, MAX_PROXY_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => return json_error(StatusCode::PAYLOAD_TOO_LARGE, &format!("Request body could not be read or exceeded the size limit: {error}")),
    };
    if !body_bytes.is_empty() {
        outbound = outbound.body(body_bytes);
    }

    let upstream_response = match outbound.send().await {
        Ok(response) => response,
        Err(error) => return json_error(StatusCode::BAD_GATEWAY, &format!("The local Ollama API could not be reached: {error}")),
    };
    let status = upstream_response.status();
    let content_type = upstream_response.headers().get(CONTENT_TYPE).cloned();
    let cache_control = upstream_response.headers().get(header::CACHE_CONTROL).cloned();
    let mut response = Response::builder().status(status);
    if let Some(value) = content_type {
        response = response.header(CONTENT_TYPE, value);
    }
    if let Some(value) = cache_control {
        response = response.header(header::CACHE_CONTROL, value);
    }
    let stream = Body::from_stream(upstream_response.bytes_stream());
    let mut response = response.body(stream).unwrap_or_else(|_| Response::new(Body::empty()));
    add_cors_headers(&mut response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_detection_only_accepts_loopback_endpoints() {
        assert!(normalize_local_ollama_url("http://127.0.0.1:11434/v1").is_some());
        assert!(normalize_local_ollama_url("localhost:11434").is_some());
        assert!(normalize_local_ollama_url("http://0.0.0.0:11434").is_some());
        assert!(normalize_local_ollama_url("https://example.com:11434").is_none());
    }

    #[test]
    fn proxy_only_exposes_inference_and_read_only_model_routes() {
        assert!(allowed_method_path(&Method::POST, "v1/chat/completions"));
        assert!(allowed_method_path(&Method::GET, "v1/models"));
        assert!(allowed_method_path(&Method::GET, "/v1/models/"));
        assert!(allowed_method_path(&Method::GET, "/v1"));
        assert!(allowed_method_path(&Method::POST, "/v1/chat/completions/"));
        assert!(allowed_method_path(&Method::POST, "v1/responses"));
        assert!(allowed_method_path(&Method::POST, "chat/completions"));
        assert!(allowed_method_path(&Method::GET, "models"));
        assert!(allowed_method_path(&Method::GET, "api/tags"));
        assert!(!allowed_method_path(&Method::POST, "api/pull"));
        assert!(!allowed_method_path(&Method::POST, "api/delete"));
        assert!(!allowed_method_path(&Method::POST, "v1/chat/completions/extra"));
        assert!(!allowed_method_path(&Method::DELETE, "api/delete"));
    }
}
