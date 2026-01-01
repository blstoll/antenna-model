// K6 Load Testing Scenarios for Antenna Model Service
// Usage: k6 run tests/load/load_test_scenarios.js
//
// To run specific scenario:
//   k6 run -e SCENARIO=normal tests/load/load_test_scenarios.js
//   k6 run -e SCENARIO=peak tests/load/load_test_scenarios.js
//   k6 run -e SCENARIO=stress tests/load/load_test_scenarios.js
//   k6 run -e SCENARIO=mixed tests/load/load_test_scenarios.js
//   k6 run -e SCENARIO=rampup tests/load/load_test_scenarios.js

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate, Trend, Counter } from 'k6/metrics';

// Custom metrics
const errorRate = new Rate('errors');
const gainLatency = new Trend('gain_latency');
const batchLatency = new Trend('batch_latency');
const heatmapLatency = new Trend('heatmap_latency');
const requestCounter = new Counter('requests_total');

// Configuration
const BASE_URL = __ENV.BASE_URL || 'http://localhost:3000';
const SCENARIO = __ENV.SCENARIO || 'normal';

// Test data
const ANTENNAS = ['test_boresight_xband', 'test_boresight_sband', 'test_uncalibrated'];
const FEEDS = {
    'test_boresight_xband': 'x_band',
    'test_boresight_sband': 's_band',
    'test_uncalibrated': 'primary'
};

// Scenario configurations
export const options = {
    scenarios: {
        // Normal load: Sustained 10 req/s for 5 minutes
        normal: {
            executor: 'constant-arrival-rate',
            rate: 10,
            timeUnit: '1s',
            duration: '5m',
            preAllocatedVUs: 20,
            maxVUs: 50,
            exec: 'singleEvaluationScenario',
            startTime: '0s',
            gracefulStop: '30s',
        },

        // Peak load: Burst to 20 req/s for 1 minute
        peak: {
            executor: 'constant-arrival-rate',
            rate: 20,
            timeUnit: '1s',
            duration: '1m',
            preAllocatedVUs: 40,
            maxVUs: 100,
            exec: 'singleEvaluationScenario',
            startTime: '0s',
            gracefulStop: '30s',
        },

        // Stress test: Gradual ramp-up to find breaking point
        stress: {
            executor: 'ramping-arrival-rate',
            startRate: 5,
            timeUnit: '1s',
            preAllocatedVUs: 50,
            maxVUs: 300,
            exec: 'singleEvaluationScenario',
            stages: [
                { duration: '2m', target: 10 },   // Ramp to 10 req/s
                { duration: '2m', target: 20 },   // Ramp to 20 req/s
                { duration: '2m', target: 40 },   // Ramp to 40 req/s
                { duration: '2m', target: 60 },   // Ramp to 60 req/s
                { duration: '2m', target: 80 },   // Ramp to 80 req/s
                { duration: '2m', target: 100 },  // Ramp to 100 req/s
                { duration: '1m', target: 0 },    // Ramp down
            ],
            gracefulStop: '30s',
        },

        // Mixed workload: 70% single, 20% batch, 10% heatmap
        mixed: {
            executor: 'constant-arrival-rate',
            rate: 10,
            timeUnit: '1s',
            duration: '5m',
            preAllocatedVUs: 30,
            maxVUs: 100,
            exec: 'mixedWorkloadScenario',
            startTime: '0s',
            gracefulStop: '30s',
        },

        // Gradual ramp-up: Smooth increase to validate scaling
        rampup: {
            executor: 'ramping-arrival-rate',
            startRate: 1,
            timeUnit: '1s',
            preAllocatedVUs: 50,
            maxVUs: 150,
            exec: 'singleEvaluationScenario',
            stages: [
                { duration: '1m', target: 5 },
                { duration: '1m', target: 10 },
                { duration: '1m', target: 15 },
                { duration: '1m', target: 20 },
                { duration: '2m', target: 20 },  // Sustain peak
                { duration: '1m', target: 0 },   // Ramp down
            ],
            gracefulStop: '30s',
        },
    },

    thresholds: {
        'http_req_duration': ['p(95)<100', 'p(99)<200'],  // 95th percentile < 100ms, 99th < 200ms
        'http_req_failed': ['rate<0.01'],                  // Error rate < 1%
        'errors': ['rate<0.01'],
        'gain_latency': ['p(95)<100', 'p(99)<200'],
        'batch_latency': ['p(95)<500', 'p(99)<1000'],
        'heatmap_latency': ['p(95)<2000', 'p(99)<5000'],
    },

    // Only run the selected scenario
    summaryTrendStats: ['avg', 'min', 'med', 'max', 'p(90)', 'p(95)', 'p(99)'],
};

// Filter to run only selected scenario
if (SCENARIO !== 'all') {
    for (let scenario in options.scenarios) {
        if (scenario !== SCENARIO) {
            delete options.scenarios[scenario];
        }
    }
}

// Helper function to get random antenna
function getRandomAntenna() {
    const antenna = ANTENNAS[Math.floor(Math.random() * ANTENNAS.length)];
    return { antenna_id: antenna, feed_id: FEEDS[antenna] };
}

// Helper function to generate random position
function generateRandomPosition() {
    // Generate positions in geodetic coordinates (more realistic)
    return {
        x: -122.0 + Math.random() * 0.1,  // Longitude around -122°
        y: 37.0 + Math.random() * 0.1,    // Latitude around 37°
        z: Math.random() * 1000            // Altitude 0-1000m
    };
}

// Helper function to generate emitter direction
function generateRandomDirection() {
    return {
        azimuth: Math.random() * 360 - 180,     // -180 to 180 degrees
        elevation: Math.random() * 90           // 0 to 90 degrees
    };
}

// Single gain evaluation scenario
export function singleEvaluationScenario() {
    const { antenna_id, feed_id } = getRandomAntenna();
    const position = generateRandomPosition();
    const direction = generateRandomDirection();

    const payload = JSON.stringify({
        antenna_id,
        feed_id,
        emitter_position: position,
        frequency_hz: 8.4e9 + Math.random() * 1e9,  // X-band range
        vehicle_position: {
            x: -122.0,
            y: 37.0,
            z: 0.0
        },
        vehicle_attitude: {
            roll: 0.0,
            pitch: 0.0,
            yaw: 0.0
        },
        reference_gain_db: null,
        output_format: "dB"
    });

    const params = {
        headers: { 'Content-Type': 'application/json' },
        timeout: '30s',
    };

    const start = new Date();
    const response = http.post(`${BASE_URL}/api/v1/gain`, payload, params);
    const duration = new Date() - start;

    requestCounter.add(1);
    gainLatency.add(duration);

    const success = check(response, {
        'status is 200': (r) => r.status === 200,
        'has gain value': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.gain_db !== undefined || body.gain_linear !== undefined;
            } catch {
                return false;
            }
        },
        'response time < 100ms': () => duration < 100,
    });

    if (!success) {
        errorRate.add(1);
        console.log(`Error: ${response.status} - ${response.body}`);
    } else {
        errorRate.add(0);
    }

    sleep(0.1);  // Small sleep to prevent overwhelming the service
}

// Batch evaluation scenario
export function batchEvaluationScenario() {
    const { antenna_id, feed_id } = getRandomAntenna();

    // Generate 10-50 random requests for batch
    const batchSize = 10 + Math.floor(Math.random() * 40);
    const requests = [];

    for (let i = 0; i < batchSize; i++) {
        requests.push({
            emitter_position: generateRandomPosition(),
            frequency_hz: 8.4e9 + Math.random() * 1e9,
            vehicle_position: { x: -122.0, y: 37.0, z: 0.0 },
            vehicle_attitude: { roll: 0.0, pitch: 0.0, yaw: 0.0 },
            reference_gain_db: null,
            output_format: "dB"
        });
    }

    const payload = JSON.stringify({
        antenna_id,
        feed_id,
        requests
    });

    const params = {
        headers: { 'Content-Type': 'application/json' },
        timeout: '60s',
    };

    const start = new Date();
    const response = http.post(`${BASE_URL}/api/v1/gain/batch`, payload, params);
    const duration = new Date() - start;

    requestCounter.add(1);
    batchLatency.add(duration);

    const success = check(response, {
        'status is 200': (r) => r.status === 200,
        'has results': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.results && body.results.length === batchSize;
            } catch {
                return false;
            }
        },
    });

    if (!success) {
        errorRate.add(1);
    } else {
        errorRate.add(0);
    }

    sleep(0.5);
}

// Heatmap generation scenario
export function heatmapScenario() {
    const { antenna_id, feed_id } = getRandomAntenna();

    const payload = JSON.stringify({
        antenna_id,
        feed_id,
        frequency_hz: 8.4e9,
        vehicle_position: { x: -122.0, y: 37.0, z: 0.0 },
        vehicle_attitude: { roll: 0.0, pitch: 0.0, yaw: 0.0 },
        grid: {
            type: "rectangular",
            azimuth_start: -10.0,
            azimuth_end: 10.0,
            azimuth_step: 1.0,
            elevation_start: 0.0,
            elevation_end: 20.0,
            elevation_step: 1.0
        },
        reference_gain_db: 45.0,
        output_format: "dB"
    });

    const params = {
        headers: { 'Content-Type': 'application/json' },
        timeout: '120s',
    };

    const start = new Date();
    const response = http.post(`${BASE_URL}/api/v1/heatmap`, payload, params);
    const duration = new Date() - start;

    requestCounter.add(1);
    heatmapLatency.add(duration);

    const success = check(response, {
        'status is 200': (r) => r.status === 200,
        'has points': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.points && body.points.length > 0;
            } catch {
                return false;
            }
        },
    });

    if (!success) {
        errorRate.add(1);
    } else {
        errorRate.add(0);
    }

    sleep(2);  // Heatmaps are expensive, longer sleep
}

// Mixed workload scenario: 70% single, 20% batch, 10% heatmap
export function mixedWorkloadScenario() {
    const rand = Math.random();

    if (rand < 0.70) {
        // 70% single evaluations
        singleEvaluationScenario();
    } else if (rand < 0.90) {
        // 20% batch evaluations
        batchEvaluationScenario();
    } else {
        // 10% heatmap generations
        heatmapScenario();
    }
}

// Setup function - verify service is running
export function setup() {
    const response = http.get(`${BASE_URL}/health`);
    check(response, {
        'service is healthy': (r) => r.status === 200,
    });

    if (response.status !== 200) {
        throw new Error(`Service not available at ${BASE_URL}. Health check failed.`);
    }

    console.log(`Load testing ${BASE_URL} with scenario: ${SCENARIO}`);
    console.log(`Available antennas: ${ANTENNAS.join(', ')}`);

    return { startTime: new Date().toISOString() };
}

// Teardown function - final summary
export function teardown(data) {
    console.log(`Load test completed. Started at: ${data.startTime}`);
}

// Handle summary for custom output
export function handleSummary(data) {
    return {
        'stdout': textSummary(data, { indent: ' ', enableColors: true }),
        'tests/load/results.json': JSON.stringify(data, null, 2),
    };
}

// Simple text summary function
function textSummary(data, options) {
    const indent = options.indent || '';
    const enableColors = options.enableColors || false;

    let summary = '\n';
    summary += `${indent}Scenario: ${SCENARIO}\n`;
    summary += `${indent}Duration: ${data.state.testRunDurationMs / 1000}s\n`;
    summary += `${indent}VUs: ${data.metrics.vus ? data.metrics.vus.values.max : 'N/A'}\n`;
    summary += `${indent}Iterations: ${data.metrics.iterations ? data.metrics.iterations.values.count : 'N/A'}\n`;
    summary += `\n${indent}HTTP Metrics:\n`;

    if (data.metrics.http_req_duration) {
        const dur = data.metrics.http_req_duration.values;
        summary += `${indent}  Request Duration:\n`;
        summary += `${indent}    avg: ${dur.avg.toFixed(2)}ms\n`;
        summary += `${indent}    min: ${dur.min.toFixed(2)}ms\n`;
        summary += `${indent}    med: ${dur.med.toFixed(2)}ms\n`;
        summary += `${indent}    max: ${dur.max.toFixed(2)}ms\n`;
        summary += `${indent}    p(90): ${dur['p(90)'].toFixed(2)}ms\n`;
        summary += `${indent}    p(95): ${dur['p(95)'].toFixed(2)}ms\n`;
        summary += `${indent}    p(99): ${dur['p(99)'].toFixed(2)}ms\n`;
    }

    if (data.metrics.http_req_failed) {
        const failed = data.metrics.http_req_failed.values.rate * 100;
        summary += `${indent}  Failed Requests: ${failed.toFixed(2)}%\n`;
    }

    return summary;
}
