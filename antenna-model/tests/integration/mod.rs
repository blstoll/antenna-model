//! Integration Tests Module
//!
//! Comprehensive integration tests for the Antenna Model Service.
//!
//! Test Organization:
//! - `helpers` - Test utilities (server management, API client, validators)
//! - `api_tests` - Core API endpoint tests
//! - `partial_calibration_tests` - Calibration status tests
//! - `concurrent_tests` - Concurrent access and load tests
//!
//! ## Running Tests
//!
//! Run all integration tests:
//! ```bash
//! cargo test --test integration
//! ```
//!
//! Run specific test module:
//! ```bash
//! cargo test --test integration api_tests
//! cargo test --test integration partial_calibration_tests
//! cargo test --test integration concurrent_tests
//! ```
//!
//! Run specific test:
//! ```bash
//! cargo test --test integration test_health_endpoint
//! ```
//!
//! ## Test Coverage
//!
//! ### API Tests (`api_tests.rs`)
//! - Health, ready, and status endpoints
//! - Single gain computation (ECEF and Geodetic coordinates)
//! - Batch gain computation
//! - Heatmap generation (rectangular grids)
//! - Antenna and feed listing
//! - Error handling (invalid antenna/feed IDs)
//! - Multi-feed antenna support
//!
//! ### Partial Calibration Tests (`partial_calibration_tests.rs`)
//! - Uncalibrated antenna queries
//! - Calibration status in API responses
//! - Warning generation for uncalibrated antennas
//! - Loss computation for all calibration statuses
//! - Mixed calibration status batch requests
//! - Frequency validation
//!
//! ### Concurrent Tests (`concurrent_tests.rs`)
//! - Concurrent gain computations
//! - Concurrent batch requests
//! - Mixed request types under load
//! - Thread safety of calibration repository
//! - Sustained load testing
//! - Error handling under concurrent load
//!
//! ### Error Tests (`error_tests.rs`)
//! - Startup failures (missing files, invalid config)
//! - Runtime errors (invalid requests, out-of-range values)
//! - Resource exhaustion (large requests, memory limits)
//! - Malformed API requests
//! - Extreme parameter values
//! - HTTP method and content type validation
//!
//! ### Resilience Tests (`resilience_tests.rs`)
//! - Graceful degradation under partial failures
//! - Recovery from transient errors
//! - Service stability under error conditions
//! - Partial antenna loading failures
//! - Concurrent error conditions
//! - Resource cleanup verification
//!
//! ### H3 Link Budget Tests (`h3_link_budget_tests.rs`)
//! - Cell count for n_rings values (0 = 1 cell, 2 = 19 cells)
//! - Center cell minimum loss (approximately 0.0 dB at boresight)
//! - Link budget arithmetic consistency (total = loss + FSPL)
//! - Error handling (unknown antenna returns 404, n_rings > 10 returns 422)
//! - Calibration status presence in responses
//! - Cache consistency across identical requests
//! - Auto-resolution selection from frequency

pub mod helpers;

pub mod api_tests;
pub mod concurrent_tests;
pub mod error_tests;
pub mod h3_link_budget_tests;
pub mod off_axis_warning_tests;
pub mod partial_calibration_tests;
pub mod ray_trace_stub_warning_tests;
pub mod resilience_tests;
pub mod timeout_tests;
