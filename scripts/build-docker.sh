#!/usr/bin/env bash
# =============================================================================
# Docker Image Build Script for Antenna Model Service
# =============================================================================
#
# Usage:
#   ./scripts/build-docker.sh [OPTIONS]
#
# Options:
#   --tag TAG        Custom image tag (default: latest)
#   --version VER    Version number (default: from Cargo.toml)
#   --no-cache       Build without using cache
#   --push           Push image to registry after building
#   --registry REG   Registry URL (e.g., ghcr.io/org)
#   --platform PLAT  Target platform (default: linux/amd64)
#   --help           Show this help message
#
# Examples:
#   ./scripts/build-docker.sh
#   ./scripts/build-docker.sh --tag v0.1.0
#   ./scripts/build-docker.sh --version 0.1.0 --push --registry ghcr.io/myorg
#   ./scripts/build-docker.sh --platform linux/arm64
#
# =============================================================================

set -euo pipefail

# Default configuration
IMAGE_NAME="antenna-model"
TAG="${TAG:-latest}"
VERSION="${VERSION:-0.1.0}"
NO_CACHE=""
PUSH=false
REGISTRY=""
PLATFORM="${PLATFORM:-linux/amd64}"
BUILDKIT_ENABLED=1

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

show_help() {
    head -n 30 "$0" | grep "^#" | sed 's/^# //' | sed 's/^#!//'
    exit 0
}

# Parse command-line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --tag)
            TAG="$2"
            shift 2
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --no-cache)
            NO_CACHE="--no-cache"
            shift
            ;;
        --push)
            PUSH=true
            shift
            ;;
        --registry)
            REGISTRY="$2"
            shift 2
            ;;
        --platform)
            PLATFORM="$2"
            shift 2
            ;;
        --help)
            show_help
            ;;
        *)
            log_error "Unknown option: $1"
            show_help
            ;;
    esac
done

# Construct full image name
if [[ -n "$REGISTRY" ]]; then
    FULL_IMAGE_NAME="${REGISTRY}/${IMAGE_NAME}:${TAG}"
else
    FULL_IMAGE_NAME="${IMAGE_NAME}:${TAG}"
fi

# Display build configuration
log_info "Docker Image Build Configuration"
echo "  Image Name:  ${FULL_IMAGE_NAME}"
echo "  Version:     ${VERSION}"
echo "  Platform:    ${PLATFORM}"
echo "  Build Cache: $([ -n "$NO_CACHE" ] && echo "disabled" || echo "enabled")"
echo "  Push:        ${PUSH}"
echo ""

# Check if Docker is available
if ! command -v docker &> /dev/null; then
    log_error "Docker is not installed or not in PATH"
    exit 1
fi

# Check if we're in the project root
if [[ ! -f "Cargo.toml" ]] || [[ ! -f "Dockerfile" ]]; then
    log_error "Must run from project root directory (where Cargo.toml and Dockerfile are located)"
    exit 1
fi

# Enable BuildKit for better build performance
export DOCKER_BUILDKIT=${BUILDKIT_ENABLED}

# Build the Docker image
log_info "Building Docker image..."
log_info "This may take several minutes for the first build (compiling Rust dependencies)..."

if docker build \
    --platform "${PLATFORM}" \
    --build-arg VERSION="${VERSION}" \
    --tag "${FULL_IMAGE_NAME}" \
    --tag "${IMAGE_NAME}:latest" \
    ${NO_CACHE} \
    --file Dockerfile \
    . ; then
    log_success "Docker image built successfully: ${FULL_IMAGE_NAME}"
else
    log_error "Docker build failed"
    exit 1
fi

# Display image information
log_info "Image information:"
docker images --filter "reference=${IMAGE_NAME}" --format "table {{.Repository}}\t{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}" | head -3

# Get image size in MB
IMAGE_SIZE=$(docker images "${FULL_IMAGE_NAME}" --format "{{.Size}}")
log_info "Image size: ${IMAGE_SIZE}"

# Verify image was created
if ! docker image inspect "${FULL_IMAGE_NAME}" &> /dev/null; then
    log_error "Image verification failed: ${FULL_IMAGE_NAME} not found"
    exit 1
fi

# Tag additional versions if needed
if [[ "${TAG}" != "latest" ]] && [[ "${TAG}" != "${VERSION}" ]]; then
    log_info "Tagging image with version: ${VERSION}"
    docker tag "${FULL_IMAGE_NAME}" "${IMAGE_NAME}:${VERSION}"
fi

# Push to registry if requested
if [[ "${PUSH}" == true ]]; then
    if [[ -z "$REGISTRY" ]]; then
        log_error "Cannot push: --registry not specified"
        exit 1
    fi

    log_info "Pushing image to registry: ${REGISTRY}"
    if docker push "${FULL_IMAGE_NAME}"; then
        log_success "Image pushed successfully: ${FULL_IMAGE_NAME}"
    else
        log_error "Docker push failed"
        exit 1
    fi
fi

# Summary
echo ""
log_success "Build completed successfully!"
echo ""
echo "Next steps:"
echo "  1. Test the image locally:"
echo "     docker run --rm -p 3000:3000 ${FULL_IMAGE_NAME}"
echo ""
echo "  2. Or use docker-compose:"
echo "     docker-compose up"
echo ""
echo "  3. Test the health endpoint:"
echo "     curl http://localhost:3000/health"
echo ""
echo "  4. View logs:"
echo "     docker logs <container-id>"
echo ""

# Optional: Run quick validation test
read -p "Run quick validation test? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    log_info "Starting container for validation..."
    CONTAINER_ID=$(docker run -d -p 3000:3000 "${FULL_IMAGE_NAME}")

    log_info "Container started: ${CONTAINER_ID:0:12}"
    log_info "Waiting for service to start (10 seconds)..."
    sleep 10

    log_info "Testing /health endpoint..."
    if curl -f -s http://localhost:3000/health > /dev/null; then
        log_success "Health check passed!"
    else
        log_warn "Health check failed or service not ready"
    fi

    log_info "Stopping test container..."
    docker stop "${CONTAINER_ID}" > /dev/null
    docker rm "${CONTAINER_ID}" > /dev/null

    log_success "Validation complete"
fi

exit 0
