//! Aperture Integration Benchmarks
//!
//! Task 7.5: Performance benchmarking suite - aperture integration layer
//!
//! Benchmarks the most performance-critical component: aperture integration.
//! This is expected to be the computational bottleneck.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

use antenna_model::model::{
    geometry::{AntennaConfiguration, FeedParameters, FeedPosition, ReflectorGeometry},
    integration::IntegrationParams,
    pattern::{compute_gain, compute_gain_db},
};

/// Create standard test antenna (34m dish)
fn create_standard_antenna() -> AntennaConfiguration {
    let focal_length = 13.6;
    AntennaConfiguration::new(
        "bench_standard".to_string(),
        "Benchmark Standard".to_string(),
        ReflectorGeometry::new(34.0, focal_length, 0.0003).unwrap(),
        FeedParameters::new(FeedPosition::at_focus(focal_length), 8.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Create small antenna (7.3m dish)
fn create_small_antenna() -> AntennaConfiguration {
    let focal_length = 2.9;
    AntennaConfiguration::new(
        "bench_small".to_string(),
        "Benchmark Small".to_string(),
        ReflectorGeometry::new(7.3, focal_length, 0.0005).unwrap(),
        FeedParameters::new(FeedPosition::at_focus(focal_length), 10.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Create large antenna (70m DSN class)
fn create_large_antenna() -> AntennaConfiguration {
    let focal_length = 26.0;
    AntennaConfiguration::new(
        "bench_large".to_string(),
        "Benchmark Large".to_string(),
        ReflectorGeometry::new(70.0, focal_length, 0.0002).unwrap(),
        FeedParameters::new(FeedPosition::at_focus(focal_length), 6.0, 0.0, 1.0).unwrap(),
        None,
    )
    .unwrap()
}

/// Benchmark integration parameter settings
fn bench_integration_params(c: &mut Criterion) {
    let config = create_standard_antenna();
    let frequency_hz = 8.4e9;

    let mut group = c.benchmark_group("integration_params");

    for (name, params) in [
        ("fast", IntegrationParams::fast()),
        ("default", IntegrationParams::default()),
        ("high_accuracy", IntegrationParams::high_accuracy()),
    ] {
        group.bench_with_input(BenchmarkId::new("boresight", name), &params, |b, params| {
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
    }

    group.finish();
}

/// Benchmark different antenna sizes
fn bench_antenna_sizes(c: &mut Criterion) {
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::default();

    let mut group = c.benchmark_group("antenna_sizes");

    let antennas = [
        ("small_7.3m", create_small_antenna()),
        ("standard_34m", create_standard_antenna()),
        ("large_70m", create_large_antenna()),
    ];

    for (name, config) in &antennas {
        group.bench_with_input(BenchmarkId::new("size", name), &config, |b, cfg| {
            b.iter(|| {
                compute_gain(
                    black_box(0.0),
                    black_box(0.0),
                    black_box(cfg),
                    black_box(frequency_hz),
                    black_box(&params),
                )
            })
        });
    }

    group.finish();
}

/// Benchmark frequency sweep (different wavelengths affect integration)
fn bench_frequency_range(c: &mut Criterion) {
    let config = create_standard_antenna();
    let params = IntegrationParams::fast();

    let mut group = c.benchmark_group("frequency_range");

    for (name, freq) in [
        ("L-band_1.5GHz", 1.5e9),
        ("S-band_2.3GHz", 2.3e9),
        ("C-band_6.0GHz", 6.0e9),
        ("X-band_8.4GHz", 8.4e9),
        ("Ku-band_14GHz", 14.0e9),
        ("Ka-band_32GHz", 32.0e9),
    ] {
        group.bench_with_input(BenchmarkId::new("band", name), &freq, |b, frequency| {
            b.iter(|| {
                compute_gain(
                    black_box(0.0),
                    black_box(0.0),
                    black_box(&config),
                    black_box(*frequency),
                    black_box(&params),
                )
            })
        });
    }

    group.finish();
}

/// Benchmark angular coverage (main lobe to sidelobes)
fn bench_angular_coverage(c: &mut Criterion) {
    let config = create_standard_antenna();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    let mut group = c.benchmark_group("angular_coverage");

    for angle_deg in [0.0_f64, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0] {
        let angle_rad = angle_deg.to_radians();
        group.bench_with_input(
            BenchmarkId::new("angle_deg", format!("{:.1}", angle_deg)),
            &angle_rad,
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

/// Benchmark linear vs dB conversion
fn bench_gain_output_format(c: &mut Criterion) {
    let config = create_standard_antenna();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    let mut group = c.benchmark_group("gain_output_format");

    group.bench_function("linear_gain", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.0),
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    group.bench_function("gain_db", |b| {
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

    group.finish();
}

/// Benchmark convergence characteristics (near nulls require more subdivisions)
fn bench_convergence(c: &mut Criterion) {
    let config = create_standard_antenna();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::default();

    let mut group = c.benchmark_group("convergence");
    group.sample_size(50);

    // Boresight (easy convergence)
    group.bench_function("easy_boresight", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.0),
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    // First sidelobe (moderate)
    group.bench_function("moderate_sidelobe", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.05), // ~3 degrees
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    // Near null (hard convergence - requires adaptive integration)
    group.bench_function("hard_near_null", |b| {
        b.iter(|| {
            compute_gain(
                black_box(0.15), // ~8.6 degrees, near first null
                black_box(0.0),
                black_box(&config),
                black_box(frequency_hz),
                black_box(&params),
            )
        })
    });

    group.finish();
}

/// Benchmark memory usage characteristics (not time-based)
/// This measures how many iterations can be done without memory growth
fn bench_memory_stability(c: &mut Criterion) {
    let config = create_standard_antenna();
    let frequency_hz = 8.4e9;
    let params = IntegrationParams::fast();

    c.bench_function("memory_stability_sustained_load", |b| {
        b.iter(|| {
            // Run 100 evaluations in a loop to test memory stability
            for i in 0..100 {
                let theta = (i as f64 * 0.001).to_radians();
                let _ = compute_gain(
                    black_box(theta),
                    black_box(0.0),
                    black_box(&config),
                    black_box(frequency_hz),
                    black_box(&params),
                );
            }
        })
    });
}

criterion_group!(
    benches,
    bench_integration_params,
    bench_antenna_sizes,
    bench_frequency_range,
    bench_angular_coverage,
    bench_gain_output_format,
    bench_convergence,
    bench_memory_stability,
);

criterion_main!(benches);
