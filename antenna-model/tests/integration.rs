//! Integration Tests for Antenna Model Service
//!
//! This file serves as the entry point for all integration tests.
//! The actual test implementations are in the `integration/` module.
//!
//! # Running Tests
//!
//! ```bash
//! # Run all integration tests
//! cargo test --test integration
//!
//! # Run with output
//! cargo test --test integration -- --nocapture
//!
//! # Run specific test module
//! cargo test --test integration api_tests
//! cargo test --test integration partial_calibration_tests
//! cargo test --test integration concurrent_tests
//!
//! # Run specific test
//! cargo test --test integration test_health_endpoint
//! ```

#[path = "integration/mod.rs"]
mod integration;
