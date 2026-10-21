# syntax=docker/dockerfile:1

# ── Build Stage ──────────────────────────────────────────────────────────────
# Multi-stage build optimized for production deployment.
# Uses layer caching to speed up rebuilds when only source code changes.

FROM rust:1.85-bookworm@sha256:e51d0265072d2d9d5d320f6a44dde6b9ef13653b035098febd68cce8fa7c0bc4 AS builder
WORKDIR /app

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies separately
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code
COPY src ./src

# Build the application with optimizations
RUN cargo build --release --locked && \
    strip /app/target/release/predifi-backend

# ── Runtime Stage ────────────────────────────────────────────────────────────
# Minimal runtime image using debian:bookworm-slim to keep size minimal.

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS runtime
WORKDIR /app

# Install only runtime dependencies, remove package manager caches and unnecessary packages
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root system user and dedicated writable directory with explicit permissions
RUN useradd -r -u 10001 -s /usr/sbin/nologin predifi && \
    mkdir -p /tmp/predifi && \
    chown -R predifi:predifi /tmp/predifi && \
    chmod 700 /tmp/predifi

# Copy binary from builder
COPY --from=builder /app/target/release/predifi-backend /usr/local/bin/predifi-backend

# Explicit writable volume path for temporary files if needed under read-only root filesystems
VOLUME ["/tmp/predifi"]

# Switch to non-root user (numeric UID for strict container orchestrator security policies)
USER 10001:10001

# Environment configuration
ENV PREDIFI_APP_HOST=0.0.0.0
ENV PREDIFI_APP_PORT=3000
ENV RUST_LOG=info
ENV TMPDIR=/tmp/predifi

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/predifi-backend", "--version"] || exit 1

EXPOSE 3000

CMD ["predifi-backend"]
