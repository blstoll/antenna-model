# Load Testing Quick Start

Quick reference for running load tests against the Antenna Model Service.

## Prerequisites

```bash
# Install k6 (macOS)
brew install k6

# Install k6 (Linux - see README.md for other distros)
sudo apt-get install k6

# Optional: Install jq for detailed analysis
brew install jq  # macOS
sudo apt-get install jq  # Linux
```

## Run Tests

### 1. Start the Service

```bash
# In one terminal
cd /path/to/antenna_model
cargo run --release --bin antenna-model
```

### 2. Run Load Tests

```bash
# In another terminal
cd /path/to/antenna_model

# Run all scenarios (recommended for first time)
./tests/load/run_load_tests.sh all

# Or run individual scenarios
./tests/load/run_load_tests.sh normal   # 10 req/s for 5 min
./tests/load/run_load_tests.sh peak     # 20 req/s for 1 min
./tests/load/run_load_tests.sh stress   # Ramp to 100 req/s
./tests/load/run_load_tests.sh mixed    # Mixed workload
./tests/load/run_load_tests.sh rampup   # Gradual ramp
```

## View Results

Results are saved in `tests/load/results/` with timestamped directories.

```bash
# View summary
cat tests/load/results/summary_*.txt

# View detailed results for a scenario
ls tests/load/results/normal_*/

# View resource usage
cat tests/load/results/normal_*/resources.csv

# View k6 metrics (requires jq)
jq '.metrics' tests/load/results/normal_*/results.json
```

## Interpret Results

### Key Metrics

**Response Time:**
- p(95) should be <100ms for single evaluations
- p(95) should be <500ms for batch evaluations
- p(95) should be <2000ms for heatmaps

**Error Rate:**
- Should be <1% under normal/peak load
- Acceptable degradation under stress test

**Resource Usage:**
- Memory should stay <512 MB
- CPU scales with request rate
- No memory leaks (check trend over time)

### What Good Looks Like

```
✓ p(95) latency: 45ms (target: <100ms)
✓ Error rate: 0.02% (target: <1%)
✓ Memory (RSS): avg 145 MB, max 178 MB (target: <512 MB)
✓ Throughput: 10.2 req/s (sustained)
```

### What Bad Looks Like

```
✗ p(95) latency: 250ms (target: <100ms) - PERFORMANCE ISSUE
✗ Error rate: 5.2% (target: <1%) - STABILITY ISSUE
✗ Memory (RSS): avg 520 MB, max 850 MB - MEMORY LEAK
✗ Throughput: 3.5 req/s (expected 10) - BOTTLENECK
```

## Troubleshooting

**Service crashes:**
- Check service logs
- Review error responses in k6 output
- Reduce load and retry

**High latency:**
- Check CPU usage (might be at 100%)
- Review which endpoints are slow
- Consider using fast computation mode

**Memory issues:**
- Check for memory leaks (memory increasing over time)
- Verify calibration data size is reasonable
- Review concurrent request count

## Advanced Usage

```bash
# Run specific scenario manually
BASE_URL=http://localhost:3000 SCENARIO=normal k6 run tests/load/load_test_scenarios.js

# Monitor resources manually
./tests/load/monitor_resources.sh <PID> resources.csv 2

# Analyze results manually
./tests/load/analyze_results.sh tests/load/results
```

## For More Information

See `tests/load/README.md` for comprehensive documentation.
