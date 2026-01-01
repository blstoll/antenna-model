#!/usr/bin/env bash
# Analyze load test results and generate summary report
# Usage: ./analyze_results.sh <results_directory>

set -euo pipefail

RESULTS_DIR="${1:-.}"

if [ ! -d "$RESULTS_DIR" ]; then
    echo "Error: Results directory not found: $RESULTS_DIR"
    exit 1
fi

echo "Antenna Model Service - Load Test Summary Report"
echo "================================================="
echo ""
echo "Generated: $(date)"
echo "Results Directory: $RESULTS_DIR"
echo ""

# Find all scenario result directories
SCENARIO_DIRS=$(find "$RESULTS_DIR" -type d -name "*_20*" | sort)

if [ -z "$SCENARIO_DIRS" ]; then
    echo "No test results found in $RESULTS_DIR"
    exit 0
fi

for dir in $SCENARIO_DIRS; do
    SCENARIO=$(basename "$dir" | sed 's/_20.*//')
    echo "Scenario: $SCENARIO"
    echo "----------------------------------------"

    # Check if results.json exists
    if [ -f "$dir/results.json" ]; then
        # Extract key metrics from k6 JSON output using jq if available
        if command -v jq &> /dev/null; then
            echo "K6 Metrics:"

            # HTTP request duration
            if jq -e '.metrics.http_req_duration' "$dir/results.json" > /dev/null 2>&1; then
                echo "  HTTP Request Duration:"
                jq -r '.metrics.http_req_duration.values |
                    "    avg:    \(.avg | tostring | .[0:6])ms\n" +
                    "    min:    \(.min | tostring | .[0:6])ms\n" +
                    "    med:    \(.med | tostring | .[0:6])ms\n" +
                    "    max:    \(.max | tostring | .[0:6])ms\n" +
                    "    p(90):  \(.["p(90)"] | tostring | .[0:6])ms\n" +
                    "    p(95):  \(.["p(95)"] | tostring | .[0:6])ms\n" +
                    "    p(99):  \(.["p(99)"] | tostring | .[0:6])ms"' \
                    "$dir/results.json"
            fi

            # Request rate
            if jq -e '.metrics.http_reqs' "$dir/results.json" > /dev/null 2>&1; then
                TOTAL_REQS=$(jq -r '.metrics.http_reqs.values.count' "$dir/results.json")
                RATE=$(jq -r '.metrics.http_reqs.values.rate' "$dir/results.json")
                echo "  HTTP Requests:"
                echo "    total:  $TOTAL_REQS"
                echo "    rate:   $(printf "%.2f" "$RATE") req/s"
            fi

            # Error rate
            if jq -e '.metrics.http_req_failed' "$dir/results.json" > /dev/null 2>&1; then
                ERROR_RATE=$(jq -r '.metrics.http_req_failed.values.rate * 100' "$dir/results.json")
                echo "  Error Rate: $(printf "%.2f" "$ERROR_RATE")%"
            fi

            # VUs
            if jq -e '.metrics.vus' "$dir/results.json" > /dev/null 2>&1; then
                MAX_VUS=$(jq -r '.metrics.vus.values.max' "$dir/results.json")
                echo "  Max VUs: $MAX_VUS"
            fi

            # Custom metrics
            for metric in gain_latency batch_latency heatmap_latency; do
                if jq -e ".metrics.$metric" "$dir/results.json" > /dev/null 2>&1; then
                    echo "  $metric:"
                    jq -r ".metrics.$metric.values | \"    p(95):  \(.\"p(95)\" | tostring | .[0:7])ms\n    p(99):  \(.\"p(99)\" | tostring | .[0:7])ms\"" \
                        "$dir/results.json"
                fi
            done

        else
            echo "  (Install jq for detailed metrics analysis)"
            echo "  Raw results: $dir/results.json"
        fi
    fi

    # Analyze resource usage if available
    if [ -f "$dir/resources.csv" ]; then
        echo ""
        echo "Resource Usage:"

        # Use awk to compute statistics
        awk -F',' 'NR>1 {
            cpu_sum += $2; cpu_count++;
            if ($2 > cpu_max || cpu_max == 0) cpu_max = $2;
            if ($2 < cpu_min || cpu_min == 0) cpu_min = $2;

            rss_sum += $3; rss_count++;
            if ($3 > rss_max || rss_max == 0) rss_max = $3;
            if ($3 < rss_min || rss_min == 0) rss_min = $3;

            if ($5 > threads_max || threads_max == 0) threads_max = $5;
            if ($6 > files_max || files_max == 0) files_max = $6;
        }
        END {
            printf "  CPU Usage:\n";
            printf "    avg:    %.2f%%\n", cpu_sum/cpu_count;
            printf "    min:    %.2f%%\n", cpu_min;
            printf "    max:    %.2f%%\n", cpu_max;
            printf "  Memory (RSS):\n";
            printf "    avg:    %.2f MB\n", rss_sum/rss_count;
            printf "    min:    %.2f MB\n", rss_min;
            printf "    max:    %.2f MB\n", rss_max;
            printf "  Max Threads: %d\n", threads_max;
            printf "  Max Open Files: %d\n", files_max;
        }' "$dir/resources.csv"
    fi

    echo ""
    echo ""
done

# Overall summary
echo "Overall Summary"
echo "==============="
echo ""

if command -v jq &> /dev/null; then
    echo "Performance Targets vs Actual:"
    echo ""
    echo "Target                          | Expected      | Status"
    echo "--------------------------------|---------------|--------"

    # Check p95 latency across all scenarios
    for dir in $SCENARIO_DIRS; do
        if [ -f "$dir/results.json" ]; then
            SCENARIO=$(basename "$dir" | sed 's/_20.*//')
            P95=$(jq -r '.metrics.http_req_duration.values["p(95)"]' "$dir/results.json" 2>/dev/null || echo "N/A")

            if [ "$P95" != "N/A" ] && [ "$P95" != "null" ]; then
                STATUS="✓ PASS"
                if (( $(echo "$P95 > 100" | bc -l) )); then
                    STATUS="✗ FAIL"
                fi
                printf "%-30s  | <100ms        | %s (%.2fms)\n" "$SCENARIO p95 latency" "$STATUS" "$P95"
            fi
        fi
    done

    # Check error rates
    for dir in $SCENARIO_DIRS; do
        if [ -f "$dir/results.json" ]; then
            SCENARIO=$(basename "$dir" | sed 's/_20.*//')
            ERROR_RATE=$(jq -r '.metrics.http_req_failed.values.rate * 100' "$dir/results.json" 2>/dev/null || echo "N/A")

            if [ "$ERROR_RATE" != "N/A" ] && [ "$ERROR_RATE" != "null" ]; then
                STATUS="✓ PASS"
                if (( $(echo "$ERROR_RATE > 1" | bc -l) )); then
                    STATUS="✗ FAIL"
                fi
                printf "%-30s  | <1%%           | %s (%.2f%%)\n" "$SCENARIO error rate" "$STATUS" "$ERROR_RATE"
            fi
        fi
    done

    echo ""
fi

echo "Recommendations:"
echo "----------------"
echo "1. Review p95 and p99 latencies to ensure SLA compliance"
echo "2. Check resource usage trends for capacity planning"
echo "3. Investigate any error rates > 1%"
echo "4. Monitor memory growth for potential leaks"
echo "5. Review thread and file descriptor counts for resource limits"
echo ""
echo "For detailed results, see individual scenario directories in:"
echo "$RESULTS_DIR"
