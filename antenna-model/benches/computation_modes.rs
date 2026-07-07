//! Benchmarks for antenna gain computation modes
//!
//! Task 8.2: Performance optimization and benchmarking of computation modes
//!
//! Benchmarks the three computation modes:
//! - StandardPhysicalOptics
//! - HigherOrderAberrations
//! - RayTracing

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

use antenna_model::model::{
    coordinates::EClockConeCoordinates,
    geometry::{AntennaConfiguration, FeedParameters, FeedPosition, ReflectorGeometry},
    integration::IntegrationParams,
    pattern::{compute_gain, compute_gain_db},
};

/// Create antenna with feed at focus (StandardPhysicalOptics mode)
fn antenna_on_axis() -> AntennaConfiguration {
    let focal_length = 13.6; // Realistic GEO antenna focal length
    AntennaConfiguration::new(
        "bench_on_axis".to_string(),
        "Benchmark On-Axis".to_string(),
        ReflectorGeometry::new(34.0, focal_length, 0.0003).unwrap(), // 34m dish
        FeedParameters::new(FeedPosition::at_focus(focal_length), 8.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Create antenna with moderate feed offset (HigherOrderAberrations mode)
/// Feed offset ratio ~0.35 (between 0.3f and 0.5f)
fn antenna_moderate_offset() -> AntennaConfiguration {
    let focal_length = 13.6;
    // E-cone ~20 degrees gives offset/f ~0.35
    let ecc = EClockConeCoordinates::from_degrees(20.0, 0.0);
    let (x, y, z) = ecc.to_feed_position(focal_length);

    AntennaConfiguration::new(
        "bench_moderate_offset".to_string(),
        "Benchmark Moderate Offset".to_string(),
        ReflectorGeometry::new(34.0, focal_length, 0.0003).unwrap(),
        FeedParameters::new(FeedPosition::new(x, y, z), 8.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Create antenna with large feed offset (RayTracing mode)
/// Feed offset ratio > 0.5f
fn antenna_large_offset() -> AntennaConfiguration {
    let focal_length = 13.6;
    // E-cone ~35 degrees gives offset/f ~0.6
    let ecc = EClockConeCoordinates::from_degrees(35.0, 0.0);
    let (x, y, z) = ecc.to_feed_position(focal_length);

    AntennaConfiguration::new(
        "bench_large_offset".to_string(),
        "Benchmark Large Offset".to_string(),
        ReflectorGeometry::new(34.0, focal_length, 0.0003).unwrap(),
        FeedParameters::new(FeedPosition::new(x, y, z), 8.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Benchmark standard physical optics mode
fn bench_standard_physical_optics(c: &mut Criterion) {
    let config = antenna_on_axis();
    let frequency_hz = 8.4e9;

    let mut group = c.benchmark_group("standard_physical_optics");

    // Benchmark with different integration parameters
    for (name, params) in [
        ("fast", IntegrationParams::fast()),
        ("default", IntegrationParams::default()),
        ("high_accuracy", IntegrationParams::high_accuracy()),
    ] {
        group.bench_with_input(BenchmarkId::new("on_axis", name), &params, |b, params| {
            b.iter(|| {
                compute_gain(
                    black_box(0.0),
                    black_box(0.0),
                    black_box(&config),
                    black_box(frequency_hz),
                    black_box(params),
                )
            })
        });

        // Off-axis computation (5 degrees)
        group.bench_with_input(
            BenchmarkId::new("off_axis_5deg", name),
            &params,
            |b, params| {
                b.iter(|| {
                    compute_gain(
                        black_box(5.0_f64.to_radians()),
                        black_box(0.0),
                        black_box(&config),
                        black_box(frequency_hz),
                        black_box(params),
                    )
                })
            },
        );
    }

    group.finish();
}

/// Benchmark higher-order aberrations mode
fn bench_higher_order_aberrations(c: &mut Criterion) {
    let config = antenna_moderate_offset();
    let frequency_hz = 8.4e9;

    let mut group = c.benchmark_group("higher_order_aberrations");

    for (name, params) in [
        ("fast", IntegrationParams::fast()),
        ("default", IntegrationParams::default()),
    ] {
        group.bench_with_input(
            BenchmarkId::new("moderate_offset", name),
            &params,
            |b, params| {
                b.iter(|| {
                    compute_gain(
                        black_box(0.05), // ~2.9 degrees
                        black_box(0.0),
                        black_box(&config),
                        black_box(frequency_hz),
                        black_box(params),
                    )
                })
            },
        );
    }

    group.finish();
}

/// Benchmark ray tracing mode
fn bench_ray_tracing(c: &mut Criterion) {
    let config = antenna_large_offset();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    let mut group = c.benchmark_group("ray_tracing");

    // Ray tracing at various angles
    for angle_deg in [0.0_f64, 1.0, 5.0] {
        let theta = angle_deg.to_radians();

        group.bench_with_input(
            BenchmarkId::new("large_offset", format!("{:.0}deg", angle_deg)),
            &theta,
            |b, theta| {
                b.iter(|| {
                    compute_gain(
                        black_box(*theta),
                        black_box(0.0),
                        black_box(&config),
                        black_box(frequency_hz),
                        black_box(&params),
                    )
                })
            },
        );
    }

    group.finish();
}

/// Benchmark compute_gain_db (includes log conversion)
fn bench_gain_db(c: &mut Criterion) {
    let config = antenna_on_axis();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    c.bench_function("compute_gain_db", |b| {
        b.iter(|| {
            compute_gain_db(
                black_box(0.0),
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });
}

/// Benchmark adaptive integration (near null regions)
fn bench_adaptive_integration(c: &mut Criterion) {
    let config = antenna_on_axis();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::default();

    let mut group = c.benchmark_group("adaptive_integration");

    // Near boresight (standard integration)
    group.bench_function("no_adaptive", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.05), // < 0.1 rad, no adaptive
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    // Near null (adaptive integration)
    group.bench_function("with_adaptive", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.2), // > 0.1 rad, triggers adaptive
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    group.finish();
}

/// Compare all computation modes at same conditions
fn bench_mode_comparison(c: &mut Criterion) {
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    let mut group = c.benchmark_group("mode_comparison");
    group.sample_size(50); // Reduce sample size for expensive modes

    // Standard mode
    let config_std = antenna_on_axis();
    group.bench_function("StandardPhysicalOptics", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.05),
                black_box(0.0),
                black_box(&config_std),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    // Higher-order mode
    let config_ho = antenna_moderate_offset();
    group.bench_function("HigherOrderAberrations", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.05),
                black_box(0.0),
                black_box(&config_ho),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    // Ray tracing mode
    let config_rt = antenna_large_offset();
    group.bench_function("RayTracing", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.05),
                black_box(0.0),
                black_box(&config_rt),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_standard_physical_optics,
    bench_higher_order_aberrations,
    bench_ray_tracing,
    bench_gain_db,
    bench_adaptive_integration,
    bench_mode_comparison,
);

criterion_main!(benches);
