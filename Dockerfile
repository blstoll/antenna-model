# Multi-stage Dockerfile for Antenna Model Service
# Uses Red Hat Universal Base Image 9 (UBI9) minimal for minimal runtime footprint with glibc

# =============================================================================
# Build Stage: Compile Rust binaries with optimizations
# =============================================================================
# The builder MUST be built on the same glibc as the runtime stage below.
#
# This used to be `FROM rust:latest`, which is Debian: glibc 2.41 on trixie,
# 2.36 on bookworm before it. The runtime is UBI9, which is glibc 2.34. The
# binary therefore linked symbols the runtime did not have, and every image
# this Dockerfile produced died on startup with
#
#   /app/antenna-model: /lib64/libm.so.6: version `GLIBC_2.35' not found
#
# The build succeeded, so nothing caught it until the image was actually run.
# Building the toolchain onto UBI9 itself makes the ABI match by construction
# rather than by a version coincidence that Debian can break again.
FROM registry.access.redhat.com/ubi9/ubi:latest AS builder

# Build dependencies: gcc only, and it is here as the LINKER DRIVER (rustc shells
# out to `cc` to link), not to compile any C.
#
# `cargo tree -p antenna-model -e normal` contains no `-sys` crate at all, so
# nothing in the shipped binary builds native code. An earlier version of this
# layer also installed gcc-c++, make, cmake and perl, justified as "for the
# aws-lc-sys and ring native builds" -- but neither crate is in the normal graph.
# rustls and reqwest reach the lockfile only through dev-dependencies (reqwest
# with default-features = false), and `cargo build --bin` never builds those.
# Verified by building this image with gcc alone.
#
# No openssl-devel or pkg-config either: nothing links openssl.
RUN dnf install -y --setopt=install_weak_deps=False \
    gcc \
    && dnf clean all && rm -rf /var/cache/dnf

# Install the Rust toolchain, pinned. `stable` floats, so the same commit could
# compile under a different compiler on a later rebuild -- which defeats the point
# of the stage header above: the image is meant to be what it is by construction,
# not by whatever the day's stable happens to be. Bump this alongside CI
# (.github/workflows/ci.yml uses dtolnay/rust-toolchain@stable); override for a
# one-off with --build-arg RUST_VERSION=...
ARG RUST_VERSION=1.96.1
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}"
ENV PATH=/root/.cargo/bin:$PATH

# Set working directory
WORKDIR /build

# Copy all source code
# Note: We copy everything at once for simplicity. For larger projects,
# consider using cargo-chef for better layer caching of dependencies.
COPY . .

# Build release binary with full optimizations
# Profile settings from Cargo.toml: LTO=true, strip=true, opt-level=3
# Use --bin to build only the binary, skipping tests and benchmarks
# --locked: build the dependency versions Cargo.lock records, not whatever
# resolves today. Together with the pinned RUST_VERSION above, an identical
# commit produces an identical build. (This was dropped at one point to work
# around resolution against a floating stable; the pin removes that reason.)
RUN cargo build --locked --release --bin antenna-model

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
# org.opencontainers.image.source is what links the published package to this
# repository on ghcr.io. Without it the package shows up unlinked, and it does
# not inherit the repository's visibility or its permissions. The CI workflow
# also injects this label, but keeping it here means a local `docker build`
# produces an image that links too.
LABEL org.opencontainers.image.source="https://github.com/blstoll/antenna-model" \
      name="antenna-model-service" \
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

# Health check configuration
# ubi-minimal ships curl (curl-minimal) but no wget -- a wget-based check cannot
# run in this image. Health checks are defined by the orchestrator:
# docker-compose.yml uses `curl -sf`, and Kubernetes probes go straight to
#   HTTP GET http://localhost:3000/health (liveness)
#   HTTP GET http://localhost:3000/ready (readiness)

# Run the antenna model service
# Use exec form to ensure proper signal handling
CMD ["/app/antenna-model"]
