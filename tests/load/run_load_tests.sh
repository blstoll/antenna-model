#!/usr/bin/env bash
# Comprehensive load testing runner with resource monitoring
# Usage: ./run_load_tests.sh [scenario]
#
# Scenarios: normal, peak, stress, mixed, rampup, all

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SCENARIO="${1:-all}"
BASE_URL="${BASE_URL:-http://localhost:3000}"
RESULTS_DIR="$SCRIPT_DIR/results"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if k6 is installed
if ! command -v k6 &> /dev/null; then
    log_error "k6 is not installed. Please install k6:"
    echo "  macOS:   brew install k6"
    echo "  Linux:   See https://k6.io/docs/getting-started/installation/"
    exit 1
fi

# Check if service is running
log_info "Checking if service is available at $BASE_URL..."
if ! curl -sf "$BASE_URL/health" > /dev/null 2>&1; then
    log_error "Service not available at $BASE_URL"
    log_info "Please start the service first:"
    log_info "  cargo run --release --bin antenna-model"
    exit 1
fi

log_info "Service is healthy ✓"

# Create results directory
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Function to run a single scenario
run_scenario() {
    local scenario=$1
    local output_dir="$RESULTS_DIR/${scenario}_${TIMESTAMP}"
    mkdir -p "$output_dir"

    log_info "Running scenario: $scenario"
    log_info "Results will be saved to: $output_dir"

    # Find service PID
    local service_pid
    if [[ "$OSTYPE" == "darwin"* ]]; then
        service_pid=$(lsof -ti :3000 || echo "")
    else
        service_pid=$(lsof -ti :3000 || echo "")
    fi

    if [ -z "$service_pid" ]; then
        log_warn "Could not find service PID, resource monitoring disabled"
    else
        log_info "Service PID: $service_pid"
        # Start resource monitoring in background
        "$SCRIPT_DIR/monitor_resources.sh" "$service_pid" "$output_dir/resources.csv" 2 &
        MONITOR_PID=$!
        log_info "Resource monitoring started (PID: $MONITOR_PID)"
    fi

    # Run k6 test
    log_info "Starting k6 load test..."
    BASE_URL="$BASE_URL" SCENARIO="$scenario" k6 run \
        --out json="$output_dir/results.json" \
        "$SCRIPT_DIR/load_test_scenarios.js" \
        | tee "$output_dir/output.log"

    # Stop resource monitoring
    if [ -n "${MONITOR_PID:-}" ]; then
        kill "$MONITOR_PID" 2>/dev/null || true
        wait "$MONITOR_PID" 2>/dev/null || true
        log_info "Resource monitoring stopped"
    fi

    log_info "Scenario $scenario completed ✓"
    log_info "Results saved to: $output_dir"
    echo ""
}

# Main execution
log_info "Antenna Model Service Load Testing"
log_info "===================================="
log_info "Target URL: $BASE_URL"
log_info "Scenario: $SCENARIO"
log_info "Results directory: $RESULTS_DIR"
echo ""

if [ "$SCENARIO" = "all" ]; then
    # Run all scenarios in sequence
    for scenario in normal peak stress mixed rampup; do
        run_scenario "$scenario"
        log_info "Waiting 30 seconds before next scenario..."
        sleep 30
    done
else
    # Run single scenario
    run_scenario "$SCENARIO"
fi

log_info "All load tests completed!"
log_info "Results are in: $RESULTS_DIR"

# Generate summary
log_info "Generating summary report..."
"$SCRIPT_DIR/analyze_results.sh" "$RESULTS_DIR" > "$RESULTS_DIR/summary_${TIMESTAMP}.txt"

log_info "Summary saved to: $RESULTS_DIR/summary_${TIMESTAMP}.txt"
echo ""
cat "$RESULTS_DIR/summary_${TIMESTAMP}.txt"
