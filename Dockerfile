# syntax=docker/dockerfile:1

# ---------- Build stage ----------
FROM rust:1.92-bookworm AS builder

WORKDIR /build

# System deps (certs only; reqwest defaults to rustls in this project)
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifest first to leverage Docker layer caching for deps
COPY Cargo.toml Cargo.lock ./
# Copy sources
COPY src ./src


# Build release binary and strip symbols to reduce size
RUN cargo build --release --locked \
    && strip target/release/routiium

# ---------- Runtime stage ----------
FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

# Create non-root user and prepare runtime packages
RUN useradd -u 10001 -ms /bin/bash app \
    && ln -fs /usr/share/zoneinfo/$TZ /etc/localtime \
    && echo $TZ > /etc/timezone \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data /app \
    && chown -R app:app /data /app

WORKDIR /app

# Copy the built binary
COPY --from=builder /build/target/release/routiium /usr/local/bin/routiium
COPY mcp.json.example /opt/routiium/examples/mcp.json
COPY system_prompt.json.example /opt/routiium/examples/system_prompt.json
COPY routing.json.example /opt/routiium/examples/routing.json
COPY routiium.yaml.example /opt/routiium/examples/routiium.yaml

# Run as non-root
USER app

# Sensible defaults (override via `docker run -e KEY=VAL ...`)
ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8088 \
    ROUTIIUM_SLED_PATH=/data/keys.db \
    ROUTIIUM_ANALYTICS_JSONL_PATH=/data/analytics.jsonl \
    ROUTIIUM_CHAT_HISTORY_JSONL_PATH=/data/chat_history.jsonl

# Persist sled data outside the container. Mount runtime YAML at /config/routiium.yaml when used.
VOLUME ["/data", "/config"]

# The service listens on 8088 by default
EXPOSE 8088

# Entrypoint: pass CLI args after `serve`, e.g.:
#   docker run ... routiium serve --keys-backend=redis://127.0.0.1/
#   docker run ... routiium serve --config-yaml=/config/routiium.yaml
ENTRYPOINT ["routiium"]
CMD ["serve"]
