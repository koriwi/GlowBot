# Multi-stage Dockerfile for GlowBot
# Stage 1: Build
FROM rust:1.95-bookworm AS builder

WORKDIR /app

# Cache dependency builds: copy manifests first, build deps with dummy src,
# then copy real source and build the app. This way dependency compilation
# is cached in a Docker layer and only re-runs when Cargo.toml changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/lib.rs && \
    cargo build --release && \
    rm -rf src

COPY src ./src
# Touch lib.rs/main.rs so cargo rebuilds the app (deps stay cached)
RUN touch src/lib.rs src/main.rs && \
    cargo build --release

# Stage 2: Slim runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    curl \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash glowbot
USER glowbot
WORKDIR /home/glowbot

# Copy the binary
COPY --from=builder /app/target/release/glowbot /usr/local/bin/glowbot

# Create data directory
RUN mkdir -p /home/glowbot/glowbot_data

ENV GLOWBOT_DATA_DIR=/home/glowbot/glowbot_data

ENTRYPOINT ["glowbot"]
