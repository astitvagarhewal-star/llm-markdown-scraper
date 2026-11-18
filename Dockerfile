FROM rust:1.85-slim AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
COPY static ./static
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/llm-markdown-scraper /usr/local/bin/llm-markdown-scraper
ENV PORT=3000
EXPOSE 3000
CMD ["llm-markdown-scraper"]
