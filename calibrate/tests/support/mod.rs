//! Deterministic perturbed-truth fixture generation for the `calibrate` CLI tests.
//!
//! The measurements written here are produced by evaluating the *real* physics model
//! with a **perturbed** surface RMS and adding a **closed-form bias**. Calibrating the
//! *nominal* `UHF_Array_Element` class against this data therefore has a known answer:
//! the residual the correction surface must absorb is the injected bias plus the
//! (smooth, small) difference the RMS perturbation makes.
//!
//! Nothing here uses randomness, wall-clock time, or hash iteration order — two runs
//! produce byte-identical output, which the CLI test relies on.

#![allow(dead_code)] // Not every consumer of this module uses every helper.

use antenna_model::model::{
    compute_g_over_t, AntennaConfiguration, AntennaConfigurationBuilder, FeedParametersBuilder,
    IntegrationParams, MeshParametersBuilder, ReflectorGeometryBuilder,
};
use std::path::Path;

// ============================================================================
// The injected truth
// ============================================================================

/// Antenna class the fixture is generated for and calibrated against.
///
/// Chosen for its broad beam: at 450 MHz this class measures 8.91 dB/K at boresight,
/// 7.93 at 2°, 2.55 at 5°, −19.75 at 10° and −41.53 at 20°. A narrow-beam class such as
/// `GroundStation_13m` (33.08 → −10.60 dB/K between 0° and 1°) has a sub-degree main lobe
/// that the fitter's angular knots cannot represent.
///
/// **That choice was right for this fixture's purpose and wrong to leave unexamined**, which
/// is what happened for a month: it made the pipeline's angular-resolution limit invisible to
/// every test in the suite until D14 fitted a narrow-beam antenna and D21 measured it. The
/// limit is no longer silent — `calibrate` warns and the artifact records the figures — but
/// this comment is the reason it was ever silent, so it stays. Do not resolve a resolution
/// finding by picking a broader-beam fixture again; see roadmap D21 and D24.
pub const FIXTURE_CLASS: &str = "UHF_Array_Element";

/// Nominal surface RMS of `UHF_Array_Element`, from `calibrate/antenna_classes.yaml`.
pub const NOMINAL_SURFACE_RMS_MM: f64 = 2.0;

/// Surface RMS the "measurements" are actually generated at.
///
/// This is the perturbation `--tune-parameters` must recover. It is the ONLY physical
/// parameter perturbed: `TunableParameters` carries `surface_rms_mm`, `mesh_spacing_mm`
/// and `mesh_wire_diameter_mm` only, so a q-factor perturbation could never be recovered
/// by the full-mode tuner and would just add an unattributable residual.
pub const PERTURBED_SURFACE_RMS_MM: f64 = 2.6;

/// E/H illumination asymmetry of `UHF_Array_Element`, from `calibrate/antenna_classes.yaml`.
///
/// The largest of any shipped class, and the geometry roadmap **D23** measured its worst case
/// on: 1.20 dB at cone 14° / 700 MHz between this value and the 1.0 the service used to
/// substitute, against 0.0003 dB at boresight. Named here rather than written inline so
/// [`fixture_config`] and the artifact assertion in `cli_full_mode_e2e.rs` cannot drift apart —
/// if they did, the fixture would be *generated* with one illumination and *checked* against
/// another, which is a small-scale copy of the defect D23 closed.
pub const FIXTURE_ASYMMETRY_FACTOR: f64 = 1.1;

/// System noise temperature of `UHF_Array_Element`, from `antenna_classes.yaml`.
/// The CSV's `temperature_k` column must match this or the G/T values are inconsistent
/// with what the calibrator computes.
pub const FIXTURE_TEMPERATURE_K: f64 = 100.0;

/// Coefficients of the injected systematic bias, in dB.
///
/// `bias(f, cone, clock) = A + B*(f - f0)/f_span + C*cos(clock) + D*(cone/cone_span)`
///
/// Deliberately smooth at the scale of the knot spacing (50 MHz / 2° / 5°): one cosine
/// cycle over the full clock range and linear ramps elsewhere. A higher-frequency bias
/// would be unrepresentable by a 4/6/8-knot spline and the recovery assertion would fail
/// for legitimate reasons.
pub const BIAS_CONST_DB: f64 = 0.80;
pub const BIAS_FREQ_DB: f64 = 0.50;
pub const BIAS_CLOCK_DB: f64 = 0.60;
pub const BIAS_CONE_DB: f64 = 0.40;

// ============================================================================
// Grid definition
// ============================================================================

// The grid is sized by roadmap D20, which made the fitter reject an underdetermined
// system instead of silently fitting one. Three constraints set it, in order:
//
// 1. **The coefficient count is what must be covered**, not the old `(order+1)³ = 125`.
//    Full mode requests 4/6/8 internal knots at order 4 (`main.rs::surface_fitting_params`),
//    so each axis contributes `placed_internal_knots + order` basis functions, capping at
//    8 × 10 × 12 = **960** coefficients once every axis carries enough distinct values to
//    place every requested knot.
// 2. **An axis only reaches its requested knot count if it has the distinct values to
//    support them.** Knots are placed at data quantiles and must be strictly interior
//    (roadmap D19), so `n` internal knots need at least `n + 2` distinct values on that
//    axis. Hence >= 6 frequencies, >= 8 cone angles, >= 10 clock angles.
// 3. **Cross-validation folds must clear the count too.** `cli_cv_folds_controls_the_
//    reported_fold_count` exercises `--cv-folds 3`, whose training split is 2/3 of the
//    grid, so the grid needs >= 960 / (2/3) = 1440 rows before that test can pass.
//
// 1728 rows gives the 3-fold training split 1152 points against 960 coefficients. That is
// the thinnest margin any test here runs at; the full-grid fit sees 1.8x its coefficients.
//
// This fixture is synthetic and known to be unrealistic — no public antenna calibration
// dataset of this shape exists (see the D14 assessment in docs/roadmap-2026-07.md §1). It
// is sized to the production configuration deliberately, rather than the production
// configuration being cut down to fit it, because the target is a real high-quality
// dataset in a production environment.

/// Frequencies in MHz. Span 300 MHz across 6 values at 60 MHz steps — over the 50 MHz
/// minimum knot spacing, with 4 interior values for the artifact's 4 frequency knots.
pub const FIXTURE_FREQUENCIES_MHZ: [f64; 6] = [400.0, 460.0, 520.0, 580.0, 640.0, 700.0];

/// E-cone (polar) angles in degrees. Spans 0–24°: main lobe (0–5°), the shoulder, and
/// deep sidelobes past 10° where G/T falls below −20 dB/K. Deliberately non-uniform —
/// denser near boresight, where the pattern actually varies — with every adjacent pair at
/// least the 2° minimum knot spacing apart, so quantile-placed knots are not thinned by
/// `enforce_min_spacing`. 12 values leaves 10 interior for the artifact's 6 cone knots.
pub const FIXTURE_CONE_DEG: [f64; 12] = [
    0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 19.0, 21.5, 24.0,
];

/// E-clock (azimuthal) angles in degrees. Spans 0–345° in 15° steps — over the 5°
/// minimum knot spacing, with 22 interior values for the artifact's 8 clock knots.
pub const FIXTURE_CLOCK_DEG: [f64; 24] = [
    0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0, 105.0, 120.0, 135.0, 150.0, 165.0, 180.0, 195.0,
    210.0, 225.0, 240.0, 255.0, 270.0, 285.0, 300.0, 315.0, 330.0, 345.0,
];

/// Total rows the generator emits: 6 × 12 × 24 = 1728.
pub const FIXTURE_ROW_COUNT: usize =
    FIXTURE_FREQUENCIES_MHZ.len() * FIXTURE_CONE_DEG.len() * FIXTURE_CLOCK_DEG.len();

/// The injected bias in dB at a grid location. Pure function — no state, no RNG.
pub fn injected_bias_db(frequency_mhz: f64, e_cone_deg: f64, e_clock_deg: f64) -> f64 {
    let f_lo = FIXTURE_FREQUENCIES_MHZ[0];
    let f_hi = FIXTURE_FREQUENCIES_MHZ[FIXTURE_FREQUENCIES_MHZ.len() - 1];
    let cone_hi = FIXTURE_CONE_DEG[FIXTURE_CONE_DEG.len() - 1];

    BIAS_CONST_DB
        + BIAS_FREQ_DB * (frequency_mhz - f_lo) / (f_hi - f_lo)
        + BIAS_CLOCK_DB * e_clock_deg.to_radians().cos()
        + BIAS_CONE_DB * (e_cone_deg / cone_hi)
}

/// Build the physics configuration for `UHF_Array_Element` at a given surface RMS.
///
/// Mirrors `calibrate/src/main.rs::compute_model_predictions` exactly — same builders,
/// same mm→m conversions, same at-focus feed placement. If that function changes, this
/// must change with it or the fixture stops being perturbed truth.
pub fn fixture_config(surface_rms_mm: f64) -> AntennaConfiguration {
    let diameter_m = 8.0;
    let f_over_d = 0.45;
    let focal_length = diameter_m * f_over_d;

    let reflector = ReflectorGeometryBuilder::default()
        .diameter(diameter_m)
        .focal_length(focal_length)
        .surface_rms(surface_rms_mm / 1000.0)
        .build()
        .expect("fixture reflector geometry");

    let feed = FeedParametersBuilder::default()
        .at_focus(focal_length)
        .q_factor(5.0)
        .phase_center_offset(0.0)
        .asymmetry_factor(FIXTURE_ASYMMETRY_FACTOR)
        .build()
        .expect("fixture feed parameters");

    let mesh = MeshParametersBuilder::default()
        .spacing(10.0 / 1000.0)
        .wire_diameter(1.0 / 1000.0)
        .build()
        .expect("fixture mesh parameters");

    AntennaConfigurationBuilder::default()
        .id("UHF_Array_Element")
        .name("UHF phased array element (low frequency)")
        .reflector(reflector)
        .feed(feed)
        .mesh(mesh)
        .build()
        .expect("fixture antenna configuration")
}

/// One generated measurement row.
pub struct FixtureRow {
    pub e_clock_deg: f64,
    pub e_cone_deg: f64,
    pub frequency_mhz: f64,
    pub g_over_t_db: f64,
    pub temperature_k: f64,
}

/// Generate the full perturbed-truth grid.
pub fn generate_rows() -> Vec<FixtureRow> {
    generate_grid(injected_bias_db)
}

/// Generate the same grid with the systematic bias omitted.
///
/// The tuner and the correction surface are each exercised by a known answer, and on one
/// fixture those two answers fight each other. The injected bias averages **+1.22 dB**,
/// while the entire 2.0 → 2.6 mm surface-RMS perturbation moves G/T by only **0.003 dB at
/// 400 MHz and 0.010 dB at 700 MHz** — Ruze loss is `exp(-(4πσ/λ)²)` and λ is 43–75 cm in
/// this band, so surface RMS is very nearly inert here. The bias is 120–360× larger *and*
/// has the same near-constant shape, so the two are confounded: minimising RMSE against
/// biased data is best served by raising gain, which means driving surface RMS to its
/// **lower** bound, away from truth. Measured 2026-07-31, once the degenerate-simplex crash
/// was fixed: the tuned run reported 0.1 mm against a 2.6 mm truth.
///
/// Removing the bias makes surface RMS the only thing that can explain the residual, so the
/// objective's minimum sits exactly at `PERTURBED_SURFACE_RMS_MM` and the tuner has a
/// genuine known answer to recover. The correction-surface recovery assertions keep using
/// [`generate_rows`], which is what the bias is for.
pub fn generate_rows_without_bias() -> Vec<FixtureRow> {
    generate_grid(|_, _, _| 0.0)
}

/// Shared grid walk. `bias` is `(frequency_mhz, e_cone_deg, e_clock_deg) -> dB`.
fn generate_grid(bias: impl Fn(f64, f64, f64) -> f64) -> Vec<FixtureRow> {
    let config = fixture_config(PERTURBED_SURFACE_RMS_MM);
    let params = IntegrationParams::default();
    let mut rows = Vec::with_capacity(FIXTURE_ROW_COUNT);

    for &frequency_mhz in &FIXTURE_FREQUENCIES_MHZ {
        for &e_cone_deg in &FIXTURE_CONE_DEG {
            for &e_clock_deg in &FIXTURE_CLOCK_DEG {
                let truth = compute_g_over_t(
                    e_cone_deg.to_radians(),
                    e_clock_deg.to_radians(),
                    &config,
                    frequency_mhz * 1e6,
                    FIXTURE_TEMPERATURE_K,
                    &params,
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "fixture G/T evaluation failed at f={frequency_mhz} MHz \
                         cone={e_cone_deg} deg clock={e_clock_deg} deg: {e}"
                    )
                });

                rows.push(FixtureRow {
                    e_clock_deg,
                    e_cone_deg,
                    frequency_mhz,
                    g_over_t_db: truth + bias(frequency_mhz, e_cone_deg, e_clock_deg),
                    temperature_k: FIXTURE_TEMPERATURE_K,
                });
            }
        }
    }

    rows
}

/// Render rows as CSV text in the full-mode column order.
///
/// Fixed 6-decimal formatting keeps the output byte-identical across runs and platforms.
pub fn rows_to_csv(rows: &[FixtureRow]) -> String {
    let mut csv = String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
    for r in rows {
        csv.push_str(&format!(
            "{:.6},{:.6},{:.6},{:.6},{:.6}\n",
            r.e_clock_deg, r.e_cone_deg, r.frequency_mhz, r.g_over_t_db, r.temperature_k
        ));
    }
    csv
}

/// Generate the fixture and write it to `path`.
pub fn write_fixture_csv(path: &Path) -> Vec<FixtureRow> {
    let rows = generate_rows();
    std::fs::write(path, rows_to_csv(&rows)).expect("write fixture CSV");
    rows
}
