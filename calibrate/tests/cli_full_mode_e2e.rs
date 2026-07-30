//! End-to-end test of the `calibrate` binary in full mode, on perturbed-truth data.

mod support;

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

    assert!(
        freq_span >= 200.0,
        "frequency span {freq_span} MHz too narrow"
    );
    assert!(cone_span >= 12.0, "cone span {cone_span} deg too narrow");
    assert!(clock_span >= 40.0, "clock span {clock_span} deg too narrow");
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
fn injected_bias_is_bounded_and_smooth() {
    // The bias must stay well inside the accuracy targets it will be measured against,
    // and vary smoothly enough for a 4/6/8-knot spline to represent it.
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
