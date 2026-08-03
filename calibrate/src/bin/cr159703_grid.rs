//! Real-anchored measurement-grid generator for the NASA CR-159703 1.22 m dish
//! (roadmap unit **D14**).
//!
//! # What this produces, and what it is not
//!
//! It writes a full-mode measurement CSV that `calibrate --calibration-mode full` can fit.
//! **The dataset is not measured data and must never be quoted as such.** It is this
//! repository's own physics model, evaluated over a dense grid, plus a residual derived
//! from real digitized measurements — the "hybrid fill" the maintainer approved on
//! 2026-07-29 after the calibration-data assessment concluded that no published dataset
//! can drive full-mode fitting (the fitter needs ≥ 960 points over a real 3D domain; the
//! best real single-configuration set in existence here is ~11 envelope peaks at one
//! frequency).
//!
//! Every fabricated element is listed by [`FABRICATIONS`] below, echoed to stdout on every
//! run, and copied into the `--summary` JSON.
//!
//! # The fill, step by step
//!
//! 1. **Anchors.** Read the digitized sidelobe-envelope peaks (`--peaks`, the committed
//!    `nasa_cr159703_pattern_peaks.psv`) for the two 1.22 m Kumar-feed cuts that cover both
//!    principal planes of the same reflector state: [`H_CUT_ID`] and [`E_CUT_ID`].
//! 2. **Absolute levels.** The chart levels are relative to the main-beam apex; the report's
//!    text gain for this configuration ([`ABSOLUTE_ANCHOR_DBI`]) makes them absolute.
//! 3. **Residuals.** `residual = measured_peak − model_envelope(cone)`, both in dBi. The
//!    comparison is envelope-to-envelope (see [`ENVELOPE_HALF_WIDTH_DEG`]): the digitized
//!    data is a peak envelope, and the model's lobes sit at their own angles, so comparing
//!    at a single angle would difference a measured peak against a modelled null.
//! 4. **Trend across cone.** A weighted least-squares quadratic per half-plane, held
//!    constant beyond that half-plane's outermost anchor. Individual peaks deviate from the
//!    trend by several dB — that is genuine lobe-to-lobe structure, and it is *deliberately*
//!    not carried into the fill, because the shipped correction surface cannot represent it
//!    (2° minimum cone knot spacing against a 1.16° lobe period at this D/λ). See
//!    `docs/findings-2026-08-02-correction-surface-angular-resolution.md`.
//! 5. **Interpolation across clock.** The four half-planes sit at clock 0/90/180/270; the
//!    unique band-limited interpolant through four equally spaced samples fills between them.
//! 6. **Frequency.** The residual is assumed flat across the report's 11.7–12.2 GHz band
//!    (the digitized cuts are single-frequency). The *model* still varies with frequency, so
//!    the synthesized measurements do too.
//!
//! # Why the generator loads the antenna class rather than hardcoding it
//!
//! `calibrate` builds its physics model from `--classes-file`/`--antenna-class`. This binary
//! takes the same two arguments and builds the same model from the same file, so the fill's
//! "model" and the calibrator's "model" cannot drift apart. The one thing that *is* restated
//! here is the builder wiring (mm→m conversions, feed at focus), mirroring
//! `main.rs::compute_model_predictions`; the D14 e2e test pins the two against each other by
//! asserting calibrate's reported model-only RMSE equals the residual RMS reported here.

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;

use antenna_model::model::{
    compute_gain_db, g_over_t_from_gain_db, AntennaConfiguration, AntennaConfigurationBuilder,
    FeedParametersBuilder, IntegrationParams, MeshParametersBuilder, ReflectorGeometryBuilder,
};
use calibrate::{AntennaClass, AntennaClassRegistry};

// ============================================================================
// The real data this is anchored to
// ============================================================================

/// H-plane cut: report figure 4.22 (PDF page 74) — the best 1.22 m H-plane cut.
const H_CUT_ID: &str = "122_kumar_C_h_121";

/// E-plane cut: report figure 4.37 (PDF page 86).
///
/// **Fabrication:** this is Series *D* — the same reflector and feed as the Series C
/// H-plane cut with the mixer stub turned outward, which the report says removed a spurious
/// E-plane lobe. Pairing it with the Series C H-plane cut assumes the stub rotation left the
/// H-plane unchanged. It is the only digitized E-plane cut of this reflector state, and
/// without it the clock axis would rest on an axisymmetry assumption instead of on data.
const E_CUT_ID: &str = "122_kumar_D_e_121";

/// Absolute gain of the modelled configuration, dBi. Report text, page 58 of the PDF
/// ("best gain and sidelobe definition"), also recorded in the PSV header.
const ABSOLUTE_ANCHOR_DBI: f64 = 41.4;

/// Uncertainty assigned to the absolute anchor, dB.
///
/// The report states the gain to 0.1 dB; 0.5 dB is a conventional allowance for a
/// gain-comparison measurement of this era and is used both as the boresight anchor's
/// fitting weight and as a term in the e2e test's uncertainty budget. It is **common mode**
/// — every absolute level below is referenced to it — so it shifts the whole fill together
/// and never distorts its shape.
const ABSOLUTE_ANCHOR_UNCERTAINTY_DB: f64 = 0.5;

/// Frequency of both digitized cuts, MHz (PSV `frequency_ghz` column = 12.1).
const ANCHOR_FREQUENCY_MHZ: f64 = 12100.0;

/// Half-width of the envelope window, degrees.
///
/// The digitized rows are *peaks of an envelope*, so the model has to be read the same way:
/// `model_envelope(cone) = max |G| over cone ± this`. The value is half the lobe period
/// λ/D = 1.16° at 12.1 GHz, which guarantees the window contains a modelled lobe peak
/// however far the modelled and measured lobe positions have drifted apart.
///
/// It is deliberately *not* widened to also cover the ±0.3–0.5° digitization angle
/// uncertainty: measured 2026-08-02, a ±1.2° window at the innermost E-plane anchor (3.2°)
/// reaches into the main-lobe skirt and returns a value 13.7 dB above the local sidelobe,
/// which would corrupt the residual it is supposed to measure.
const ENVELOPE_HALF_WIDTH_DEG: f64 = 0.6;

/// Sampling step of the model pattern used to build the envelope, degrees.
const ENVELOPE_SAMPLE_STEP_DEG: f64 = 0.05;

// ============================================================================
// The synthesized grid
// ============================================================================

/// Frequencies, MHz. The report's broadcasting band is 11.7–12.2 GHz.
///
/// Six values at 100 MHz spacing: over the fitter's 50 MHz minimum knot spacing, and enough
/// distinct values for all four requested frequency knots to be placed strictly inside the
/// axis (roadmap D19 — `n` interior knots need `n + 2` distinct values).
const FREQUENCIES_MHZ: [f64; 6] = [11700.0, 11800.0, 11900.0, 12000.0, 12100.0, 12200.0];

/// E-cone (polar) angles, degrees: 0–14° in 1° steps.
///
/// The digitized peaks reach 12.0° (H) and 10.3° (E), so the grid covers the measured span
/// with margin for the envelope window. 15 values leave 13 interior positions for the
/// fitter's 6 cone knots, which land on 2/4/6/8/10/12° — exactly the 2° minimum spacing.
const CONE_DEG: [f64; 15] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
];

/// E-clock (azimuthal) angles, degrees: 0–350° in 10° steps.
///
/// 36 values, well over both the 5° minimum knot spacing and the 10 distinct values the
/// fitter's 8 clock knots need.
const CLOCK_DEG: [f64; 36] = [
    0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0,
    150.0, 160.0, 170.0, 180.0, 190.0, 200.0, 210.0, 220.0, 230.0, 240.0, 250.0, 260.0, 270.0,
    280.0, 290.0, 300.0, 310.0, 320.0, 330.0, 340.0, 350.0,
];

/// Total rows: 6 × 15 × 36 = 3240, against the 8 × 10 × 12 = 960 coefficients the shipped
/// knot counts declare (roadmap D20). A 5-fold cross-validation's training split still sees
/// 2592 points, so `--validate` is usable on this grid.
const ROW_COUNT: usize = FREQUENCIES_MHZ.len() * CONE_DEG.len() * CLOCK_DEG.len();

/// Clock angle assigned to each digitized half-plane, degrees.
///
/// **Fabrication.** The charts record signed angles either side of boresight; which side is
/// "clock 0" is a convention, not a measurement, and the model is azimuthally symmetric so
/// nothing physical depends on the choice. H-plane on 0/180, E-plane on 90/270 is the
/// standard principal-plane layout.
const CLOCK_H_POSITIVE: f64 = 0.0;
const CLOCK_E_POSITIVE: f64 = 90.0;
const CLOCK_H_NEGATIVE: f64 = 180.0;
const CLOCK_E_NEGATIVE: f64 = 270.0;

/// Everything in the output that is not a measurement. Echoed on every run.
const FABRICATIONS: &[&str] = &[
    "The dataset is the repository's own physical-optics model plus a residual trend; only \
     the residual is derived from measurements, and only at the digitized peak angles.",
    "Between the digitized peaks — and everywhere off the two principal planes — the grid is \
     model, not measurement.",
    "The residual is a weighted least-squares quadratic in cone angle per half-plane, held \
     constant beyond the outermost digitized peak of that half-plane. Individual peaks \
     deviate from it by several dB (see the summary's anchor table); that lobe-to-lobe \
     structure is intentionally not carried.",
    "Between the four measured half-planes the residual is the band-limited trigonometric \
     interpolant through clock 0/90/180/270; no data constrains it there.",
    "The residual is assumed flat across 11.7-12.2 GHz. Both digitized cuts are at 12.1 GHz.",
    "The E-plane cut is the report's Series D (mixer stub turned outward) paired with the \
     Series C H-plane cut, assuming the stub rotation left the H-plane unchanged.",
    "Absolute levels come from the report's 41.4 dBi text gain for this configuration; the \
     charts themselves are referenced to their own main-beam apex.",
    "The system noise temperature is assumed (the report publishes gain, not G/T) and \
     cancels: it is written into the CSV and read back from the antenna class.",
    "Which side of the chart becomes clock 0 versus clock 180 is a convention.",
];

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "cr159703_grid")]
#[command(
    about = "Generate the NASA CR-159703 real-anchored full-mode measurement grid (roadmap D14)",
    long_about = None
)]
struct Args {
    /// Digitized sidelobe-envelope peaks (PSV).
    #[arg(
        long,
        default_value = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../antenna-model/tests/fixtures/reference_datasets/sidelobe_data/\
             nasa_cr159703_pattern_peaks.psv"
        )
    )]
    peaks: PathBuf,

    /// Antenna class definitions (the same file `calibrate --classes-file` is given).
    #[arg(
        long,
        default_value = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/nasa_cr159703_122m_classes.yaml"
        )
    )]
    classes_file: PathBuf,

    /// Antenna class to build the physics model from.
    #[arg(long, default_value = "NASA_CR159703_1p22m")]
    antenna_class: String,

    /// Output measurement CSV.
    #[arg(short, long)]
    output: PathBuf,

    /// Optional JSON summary: anchors, trend, fabrications, injected residual RMS.
    #[arg(long)]
    summary: Option<PathBuf>,
}

// ============================================================================
// Digitized peaks
// ============================================================================

/// One digitized envelope peak.
#[derive(Debug, Clone)]
struct DigitizedPeak {
    cut_id: String,
    /// As drawn on the chart: negative is left of boresight.
    peak_angle_deg: f64,
    level_db_rel_peak: f64,
    uncertainty_db: f64,
}

/// Parse the committed PSV, keeping the two cuts this fill is built from.
///
/// `#` lines are provenance, not data — the file documents its own digitization method and
/// has to stay runnable as committed (the same property roadmap D13 gave the boresight
/// parser).
fn read_peaks(path: &PathBuf) -> Result<Vec<DigitizedPeak>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read digitized peaks: {}", path.display()))?;

    let mut header: Option<Vec<String>> = None;
    let mut peaks = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();

        let Some(header) = header.as_ref() else {
            header = Some(fields.iter().map(|s| s.to_string()).collect());
            continue;
        };

        let column = |name: &str| -> Result<&str> {
            let idx = header
                .iter()
                .position(|h| h == name)
                .with_context(|| format!("{}: no '{name}' column", path.display()))?;
            fields
                .get(idx)
                .copied()
                .with_context(|| format!("{}:{line_no}: missing '{name}' field", path.display()))
        };

        let cut_id = column("cut_id")?.to_string();
        if cut_id != H_CUT_ID && cut_id != E_CUT_ID {
            continue;
        }

        let parse = |name: &str| -> Result<f64> {
            let raw = column(name)?;
            raw.parse::<f64>()
                .with_context(|| format!("{}:{line_no}: '{name}' = {raw:?}", path.display()))
        };

        // Guards on the two columns whose value this fill silently depends on. A cut
        // recorded at another frequency or another dish would fit just as smoothly and be
        // wrong; catching it here beats explaining an unexplained residual later.
        let frequency_ghz = parse("frequency_ghz")?;
        if (frequency_ghz * 1000.0 - ANCHOR_FREQUENCY_MHZ).abs() > 1e-6 {
            bail!(
                "{}:{line_no}: cut {cut_id} is recorded at {frequency_ghz} GHz, but the fill \
                 treats every anchor as {ANCHOR_FREQUENCY_MHZ} MHz",
                path.display()
            );
        }
        let diameter_m = parse("antenna_diameter_m")?;
        if (diameter_m - 1.22).abs() > 1e-9 {
            bail!(
                "{}:{line_no}: cut {cut_id} is a {diameter_m} m antenna; this generator models \
                 the 1.22 m dish",
                path.display()
            );
        }

        // The trend fit weights by `1 / uncertainty²`, so a zero (or negative, or non-finite)
        // uncertainty is not a small number — it is `inf`, and it turns every fitted
        // coefficient into `NaN`. That failure does not surface here: the elimination's
        // `pivot < 1e-12` guard is *false* for NaN, so NaNs flow into the residual, into the
        // CSV, and finally out of `calibrate`'s parser as dropped malformed rows, a long way
        // from the column that caused them. Guarded at the point of entry, like the two
        // columns above.
        let uncertainty_db = parse("uncertainty_db")?;
        if !(uncertainty_db.is_finite() && uncertainty_db > 0.0) {
            bail!(
                "{}:{line_no}: cut {cut_id} carries uncertainty_db = {uncertainty_db}; the \
                 residual trend weights anchors by 1/uncertainty², which requires a finite \
                 positive value",
                path.display()
            );
        }

        peaks.push(DigitizedPeak {
            cut_id,
            peak_angle_deg: parse("peak_angle_deg")?,
            level_db_rel_peak: parse("level_db_rel_peak")?,
            uncertainty_db,
        });
    }

    if peaks.is_empty() {
        bail!(
            "{} contains no rows for cuts {H_CUT_ID} / {E_CUT_ID}",
            path.display()
        );
    }

    Ok(peaks)
}

// ============================================================================
// The physics model, built from the same class file `calibrate` will use
// ============================================================================

/// Mirrors `calibrate/src/main.rs::compute_model_predictions` — same builders, same mm→m
/// conversions, same at-focus feed. If that function changes, this must change with it, and
/// the D14 e2e test's model-only-RMSE agreement assertion is what will notice.
fn model_config(class: &AntennaClass) -> Result<AntennaConfiguration> {
    let focal_length = class.geometry.diameter_m * class.geometry.f_over_d;

    let reflector = ReflectorGeometryBuilder::default()
        .diameter(class.geometry.diameter_m)
        .focal_length(focal_length)
        .surface_rms(class.surface.rms_mm / 1000.0)
        .build()
        .map_err(|e| anyhow::anyhow!("reflector geometry: {e}"))?;

    let feed = FeedParametersBuilder::default()
        .at_focus(focal_length)
        .q_factor(class.feed.q_factor)
        .phase_center_offset(class.feed.phase_center_offset_wavelengths)
        .asymmetry_factor(class.feed.asymmetry_factor)
        .build()
        .map_err(|e| anyhow::anyhow!("feed parameters: {e}"))?;

    let mesh = MeshParametersBuilder::default()
        .spacing(class.mesh.spacing_mm / 1000.0)
        .wire_diameter(class.mesh.wire_diameter_mm / 1000.0)
        .build()
        .map_err(|e| anyhow::anyhow!("mesh parameters: {e}"))?;

    AntennaConfigurationBuilder::default()
        .id(&class.class_id)
        .name(&class.description)
        .reflector(reflector)
        .feed(feed)
        .mesh(mesh)
        .build()
        .map_err(|e| anyhow::anyhow!("antenna configuration: {e}"))
}

/// The integration settings the calibrator will use on this data.
///
/// `default()` with the uncorrected-physics gates **off**, exactly as
/// `compute_model_predictions` sets them: full mode always attaches a correction surface, so
/// the service evaluates the resulting artifact with spillover and the F7 floor off, and the
/// residual this generator injects must be defined against that same model (roadmap D17).
fn integration_params() -> IntegrationParams {
    IntegrationParams::default().with_uncorrected_physics_gates(false)
}

/// Model gain in dBi at a polar angle, in degrees.
fn model_gain_dbi(
    config: &AntennaConfiguration,
    cone_deg: f64,
    frequency_mhz: f64,
    params: &IntegrationParams,
) -> Result<f64> {
    Ok(compute_gain_db(
        cone_deg.to_radians(),
        0.0,
        config,
        frequency_mhz * 1e6,
        params,
    )
    .with_context(|| format!("model gain at cone {cone_deg}°, {frequency_mhz} MHz"))?
    .gain)
}

/// The model's peak envelope near `cone_deg`: `max` over `± ENVELOPE_HALF_WIDTH_DEG`.
fn model_envelope_dbi(
    config: &AntennaConfiguration,
    cone_deg: f64,
    frequency_mhz: f64,
    params: &IntegrationParams,
) -> Result<f64> {
    let lo = (cone_deg - ENVELOPE_HALF_WIDTH_DEG).max(0.0);
    let hi = cone_deg + ENVELOPE_HALF_WIDTH_DEG;
    let steps = ((hi - lo) / ENVELOPE_SAMPLE_STEP_DEG).round() as usize;

    let mut best = f64::NEG_INFINITY;
    for i in 0..=steps {
        let theta = lo + (i as f64) * ENVELOPE_SAMPLE_STEP_DEG;
        best = best.max(model_gain_dbi(config, theta, frequency_mhz, params)?);
    }
    Ok(best)
}

// ============================================================================
// Residual trend
// ============================================================================

/// One anchor: a digitized peak, made absolute and differenced against the model.
#[derive(Debug, Clone, serde::Serialize)]
struct Anchor {
    cut_id: String,
    clock_deg: f64,
    cone_deg: f64,
    level_db_rel_peak: f64,
    uncertainty_db: f64,
    measured_dbi: f64,
    model_envelope_dbi: f64,
    residual_db: f64,
    /// The fitted trend at this anchor — what the fill actually injects here.
    trend_db: f64,
    /// `residual_db − trend_db`: the lobe-scale structure the fill does not carry.
    deviation_from_trend_db: f64,
}

/// A half-plane's residual: a quadratic in cone, held flat past the outermost anchor.
#[derive(Debug, Clone)]
struct HalfPlaneTrend {
    clock_deg: f64,
    coefficients: [f64; 3],
    max_anchor_cone_deg: f64,
}

impl HalfPlaneTrend {
    fn evaluate(&self, cone_deg: f64) -> f64 {
        let x = cone_deg.min(self.max_anchor_cone_deg);
        self.coefficients[0] + self.coefficients[1] * x + self.coefficients[2] * x * x
    }
}

/// Weighted least-squares quadratic through `(x, y)` with weights `w`.
///
/// Solved as the 3×3 normal equations by Gaussian elimination with partial pivoting. Three
/// unknowns against at least four anchors per half-plane, so the system is well determined;
/// a singular matrix here means an axis collapsed and is an error, not something to
/// regularize away.
fn weighted_quadratic_fit(x: &[f64], y: &[f64], w: &[f64]) -> Result<[f64; 3]> {
    const N: usize = 3;
    let mut a = [[0.0_f64; N]; N];
    let mut b = [0.0_f64; N];

    for k in 0..x.len() {
        let basis = [1.0, x[k], x[k] * x[k]];
        for i in 0..N {
            b[i] += w[k] * basis[i] * y[k];
            for j in 0..N {
                a[i][j] += w[k] * basis[i] * basis[j];
            }
        }
    }

    for i in 0..N {
        let (pivot_row, pivot) = (i..N).fold((i, 0.0_f64), |acc, r| {
            if a[r][i].abs() > acc.1 {
                (r, a[r][i].abs())
            } else {
                acc
            }
        });
        if pivot < 1e-12 {
            bail!("residual trend fit is singular (pivot {pivot:e} at row {i})");
        }
        a.swap(i, pivot_row);
        b.swap(i, pivot_row);

        let (pivot_values, pivot_rhs) = (a[i], b[i]);
        for r in (i + 1)..N {
            let factor = a[r][i] / pivot_values[i];
            for (target, pivot) in a[r].iter_mut().zip(pivot_values.iter()).skip(i) {
                *target -= factor * pivot;
            }
            b[r] -= factor * pivot_rhs;
        }
    }

    let mut coefficients = [0.0_f64; N];
    for i in (0..N).rev() {
        let mut acc = b[i];
        for j in (i + 1)..N {
            acc -= a[i][j] * coefficients[j];
        }
        coefficients[i] = acc / a[i][i];
    }
    Ok(coefficients)
}

/// Build the four half-plane trends and the anchor table they were fitted to.
fn build_trends(
    peaks: &[DigitizedPeak],
    config: &AntennaConfiguration,
    params: &IntegrationParams,
) -> Result<(Vec<HalfPlaneTrend>, Vec<Anchor>)> {
    // The boresight anchor: the report's text gain against the model's boresight value. It
    // belongs to every half-plane — it is the one point all four share.
    let model_boresight = model_gain_dbi(config, 0.0, ANCHOR_FREQUENCY_MHZ, params)?;
    let boresight_residual = ABSOLUTE_ANCHOR_DBI - model_boresight;

    let half_planes = [
        (CLOCK_H_POSITIVE, H_CUT_ID, true),
        (CLOCK_E_POSITIVE, E_CUT_ID, true),
        (CLOCK_H_NEGATIVE, H_CUT_ID, false),
        (CLOCK_E_NEGATIVE, E_CUT_ID, false),
    ];

    let mut trends = Vec::with_capacity(half_planes.len());
    let mut anchors = Vec::new();

    for (clock_deg, cut_id, positive_side) in half_planes {
        let mut cones = vec![0.0];
        let mut residuals = vec![boresight_residual];
        let mut weights = vec![1.0 / ABSOLUTE_ANCHOR_UNCERTAINTY_DB.powi(2)];
        let mut side = Vec::new();

        for peak in peaks
            .iter()
            .filter(|p| p.cut_id == cut_id && (p.peak_angle_deg > 0.0) == positive_side)
        {
            let cone_deg = peak.peak_angle_deg.abs();
            let measured_dbi = ABSOLUTE_ANCHOR_DBI + peak.level_db_rel_peak;
            let model_envelope_dbi =
                model_envelope_dbi(config, cone_deg, ANCHOR_FREQUENCY_MHZ, params)?;

            cones.push(cone_deg);
            residuals.push(measured_dbi - model_envelope_dbi);
            weights.push(1.0 / peak.uncertainty_db.powi(2));
            side.push((peak.clone(), cone_deg, measured_dbi, model_envelope_dbi));
        }

        if side.len() < 3 {
            bail!(
                "half-plane at clock {clock_deg}° has only {} digitized peaks; a quadratic \
                 trend needs at least three besides the boresight anchor",
                side.len()
            );
        }

        let coefficients = weighted_quadratic_fit(&cones, &residuals, &weights)?;
        let trend = HalfPlaneTrend {
            clock_deg,
            coefficients,
            max_anchor_cone_deg: cones.iter().cloned().fold(0.0_f64, f64::max),
        };

        for (peak, cone_deg, measured_dbi, model_envelope_dbi) in side {
            let residual_db = measured_dbi - model_envelope_dbi;
            let trend_db = trend.evaluate(cone_deg);
            anchors.push(Anchor {
                cut_id: peak.cut_id,
                clock_deg,
                cone_deg,
                level_db_rel_peak: peak.level_db_rel_peak,
                uncertainty_db: peak.uncertainty_db,
                measured_dbi,
                model_envelope_dbi,
                residual_db,
                trend_db,
                deviation_from_trend_db: residual_db - trend_db,
            });
        }

        trends.push(trend);
    }

    Ok((trends, anchors))
}

/// The injected residual at a grid point, in dB.
///
/// Across clock this is the unique band-limited interpolant through the four half-plane
/// values — `a0 + a1·cos φ + b1·sin φ + a2·cos 2φ`, which reproduces each measured
/// half-plane exactly and is smooth on the scale of the fitter's clock knots (~40°).
fn injected_residual_db(trends: &[HalfPlaneTrend], cone_deg: f64, clock_deg: f64) -> f64 {
    let at = |clock: f64| -> f64 {
        trends
            .iter()
            .find(|t| t.clock_deg == clock)
            .map(|t| t.evaluate(cone_deg))
            .unwrap_or(0.0)
    };
    let (v0, v90, v180, v270) = (
        at(CLOCK_H_POSITIVE),
        at(CLOCK_E_POSITIVE),
        at(CLOCK_H_NEGATIVE),
        at(CLOCK_E_NEGATIVE),
    );

    let a0 = 0.25 * (v0 + v90 + v180 + v270);
    let a1 = 0.5 * (v0 - v180);
    let b1 = 0.5 * (v90 - v270);
    let a2 = 0.25 * (v0 - v90 + v180 - v270);

    let phi = clock_deg.to_radians();
    a0 + a1 * phi.cos() + b1 * phi.sin() + a2 * (2.0 * phi).cos()
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<()> {
    let args = Args::parse();

    let registry = AntennaClassRegistry::load_from_file(&args.classes_file)
        .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", args.classes_file.display()))?;
    let class = registry
        .get_class(&args.antenna_class)
        .with_context(|| {
            format!(
                "antenna class '{}' not found in {}",
                args.antenna_class,
                args.classes_file.display()
            )
        })?
        .clone();

    let config = model_config(&class)?;
    let params = integration_params();
    let temperature_k = class.system_noise_temperature_k;

    let peaks = read_peaks(&args.peaks)?;
    let (trends, anchors) = build_trends(&peaks, &config, &params)?;

    // Model gain depends on (frequency, cone) only — the modelled feed is azimuthally
    // symmetric, so the clock dependence of this dataset is entirely the measured E/H and
    // left/right asymmetry carried by the residual. Evaluating once per (frequency, cone)
    // rather than per row is exact here, not an approximation.
    let mut rows = String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
    let mut sum_sq_residual = 0.0;

    for &frequency_mhz in &FREQUENCIES_MHZ {
        for &cone_deg in &CONE_DEG {
            let gain_dbi = model_gain_dbi(&config, cone_deg, frequency_mhz, &params)?;
            for &clock_deg in &CLOCK_DEG {
                let residual_db = injected_residual_db(&trends, cone_deg, clock_deg);
                sum_sq_residual += residual_db * residual_db;
                let g_over_t_db = g_over_t_from_gain_db(gain_dbi + residual_db, temperature_k);
                rows.push_str(&format!(
                    "{clock_deg:.6},{cone_deg:.6},{frequency_mhz:.6},{g_over_t_db:.6},\
                     {temperature_k:.6}\n"
                ));
            }
        }
    }

    let injected_residual_rms_db = (sum_sq_residual / ROW_COUNT as f64).sqrt();

    std::fs::write(&args.output, &rows)
        .with_context(|| format!("failed to write {}", args.output.display()))?;

    println!("NASA CR-159703 1.22 m — real-anchored measurement grid (roadmap D14)");
    println!("  THIS DATASET IS NOT MEASURED DATA. Fabrications:");
    for note in FABRICATIONS {
        println!("    - {note}");
    }
    println!(
        "\n  antenna class      {} ({} m, f/D {}, q {}, surface RMS {} mm, T_sys {} K)",
        class.class_id,
        class.geometry.diameter_m,
        class.geometry.f_over_d,
        class.feed.q_factor,
        class.surface.rms_mm,
        temperature_k
    );
    println!("  digitized cuts     {H_CUT_ID} (H), {E_CUT_ID} (E) at {ANCHOR_FREQUENCY_MHZ} MHz");
    println!("  absolute anchor    {ABSOLUTE_ANCHOR_DBI} dBi (report text gain)");
    println!(
        "  anchors            {} peaks over 4 half-planes",
        anchors.len()
    );
    println!(
        "  grid               {} rows ({} freq x {} cone x {} clock)",
        ROW_COUNT,
        FREQUENCIES_MHZ.len(),
        CONE_DEG.len(),
        CLOCK_DEG.len()
    );
    println!("  injected residual  RMS {injected_residual_rms_db:.4} dB over the grid");
    println!("  output             {}", args.output.display());
    println!("\n  anchor table (dB):");
    println!(
        "    {:<22} {:>6} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "cut", "clock", "cone", "measured", "model", "residual", "trend", "dev"
    );
    for a in &anchors {
        println!(
            "    {:<22} {:>6.0} {:>6.1} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
            a.cut_id,
            a.clock_deg,
            a.cone_deg,
            a.measured_dbi,
            a.model_envelope_dbi,
            a.residual_db,
            a.trend_db,
            a.deviation_from_trend_db
        );
    }

    if let Some(summary_path) = &args.summary {
        let summary = serde_json::json!({
            "generator": "cr159703_grid",
            "roadmap_unit": "D14",
            "not_measured_data": true,
            "fabrications": FABRICATIONS,
            "source": {
                "report": "NASA CR-159703 (Collin & Gabel, 1979)",
                "peaks_file": args.peaks.display().to_string(),
                "h_cut_id": H_CUT_ID,
                "e_cut_id": E_CUT_ID,
                "anchor_frequency_mhz": ANCHOR_FREQUENCY_MHZ,
                "absolute_anchor_dbi": ABSOLUTE_ANCHOR_DBI,
                "absolute_anchor_uncertainty_db": ABSOLUTE_ANCHOR_UNCERTAINTY_DB,
            },
            "model": {
                "classes_file": args.classes_file.display().to_string(),
                "antenna_class": class.class_id,
                "diameter_m": class.geometry.diameter_m,
                "f_over_d": class.geometry.f_over_d,
                "q_factor": class.feed.q_factor,
                "surface_rms_mm": class.surface.rms_mm,
                "system_noise_temperature_k": temperature_k,
                "envelope_half_width_deg": ENVELOPE_HALF_WIDTH_DEG,
            },
            "grid": {
                "rows": ROW_COUNT,
                "frequencies_mhz": FREQUENCIES_MHZ.to_vec(),
                "cone_deg": CONE_DEG.to_vec(),
                "clock_deg": CLOCK_DEG.to_vec(),
            },
            "injected_residual_rms_db": injected_residual_rms_db,
            "anchors": anchors,
        });
        std::fs::write(summary_path, serde_json::to_string_pretty(&summary)?)
            .with_context(|| format!("failed to write {}", summary_path.display()))?;
        println!("\n  summary            {}", summary_path.display());
    }

    Ok(())
}
