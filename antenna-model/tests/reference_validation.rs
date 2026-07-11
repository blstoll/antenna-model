//! Reference-data validation harness.
//!
//! Scores the service's UNCALIBRATED physics model against published, real-world antenna
//! performance so we can gauge how well the design-spec model tracks reality across aperture
//! size, frequency, and geometry. Reference antennas (see `tests/fixtures/reference_datasets/`):
//! DSN 34-m BWG, DSN 70-m, and GBT 100-m — documented in the DESCANSO monographs, the DSN 810-005
//! handbook, TDA/IPN progress reports, and NRAO/GBT publications.
//!
//! ## Modes
//!
//! - `reference_residuals_within_tolerance` — peak (boresight) gain vs measured, per `.psv` row.
//!   Loads the real `calibration_data/antennas.yaml`, builds each antenna's config with the feed
//!   AT FOCUS so `compute_gain_db(0, 0, ...)` yields the true beam peak (design surface RMS kept,
//!   so Ruze applies), and asserts the residual is within the row's `tolerance_db`.
//! - `feed_taper_q_sweep_dsn_34m_xband` — documents the feed q-factor ↔ edge-taper ↔ efficiency
//!   relationship that drove the config fix.
//! - `itu_r_s580_sidelobe_envelope_small_dish` — off-axis pattern shape vs the ITU-R S.580
//!   sidelobe envelope (a different axis than peak gain).
//!
//! ## Caveats
//!
//! These are design-spec (uncalibrated) antennas with feeds set to a sensible edge taper; the
//! real dishes are dual-reflector while the model is prime-focus, so a small systematic offset is
//! expected. Tolerances are regression guards, not accuracy claims. The tables printed with
//! `--nocapture` are the real deliverable. Ka-band phase-center defocus resolved by P7
//! auto-refocus (2026-07-10) — `phase_center_offset_m` is compensated; see the fixture notes.
//!
//! Run with:
//!   cargo test -p antenna-model --test reference_validation -- --nocapture --test-threads=1

use antenna_model::config::CalibrationConfig;
use antenna_model::data::repository::CalibrationRepository;
use antenna_model::data::AntennaCalibration;
use antenna_model::model::{
    compute_gain_db, edge_taper_db, AntennaConfiguration, FeedParameters, IntegrationParams,
    MeshParameters, ReflectorGeometry,
};
use std::f64::consts::PI;
use std::path::PathBuf;

const SPEED_OF_LIGHT_M_S: f64 = 299_792_458.0;

/// One published reference performance point.
#[derive(Debug, Clone)]
struct ReferencePoint {
    antenna_id: String,
    feed_id: String,
    frequency_mhz: f64,
    #[allow(dead_code)] // provenance metadata: elevation at which peak was measured
    elevation_deg: f64,
    reference_gain_dbi: f64,
    reference_efficiency: f64,
    tolerance_db: f64,
    source: String,
}

/// Workspace-root-relative path resolved from this crate's manifest dir.
fn workspace_path(rel: &str) -> PathBuf {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    // The `antenna-model` crate lives one level below the workspace root.
    PathBuf::from(manifest_dir).join("..").join(rel)
}

/// Load every `*.psv` reference dataset in the directory (sorted for stable output).
fn load_all_reference_points(dir: &PathBuf) -> Vec<ReferencePoint> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading reference dataset dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "psv"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .psv reference datasets found in {}",
        dir.display()
    );

    let mut points = Vec::new();
    for path in &files {
        points.extend(parse_reference_file(path));
    }
    assert!(!points.is_empty(), "reference datasets are empty");
    points
}

/// Parse one pipe-delimited reference file, skipping `#` comments and blank lines.
fn parse_reference_file(path: &PathBuf) -> Vec<ReferencePoint> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("reading reference dataset {}: {e}", path.display()));

    let mut points = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('|').map(|f| f.trim()).collect();
        assert!(
            fields.len() >= 8,
            "{}: line {} has {} fields, expected 8: {:?}",
            path.display(),
            lineno + 1,
            fields.len(),
            line
        );
        let parse = |idx: usize, name: &str| -> f64 {
            fields[idx].parse::<f64>().unwrap_or_else(|e| {
                panic!(
                    "{}: line {} field '{}' = {:?}: {e}",
                    path.display(),
                    lineno + 1,
                    name,
                    fields[idx]
                )
            })
        };
        points.push(ReferencePoint {
            antenna_id: fields[0].to_string(),
            feed_id: fields[1].to_string(),
            frequency_mhz: parse(2, "frequency_mhz"),
            elevation_deg: parse(3, "elevation_deg"),
            reference_gain_dbi: parse(4, "reference_gain_dbi"),
            reference_efficiency: parse(5, "reference_efficiency"),
            tolerance_db: parse(6, "tolerance_db"),
            source: fields[7].to_string(),
        });
    }
    points
}

/// Load the real service antenna repository (from `calibration_data/antennas.yaml`).
fn load_real_repository() -> CalibrationRepository {
    let config = CalibrationConfig {
        data_directory: workspace_path("calibration_data"),
        antenna_config_file: workspace_path("calibration_data/antennas.yaml"),
        // Enabled antennas are all inline design-spec (uncalibrated); the disabled
        // calibrated entries reference .bin files we don't ship. Don't fail on them.
        fail_fast: false,
    };
    CalibrationRepository::load_from_config(&config)
        .expect("failed to load real calibration_data/antennas.yaml")
}

/// Diffraction-limited (100% efficiency) gain of a uniformly illuminated circular
/// aperture: `G = (pi * D / lambda)^2`, in dBi.
fn ideal_gain_dbi(diameter_m: f64, frequency_mhz: f64) -> f64 {
    let wavelength_m = SPEED_OF_LIGHT_M_S / (frequency_mhz * 1e6);
    let g_linear = (PI * diameter_m / wavelength_m).powi(2);
    10.0 * g_linear.log10()
}

/// Build the physics-model config for an uncalibrated antenna, feed placed AT FOCUS (on-axis):
/// measured peak gain is defined with the feed on the optical axis, so we ignore the design's
/// lateral multi-feed packaging offset (which only squints the beam). The design surface RMS is
/// kept, so the Ruze term still applies.
///
/// `q_override` substitutes a feed q-factor in place of the design spec's — used by the
/// feed-taper sweep to probe how illumination taper drives efficiency.
fn focused_config(cal: &AntennaCalibration, q_override: Option<f64>) -> AntennaConfiguration {
    let focal_length_m = cal.physical_config.reflector.focal_length_m;
    let diameter_m = cal.physical_config.reflector.diameter_m;

    let reflector = ReflectorGeometry::builder()
        .diameter(diameter_m)
        .focal_length(focal_length_m)
        .surface_rms(cal.physical_config.reflector.surface_rms_mm / 1000.0) // mm -> m
        .build()
        .expect("build reflector");

    let feed = FeedParameters::builder()
        .at_focus(focal_length_m)
        .q_factor(q_override.unwrap_or(cal.physical_config.feed.q_factor))
        .phase_center_offset(cal.physical_config.feed.phase_center_offset_m)
        .build()
        .expect("build feed");

    let mut builder = AntennaConfiguration::builder()
        .id(&cal.antenna_id)
        .name(&cal.metadata.antenna_name)
        .reflector(reflector)
        .feed(feed);

    if let Some(ref mesh) = cal.physical_config.mesh {
        let mesh = MeshParameters::builder()
            .spacing(mesh.mesh_spacing_mm / 1000.0)
            .wire_diameter(mesh.wire_diameter_mm / 1000.0)
            .build()
            .expect("build mesh");
        builder = builder.mesh(mesh);
    }

    builder.build().expect("build antenna configuration")
}

/// Predict peak (boresight) gain in dBi for an uncalibrated antenna at a given frequency.
///
/// Spillover is ON: matches the uncalibrated service path (no correction surface), and the
/// measured reference efficiency likewise includes spillover, so this is apples-to-apples.
fn predict_peak_gain_dbi(
    cal: &AntennaCalibration,
    frequency_mhz: f64,
    q_override: Option<f64>,
) -> f64 {
    let config = focused_config(cal, q_override);
    let mut params = IntegrationParams::fast();
    params.apply_spillover = true; // uncalibrated path folds in physical spillover

    let result = compute_gain_db(0.0, 0.0, &config, frequency_mhz * 1e6, &params)
        .expect("compute_gain_db at boresight");
    result.gain
}

#[test]
fn reference_residuals_within_tolerance() {
    let points = load_all_reference_points(&workspace_path(
        "antenna-model/tests/fixtures/reference_datasets",
    ));
    let repo = load_real_repository();

    println!("\n=== Uncalibrated model vs. published real-antenna references ===");
    println!(
        "{:<22} {:<7} {:>9} {:>7} {:>7} {:>7} {:>8} {:>8} {:>7}",
        "antenna",
        "feed",
        "freq_MHz",
        "ref_dBi",
        "mdl_dBi",
        "resid",
        "ref_eff",
        "mdl_eff",
        "verdict"
    );

    let mut failures: Vec<String> = Vec::new();

    for pt in &points {
        let cal = repo
            .get_calibration(&pt.antenna_id, &pt.feed_id)
            .unwrap_or_else(|| {
                panic!(
                    "antenna '{}' feed '{}' not found in real config (is it enabled?)",
                    pt.antenna_id, pt.feed_id
                )
            });

        let diameter_m = cal.physical_config.reflector.diameter_m;
        let predicted_dbi = predict_peak_gain_dbi(&cal, pt.frequency_mhz, None);
        let residual_db = predicted_dbi - pt.reference_gain_dbi;

        // Implied model aperture efficiency (relative to the diffraction limit), for an
        // interpretable side-by-side against the measured reference efficiency.
        let ideal_dbi = ideal_gain_dbi(diameter_m, pt.frequency_mhz);
        let model_eff = 10f64.powf((predicted_dbi - ideal_dbi) / 10.0);

        let within = residual_db.abs() <= pt.tolerance_db;
        let verdict = if within { "ok" } else { "OUT" };

        println!(
            "{:<22} {:<7} {:>9.1} {:>7.2} {:>7.2} {:>+7.2} {:>8.3} {:>8.3} {:>7}",
            pt.antenna_id,
            pt.feed_id,
            pt.frequency_mhz,
            pt.reference_gain_dbi,
            predicted_dbi,
            residual_db,
            pt.reference_efficiency,
            model_eff,
            verdict
        );

        if !within {
            failures.push(format!(
                "{}/{} @ {:.0} MHz: |residual| {:.2} dB > tolerance {:.2} dB (model {:.2} dBi vs ref {:.2} dBi) [src: {}]",
                pt.antenna_id,
                pt.feed_id,
                pt.frequency_mhz,
                residual_db.abs(),
                pt.tolerance_db,
                predicted_dbi,
                pt.reference_gain_dbi,
                pt.source
            ));
        }
    }
    println!();

    assert!(
        failures.is_empty(),
        "reference residual(s) exceeded tolerance:\n  {}",
        failures.join("\n  ")
    );
}

/// Feed-taper sensitivity sweep (documents *why* the design-spec q-factors were corrected).
///
/// Background: the uncalibrated model originally under-predicted DSN 34-m peak gain by ~5 dB.
/// Root cause was the design-spec feed `q_factor` (9.5 X-band) being far too high for this
/// dish's f/D (0.4). In this model `cos_q_pattern` is the FIELD amplitude, so q=9.5 yields a
/// ~-71 dB edge taper (optimal ~-11 dB) → the aperture rim is dark → efficiency collapses.
/// The config was fixed to q≈1.14 (~-11 dB edge taper via `q_factor_from_taper`).
///
/// This sweep is config-independent: it evaluates the model across q with an explicit
/// over-tapered baseline (`OVER_TAPERED_Q`) so it keeps demonstrating the physics regardless
/// of what the config currently holds. It guards two invariants:
///   1. peak efficiency occurs at a SENSIBLE q (broad feed), not the old horn-rule 8-12; and
///   2. that optimum beats the over-tapered baseline by several dB.
///
/// Run with: cargo test -p antenna-model --test reference_validation feed_taper -- --nocapture
#[test]
fn feed_taper_q_sweep_dsn_34m_xband() {
    /// The pre-fix design value — an explicit baseline so this test documents the bug
    /// independently of the (now-corrected) config.
    const OVER_TAPERED_Q: f64 = 9.5;

    let repo = load_real_repository();
    let cal = repo
        .get_calibration("dsn_34m_uncalibrated", "x_band")
        .expect("dsn_34m_uncalibrated x_band must be enabled in the real config");

    let frequency_mhz = 8420.0;
    let diameter_m = cal.physical_config.reflector.diameter_m;
    let focal_length_m = cal.physical_config.reflector.focal_length_m;
    let f_over_d = focal_length_m / diameter_m;
    let config_q = cal.physical_config.feed.q_factor;
    let ideal_dbi = ideal_gain_dbi(diameter_m, frequency_mhz);
    let reference_dbi = 68.0; // measured DSN 34-m X-band peak (see .psv fixture)

    println!(
        "\n=== Feed-taper q sweep — DSN 34-m, f/D={:.2}, X-band {:.0} MHz ===",
        f_over_d, frequency_mhz
    );
    println!(
        "diffraction limit (100% eff) = {:.2} dBi; measured reference = {:.2} dBi; \
         current config q = {:.2}; pre-fix q = {:.1}",
        ideal_dbi, reference_dbi, config_q, OVER_TAPERED_Q
    );
    println!(
        "{:>6} {:>13} {:>9} {:>8} {:>9}",
        "q", "edge_taper_dB", "gain_dBi", "eff", "vs_ref_dB"
    );

    // Include the live config q so the "<- config" row actually appears (it falls between
    // grid points, so it must be inserted explicitly, then the grid sorted).
    let mut q_grid: Vec<f64> = vec![
        0.5,
        0.75,
        1.0,
        1.25,
        1.5,
        2.0,
        2.5,
        3.0,
        4.0,
        6.0,
        8.0,
        OVER_TAPERED_Q,
        config_q,
    ];
    q_grid.sort_by(|a, b| a.partial_cmp(b).expect("no NaN q values"));
    let mut best_q = q_grid[0];
    let mut best_gain = f64::NEG_INFINITY;
    for &q in &q_grid {
        let gain_dbi = predict_peak_gain_dbi(&cal, frequency_mhz, Some(q));
        let eff = 10f64.powf((gain_dbi - ideal_dbi) / 10.0);
        let taper = edge_taper_db(q, f_over_d);
        let tag = if (q - OVER_TAPERED_Q).abs() < 1e-9 {
            " <- pre-fix"
        } else if (q - config_q).abs() < 1e-9 {
            " <- config"
        } else {
            ""
        };
        println!(
            "{:>6.2} {:>13.1} {:>9.2} {:>8.3} {:>+9.2}{}",
            q,
            taper,
            gain_dbi,
            eff,
            gain_dbi - reference_dbi,
            tag
        );
        if gain_dbi > best_gain {
            best_gain = gain_dbi;
            best_q = q;
        }
    }

    let best_eff = 10f64.powf((best_gain - ideal_dbi) / 10.0);
    let over_tapered_gain = predict_peak_gain_dbi(&cal, frequency_mhz, Some(OVER_TAPERED_Q));
    println!(
        "\nbest on grid: q={:.2} -> {:.2} dBi (eff {:.3}), {:+.2} dB vs measured reference {:.2} dBi",
        best_q,
        best_gain,
        best_eff,
        best_gain - reference_dbi,
        reference_dbi
    );
    println!(
        "over-tapered q={:.1} -> {:.2} dBi: {:.2} dB worse than the optimum -> gross edge under-illumination.\n",
        OVER_TAPERED_Q,
        over_tapered_gain,
        best_gain - over_tapered_gain
    );

    // Invariant 1: the efficiency optimum is a broad feed, not the old horn-rule 8-12.
    assert!(
        best_q <= 3.0,
        "peak-gain q should be a broad feed (<=3) for f/D {f_over_d:.2}, got {best_q}"
    );
    // Invariant 2: that optimum beats the over-tapered baseline by several dB.
    assert!(
        best_gain > over_tapered_gain + 3.0,
        "sensible q should beat over-tapered q={OVER_TAPERED_Q} by >3 dB; best {best_gain:.2} vs {over_tapered_gain:.2}"
    );
}

/// ITU-R S.580 co-polar sidelobe reference envelope, in dBi, for off-axis angle `theta_deg`.
///
/// S.580-6 design objective (applies for D/λ ≥ 50): ≥90% of sidelobe peaks should not exceed
///   29 − 25·log10(θ)   dBi,  for 1° ≤ θ ≤ 20°
///   −3.5 dBi,                for 20° < θ ≤ 26.3°
/// Returns `None` inside the main-beam region (θ < 1°), where the envelope does not apply.
fn itu_r_s580_mask_dbi(theta_deg: f64) -> Option<f64> {
    if (1.0..=20.0).contains(&theta_deg) {
        Some(29.0 - 25.0 * theta_deg.log10())
    } else if theta_deg > 20.0 && theta_deg <= 26.3 {
        Some(-3.5)
    } else {
        None
    }
}

#[test]
#[ignore = "diagnostic; run explicitly"]
fn itu_probe_fine_envelope() {
    let repo = load_real_repository();
    let cal = repo
        .get_calibration("gs_3.7m_uncalibrated", "x_band_feed")
        .unwrap();
    let config = focused_config(&cal, None);
    let freq_hz = 8000.0e6;
    let mut params = IntegrationParams::high_accuracy();
    params.apply_spillover = true;
    let peak = compute_gain_db(0.0, 0.0, &config, freq_hz, &params)
        .unwrap()
        .gain;

    // Fine sweep; track running gain to detect local maxima (sidelobe peaks).
    const STEP: f64 = 0.2;
    let mut prev2 = f64::NEG_INFINITY;
    let mut prev1 = f64::NEG_INFINITY; // gain one STEP behind `theta`
    println!("\npeak(boresight) = {peak:.2} dBi. Sidelobe PEAKS (local maxima) vs S.580:");
    println!(
        "{:>7} {:>9} {:>9} {:>9} {:>9}",
        "θ(deg)", "peak_dBi", "rel_dB", "mask_dBi", "margin"
    );
    let mut theta: f64 = 0.4;
    while theta <= 20.001 {
        let g = compute_gain_db(theta.to_radians(), 0.0, &config, freq_hz, &params)
            .unwrap()
            .gain;
        // prev1 = gain(theta - STEP); it is a local max if it exceeds both neighbors
        // (prev2 at theta-2·STEP, g at theta). Its angle is therefore theta - STEP.
        let peak_theta = theta - STEP;
        if prev1 > prev2 && prev1 > g && peak_theta >= 1.0 {
            let mask = itu_r_s580_mask_dbi(peak_theta).unwrap_or(f64::NAN);
            println!(
                "{:>7.2} {:>9.2} {:>9.2} {:>9.2} {:>+9.2}",
                peak_theta,
                prev1,
                prev1 - peak,
                mask,
                mask - prev1
            );
        }
        prev2 = prev1;
        prev1 = g;
        theta += STEP;
    }
}

/// Off-axis pattern check against the ITU-R S.580 sidelobe envelope.
///
/// This is a DIFFERENT harness mode from the peak-gain rows: it validates beam/sidelobe SHAPE,
/// not peak efficiency. We sweep the modeled gain vs off-axis angle (φ=0 cut) and compare to the
/// 29−25·log10(θ) envelope.
///
/// Antenna choice matters: physical-optics far-sidelobe accuracy needs the aperture-phase
/// variation (∝ D·sinθ/λ) to be resolved by the integration grid, which is infeasible for an
/// electrically huge dish. So we use the SMALL 3.7 m ground station at X-band (D/λ ≈ 99, still
/// ≥ 50 as S.580 requires) with high-accuracy integration, where the sidelobes are computable.
///
/// The S.580 objective allows 10% of sidelobe peaks to exceed the envelope, and the near-in first
/// sidelobes routinely do. So the guard is deliberately lenient near the main beam and strict in
/// the far region (θ ≥ 5°), where a physical pattern must be well below the envelope. The printed
/// table is the deliverable. Run with:
///   cargo test -p antenna-model --test reference_validation itu -- --nocapture
#[test]
fn itu_r_s580_sidelobe_envelope_small_dish() {
    let repo = load_real_repository();
    let cal = repo
        .get_calibration("gs_3.7m_uncalibrated", "x_band_feed")
        .expect("gs_3.7m_uncalibrated x_band_feed must be enabled");
    let config = focused_config(&cal, None);

    let frequency_mhz = 8000.0;
    let frequency_hz = frequency_mhz * 1e6;
    let d = cal.physical_config.reflector.diameter_m;
    let lambda = SPEED_OF_LIGHT_M_S / frequency_hz;
    let d_over_lambda = d / lambda;
    assert!(
        d_over_lambda >= 50.0,
        "ITU-R S.580 applies for D/λ ≥ 50; got {d_over_lambda:.1}"
    );

    // High-accuracy integration: far sidelobes need fine aperture sampling.
    let mut params = IntegrationParams::high_accuracy();
    params.apply_spillover = true;

    println!(
        "\n=== ITU-R S.580 sidelobe envelope — GS 3.7 m, X-band {:.0} MHz (D/λ={:.0}) ===",
        frequency_mhz, d_over_lambda
    );
    println!(
        "{:>7} {:>9} {:>9} {:>9} {:>8}",
        "θ(deg)", "mdl_dBi", "mask_dBi", "margin", "verdict"
    );

    let thetas: [f64; 9] = [1.0, 1.5, 2.0, 3.0, 5.0, 7.0, 10.0, 15.0, 20.0];
    let mut far_violations: Vec<String> = Vec::new();
    for &theta_deg in &thetas {
        let theta_rad = theta_deg.to_radians();
        let gain = compute_gain_db(theta_rad, 0.0, &config, frequency_hz, &params)
            .expect("compute_gain_db off-axis")
            .gain;
        let mask = itu_r_s580_mask_dbi(theta_deg).expect("mask defined on [1,20]");
        let margin = mask - gain; // positive => model is under the envelope (good)
        let verdict = if margin >= 0.0 { "under" } else { "OVER" };
        println!(
            "{:>7.1} {:>9.2} {:>9.2} {:>+9.2} {:>8}",
            theta_deg, gain, mask, margin, verdict
        );
        // Strict only in the far region, where a physical pattern must be well suppressed.
        if theta_deg >= 5.0 && margin < 0.0 {
            far_violations.push(format!(
                "θ={theta_deg:.1}°: model {gain:.2} dBi exceeds S.580 envelope {mask:.2} dBi"
            ));
        }
    }
    println!();

    assert!(
        far_violations.is_empty(),
        "model sidelobes exceed the ITU-R S.580 envelope in the far region (θ≥5°):\n  {}",
        far_violations.join("\n  ")
    );
}
