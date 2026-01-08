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

# Run as non-root
USER app

# Sensible defaults (override via `docker run -e KEY=VAL ...`)
ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8088 \
    ROUTIIUM_SLED_PATH=/data/keys.db

# Persist sled data outside the container
VOLUME ["/data"]

# The service listens on 8088 by default
EXPOSE 8088

# Entrypoint: pass CLI args to select backend or MCP config, e.g.:
#   docker run ... routiium --keys-backend=redis://127.0.0.1/
#   docker run ... routiium --mcp-config=mcp.json
ENTRYPOINT ["routiium"]
CMD []
