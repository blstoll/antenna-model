//! End-to-end test of the `calibrate` binary in full mode, on perturbed-truth data.

mod support;

use calibrate::{AntennaClassRegistry, CorrectionSurfaceParams};
use support::*;

#[test]
fn generator_is_deterministic() {
    let a = rows_to_csv(&generate_rows());
    let b = rows_to_csv(&generate_rows());
    assert_eq!(a, b, "the fixture generator must be byte-reproducible");
}

#[test]
fn generator_grid_satisfies_the_fitter_constraints() {
    let rows = generate_rows();

    assert_eq!(rows.len(), FIXTURE_ROW_COUNT);
    assert!(
        rows.len() >= 200,
        "a 5-fold CV training split must still clear the fitter's 125-point minimum, \
         got {} rows",
        rows.len()
    );

    let freq_span =
        FIXTURE_FREQUENCIES_MHZ[FIXTURE_FREQUENCIES_MHZ.len() - 1] - FIXTURE_FREQUENCIES_MHZ[0];
    let cone_span = FIXTURE_CONE_DEG[FIXTURE_CONE_DEG.len() - 1] - FIXTURE_CONE_DEG[0];
    let clock_span = FIXTURE_CLOCK_DEG[FIXTURE_CLOCK_DEG.len() - 1] - FIXTURE_CLOCK_DEG[0];

    // Knot *counts* full mode fits with. These are not importable — they're a private
    // local in `calibrate/src/main.rs::surface_fitting_params` (4/6/8) — so they're
    // mirrored here as named constants. If that function's knot counts change, these
    // must change with it.
    const NUM_KNOTS_FREQUENCY: f64 = 4.0;
    const NUM_KNOTS_ECONE: f64 = 6.0;
    const NUM_KNOTS_ECLOCK: f64 = 8.0;

    // The minimum knot *spacing* floors, by contrast, ARE importable: they're public
    // fields of `CorrectionSurfaceParams`, and `default()` carries the same values
    // `surface_fitting_params` hardcodes. Deriving the required spans from here means a
    // change to the floors (e.g. widening `min_knot_spacing_frequency`) is caught
    // automatically instead of silently passing against a stale bare literal.
    let knot_floors = CorrectionSurfaceParams::default();
    let required_freq_span = NUM_KNOTS_FREQUENCY * knot_floors.min_knot_spacing_frequency;
    let required_cone_span = NUM_KNOTS_ECONE * knot_floors.min_knot_spacing_econe;
    let required_clock_span = NUM_KNOTS_ECLOCK * knot_floors.min_knot_spacing_eclock;

    assert!(
        freq_span >= required_freq_span,
        "frequency span {freq_span} MHz too narrow for {NUM_KNOTS_FREQUENCY} knots at \
         {} MHz minimum spacing (need >= {required_freq_span})",
        knot_floors.min_knot_spacing_frequency
    );
    assert!(
        cone_span >= required_cone_span,
        "cone span {cone_span} deg too narrow for {NUM_KNOTS_ECONE} knots at {} deg \
         minimum spacing (need >= {required_cone_span})",
        knot_floors.min_knot_spacing_econe
    );
    assert!(
        clock_span >= required_clock_span,
        "clock span {clock_span} deg too narrow for {NUM_KNOTS_ECLOCK} knots at {} deg \
         minimum spacing (need >= {required_clock_span})",
        knot_floors.min_knot_spacing_eclock
    );
}

/// Standing pin on roadmap unit D11: the fixture must contain rows the pre-D11 parser
/// discarded (it rejected anything below -20 dB/K as "atypical G/T", which is a boresight
/// figure, silently dropping legitimate sidelobe measurements).
#[test]
fn generator_produces_realistic_sub_minus_twenty_sidelobes() {
    let rows = generate_rows();
    let deep = rows.iter().filter(|r| r.g_over_t_db < -20.0).count();

    assert!(
        deep * 5 >= rows.len(),
        "at least 20% of rows should sit below -20 dB/K (D11 pin), got {deep} of {}",
        rows.len()
    );

    let min = rows
        .iter()
        .map(|r| r.g_over_t_db)
        .fold(f64::INFINITY, f64::min);
    println!("fixture: {} rows, minimum G/T {:.2} dB/K", rows.len(), min);
}

#[test]
fn injected_bias_is_bounded() {
    // The bias must stay well inside the accuracy targets it will be measured against.
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &f in &FIXTURE_FREQUENCIES_MHZ {
        for &cone in &FIXTURE_CONE_DEG {
            for &clock in &FIXTURE_CLOCK_DEG {
                let b = injected_bias_db(f, cone, clock);
                min = min.min(b);
                max = max.max(b);
            }
        }
    }
    assert!(min > -1.0 && max < 3.0, "bias range [{min}, {max}] dB");
    println!("injected bias range: [{min:.3}, {max:.3}] dB");
}

/// The bounds check above says nothing about smoothness: swap the clock cosine for a
/// sawtooth and min/max stay identical. Smoothness is what "deliberately smooth at the
/// scale of the knot spacing" (see the doc comment on `BIAS_*_DB` in support/mod.rs)
/// actually buys us, and it's load-bearing for a later task's recovery assertion — a
/// 4/6/8-knot spline cannot represent a bias with high-frequency content.
///
/// Extrapolate each axis's worst-case adjacent-grid-point change out to the fitter's
/// minimum knot span for that axis; the result must stay under the axis's own
/// coefficient, i.e. even over the tightest span the fitter is willing to place a knot,
/// the term must not swing past its own designed amplitude.
#[test]
fn injected_bias_is_smooth() {
    let knot_floors = CorrectionSurfaceParams::default();

    let freq_rate = FIXTURE_FREQUENCIES_MHZ
        .windows(2)
        .map(|w| {
            (injected_bias_db(w[1], 0.0, 0.0) - injected_bias_db(w[0], 0.0, 0.0)).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let freq_change_per_knot_span = freq_rate * knot_floors.min_knot_spacing_frequency;
    assert!(
        freq_change_per_knot_span < BIAS_FREQ_DB,
        "frequency-axis bias changes {freq_change_per_knot_span:.4} dB over one minimum \
         knot span ({} MHz) — exceeds its own amplitude ({BIAS_FREQ_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_frequency
    );

    let cone_rate = FIXTURE_CONE_DEG
        .windows(2)
        .map(|w| {
            (injected_bias_db(400.0, w[1], 0.0) - injected_bias_db(400.0, w[0], 0.0)).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let cone_change_per_knot_span = cone_rate * knot_floors.min_knot_spacing_econe;
    assert!(
        cone_change_per_knot_span < BIAS_CONE_DB,
        "cone-axis bias changes {cone_change_per_knot_span:.4} dB over one minimum knot \
         span ({} deg) — exceeds its own amplitude ({BIAS_CONE_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_econe
    );

    let clock_rate = FIXTURE_CLOCK_DEG
        .windows(2)
        .map(|w| {
            (injected_bias_db(400.0, 0.0, w[1]) - injected_bias_db(400.0, 0.0, w[0])).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let clock_change_per_knot_span = clock_rate * knot_floors.min_knot_spacing_eclock;
    assert!(
        clock_change_per_knot_span < BIAS_CLOCK_DB,
        "clock-axis bias changes {clock_change_per_knot_span:.4} dB over one minimum \
         knot span ({} deg) — exceeds its own amplitude ({BIAS_CLOCK_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_eclock
    );
}

/// Drift guard: `support::fixture_config` hardcodes the `UHF_Array_Element` parameters
/// (rather than loading `antenna_classes.yaml` itself) so that `generate_rows()` stays
/// free of file I/O and the physics config stays visibly self-contained. That means
/// nothing fails automatically if the YAML entry is edited — the drift would otherwise
/// surface much later as a confusing recovery-tolerance failure in a binary-execution
/// test, with no obvious link back to "the fixture is stale". This test closes that gap
/// by asserting the hardcoded values still match the registry, field by field.
#[test]
fn fixture_config_matches_antenna_classes_yaml() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("antenna_classes.yaml");
    let registry = AntennaClassRegistry::load_from_file(&path).unwrap_or_else(|e| {
        panic!(
            "failed to load antenna classes from {}: {e}",
            path.display()
        )
    });
    let class = registry
        .get_class(FIXTURE_CLASS)
        .unwrap_or_else(|| panic!("{FIXTURE_CLASS} missing from {}", path.display()));

    assert_eq!(
        class.geometry.diameter_m, 8.0,
        "fixture is stale: antenna_classes.yaml diameter_m for {FIXTURE_CLASS} no longer \
         matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.geometry.f_over_d, 0.45,
        "fixture is stale: antenna_classes.yaml f_over_d for {FIXTURE_CLASS} no longer \
         matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.q_factor, 5.0,
        "fixture is stale: antenna_classes.yaml feed.q_factor for {FIXTURE_CLASS} no \
         longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.phase_center_offset_wavelengths, 0.0,
        "fixture is stale: antenna_classes.yaml feed.phase_center_offset_wavelengths for \
         {FIXTURE_CLASS} no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.asymmetry_factor, 1.1,
        "fixture is stale: antenna_classes.yaml feed.asymmetry_factor for {FIXTURE_CLASS} \
         no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.mesh.spacing_mm, 10.0,
        "fixture is stale: antenna_classes.yaml mesh.spacing_mm for {FIXTURE_CLASS} no \
         longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.mesh.wire_diameter_mm, 1.0,
        "fixture is stale: antenna_classes.yaml mesh.wire_diameter_mm for {FIXTURE_CLASS} \
         no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.surface.rms_mm, NOMINAL_SURFACE_RMS_MM,
        "fixture is stale: antenna_classes.yaml surface.rms_mm for {FIXTURE_CLASS} no \
         longer matches support::NOMINAL_SURFACE_RMS_MM — update fixture_config"
    );
    assert_eq!(
        class.system_noise_temperature_k, FIXTURE_TEMPERATURE_K,
        "fixture is stale: antenna_classes.yaml system_noise_temperature_k for \
         {FIXTURE_CLASS} no longer matches support::FIXTURE_TEMPERATURE_K — update \
         fixture_config"
    );
}
