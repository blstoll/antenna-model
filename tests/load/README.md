# Load Testing Guide for Antenna Model Service

This directory contains load testing infrastructure for validating the Antenna Model Service's performance and scalability characteristics under production-like loads.

## Overview

The load testing suite uses [k6](https://k6.io/) to simulate realistic usage patterns and measure:
- **Throughput**: Requests per second at various load levels
- **Latency**: Response time distribution (p50, p95, p99)
- **Error rates**: Percentage of failed requests
- **Resource usage**: CPU, memory, threads, file descriptors
- **Scalability**: Behavior under increasing load

## Prerequisites

### 1. Install k6

**macOS:**
```bash
brew install k6
```

**Linux (Debian/Ubuntu):**
```bash
sudo gpg -k
sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1DBD
echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
sudo apt-get update
sudo apt-get install k6
```

**Linux (Fedora/CentOS):**
```bash
sudo dnf install https://dl.k6.io/rpm/repo.rpm
sudo dnf install k6
```

**Other platforms:** See https://k6.io/docs/getting-started/installation/

### 2. Optional Tools

- **jq** - For detailed metrics analysis (recommended)
  ```bash
  # macOS
  brew install jq

  # Linux
  sudo apt-get install jq  # Debian/Ubuntu
  sudo dnf install jq      # Fedora/CentOS
  ```

- **bc** - For numeric calculations (usually pre-installed)

## Quick Start

### 1. Start the Service

In a separate terminal, start the Antenna Model Service:

```bash
cd /path/to/antenna_model
cargo run --release --bin antenna-model
```

Wait for the service to be ready (you should see "Server started" message).

### 2. Run Load Tests

Run all scenarios:
```bash
./tests/load/run_load_tests.sh all
```

Run specific scenario:
```bash
./tests/load/run_load_tests.sh normal   # Normal load (10 req/s for 5 min)
./tests/load/run_load_tests.sh peak     # Peak load (20 req/s for 1 min)
./tests/load/run_load_tests.sh stress   # Stress test (ramp to 100 req/s)
./tests/load/run_load_tests.sh mixed    # Mixed workload (single/batch/heatmap)
./tests/load/run_load_tests.sh rampup   # Gradual ramp-up
```

### 3. View Results

Results are saved in `tests/load/results/` with timestamped directories for each scenario.

The summary report is automatically generated after tests complete.

## Test Scenarios

### Normal Load (normal)
- **Load**: 10 requests/second
- **Duration**: 5 minutes
- **Workload**: Single gain evaluations
- **Purpose**: Validate baseline performance under typical usage

### Peak Load (peak)
- **Load**: 20 requests/second
- **Duration**: 1 minute
- **Workload**: Single gain evaluations
- **Purpose**: Validate performance under peak expected load

### Stress Test (stress)
- **Load**: Ramps from 5 to 100 req/s
- **Duration**: 13 minutes total
- **Stages**:
  - 2 min @ 10 req/s
  - 2 min @ 20 req/s
  - 2 min @ 40 req/s
  - 2 min @ 60 req/s
  - 2 min @ 80 req/s
  - 2 min @ 100 req/s
  - 1 min ramp down
- **Purpose**: Find the breaking point and maximum throughput

### Mixed Workload (mixed)
- **Load**: 10 requests/second total
- **Duration**: 5 minutes
- **Mix**:
  - 70% single evaluations
  - 20% batch evaluations (10-50 points)
  - 10% heatmap generation
- **Purpose**: Simulate realistic production traffic patterns

### Gradual Ramp-up (rampup)
- **Load**: Ramps from 1 to 20 req/s
- **Duration**: 7 minutes
- **Stages**:
  - 1 min @ 5 req/s
  - 1 min @ 10 req/s
  - 1 min @ 15 req/s
  - 1 min @ 20 req/s
  - 2 min sustained @ 20 req/s
  - 1 min ramp down
- **Purpose**: Validate smooth scaling behavior

## Manual Testing

You can run k6 tests manually for more control:

```bash
# Run specific scenario with custom duration
BASE_URL=http://localhost:3000 SCENARIO=normal k6 run tests/load/load_test_scenarios.js

# Run with custom VU count
k6 run --vus 50 --duration 30s tests/load/load_test_scenarios.js

# Run with different output formats
k6 run --out json=results.json tests/load/load_test_scenarios.js
k6 run --out csv=results.csv tests/load/load_test_scenarios.js
```

## Resource Monitoring

The `monitor_resources.sh` script tracks resource usage during tests:

```bash
# Manual monitoring
./tests/load/monitor_resources.sh <PID> resources.csv 2

# Monitor every 2 seconds, save to resources.csv
```

Metrics tracked:
- **CPU**: Percentage utilization
- **Memory (RSS)**: Resident set size in MB
- **Memory (VSZ)**: Virtual memory size in MB
- **Threads**: Thread count
- **Open Files**: File descriptor count

The monitoring script is automatically started by `run_load_tests.sh`.

## Understanding Results

### K6 Metrics

**HTTP Request Duration:**
- **avg**: Average response time
- **min/max**: Range of response times
- **med**: Median (50th percentile)
- **p(90), p(95), p(99)**: 90th, 95th, 99th percentiles

**Request Rate:**
- **total**: Total requests completed
- **rate**: Requests per second

**Error Rate:**
- Percentage of failed requests (status != 200)

### Custom Metrics

- **gain_latency**: Latency for single gain evaluations
- **batch_latency**: Latency for batch evaluations
- **heatmap_latency**: Latency for heatmap generation

### Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Single evaluation p95 | <100ms | 95% of requests |
| Batch evaluation p95 | <500ms | For 10-50 point batches |
| Heatmap p95 | <2000ms | For typical grids (~400 points) |
| Error rate | <1% | Under normal/peak load |
| Throughput | 10-20 req/s | Per service instance |
| Memory footprint | <512MB | Resident set size |

## Analyzing Results

### Automated Analysis

The `analyze_results.sh` script generates summary reports:

```bash
./tests/load/analyze_results.sh tests/load/results
```

### Manual Analysis

**View k6 metrics:**
```bash
jq '.metrics' tests/load/results/normal_*/results.json
```

**Extract specific metric:**
```bash
jq '.metrics.http_req_duration.values' tests/load/results/normal_*/results.json
```

**Plot resource usage (requires gnuplot):**
```bash
gnuplot <<EOF
set datafile separator ','
set xlabel 'Time (s)'
set ylabel 'CPU %'
set y2label 'Memory (MB)'
set ytics nomirror
set y2tics
plot 'tests/load/results/normal_*/resources.csv' using 1:2 with lines title 'CPU' axes x1y1, \
     '' using 1:3 with lines title 'Memory' axes x1y2
pause -1
EOF
```

## Troubleshooting

### Service Not Available

**Problem:** `Service not available at http://localhost:3000`

**Solution:**
1. Verify service is running: `curl http://localhost:3000/health`
2. Check for port conflicts: `lsof -i :3000`
3. Review service logs for errors

### High Error Rates

**Problem:** Error rate >1%

**Solution:**
1. Check service logs for error messages
2. Review error responses in k6 output
3. Verify test data (antenna IDs, frequency ranges)
4. Reduce load to isolate issue

### Resource Monitoring Fails

**Problem:** `monitor_resources.sh` can't find process

**Solution:**
1. Find correct PID: `lsof -ti :3000`
2. Check process is running: `ps -p <PID>`
3. Ensure sufficient permissions: `lsof` may require sudo on some systems

### k6 Installation Issues

**Problem:** k6 command not found

**Solution:**
- Follow installation instructions above
- Verify installation: `k6 version`
- Check PATH includes k6 binary location

## Advanced Usage

### Custom Scenarios

Edit `load_test_scenarios.js` to add custom scenarios:

```javascript
export const options = {
    scenarios: {
        my_custom_scenario: {
            executor: 'constant-arrival-rate',
            rate: 15,
            timeUnit: '1s',
            duration: '2m',
            preAllocatedVUs: 30,
            exec: 'singleEvaluationScenario',
        },
    },
};
```

### Cloud Testing

Run tests from k6 cloud:

```bash
k6 cloud tests/load/load_test_scenarios.js
```

Requires k6 cloud account: https://app.k6.io/

### Distributed Testing

Run tests from multiple machines for higher load:

```bash
# Machine 1
k6 run --vus 100 tests/load/load_test_scenarios.js

# Machine 2
k6 run --vus 100 tests/load/load_test_scenarios.js
```

Aggregate results manually or use k6 cloud for automatic aggregation.

## Integration with CI/CD

### GitHub Actions Example

```yaml
name: Load Tests

on:
  schedule:
    - cron: '0 2 * * *'  # Daily at 2 AM

jobs:
  load-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install k6
        run: |
          sudo apt-get update
          sudo apt-get install -y gnupg software-properties-common
          curl -s https://dl.k6.io/key.gpg | sudo apt-key add -
          echo "deb https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
          sudo apt-get update
          sudo apt-get install k6

      - name: Build service
        run: cargo build --release

      - name: Start service
        run: cargo run --release --bin antenna-model &

      - name: Wait for service
        run: |
          timeout 60 bash -c 'until curl -sf http://localhost:3000/health; do sleep 1; done'

      - name: Run load tests
        run: ./tests/load/run_load_tests.sh normal

      - name: Upload results
        uses: actions/upload-artifact@v3
        with:
          name: load-test-results
          path: tests/load/results/
```

## Best Practices

1. **Warm-up**: Allow service to warm up before starting tests (JIT compilation, caches)
2. **Isolation**: Run tests on dedicated hardware to avoid interference
3. **Repeatability**: Run tests multiple times to account for variance
4. **Monitoring**: Always monitor resource usage during tests
5. **Incremental**: Start with low load and gradually increase
6. **Documentation**: Document any configuration changes that affect performance

## References

- [k6 Documentation](https://k6.io/docs/)
- [k6 Test Types](https://k6.io/docs/test-types/introduction/)
- [k6 Metrics](https://k6.io/docs/using-k6/metrics/)
- [Performance Testing Best Practices](https://k6.io/docs/testing-guides/performance-testing/)

## Support

For issues or questions:
1. Check this README
2. Review k6 documentation
3. Check service logs
4. Open an issue in the project repository
