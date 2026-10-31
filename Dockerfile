# ==============================================================================
# Stage 1: frontend-builder (Next.js Dashboard static export)
# ==============================================================================
FROM node:22-alpine AS frontend-builder

WORKDIR /app/dashboard

# Cache package dependencies
COPY dashboard/package*.json ./
RUN npm ci

# Copy frontend source and build static export (outputs to dashboard/out)
COPY dashboard/ ./
RUN npm run build

# ==============================================================================
# Stage 2: rust-builder (TraceForge Rust Workspace)
# ==============================================================================
FROM rust:1.85-bookworm AS rust-builder

WORKDIR /app

# Copy Cargo workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy internal crate sources and policy definitions
COPY crates/ crates/
COPY policies/ policies/

# Copy static frontend assets from frontend-builder
COPY --from=frontend-builder /app/dashboard/out ./dashboard/out

# Compile release binary and strip debug symbols
RUN cargo build --release --bin traceforge \
    && strip target/release/traceforge

# ==============================================================================
# Stage 3: runtime (Minimal Debian Bookworm Slim)
# ==============================================================================
FROM debian:bookworm-slim AS runtime

# Install TLS certificates and curl for healthcheck probing
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# Create unprivileged service user and group (UID/GID 10001)
RUN groupadd --gid 10001 traceforge \
    && useradd --uid 10001 --gid traceforge --shell /bin/false --no-create-home traceforge

# Prepare application directory and data storage volume mount point
WORKDIR /app
RUN mkdir -p /app/.traceforge \
    && chown -R traceforge:traceforge /app

# Copy release binary and runtime assets with proper ownership
COPY --chown=traceforge:traceforge policies/ /app/policies/
COPY --from=rust-builder --chown=traceforge:traceforge /app/target/release/traceforge /usr/local/bin/traceforge

# Set environment defaults
ENV TRACEFORGE_HOST=0.0.0.0 \
    TRACEFORGE_PORT=8080 \
    TRACEFORGE_DB_PATH=/app/.traceforge/traces.db

# Expose proxy and control plane port
EXPOSE 8080

# Switch to non-root user
USER traceforge

# Healthcheck configuration
HEALTHCHECK --interval=15s --timeout=3s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Launch TraceForge streaming proxy
ENTRYPOINT ["/usr/local/bin/traceforge", "run"]
