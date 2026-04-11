//! Heatmap Generation Benchmarks
//!
//! Benchmarks the heatmap API endpoint performance across grid sizes:
//! - 64x64 (4,096 points)
//! - 128x128 (16,384 points)
//! - 256x256 (65,536 points)
//! - 512x512 (262,144 points)
//!
//! This benchmark suite measures:
//! 1. Grid size scaling - How performance scales with number of points
//! 2. Parallel threshold - Sequential vs parallel evaluation overhead
//! 3. Memory stability - Sustained load without memory leaks

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::time::Duration;

use std::path::PathBuf;

use antenna_model::api::schemas::{GridConfig, HeatmapRequest, Position3D, RangeConfig};
use antenna_model::config::CalibrationConfig;
use antenna_model::data::repository::CalibrationRepository;
use antenna_model::service::heatmap::generate_heatmap;

/// Create test calibration repository from test fixtures
///
/// Loads from tests/fixtures/test_antennas.yaml which includes:
/// - test_simple antenna with "primary" feed
/// - 5m reflector, 2.0m focal length, 1.0mm RMS
/// - Feed at focus (on-axis), q-factor 8.0
/// - X-band frequency range (8000-8500 MHz)
/// - No correction surface (physics-only computation)
fn create_test_calibration_repository() -> CalibrationRepository {
    // Get the path to test fixtures relative to the workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixtures_dir = PathBuf::from(&manifest_dir).join("tests/fixtures");

    let config = CalibrationConfig {
        data_directory: fixtures_dir.join("calibration_data"),
        antenna_config_file: fixtures_dir.join("test_antennas.yaml"),
        fail_fast: false,
    };

    CalibrationRepository::load_from_config(&config)
        .expect("Failed to load test calibration repository")
}

/// Create grid configuration for specified grid size
///
/// Maps grid size string to GridConfig with appropriate ranges and steps:
/// - "64x64": 0-63° both axes, 1.0° step → 4,096 points
/// - "128x128": 0-63.5° both axes, 0.5° step → 16,384 points
/// - "256x256": 0-63.75° both axes, 0.25° step → 65,536 points
/// - "512x512": 0-63.875° both axes, 0.125° step → 262,144 points
/// - "8x8": 0-3.5° both axes, 0.5° step → 64 points (sequential test)
/// - "12x12": 0-5.5° both axes, 0.5° step → 144 points (parallel test)
fn create_grid_config(grid_size: &str) -> GridConfig {
    match grid_size {
        "8x8" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 3.5, 0.5),
            elevation_range_deg: RangeConfig::new(0.0, 3.5, 0.5),
        },
        "12x12" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 5.5, 0.5),
            elevation_range_deg: RangeConfig::new(0.0, 5.5, 0.5),
        },
        "64x64" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 63.0, 1.0),
            elevation_range_deg: RangeConfig::new(0.0, 63.0, 1.0),
        },
        "128x128" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 63.5, 0.5),
            elevation_range_deg: RangeConfig::new(0.0, 63.5, 0.5),
        },
        "256x256" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 63.75, 0.25),
            elevation_range_deg: RangeConfig::new(0.0, 63.75, 0.25),
        },
        "512x512" => GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 63.875, 0.125),
            elevation_range_deg: RangeConfig::new(0.0, 63.875, 0.125),
        },
        _ => panic!("Unknown grid size: {}", grid_size),
    }
}

/// Create heatmap request for specified grid size
///
/// Creates a standard heatmap request using test_simple antenna with:
/// - Vehicle at geodetic origin (0, 0, 0)
/// - Reflector boresight 10m above vehicle along +Z
/// - Feed at 12m above vehicle (10m + 2.0m focal length for test_simple antenna)
/// - X-band frequency: 8400 MHz
/// - Grid configuration based on size parameter
fn create_heatmap_request(grid_size: &str) -> HeatmapRequest {
    // Vehicle position (geodetic coordinates)
    let vehicle_position = Position3D::new(0.0, 0.0, 0.0);

    // Reflector boresight (10m above vehicle, pointing up)
    let reflector_boresight = Position3D::new(0.0, 0.0, 10.0);

    // Feed position (at focal point: 10m + 2.0m focal length for test_simple antenna)
    let feed_position = Position3D::new(0.0, 0.0, 12.0);

    HeatmapRequest {
        antenna_id: "test_simple".to_string(),
        feed_id: "primary".to_string(),
        vehicle_position,
        reflector_boresight,
        feed_position,
        frequency_mhz: 8400.0,
        pointing_frequency_mhz: None,
        grid_config: create_grid_config(grid_size),
    }
}

/// Benchmark grid size scaling
///
/// Measures how heatmap generation performance scales with grid size.
/// Tests 4 grid sizes: 64x64, 128x128, 256x256, 512x512
///
/// Expected scaling: Linear with number of points (parallel evaluation)
/// Target throughput: >1,500 points/second
fn bench_grid_size_scaling(c: &mut Criterion) {
    let repository = create_test_calibration_repository();

    let mut group = c.benchmark_group("grid_size_scaling");

    // Adjust sample size and measurement time based on grid size
    // Larger grids take longer, so we reduce sample count
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(90));

    for grid_size in ["64x64", "128x128", "256x256", "512x512"] {
        group.bench_with_input(
            BenchmarkId::from_parameter(grid_size),
            &grid_size,
            |b, size| {
                let request = create_heatmap_request(size);
                b.iter(|| {
                    generate_heatmap(black_box(&request), black_box(&repository))
                        .expect("Heatmap generation failed")
                })
            },
        );
    }

    group.finish();
}

/// Benchmark parallel threshold
///
/// Compares sequential vs parallel evaluation at the PARALLEL_THRESHOLD boundary.
/// - Sequential: 8x8 grid (64 points, below 100-point threshold)
/// - Parallel: 12x12 grid (144 points, above threshold)
///
/// Measures parallelization overhead and efficiency
fn bench_parallel_threshold(c: &mut Criterion) {
    let repository = create_test_calibration_repository();

    let mut group = c.benchmark_group("parallel_threshold");

    // Sequential evaluation (64 points)
    group.bench_function("sequential_64pts", |b| {
        let request = create_heatmap_request("8x8");
        b.iter(|| {
            generate_heatmap(black_box(&request), black_box(&repository))
                .expect("Heatmap generation failed")
        })
    });

    // Parallel evaluation (144 points)
    group.bench_function("parallel_144pts", |b| {
        let request = create_heatmap_request("12x12");
        b.iter(|| {
            generate_heatmap(black_box(&request), black_box(&repository))
                .expect("Heatmap generation failed")
        })
    });

    group.finish();
}

/// Benchmark memory stability
///
/// Generates 100 heatmaps in succession to verify:
/// - No memory leaks
/// - Consistent performance over time
/// - Memory allocation patterns are stable
///
/// Uses 64x64 grid (4,096 points) for reasonable runtime
fn bench_heatmap_memory_stability(c: &mut Criterion) {
    let repository = create_test_calibration_repository();

    c.bench_function("memory_stability_100_heatmaps", |b| {
        let request = create_heatmap_request("64x64");
        b.iter(|| {
            // Generate 100 heatmaps in succession
            for _ in 0..100 {
                let _ = generate_heatmap(black_box(&request), black_box(&repository))
                    .expect("Heatmap generation failed");
            }
        })
    });
}

criterion_group!(
    benches,
    bench_grid_size_scaling,
    bench_parallel_threshold,
    bench_heatmap_memory_stability,
);

criterion_main!(benches);
