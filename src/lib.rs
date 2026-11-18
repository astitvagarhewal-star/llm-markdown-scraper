use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use scraper::{Html as ScraperHtml, Selector};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, time::Instant};
use tower_http::cors::CorsLayer;
use url::Url;

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
}

#[derive(Debug, Deserialize)]
pub struct ScrapeRequest {
    pub url: String,
    #[serde(default = "default_true")]
    pub strip_nav: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScrapeResponse {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub markdown: String,
    pub word_count: usize,
    pub char_count: usize,
    pub estimated_tokens: usize,
    pub latency_ms: u128,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/scrape", post(scrape_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "llm-markdown-scraper",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Validates target URL and blocks private/loopback IP ranges to prevent SSRF.
pub fn validate_target_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty.".into());
    }

    let parsed = Url::parse(trimmed).map_err(|e| format!("Invalid URL syntax: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("Unsupported URL scheme '{}'. Use http or https.", other)),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL must contain a valid host.".to_string())?;

    let lower_host = host.to_ascii_lowercase();
    if lower_host == "localhost"
        || lower_host.ends_with(".local")
        || lower_host.ends_with(".internal")
    {
        return Err("Local or internal hostnames are blocked for security.".into());
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_or_loopback(&ip) {
            return Err("Private or loopback IP addresses are blocked for security.".into());
        }
    }

    Ok(parsed)
}

fn is_private_or_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Extracts page title, meta description, and converts main semantic HTML to clean Markdown.
pub fn extract_clean_markdown(raw_html: &str, strip_nav: bool) -> (String, Option<String>, String) {
    let document = ScraperHtml::parse_document(raw_html);

    let title_sel = Selector::parse("title").unwrap();
    let title = document
        .select(&title_sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Untitled Document".into());

    let desc_sel = Selector::parse("meta[name='description'], meta[property='og:description']").unwrap();
    let description = document
        .select(&desc_sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());

    let target_html = if strip_nav {
        let priority_selectors = ["article", "main", "[role='main']", ".content", "#content", "body"];
        priority_selectors
            .iter()
            .find_map(|sel_str| {
                let sel = Selector::parse(sel_str).ok()?;
                document.select(&sel).next().map(|el| el.inner_html())
            })
            .unwrap_or_else(|| raw_html.to_string())
    } else {
        raw_html.to_string()
    };

    let raw_md = html2md::parse_html(&target_html);

    // Collapse excessive blank lines while preserving valid Markdown paragraph breaks
    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut consecutive_empty = 0;

    for line in raw_md.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 1 && !cleaned_lines.is_empty() {
                cleaned_lines.push(String::new());
            }
        } else {
            consecutive_empty = 0;
            cleaned_lines.push(trimmed_end.to_string());
        }
    }

    while cleaned_lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        cleaned_lines.pop();
    }

    (title, description, cleaned_lines.join("
"))
}

async fn scrape_handler(
    State(state): State<AppState>,
    Json(payload): Json<ScrapeRequest>,
) -> Result<Json<ScrapeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let start = Instant::now();

    let parsed_url = validate_target_url(&payload.url).map_err(|msg| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: msg }),
        )
    })?;

    let response = state
        .client
        .get(parsed_url.as_str())
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Upstream fetch failed: {}", e),
                }),
            )
        })?;

    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: format!("Target site returned HTTP {}", response.status()),
            }),
        ));
    }

    let raw_html = response.text().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: format!("Failed to decode response body: {}", e),
            }),
        )
    })?;

    let (title, description, markdown) = extract_clean_markdown(&raw_html, payload.strip_nav);

    let char_count = markdown.chars().count();
    let word_count = markdown.split_whitespace().count();
    let estimated_tokens = (char_count as f64 / 4.0).ceil() as usize;
    let latency_ms = start.elapsed().as_millis();

    Ok(Json(ScrapeResponse {
        url: parsed_url.to_string(),
        title,
        description,
        markdown,
        word_count,
        char_count,
        estimated_tokens,
        latency_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn test_url_validation_accepts_public_https() {
        assert!(validate_target_url("https://example.com/blog").is_ok());
        assert!(validate_target_url("http://rust-lang.org").is_ok());
    }

    #[test]
    fn test_url_validation_blocks_ssrf_and_bad_schemes() {
        assert!(validate_target_url("file:///etc/passwd").is_err());
        assert!(validate_target_url("http://localhost:8080").is_err());
        assert!(validate_target_url("http://127.0.0.1/admin").is_err());
        assert!(validate_target_url("http://192.168.1.1").is_err());
        assert!(validate_target_url("").is_err());
    }

    #[test]
    fn test_markdown_extraction_strips_nav_and_keeps_article() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Rust Async Guide</title>
                <meta name="description" content="Learn async Rust fast." />
            </head>
            <body>
                <nav><a href="/">Home</a> | <a href="/login">Login</a></nav>
                <article>
                    <h1>Understanding Tokio</h1>
                    <p>Tokio is an event-driven runtime for Rust.</p>
                </article>
            </body>
            </html>
        "#;

        let (title, desc, md) = extract_clean_markdown(html, true);
        assert_eq!(title, "Rust Async Guide");
        assert_eq!(desc.as_deref(), Some("Learn async Rust fast."));
        assert!(md.contains("Understanding Tokio"));
        assert!(!md.contains("Login"));
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = AppState {
            client: Client::new(),
        };
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }
}
