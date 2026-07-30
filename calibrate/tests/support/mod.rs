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
/// that the fitter's 2° minimum E-cone knot spacing cannot represent.
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

/// Frequencies in MHz. Span 300 MHz across 4 values — comfortably over the 50 MHz
/// minimum knot spacing with the artifact's 4 frequency knots.
pub const FIXTURE_FREQUENCIES_MHZ: [f64; 4] = [400.0, 500.0, 600.0, 700.0];

/// E-cone (polar) angles in degrees. Spans 0–24°: main lobe (0–5°), the shoulder, and
/// deep sidelobes past 10° where G/T falls below −20 dB/K.
pub const FIXTURE_CONE_DEG: [f64; 9] = [0.0, 2.0, 4.0, 6.0, 9.0, 12.0, 16.0, 20.0, 24.0];

/// E-clock (azimuthal) angles in degrees. Spans 0–315° in 45° steps — over the 5°
/// minimum knot spacing with the artifact's 8 clock knots.
pub const FIXTURE_CLOCK_DEG: [f64; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

/// Total rows the generator emits: 4 × 9 × 8 = 288.
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
        .asymmetry_factor(1.1)
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
                    g_over_t_db: truth + injected_bias_db(frequency_mhz, e_cone_deg, e_clock_deg),
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
