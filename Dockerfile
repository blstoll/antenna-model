# Multi-stage Dockerfile for Antenna Model Service
# Uses Red Hat Universal Base Image 9 (UBI9) minimal for minimal runtime footprint with glibc

# =============================================================================
# Build Stage: Compile Rust binaries with optimizations
# =============================================================================
# Use rust:latest to ensure compatibility with edition2024 dependencies
FROM rust:latest AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /build

# Copy all source code
# Note: We copy everything at once for simplicity. For larger projects,
# consider using cargo-chef for better layer caching of dependencies.
COPY . .

# Build release binary with full optimizations
# Profile settings from Cargo.toml: LTO=true, strip=true, opt-level=3
# Use --bin to build only the binary, skipping tests and benchmarks
# Note: --locked removed to allow dependency resolution with current Rust stable
RUN cargo build --release --bin antenna-model

# Verify binary was created
RUN ls -lh /build/target/release/antenna-model

# Strip binary for minimal size (already done by profile, but ensure it)
RUN strip /build/target/release/antenna-model 2>/dev/null || true

# =============================================================================
# Runtime Stage: Minimal UBI9 minimal image
# =============================================================================
# ubi-minimal provides glibc and basic utilities while staying under 100MB
FROM registry.access.redhat.com/ubi9/ubi-minimal:latest

# Metadata labels
LABEL name="antenna-model-service" \
      vendor="Antenna Model Team" \
      version="0.1.0" \
      summary="Antenna Model Service - Physical Optics Computation API" \
      description="High-performance REST API for parabolic dish antenna gain modeling using physical optics computation"

# Create non-root user and app directory
# UBI micro doesn't have useradd, so we use numeric UID/GID
USER 0
RUN mkdir -p /app/calibration_data /app/config && \
    chown -R 1000:1000 /app

# Copy compiled binary from builder
COPY --from=builder --chown=1000:1000 /build/target/release/antenna-model /app/antenna-model

# Copy runtime configuration and calibration data
COPY --chown=1000:1000 config/ /app/config/
COPY --chown=1000:1000 calibration_data/ /app/calibration_data/

# Set working directory
WORKDIR /app

# Switch to non-root user
USER 1000

# Expose service port (default: 3000)
EXPOSE 3000

# Environment variables with defaults
ENV SERVICE_HOST=0.0.0.0 \
    SERVICE_PORT=3000 \
    RUST_LOG=info \
    CONFIG_PATH=/app/config/service.yaml

# Health check configuration
# Note: UBI micro doesn't include curl/wget for HTTP health checks
# Define health checks in docker-compose.yml or Kubernetes manifests using:
#   HTTP GET http://localhost:3000/health (liveness)
#   HTTP GET http://localhost:3000/ready (readiness)

# Run the antenna model service
# Use exec form to ensure proper signal handling
CMD ["/app/antenna-model"]
