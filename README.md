# Rust LLM Web Scraper & Markdown Extractor

A high-throughput, memory-safe web scraper and HTML-to-Markdown extraction engine written in **Rust (Axum + Tokio)**. Designed for AI/RAG pipelines that need clean, token-counted Markdown from noisy webpages in milliseconds.

## Key Engineering Highlights
- **Modular Architecture (`src/lib.rs` + `src/main.rs`):** Separates HTTP routing, SSRF URL validation, and HTML DOM extraction from server startup and graceful shutdown (`SIGINT`/`SIGTERM`).
- **SSRF Protection:** Blocks `localhost`, `.local`, `.internal`, non-HTTP(S) schemes, and private/loopback IPv4/IPv6 ranges before issuing outbound requests.
- **Semantic Content Isolation:** Targets `<article>`, `<main>`, and `[role="main"]` nodes to strip navigation bars and boilerplate clutter before Markdown conversion.
- **Built-in Interactive Playground:** Ships with a single-binary embedded web UI (`/`) and JSON REST API (`POST /api/scrape`), plus unit tests and GitHub Actions CI.

## API Usage

```bash
curl -X POST http://localhost:3000/api/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "strip_nav": true}'
```

## Run Locally (Arch Linux / Any OS with Rust)

```bash
cargo test
cargo run --release
# Visit http://localhost:3000
```

## Deploy with Docker / Vercel / Render

```bash
docker build -t llm-markdown-scraper .
docker run -p 3000:3000 llm-markdown-scraper
```
