# Multi-stage Dockerfile for GlowBot
# Stage 1: Build
FROM rust:1.95-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

RUN cargo build --release

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
