#!/bin/bash
# Antenna Model Service - API Examples
#
# This script demonstrates all API endpoints with curl commands.
# Ensure the service is running before executing: cargo run --release --bin antenna-model

# Configuration
API_BASE_URL="${API_BASE_URL:-http://localhost:3000}"
EXAMPLES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Helper function to print section headers
print_header() {
    echo -e "\n${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}\n"
}

# Helper function to print success
print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

# Helper function to print info
print_info() {
    echo -e "${YELLOW}→ $1${NC}"
}

# Helper function to execute curl with formatting
execute_curl() {
    local description="$1"
    local curl_cmd="$2"

    print_info "$description"
    echo -e "${GREEN}Command:${NC} $curl_cmd\n"

    # Execute and format JSON output
    response=$(eval "$curl_cmd" 2>&1)
    status=$?

    if [ $status -eq 0 ]; then
        echo "$response" | jq . 2>/dev/null || echo "$response"
        print_success "Request completed"
    else
        echo -e "${RED}Error: Request failed${NC}"
        echo "$response"
    fi

    echo ""
}

# Check if service is running
print_header "Checking Service Status"
if ! curl -s "$API_BASE_URL/health" > /dev/null 2>&1; then
    echo -e "${RED}Error: Service is not running at $API_BASE_URL${NC}"
    echo "Please start the service first: cargo run --release --bin antenna-model"
    exit 1
fi
print_success "Service is running at $API_BASE_URL"

# ============================================================================
# Health & Status Endpoints
# ============================================================================

print_header "1. Health Check (Liveness Probe)"
execute_curl \
    "Simple health check to verify service is alive" \
    "curl -s -w '\nHTTP Status: %{http_code}\n' $API_BASE_URL/health"

print_header "2. Readiness Check"
execute_curl \
    "Check if service is ready to accept requests" \
    "curl -s -w '\nHTTP Status: %{http_code}\n' $API_BASE_URL/ready"

print_header "3. Service Status"
execute_curl \
    "Detailed service status with version, uptime, and loaded antennas" \
    "curl -s $API_BASE_URL/status"

# ============================================================================
# Antenna Information Endpoints
# ============================================================================

print_header "4. List All Antennas"
execute_curl \
    "Get list of all available antennas" \
    "curl -s $API_BASE_URL/api/v1/antennas"

print_header "5. Get Antenna Details"
execute_curl \
    "Get detailed information about dsn_34m_uncalibrated" \
    "curl -s $API_BASE_URL/api/v1/antennas/dsn_34m_uncalibrated"

print_header "6. List Antenna Feeds"
execute_curl \
    "List all feeds for dsn_34m_uncalibrated" \
    "curl -s $API_BASE_URL/api/v1/antennas/dsn_34m_uncalibrated/feeds"

print_header "7. Get Feed Details"
execute_curl \
    "Get detailed information about x_band feed" \
    "curl -s $API_BASE_URL/api/v1/antennas/dsn_34m_uncalibrated/feeds/x_band"

# ============================================================================
# Gain Computation Endpoints
# ============================================================================

print_header "8. Single Gain Computation (ECEF Coordinates)"
execute_curl \
    "Compute gain using ECEF coordinates and quaternion attitude" \
    "curl -s -X POST $API_BASE_URL/api/v1/gain \
        -H 'Content-Type: application/json' \
        -d @$EXAMPLES_DIR/requests/gain_request.json"

print_header "9. Single Gain Computation (Geodetic Coordinates)"
execute_curl \
    "Compute gain using Geodetic coordinates with quaternion attitude" \
    "curl -s -X POST $API_BASE_URL/api/v1/gain \
        -H 'Content-Type: application/json' \
        -d @$EXAMPLES_DIR/requests/gain_request_geodetic.json"

print_header "10. Batch Gain Computation"
execute_curl \
    "Process multiple gain computations in a single request" \
    "curl -s -X POST $API_BASE_URL/api/v1/gain/batch \
        -H 'Content-Type: application/json' \
        -d @$EXAMPLES_DIR/requests/batch_request.json"

# ============================================================================
# Heatmap Endpoint
# ============================================================================

print_header "11. Generate Loss Heatmap"
execute_curl \
    "Generate a loss heatmap across antenna field of view" \
    "curl -s -X POST $API_BASE_URL/api/v1/heatmap \
        -H 'Content-Type: application/json' \
        -d @$EXAMPLES_DIR/requests/heatmap_request.json"

# ============================================================================
# Request ID Propagation Test
# ============================================================================

print_header "12. Request ID Propagation"
print_info "Testing custom request ID propagation"
echo -e "${GREEN}Command:${NC} curl -s -H 'X-Request-Id: test-custom-id-12345' $API_BASE_URL/status -v\n"

response_headers=$(curl -s -H 'X-Request-Id: test-custom-id-12345' "$API_BASE_URL/status" -v 2>&1)
echo "$response_headers" | grep -i "x-request-id" || echo "No X-Request-Id header found"
print_success "Request ID test completed"

# ============================================================================
# Error Handling Examples
# ============================================================================

print_header "13. Error Handling - Antenna Not Found"
execute_curl \
    "Request with non-existent antenna" \
    "curl -s -w '\nHTTP Status: %{http_code}\n' $API_BASE_URL/api/v1/antennas/nonexistent_antenna"

print_header "14. Error Handling - Feed Not Found"
execute_curl \
    "Request with non-existent feed" \
    "curl -s -w '\nHTTP Status: %{http_code}\n' $API_BASE_URL/api/v1/antennas/dsn_34m_uncalibrated/feeds/nonexistent_feed"

# ============================================================================
# Summary
# ============================================================================

print_header "Test Summary"
echo -e "${GREEN}All example requests completed!${NC}\n"
echo "Request files are located in: $EXAMPLES_DIR/requests/"
echo "For more information, see: $EXAMPLES_DIR/README.md"
echo ""
echo "To run individual examples:"
echo "  curl $API_BASE_URL/health"
echo "  curl -X POST $API_BASE_URL/api/v1/gain -H 'Content-Type: application/json' -d @examples/requests/gain_request.json"
echo ""
