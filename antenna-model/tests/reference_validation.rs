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

// ===========================================================================
// F7 sidelobe-floor validation against measured reference sidelobe data.
//
// The uncalibrated model now applies a Ruze scattered-power floor
// (`pattern::sidelobe_floor_gain`, gated on the uncalibrated path) so its
// off-axis gain is *envelope-conservative* rather than systematically
// optimistic. These tests pin the floor's calibration (`OMEGA_SCATTER`) as a
// one-sided conservative bound on measured sidelobe peaks — the F7 register
// decision (2026-07-12): "at or above measured peaks, never optimistic".
//
// Data (tests/fixtures/reference_datasets/sidelobe_data/, kept out of the
// peak-gain harness dir so `load_all_reference_points` does not ingest it):
//   - NTIA Report 84-164: absolute-dBi sidelobe-peak percentile envelopes for
//     22 C-band earth stations (PRIMARY — absolute levels, directly comparable
//     to the model floor).
//   - NASA CR-159703: rel-to-peak sidelobe peaks with surface-condition
//     provenance (CROSS-CHECK — validates the floor's surface-error scaling).
// ===========================================================================

fn sidelobe_data_path(file: &str) -> PathBuf {
    workspace_path(&format!(
        "antenna-model/tests/fixtures/reference_datasets/sidelobe_data/{file}"
    ))
}

/// The model's Ruze sidelobe floor in dBi for a representative reflector of the
/// given surface RMS at the given frequency. The floor is diameter-independent
/// (see `OMEGA_SCATTER`), so diameter/feed here are immaterial placeholders.
fn model_floor_dbi(surface_rms_m: f64, frequency_mhz: f64) -> f64 {
    let reflector = ReflectorGeometry::builder()
        .diameter(4.5)
        .focal_length(2.25)
        .surface_rms(surface_rms_m)
        .build()
        .expect("build reflector");
    let feed = FeedParameters::builder()
        .at_focus(2.25)
        .q_factor(2.0)
        .build()
        .expect("build feed");
    let config = AntennaConfiguration::builder()
        .id("floor_ref")
        .name("floor_ref")
        .reflector(reflector)
        .feed(feed)
        .build()
        .expect("build config");
    let wavelength = SPEED_OF_LIGHT_M_S / (frequency_mhz * 1e6);
    let floor_lin = antenna_model::model::pattern::sidelobe_floor_gain(&config, wavelength);
    10.0 * floor_lin.log10()
}

/// One NTIA 84-164 wide-angle statistical sidelobe bin (subset "all").
struct NtiaBin {
    band_center_mhz: f64,
    bin_lo_deg: f64,
    /// Provenance: the conservative-envelope columns. Retained because the F7
    /// register decision (envelope -> best-estimate) turned on the p90-vs-median
    /// choice; keeping them documents what was NOT chosen.
    #[allow(dead_code)]
    p90_dbi: f64,
    #[allow(dead_code)]
    max_dbi: f64,
    median_dbi: f64,
}

fn parse_ntia_bins() -> Vec<NtiaBin> {
    let text = std::fs::read_to_string(sidelobe_data_path("ntia_84_164_sidelobe_statistics.psv"))
        .expect("read NTIA sidelobe stats");
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('|').collect();
        // figure|band_mhz|subset|bin_lo|bin_hi|n|max|p90|median|p10|min
        if f.len() < 11 || f[2] != "all" {
            continue; // "all" avoids triple-counting the D/λ sub-populations
        }
        let (lo, hi) = f[1].split_once('-').expect("band range like 3700-4200");
        let band_center_mhz = (lo.parse::<f64>().unwrap() + hi.parse::<f64>().unwrap()) / 2.0;
        out.push(NtiaBin {
            band_center_mhz,
            bin_lo_deg: f[3].parse().unwrap(),
            max_dbi: f[6].parse().unwrap(),
            p90_dbi: f[7].parse().unwrap(),
            median_dbi: f[8].parse().unwrap(),
        });
    }
    assert!(!out.is_empty(), "parsed no NTIA bins");
    out
}

/// PRIMARY: the surface-scatter floor is a BEST-ESTIMATE that tracks the NTIA
/// 84-164 wide-angle **median** sidelobe level (F7 register decision revised
/// 2026-07-12: best-estimate, not conservative envelope — link budget / G/T need
/// accuracy, and a one-sided upper bound is anti-conservative for desired-signal
/// margin).
///
/// Also pins the two structural properties of the Ω = 4π (isotropic) derivation:
///   * POWER CONSERVATION — the pedestal never radiates more than Ruze removes.
///   * CEILING — the floor can never exceed 0 dBi, so it cannot swamp a main beam.
///
/// NOTE: this validates the floor LEVEL only. It deliberately does not exercise
/// `compute_gain`, because the served aperture integral aliases badly off-axis
/// (see docs/findings-2026-07-13-off-axis-integration-aliasing.md) — the floor
/// cannot be validated end-to-end until that P0 is fixed.
#[test]
fn sidelobe_floor_tracks_measured_median() {
    // Representative surface RMS for the NTIA C-band earth-station class: ~1 mm.
    // Their ~55–65% aperture efficiency (report gain table) implies a Ruze factor
    // ~0.97, i.e. ~1 mm RMS at C-band. This is the calibration's key assumption.
    const REP_RMS_M: f64 = 0.001;
    const WIDE_ANGLE_MIN_DEG: f64 = 40.0;
    // Band-mean residual is -2.04 dB @3950 MHz / +2.90 dB @6175 MHz (mean |err| 2.47 dB).
    // Two independent sources of spread, both documented limitations of a FLAT pedestal:
    //   * across FREQUENCY — Ruze scales as (rms/λ)² but the measured floor is nearly
    //     flat, evidence the real floor is dominated by unmodeled spillover/blockage/
    //     edge-diffraction rather than surface scatter;
    //   * across ANGLE — the measured median itself varies ±3-4 dB bin-to-bin (note the
    //     back-lobe bump near 90-100°), structure a flat pedestal cannot reproduce.
    // Hence a ±6 dB per-bin band; the band-MEAN error is the tighter ~2.5 dB figure.
    const ACCURACY_BAND_DB: f64 = 6.0;

    let bins = parse_ntia_bins();

    println!("\n=== F7 surface-scatter floor vs NTIA 84-164 wide-angle MEDIAN ===");
    println!(
        "{:<10} {:>8} {:>9} {:>9} {:>9} {:>8}",
        "band_MHz", "bin_deg", "floor_dBi", "med_dBi", "err", "verdict"
    );

    let mut checked = 0usize;
    let mut within = 0usize;
    let mut worst = 0.0f64;
    for b in bins.iter().filter(|b| b.bin_lo_deg >= WIDE_ANGLE_MIN_DEG) {
        let floor = model_floor_dbi(REP_RMS_M, b.band_center_mhz);
        let err = floor - b.median_dbi;
        let ok = err.abs() <= ACCURACY_BAND_DB;
        checked += 1;
        if ok {
            within += 1;
        }
        if err.abs() > worst.abs() {
            worst = err;
        }
        println!(
            "{:<10.0} {:>6.0}   {:>9.2} {:>9.2} {:>+9.2} {:>8}",
            b.band_center_mhz,
            b.bin_lo_deg,
            floor,
            b.median_dbi,
            err,
            if ok { "ok" } else { "OUT" }
        );
    }
    println!("\nwithin ±{ACCURACY_BAND_DB} dB: {within}/{checked}; worst err {worst:+.2} dB");
    assert!(
        within * 10 >= checked * 9,
        "floor must track the NTIA wide-angle median within ±{ACCURACY_BAND_DB} dB for >=90% \
         of bins; got {within}/{checked} (worst {worst:+.2} dB)"
    );

    // STRUCTURAL 1 — power conservation. The pedestal is applied over the whole 4π
    // sphere, so the power it radiates is exactly its directivity. That must never
    // exceed the scattered power available (p_scatter = 1 - η_ruze).
    for &(rms, f_mhz) in &[
        (0.0005, 3950.0),
        (0.001, 6175.0),
        (0.005, 12_000.0),
        (0.02, 32_000.0),
    ] {
        let lambda = SPEED_OF_LIGHT_M_S / (f_mhz * 1e6);
        let p_scatter = 1.0 - antenna_model::model::ruze_efficiency(rms, lambda);
        let radiated = 10f64.powf(model_floor_dbi(rms, f_mhz) / 10.0);
        assert!(
            radiated <= p_scatter + 1e-12,
            "floor radiates {radiated:.4} but only {p_scatter:.4} was scattered \
             (rms {rms} m @ {f_mhz} MHz) — Ω must be 4π for power conservation"
        );
    }

    // STRUCTURAL 2 — ceiling. p_scatter <= 1 ⇒ floor <= 0 dBi, always. This is what
    // guarantees the floor can never swamp a main beam or a near-in sidelobe.
    for &(rms, f_mhz) in &[(0.001, 3950.0), (0.01, 32_000.0), (0.5, 32_000.0)] {
        let floor = model_floor_dbi(rms, f_mhz);
        assert!(
            floor <= 0.0 + 1e-9,
            "floor {floor:+.2} dBi exceeds the 0 dBi ceiling (rms {rms} m @ {f_mhz} MHz)"
        );
    }
}

/// CROSS-CHECK: the floor's surface-error scaling matches the direction measured
/// in NASA CR-159703 — worse surface → higher sidelobes. The data (rel-to-peak,
/// near-in ±12°) shows as-delivered/warped surfaces peaking well above reshaped
/// ones; the model floor likewise rises monotonically with surface RMS. (NASA is
/// a *direction* cross-check, not an absolute-floor bound: its cuts are near-in,
/// where the model's diffraction pattern — not the floor — sets the level.)
#[test]
fn sidelobe_floor_surface_scaling_matches_nasa() {
    let text = std::fs::read_to_string(sidelobe_data_path("nasa_cr159703_pattern_peaks.psv"))
        .expect("read NASA pattern peaks");

    // Worst (highest, closest to 0) detached-sidelobe level per surface class.
    let mut worst_good = f64::NEG_INFINITY; // reshaped / recontoured
    let mut worst_poor = f64::NEG_INFINITY; // as-delivered / warped
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("cut_id") {
            continue;
        }
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 11 {
            continue;
        }
        let angle: f64 = f[9].parse().unwrap();
        let level: f64 = f[10].parse().unwrap(); // dB rel to peak
        let cond = f[8];
        // Detached sidelobes only (skip main-lobe shoulders near the first nulls).
        if angle.abs() < 5.0 {
            continue;
        }
        let good = cond.contains("reshap") || cond.contains("recontour");
        let poor =
            cond.contains("as_delivered") || cond.contains("warp") || cond.contains("as_received");
        if good {
            worst_good = worst_good.max(level);
        } else if poor {
            worst_poor = worst_poor.max(level);
        }
    }

    println!("\n=== F7 floor surface scaling vs NASA CR-159703 ===");
    println!("worst detached sidelobe (dB rel peak): reshaped {worst_good:.1}, as-delivered {worst_poor:.1}");

    assert!(
        worst_good.is_finite() && worst_poor.is_finite(),
        "expected both good and poor surface cuts in NASA data"
    );
    // Data direction: degraded surfaces have higher (worse) sidelobes.
    assert!(
        worst_poor > worst_good,
        "NASA data should show as-delivered sidelobes ({worst_poor:.1} dB) worse than \
         reshaped ({worst_good:.1} dB)"
    );

    // Model direction: the floor rises monotonically with surface RMS at 12 GHz
    // (matching the data's surface → sidelobe trend). Representative good→poor RMS.
    let floors: Vec<f64> = [0.0003_f64, 0.0010, 0.0016, 0.0030]
        .iter()
        .map(|&rms| model_floor_dbi(rms, 12_000.0))
        .collect();
    println!("model floor dBi at 12 GHz for RMS 0.3/1.0/1.6/3.0 mm: {floors:?}");
    for w in floors.windows(2) {
        assert!(
            w[1] > w[0],
            "model floor must increase with surface RMS (got {floors:?})"
        );
    }
}

// ===========================================================================
// P10 SPIKE — Hankel/Bessel aperture integral vs the brute-force 2D quadrature.
//
// The azimuthal integral has a closed form (Jacobi-Anger):
//     ∫₀²π exp(-j·a·cos(φ-φ')) dφ' = 2π·J₀(a),   a = k·ρ·sinθ
// so for an azimuthally symmetric aperture the 2D integral collapses to a 1D
// radial (Hankel) transform:
//     I(θ) = 2π ∫₀^R A(ρ)·exp(j·k·ρ²/(4f)·(1-cosθ))·J₀(k·ρ·sinθ)·ρ dρ
// Ground truth: the converged 2048x4096 2D quadrature gives -33.28 dBi @ θ=90°.
// ===========================================================================

/// Bessel J₀ (Numerical Recipes): polynomial for |x|<8, asymptotic beyond.
fn bessel_j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let p1 = 57_568_490_574.0
            + y * (-13_362_590_354.0
                + y * (651_619_640.7
                    + y * (-11_214_424.18 + y * (77_392.330_17 + y * (-184.905_245_6)))));
        let p2 = 57_568_490_411.0
            + y * (1_029_532_985.0
                + y * (9_494_680.718 + y * (59_272.648_53 + y * (267.853_271_2 + y))));
        p1 / p2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 0.785_398_164;
        let p1 = 1.0
            + y * (-0.109_862_862_7e-2
                + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let p2 = -0.156_249_999_5e-1
            + y * (0.143_048_876_5e-3
                + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 + y * (-0.934_935_152e-7))));
        (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p1 - z * xx.sin() * p2)
    }
}

/// Hankel-form aperture integral for an azimuthally symmetric aperture.
/// Simpson's rule over ρ with `n_rho` points (odd).
fn hankel_field(
    cfg: &AntennaConfiguration,
    theta: f64,
    frequency_hz: f64,
    n_rho: usize,
) -> num_complex::Complex64 {
    use num_complex::Complex64;
    let wl = SPEED_OF_LIGHT_M_S / frequency_hz;
    let k = 2.0 * PI / wl;
    let f = cfg.reflector.focal_length;
    let r_max = cfg.reflector.diameter / 2.0;

    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let mut sum = Complex64::new(0.0, 0.0);
    for i in 0..n {
        let rho = i as f64 * h;
        let w = if i == 0 || i == n - 1 {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        // A(ρ): illumination amplitude (φ'-independent for an on-axis feed)
        let amp = antenna_model::model::illumination_amplitude(rho, 0.0, &cfg.feed, f);
        // ρ-only aperture phase: the dish-depth chirp k·ρ²/(4f)·(1-cosθ).
        // (Feed at focus + no mesh + surface_error=0 ⇒ no other ρ-only phase.)
        let chirp = k * rho * rho / (4.0 * f) * (1.0 - theta.cos());
        let j0 = bessel_j0(k * rho * theta.sin());
        let val = Complex64::new(0.0, chirp).exp() * amp * j0 * rho;
        sum += val * w;
    }
    sum * (h / 3.0) * 2.0 * PI
}

/// P10 SPIKE (diagnostic, #[ignore]d — the 2D reference legs take ~0.4 s each).
/// Cross-validates the 1D Hankel form against the brute-force 2D quadrature and
/// pins the convergence + cost result recorded in
/// docs/findings-2026-07-13-off-axis-integration-aliasing.md §5.
///   cargo test --release -p antenna-model --test reference_validation p10_spike -- --ignored --nocapture
#[test]
#[ignore = "diagnostic spike for P10; run explicitly with --ignored"]
fn p10_spike_hankel_vs_2d() {
    use antenna_model::model::integration::{integrate_amplitude_squared, integrate_aperture};
    use antenna_model::model::{ruze_efficiency, IntegrationParams};
    use std::time::Instant;

    let repo = load_real_repository();
    let cal = repo
        .get_calibration("dsn_34m_uncalibrated", "x_band")
        .expect("dsn_34m");
    let cfg = focused_config(&cal, None);
    let f_hz = 8.4e9;
    let wl = SPEED_OF_LIGHT_M_S / f_hz;
    let d_lam = cfg.reflector.diameter / wl;

    // Same downstream gain formula the model uses:
    //   gain = η · (4π/λ²) · |I|² / ∬|A|² dA
    let eta = ruze_efficiency(cfg.reflector.surface_rms, wl);
    let amp_sq = integrate_amplitude_squared(&cfg, 513, 1025);
    let to_dbi = |field: num_complex::Complex64| {
        let g = eta * (4.0 * PI / (wl * wl)) * field.norm_sqr() / amp_sq;
        10.0 * g.log10()
    };

    println!("\n=== P10 SPIKE: Hankel (1D) vs brute-force 2D quadrature ===");
    println!("dsn_34m X-band, D/λ = {d_lam:.0}\n");

    // --- ground truth: converged 2D at 2048 x 4096 ---
    let mut gt = IntegrationParams::high_accuracy();
    gt.min_rho_points = 2048;
    gt.max_rho_points = 2048;
    gt.min_phi_points = 4096;
    gt.max_phi_points = 4096;
    gt.max_iterations = 1;
    gt.apply_sidelobe_floor = false;

    println!(
        "{:>7} {:>16} {:>12} | {:>16} {:>12} {:>10}",
        "theta", "2D(2048x4096)", "ms", "Hankel(4097)", "ms", "Δ dB"
    );
    for deg in [0.0_f64, 1.0, 5.0, 20.0, 90.0] {
        let th = deg.to_radians();

        let t = Instant::now();
        let r2d = integrate_aperture(th, 0.0, &cfg, f_hz, &gt);
        let ms2d = t.elapsed().as_secs_f64() * 1000.0;
        let g2d = to_dbi(r2d.expect("2D integrate").field);

        let t = Instant::now();
        let fh = hankel_field(&cfg, th, f_hz, 4097);
        let msh = t.elapsed().as_secs_f64() * 1000.0;
        let gh = to_dbi(fh);

        println!(
            "{deg:>7.0} {g2d:>16.2} {ms2d:>12.1} | {gh:>16.2} {msh:>12.3} {:>10.2}",
            gh - g2d
        );
    }

    // --- Hankel radial-convergence sweep at theta = 90 deg ---
    println!(
        "\nHankel radial convergence at θ=90° (Nyquist N_rho ≈ {:.0}):",
        2.0 * d_lam
    );
    println!("{:>8} {:>12} {:>10}", "N_rho", "gain_dBi", "ms");
    for n in [257usize, 513, 1025, 2049, 4097, 8193, 16385] {
        let t = Instant::now();
        let fh = hankel_field(&cfg, 90f64.to_radians(), f_hz, n);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("{n:>8} {:>12.2} {ms:>10.3}", to_dbi(fh));
    }
}
