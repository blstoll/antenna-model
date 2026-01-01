#!/usr/bin/env bash
# Resource monitoring script for load testing
# Usage: ./monitor_resources.sh <pid> <output_file> <interval_seconds>
#
# Example: ./monitor_resources.sh 12345 resources.csv 1

set -euo pipefail

PID=${1:-}
OUTPUT_FILE=${2:-"resources.csv"}
INTERVAL=${3:-1}

if [ -z "$PID" ]; then
    echo "Usage: $0 <pid> [output_file] [interval_seconds]"
    echo ""
    echo "Monitor resource usage of a process during load testing"
    exit 1
fi

# Check if process exists
if ! ps -p "$PID" > /dev/null 2>&1; then
    echo "Error: Process $PID not found"
    exit 1
fi

echo "Monitoring process $PID, writing to $OUTPUT_FILE every ${INTERVAL}s"
echo "Press Ctrl+C to stop"

# Write header
echo "timestamp,cpu_percent,mem_rss_mb,mem_vsz_mb,threads,open_files" > "$OUTPUT_FILE"

# Trap Ctrl+C for clean exit
trap 'echo ""; echo "Monitoring stopped. Results in $OUTPUT_FILE"; exit 0' INT

# Monitor loop
while ps -p "$PID" > /dev/null 2>&1; do
    TIMESTAMP=$(date +%s)

    # Get CPU and memory stats (macOS compatible)
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS version using ps
        STATS=$(ps -p "$PID" -o %cpu,rss,vsz | tail -1)
        CPU=$(echo "$STATS" | awk '{print $1}')
        RSS_KB=$(echo "$STATS" | awk '{print $2}')
        VSZ_KB=$(echo "$STATS" | awk '{print $3}')
        RSS_MB=$(echo "scale=2; $RSS_KB / 1024" | bc)
        VSZ_MB=$(echo "scale=2; $VSZ_KB / 1024" | bc)

        # Get thread count
        THREADS=$(ps -M -p "$PID" | wc -l | tr -d ' ')
        THREADS=$((THREADS - 1))  # Subtract header line

        # Get open files count
        OPEN_FILES=$(lsof -p "$PID" 2>/dev/null | wc -l | tr -d ' ')
        OPEN_FILES=$((OPEN_FILES - 1))  # Subtract header line
    else
        # Linux version
        STATS=$(ps -p "$PID" -o %cpu,rss,vsz --no-headers)
        CPU=$(echo "$STATS" | awk '{print $1}')
        RSS_KB=$(echo "$STATS" | awk '{print $2}')
        VSZ_KB=$(echo "$STATS" | awk '{print $3}')
        RSS_MB=$(echo "scale=2; $RSS_KB / 1024" | bc)
        VSZ_MB=$(echo "scale=2; $VSZ_KB / 1024" | bc)

        # Get thread count
        THREADS=$(ps -p "$PID" -o nlwp --no-headers | tr -d ' ')

        # Get open files count
        OPEN_FILES=$(ls -1 /proc/"$PID"/fd 2>/dev/null | wc -l)
    fi

    # Write data
    echo "$TIMESTAMP,$CPU,$RSS_MB,$VSZ_MB,$THREADS,$OPEN_FILES" >> "$OUTPUT_FILE"

    # Print current stats
    printf "\r[%s] CPU: %6.2f%% | RSS: %8.2f MB | VSZ: %8.2f MB | Threads: %4d | Files: %4d" \
        "$(date +%H:%M:%S)" "$CPU" "$RSS_MB" "$VSZ_MB" "$THREADS" "$OPEN_FILES"

    sleep "$INTERVAL"
done

echo ""
echo "Process $PID has terminated"
