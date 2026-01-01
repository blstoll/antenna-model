#!/bin/bash
# Extract benchmark results from criterion output

echo "=== INTEGRATION PARAMETERS PERFORMANCE ==="
grep -A 1 "integration_params/boresight" benchmark_results.txt | grep "time:" | head -3

echo ""
echo "=== ANTENNA SIZES PERFORMANCE ==="
grep -A 1 "antenna_sizes/size" benchmark_results.txt | grep "time:" | head -3

echo ""
echo "=== FREQUENCY RANGE PERFORMANCE ==="
grep -A 1 "frequency_range/band" benchmark_results.txt | grep "time:" | head -6

echo ""
echo "=== ANGULAR COVERAGE PERFORMANCE ==="
grep -A 1 "angular_coverage/angle_deg" benchmark_results.txt | grep "time:" | head -7

echo ""
echo "=== GAIN OUTPUT FORMAT PERFORMANCE ==="
grep -A 1 "gain_output_format" benchmark_results.txt | grep "time:" | head -2

echo ""
echo "=== CONVERGENCE PERFORMANCE ==="
grep -A 1 "convergence" benchmark_results.txt | grep "time:" | head -3

echo ""
echo "=== COMPUTATION MODES PERFORMANCE ==="
grep -A 1 "mode_comparison" benchmark_results.txt | grep "time:" | head -4
