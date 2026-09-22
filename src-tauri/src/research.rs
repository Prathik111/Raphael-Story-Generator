use crate::{chat, AppError, AppResult, AppSettings};
use futures_util::StreamExt;
use reqwest::{Client, Proxy, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use url::Url;

const MAX_REDIRECTS: usize = 3;
const DEFAULT_FETCH_CHARS: usize = 12_000;
const MAX_SOURCE_BYTES: u64 = 2_000_000;

pub const DEFAULT_WEB_RESEARCH_SYSTEM_PROMPT: &str = r#"You are Raphael Web Research Extractor.
You receive text fetched from real web pages discovered through a private research gateway.
Extract only information that is directly supported by the supplied source text.

Rules:
- Never invent facts.
- Never fill gaps from your own knowledge.
- Every factual claim MUST cite one or more source IDs.
- Include a short evidence quote or faithful evidence excerpt for every claim.
- Prefer primary/official sources when the supplied sources support the same fact.
- When sources disagree, preserve the disagreement instead of silently resolving it.
- Do not treat search-result snippets as stronger evidence than fetched page content.
- Keep claims concise and useful to the downstream story architect.
- Return ONLY valid JSON matching the requested schema."#;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ResearchSource {
    pub id: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ResearchFact {
    pub claim: String,
    pub evidence: String,
    pub source_ids: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ResearchBundle {
    pub queries: Vec<String>,
    pub sources: Vec<ResearchSource>,
    pub facts: Vec<ResearchFact>,
    pub retrieved_at: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SearxResponse {
    results: Vec<SearxResult>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SearxResult {
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ExtractedResearch {
    facts: Vec<ResearchFact>,
}

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{seconds}")
}

fn ensure_local_endpoint(raw: &str, label: &str) -> AppResult<Url> {
    let url = Url::parse(raw)
        .map_err(|error| AppError::WebResearch(format!("invalid {label} URL: {error}")))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::WebResearch(format!(
            "{label} URL must use http:// or https://"
        )));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::WebResearch(format!("{label} URL has no host")))?;

    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if !local {
        return Err(AppError::WebResearch(format!(
            "{label} must point to a local endpoint for privacy; refused host '{host}'"
        )));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::WebResearch(format!(
            "{label} must not contain embedded credentials"
        )));
    }

    Ok(url)
}

fn ensure_local_proxy(raw: &str) -> AppResult<Url> {
    let url = Url::parse(raw)
        .map_err(|error| AppError::WebResearch(format!("invalid web research proxy URL: {error}")))?;

    if !matches!(
        url.scheme(),
        "socks5" | "socks5h" | "socks4" | "socks4a" | "http" | "https"
    ) {
        return Err(AppError::WebResearch(
            "web research proxy must use a supported SOCKS/HTTP proxy scheme".into(),
        ));
    }

    let host = url.host_str().ok_or_else(|| {
        AppError::WebResearch("web research proxy URL has no host".into())
    })?;

    if !matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return Err(AppError::WebResearch(format!(
            "web research proxy must be local; refused host '{host}'"
        )));
    }

    Ok(url)
}

fn build_local_client(timeout: Duration) -> AppResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Raphael-Story-Generator/0.1")
        .build()
        .map_err(|error| AppError::WebResearch(format!("failed to create local research client: {error}")))
}

fn build_client(settings: &AppSettings, timeout: Duration) -> AppResult<Client> {
    if settings.web_proxy_url.trim().is_empty() {
        return Err(AppError::WebResearch(
            "private web research requires the local Tor/SOCKS proxy; direct Internet access is disabled".into(),
        ));
    }

    let proxy_url = ensure_local_proxy(settings.web_proxy_url.trim())?;
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Raphael-Story-Generator/0.1")
        .proxy(
            Proxy::all(proxy_url.as_str())
                .map_err(|error| AppError::WebResearch(format!("failed to configure web proxy: {error}")))?,
        )
        .build()
        .map_err(|error| AppError::WebResearch(format!("failed to create research HTTP client: {error}")))
}

fn blocked_host(host: &str) -> bool {
    if matches!(host, "localhost" | "localhost.localdomain" | "broadcasthost") {
        return true;
    }

    let lower = host.to_ascii_lowercase();
    if lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".home.arpa")
    {
        return true;
    }

    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };

    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_unspecified()
        }
    }
}

fn validate_source_url(raw: &str) -> AppResult<Url> {
    let url = Url::parse(raw)
        .map_err(|error| AppError::WebResearch(format!("invalid research source URL: {error}")))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::WebResearch(format!(
            "unsupported research source scheme '{}'",
            url.scheme()
        )));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::WebResearch("research source has no host".into()))?;

    if blocked_host(host) {
        return Err(AppError::WebResearch(format!(
            "refused non-public research source host '{host}'"
        )));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::WebResearch(
            "research source URLs with embedded credentials are refused".into(),
        ));
    }

    Ok(url)
}

async fn search(settings: &AppSettings, query: &str) -> AppResult<Vec<SearxResult>> {
    let base = ensure_local_endpoint(settings.web_search_url.trim(), "web search")?;
    let url = base
        .join("search")
        .map_err(|error| AppError::WebResearch(format!("invalid SearXNG search URL: {error}")))?;

    let client = build_local_client(Duration::from_secs(20))?;
    let response = client
        .get(url)
        .query(&[
            ("q", query),
            ("format", "json"),
            ("language", "en"),
            ("categories", "general"),
            ("safesearch", "1"),
        ])
        .send()
        .await
        .map_err(|error| AppError::WebResearch(format!("private web search failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::WebResearch(format!(
            "SearXNG returned {status}: {}",
            body.chars().take(500).collect::<String>()
        )));
    }

    response
        .json::<SearxResponse>()
        .await
        .map(|result| result.results)
        .map_err(|error| AppError::WebResearch(format!("invalid SearXNG JSON response: {error}")))
}

async fn fetch_source(
    client: &Client,
    initial_url: Url,
    max_chars: usize,
) -> AppResult<(Url, String)> {
    let mut current = initial_url;

    for _ in 0..=MAX_REDIRECTS {
        current = validate_source_url(current.as_str())?;

        let response = client
            .get(current.clone())
            .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9")
            .send()
            .await
            .map_err(|error| AppError::WebResearch(format!("failed to fetch {}: {error}", current)))?;

        if response.status().is_redirection() {
            let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                return Err(AppError::WebResearch(format!(
                    "research source redirected without a Location header: {current}"
                )));
            };
            let location = location
                .to_str()
                .map_err(|error| AppError::WebResearch(format!("invalid redirect from {current}: {error}")))?;
            current = current
                .join(location)
                .map_err(|error| AppError::WebResearch(format!("invalid redirect target from {current}: {error}")))?;
            continue;
        }

        if response.status() != StatusCode::OK {
            return Err(AppError::WebResearch(format!(
                "research source returned HTTP {}: {}",
                response.status(),
                current
            )));
        }

        if response
            .content_length()
            .is_some_and(|length| length > MAX_SOURCE_BYTES)
        {
            return Err(AppError::WebResearch(format!(
                "research source is larger than the {} MB safety limit: {}",
                MAX_SOURCE_BYTES / 1_000_000,
                current
            )));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !(content_type.contains("text/html")
            || content_type.contains("application/xhtml+xml")
            || content_type.starts_with("text/plain"))
        {
            return Err(AppError::WebResearch(format!(
                "unsupported research content type '{content_type}' for {current}"
            )));
        }

        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                AppError::WebResearch(format!("failed to read {current}: {error}"))
            })?;
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > MAX_SOURCE_BYTES {
                return Err(AppError::WebResearch(format!(
                    "research source exceeded the {} MB safety limit while downloading: {}",
                    MAX_SOURCE_BYTES / 1_000_000,
                    current
                )));
            }
        }

        let body = String::from_utf8_lossy(&bytes);
        let content = if content_type.contains("text/html")
            || content_type.contains("application/xhtml+xml")
        {
            extract_html_text(&body)
        } else {
            body.to_string()
        };

        return Ok((current, trim_chars(&content, max_chars)));
    }

    Err(AppError::WebResearch(
        "research source exceeded the redirect safety limit".into(),
    ))
}

fn extract_html_text(html: &str) -> String {
    let without_code = regex::Regex::new(
        r"(?is)<(script|style|noscript|svg|canvas|template|iframe)[^>]*>.*?</\1>",
    )
    .map(|regex| regex.replace_all(html, " ").into_owned())
    .unwrap_or_else(|_| html.to_string());

    let document = scraper::Html::parse_document(&without_code);
    let text = document
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ");

    normalize_whitespace(&text)
}

fn normalize_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars.max(1)).collect()
}

fn source_context(sources: &[ResearchSource], max_chars: usize) -> String {
    let mut output = String::new();

    for source in sources {
        let block = format!(
            "[{}]\nTITLE: {}\nURL: {}\nSEARCH SNIPPET: {}\nPAGE TEXT:\n{}\n\n",
            source.id,
            source.title,
            source.url,
            source.snippet,
            source.content
        );

        if output.chars().count() + block.chars().count() > max_chars.max(4_000) {
            break;
        }

        output.push_str(&block);
    }

    output
}

fn research_prompt(bundle: &ResearchBundle) -> String {
    bundle
        .facts
        .iter()
        .map(|fact| {
            format!(
                "- {} [confidence: {}] [sources: {}]\n  evidence: {}",
                fact.claim,
                if fact.confidence.trim().is_empty() {
                    "unspecified"
                } else {
                    fact.confidence.as_str()
                },
                fact.source_ids.join(", "),
                fact.evidence
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn story_architect_context(bundle: &ResearchBundle) -> String {
    if bundle.sources.is_empty() && bundle.facts.is_empty() {
        return "WEB RESEARCH:\nNo external research was performed.".into();
    }

    let sources = bundle
        .sources
        .iter()
        .map(|source| format!("[{}] {} — {}", source.id, source.title, source.url))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "WEB RESEARCH FACTS:\n{}\n\nWEB RESEARCH SOURCES:\n{}",
        research_prompt(bundle),
        sources
    )
}

pub async fn research_web(
    app: &tauri::AppHandle,
    settings: &AppSettings,
    query: &str,
) -> AppResult<ResearchBundle> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(ResearchBundle::default());
    }

    if !settings.web_research_enabled {
        return Ok(ResearchBundle::default());
    }

    let _ = ensure_local_endpoint(settings.web_search_url.trim(), "web search")?;
    let _ = ensure_local_proxy(settings.web_proxy_url.trim())?;

    let results = search(settings, query).await?;
    if results.is_empty() {
        return Err(AppError::WebResearch(
            "private web search returned no results".into(),
        ));
    }

    let max_results = settings.web_search_max_results.clamp(1, 12);
    let max_chars = settings
        .web_fetch_max_chars
        .clamp(2_000, DEFAULT_FETCH_CHARS.max(2_000));

    let client = build_client(settings, Duration::from_secs(30))?;
    let mut sources = Vec::new();

    for (index, result) in results.into_iter().take(max_results).enumerate() {
        let Ok(url) = validate_source_url(&result.url) else {
            continue;
        };

        let Ok((resolved_url, content)) = fetch_source(&client, url, max_chars).await else {
            continue;
        };

        if content.trim().len() < 120 {
            continue;
        }

        sources.push(ResearchSource {
            id: format!("S{}", index + 1),
            title: result.title.trim().to_string(),
            url: resolved_url.to_string(),
            snippet: trim_chars(&result.content, 1_500),
            content,
        });
    }

    if sources.is_empty() {
        return Err(AppError::WebResearch(
            "private search found results, but no source pages could be safely fetched".into(),
        ));
    }

    let context = source_context(&sources, settings.web_context_max_chars.clamp(8_000, 48_000));
    let system = if settings.web_research_system_prompt.trim().is_empty() {
        DEFAULT_WEB_RESEARCH_SYSTEM_PROMPT
    } else {
        settings.web_research_system_prompt.as_str()
    };

    let schema = r#"{"facts":[{"claim":"","evidence":"","source_ids":["S1"],"confidence":"high|medium|low"}]}"#;
    let user = format!(
        "RESEARCH QUERY:\n{query}\n\nSOURCE MATERIAL:\n{context}\n\nExtract source-backed facts now.\nReturn:\n{schema}"
    );

    let raw = chat(app, settings, "web_research", system, &user).await?;
    let parsed: ExtractedResearch = serde_json::from_str(crate::clean_json(&raw))
        .map_err(|error| {
            AppError::ModelResponse(format!(
                "web research extractor returned invalid JSON: {error}; output starts with: {}",
                raw.chars().take(300).collect::<String>()
            ))
        })?;

    let valid_ids = sources
        .iter()
        .map(|source| source.id.as_str())
        .collect::<std::collections::HashSet<_>>();

    let facts = parsed
        .facts
        .into_iter()
        .filter(|fact| {
            !fact.claim.trim().is_empty()
                && !fact.evidence.trim().is_empty()
                && !fact.source_ids.is_empty()
                && fact.source_ids.iter().all(|id| valid_ids.contains(id.as_str()))
        })
        .collect::<Vec<_>>();

    if facts.is_empty() {
        return Err(AppError::WebResearch(
            "web research returned sources, but the extractor could not produce source-backed facts"
                .into(),
        ));
    }

    Ok(ResearchBundle {
        queries: vec![query.to_string()],
        sources,
        facts,
        retrieved_at: now(),
    })
}

pub fn merge_into(target: &mut ResearchBundle, mut incoming: ResearchBundle) {
    let mut id_map = HashMap::new();

    for source in incoming.sources.drain(..) {
        let existing = target.sources.iter().position(|item| item.url == source.url);
        if let Some(index) = existing {
            let existing_id = target.sources[index].id.clone();
            id_map.insert(source.id, existing_id);
        } else {
            let new_id = format!("S{}", target.sources.len() + 1);
            id_map.insert(source.id, new_id.clone());
            target.sources.push(ResearchSource { id: new_id, ..source });
        }
    }

    for fact in incoming.facts {
        let mut fact = fact;
        fact.source_ids = fact
            .source_ids
            .iter()
            .filter_map(|id| id_map.get(id).cloned())
            .collect();
        fact.source_ids.sort();
        fact.source_ids.dedup();
        if fact.source_ids.is_empty() {
            continue;
        }
        target.facts.push(fact);
    }

    for query in incoming.queries {
        if !target.queries.iter().any(|existing| existing == &query) {
            target.queries.push(query);
        }
    }

    target.retrieved_at = now();
}

#[derive(Debug, Deserialize)]
struct TorCheckResponse {
    #[serde(rename = "IsTor")]
    is_tor: bool,
    #[serde(rename = "IP")]
    ip: String,
}

pub async fn check_private_search(settings: &AppSettings) -> AppResult<()> {
    let _ = ensure_local_endpoint(settings.web_search_url.trim(), "web search")?;
    let _ = ensure_local_proxy(settings.web_proxy_url.trim())?;

    let local_client = build_local_client(Duration::from_secs(5))?;
    let local_url = Url::parse(settings.web_search_url.trim())
        .map_err(|error| AppError::WebResearch(error.to_string()))?;
    let response = local_client
        .get(local_url)
        .send()
        .await
        .map_err(|error| AppError::WebResearch(format!("private web gateway is unreachable: {error}")))?;

    if !(response.status().is_success()
        || response.status() == StatusCode::NOT_FOUND
        || response.status() == StatusCode::METHOD_NOT_ALLOWED)
    {
        return Err(AppError::WebResearch(format!(
            "private web gateway returned HTTP {}",
            response.status()
        )));
    }

    let proxy_client = build_client(settings, Duration::from_secs(20))?;
    let tor_response = proxy_client
        .get("https://check.torproject.org/api/ip")
        .send()
        .await
        .map_err(|error| AppError::WebResearch(format!("local Tor proxy is unreachable: {error}")))?;

    if !tor_response.status().is_success() {
        return Err(AppError::WebResearch(format!(
            "Tor connectivity check returned HTTP {}",
            tor_response.status()
        )));
    }

    let tor = tor_response
        .json::<TorCheckResponse>()
        .await
        .map_err(|error| AppError::WebResearch(format!("invalid Tor connectivity response: {error}")))?;

    if !tor.is_tor {
        return Err(AppError::WebResearch(
            "the configured local proxy did not produce a Tor-routed connection; refusing private research"
                .into(),
        ));
    }

    if tor.ip.trim().is_empty() {
        return Err(AppError::WebResearch(
            "Tor connectivity check did not return an exit IP".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_search_endpoints_are_rejected() {
        assert!(ensure_local_endpoint("https://example.com", "web search").is_err());
        assert!(ensure_local_proxy("socks5h://example.com:9050").is_err());
    }

    #[test]
    fn source_validation_rejects_local_targets() {
        assert!(validate_source_url("http://127.0.0.1:8188").is_err());
        assert!(validate_source_url("http://localhost/internal").is_err());
        assert!(validate_source_url("https://127.0.0.1/example").is_err());
    }

    #[test]
    fn research_merge_rewrites_source_ids() {
        let mut target = ResearchBundle {
            queries: vec!["first".into()],
            sources: vec![ResearchSource {
                id: "S1".into(),
                title: "Existing".into(),
                url: "https://example.org/existing".into(),
                snippet: String::new(),
                content: String::new(),
            }],
            facts: Vec::new(),
            retrieved_at: "unix:1".into(),
        };

        let incoming = ResearchBundle {
            queries: vec!["second".into()],
            sources: vec![
                ResearchSource {
                    id: "S1".into(),
                    title: "Existing".into(),
                    url: "https://example.org/existing".into(),
                    snippet: String::new(),
                    content: String::new(),
                },
                ResearchSource {
                    id: "S2".into(),
                    title: "New".into(),
                    url: "https://example.org/new".into(),
                    snippet: String::new(),
                    content: String::new(),
                },
            ],
            facts: vec![ResearchFact {
                claim: "A sourced fact".into(),
                evidence: "Evidence".into(),
                source_ids: vec!["S2".into()],
                confidence: "high".into(),
            }],
            retrieved_at: "unix:2".into(),
        };

        merge_into(&mut target, incoming);

        assert_eq!(target.sources.len(), 2);
        assert_eq!(target.sources[1].id, "S2");
        assert_eq!(target.facts[0].source_ids, vec!["S2"]);
        assert!(target.queries.contains(&"second".to_string()));
    }
}
