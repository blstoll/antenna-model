//! Full-calibration artifact export.
//!
//! This module converts the calibrate tool's internal 3D [`CorrectionSurface`]
//! (over frequency, e-cone, e-clock) into the service-loadable 4D
//! [`BSplineModel4D`] (over azimuth, elevation, frequency, temperature) and
//! assembles a complete [`AntennaCalibration`] artifact that the antenna-model
//! service can load via `load_calibration_artifact`.
//!
//! # Dimension Mapping
//!
//! The 3D correction surface dimensions map onto the 4D service surface as:
//!
//! | 3D (calibrate)        | 4D (service)           |
//! |-----------------------|------------------------|
//! | e-clock (degrees)     | azimuth (degrees)      |
//! | e-cone (degrees)      | elevation (degrees)    |
//! | frequency (MHz)       | frequency (MHz)        |
//! | (none)                | temperature (Kelvin)   |
//!
//! All angular dimensions are in degrees on both sides, so knot vectors copy
//! directly. The temperature axis does not exist in the source surface, so it
//! is constructed as a *flat-but-valid* axis (see [`to_bspline_4d`]): the
//! coefficient slab is replicated `spline_order` times along temperature with a
//! clamped knot vector over a real, nonzero interval. Because every temperature
//! layer is identical and B-spline basis functions form a partition of unity,
//! the surface evaluates to the same temperature-independent value anywhere in
//! the interval.
//!
//! # Index Reordering
//!
//! The two B-spline representations use different flat-index conventions:
//!
//! - 3D source: `idx = i_freq + n_freq * (i_cone + n_cone * i_clock)`
//!   (frequency varies fastest, clock slowest)
//! - 4D dest:   `idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp))`
//!   (azimuth/clock varies fastest, temperature slowest)
//!
//! Because azimuth := clock, these orderings differ; coefficients must be
//! reindexed (not memcpy'd).

use antenna_model::data::loader::{ANTC_ARTIFACT_VERSION, ANTC_HEADER_LEN, ANTC_MAGIC};
use antenna_model::data::types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, CalibrationCoverageBuilder,
    CalibrationMetadataBuilder, CalibrationStatus, FeedParameters as DataFeedParameters,
    MeasurementDensity, MeshParameters as DataMeshParameters, ParameterSource,
    PhysicalAntennaConfigBuilder, ReflectorGeometry as DataReflectorGeometry,
    ValidityRangesBuilder,
};

use antenna_model::model::PHYSICS_MODEL_VERSION;

use crate::correction_surface::CorrectionSurface;
use crate::parser::MeasurementPoint;
use std::path::Path;

/// Errors that can occur while exporting a full-calibration artifact.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactExportError {
    /// The source correction surface had an unexpected shape (e.g. zero in a dimension).
    #[error("invalid correction surface: {0}")]
    InvalidSurface(String),

    /// A builder for one of the artifact sub-structures failed.
    #[error("failed to build {what}: {reason}")]
    BuildFailed {
        /// Which structure failed to build.
        what: String,
        /// The underlying reason.
        reason: String,
    },

    /// The artifact could not be postcard-encoded.
    #[error("failed to serialize calibration artifact: {reason}")]
    SerializeFailed {
        /// The underlying reason.
        reason: String,
    },

    /// The encoded artifact could not be written to disk.
    #[error("failed to write artifact to {path}: {reason}")]
    WriteFailed {
        /// Destination path.
        path: String,
        /// The underlying reason.
        reason: String,
    },
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, ArtifactExportError>;

/// Build a *flat-but-valid* axis for a dimension the surface does not vary in.
///
/// Returns `(n_layers, knots)`. Replicating the coefficient slab identically
/// across `n_layers` layers along the axis makes the surface exactly constant in
/// it, because B-spline basis functions are a partition of unity: the layers all
/// carry the same value, and the basis weights sum to one at every point of the
/// span.
///
/// This is the **only** correct way to collapse a dimension in a
/// [`BSplineModel4D`]; the obvious alternative — one coefficient layer and a
/// knot vector of `order` equal knots — is rejected on two independent counts,
/// which is why it lives here as one shared definition rather than being
/// re-derived per producer:
///
/// 1. `BSplineModel4D::validate` requires `knots.len() >= shape + order` on
///    every axis, and the service loader runs that validation on every artifact.
/// 2. The evaluator's span is `[knots[order-1], knots[len-order]]`; with a
///    single coefficient layer that interval is necessarily empty regardless of
///    knot-vector length, so the axis has nothing to evaluate over. Lengthening
///    a degenerate vector is therefore *not* a fix — the layer count has to grow
///    with it.
///
/// The layout is a clamped knot vector over the real interval `[lo, hi]` with
/// one interior knot at the midpoint: `order` copies of `lo`, then the midpoint,
/// then `order` copies of `hi` (length `2*order + 1 = n_layers + order` for
/// `n_layers = order + 1`).
///
/// # Panics (debug only)
///
/// Debug-asserts `lo < hi`; a zero-width interval would reintroduce the empty
/// span this helper exists to avoid. Callers validate the interval and return a
/// proper error, so this cannot fire in release.
pub(crate) fn flat_axis(lo: f64, hi: f64, order: usize) -> (usize, Vec<f64>) {
    debug_assert!(
        lo < hi,
        "flat axis needs a non-empty interval: [{lo}, {hi}]"
    );

    let n_layers = order + 1;
    let mid = 0.5 * (lo + hi);

    let mut knots = Vec::with_capacity(2 * order + 1);
    knots.extend(std::iter::repeat_n(lo, order));
    knots.push(mid);
    knots.extend(std::iter::repeat_n(hi, order));

    (n_layers, knots)
}

/// Convert a 3D calibrate [`CorrectionSurface`] into a service-loadable 4D
/// [`BSplineModel4D`].
///
/// The mapping is `azimuth := e_clock`, `elevation := e_cone`,
/// `frequency := frequency`, with a flat (temperature-independent) temperature
/// axis over `[t_lo, t_hi]`.
///
/// # Spatial axes
///
/// Each spatial knot vector is copied directly from the 3D surface; no padding
/// is applied.  The service's `find_knot_span` previously had an off-by-one
/// (`n = len - order - 1`) that prevented it from reaching the topmost knot
/// interval.  That bug has been fixed (`n = len - order`), so the evaluator and
/// the calibrate side now agree natively without any extra sentinel knots.
///
/// # Temperature axis construction
///
/// The temperature axis is made *flat but valid* (not degenerate) by
/// [`flat_axis`], which is shared with the boresight-mode producer in
/// `frequency_correction`: the coefficient slab is replicated identically across
/// `n_temp = spline_order + 1` temperature layers over a clamped knot vector on
/// the real interval `[t_lo, t_hi]`. Because all temperature layers are
/// identical and the basis is a partition of unity, the result is
/// temperature-independent across `[t_lo, t_hi]`.
///
/// # Arguments
/// * `surface` - The fitted 3D correction surface.
/// * `t_lo` - Lower bound of the (flat) temperature interval in Kelvin.
/// * `t_hi` - Upper bound of the (flat) temperature interval in Kelvin (must be > `t_lo`).
pub fn to_bspline_4d(surface: &CorrectionSurface, t_lo: f64, t_hi: f64) -> Result<BSplineModel4D> {
    let [n_freq, n_cone, n_clock] = surface.shape;
    if n_freq == 0 || n_cone == 0 || n_clock == 0 {
        return Err(ArtifactExportError::InvalidSurface(format!(
            "correction surface has zero-sized dimension: shape={:?}",
            surface.shape
        )));
    }

    let order = surface.spline_order;
    if t_hi <= t_lo {
        return Err(ArtifactExportError::InvalidSurface(format!(
            "temperature interval must be non-empty: t_lo={t_lo}, t_hi={t_hi}"
        )));
    }

    // Dimension mapping: azimuth <- clock, elevation <- cone, frequency <- frequency.
    // Knot vectors are copied directly (no top-padding needed now that the service's
    // find_knot_span off-by-one has been corrected).
    let knots_azimuth = surface.knots_eclock.clone();
    let knots_elevation = surface.knots_econe.clone();
    let knots_frequency = surface.knots_frequency.clone();

    let n_az = n_clock;
    let n_el = n_cone;
    let n_freq_4d = n_freq;
    // Flat-but-valid temperature axis (see [`flat_axis`] and the module/fn docs).
    let (n_temp, knots_temperature) = flat_axis(t_lo, t_hi, order);

    // Reindex coefficients from source layout (freq fastest) to dest layout (az fastest).
    //   Source: idx = i_freq + n_freq * (i_cone + n_cone * i_clock)
    //   Dest:   idx = i_az   + n_az   * (i_el   + n_el   * (i_freq + n_freq * i_temp))
    // The temperature slab is replicated identically across all `n_temp` layers.
    let total = n_az * n_el * n_freq_4d * n_temp;
    let mut coefficients = vec![0.0_f64; total];

    for i_az in 0..n_az {
        for i_el in 0..n_el {
            for i_freq in 0..n_freq_4d {
                let src_idx = i_freq + n_freq * (i_el + n_cone * i_az);
                let value = surface.coefficients[src_idx];

                for i_temp in 0..n_temp {
                    let dst_idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq_4d * i_temp));
                    coefficients[dst_idx] = value;
                }
            }
        }
    }

    Ok(BSplineModel4D {
        coefficients,
        shape: [n_az, n_el, n_freq_4d, n_temp],
        knots_azimuth,
        knots_elevation,
        knots_frequency,
        knots_temperature,
        spline_order: order as u8,
    })
}

/// Extents of the measurement set used to populate validity ranges and coverage.
#[derive(Debug, Clone, Copy)]
struct MeasurementExtents {
    azimuth_min_max: (f64, f64),   // from e_clock
    elevation_min_max: (f64, f64), // from e_cone
    frequency_min_max: (f64, f64),
    temperature_mid: f64,
    temperature_min_max: (f64, f64),
}

/// Compute measurement extents (az/el/freq/temperature ranges) from the points.
fn measurement_extents(measurements: &[MeasurementPoint]) -> Result<MeasurementExtents> {
    if measurements.is_empty() {
        return Err(ArtifactExportError::InvalidSurface(
            "no measurements provided for extent computation".to_string(),
        ));
    }

    let mut az = (f64::INFINITY, f64::NEG_INFINITY);
    let mut el = (f64::INFINITY, f64::NEG_INFINITY);
    let mut freq = (f64::INFINITY, f64::NEG_INFINITY);
    let mut temp = (f64::INFINITY, f64::NEG_INFINITY);

    for p in measurements {
        az.0 = az.0.min(p.e_clock_deg);
        az.1 = az.1.max(p.e_clock_deg);
        el.0 = el.0.min(p.e_cone_deg);
        el.1 = el.1.max(p.e_cone_deg);
        freq.0 = freq.0.min(p.frequency_mhz);
        freq.1 = freq.1.max(p.frequency_mhz);
        temp.0 = temp.0.min(p.temperature_k);
        temp.1 = temp.1.max(p.temperature_k);
    }

    let temperature_mid = 0.5 * (temp.0 + temp.1);

    Ok(MeasurementExtents {
        azimuth_min_max: az,
        elevation_min_max: el,
        frequency_min_max: freq,
        temperature_mid,
        temperature_min_max: temp,
    })
}

/// Physical parameters needed to assemble the exported artifact.
///
/// These come from the tuned/nominal antenna configuration at the full-mode
/// write point.
#[derive(Debug, Clone)]
pub struct ExportPhysicalParams {
    /// Dish diameter in meters.
    pub diameter_m: f64,
    /// Focal length in meters.
    pub focal_length_m: f64,
    /// f/D ratio.
    pub f_over_d_ratio: f64,
    /// Tuned (or nominal) surface RMS in millimeters.
    pub surface_rms_mm: f64,
    /// Feed position (x, y, z) in meters.
    pub feed_position_m: (f64, f64, f64),
    /// Tuned (or nominal) feed q-factor.
    pub q_factor: f64,
    /// Feed phase-center offset in meters.
    pub phase_center_offset_m: f64,
    /// Optional mesh parameters (spacing_mm, wire_diameter_mm).
    pub mesh: Option<(f64, f64)>,
}

/// Assemble a complete service-loadable [`AntennaCalibration`] for full mode.
///
/// # Arguments
/// * `antenna_id` / `feed_id` - Composite identity of the artifact.
/// * `antenna_name` - Human-readable name for metadata.
/// * `data_source` - Source description (e.g. `file://...`).
/// * `physical` - Tuned/nominal physical parameters.
/// * `surface` - The fitted 3D correction surface.
/// * `measurements` - All measurement points (for extents / coverage).
/// * `rmse_db` / `r_squared` - Combined-model quality metrics (from validation).
/// * `physics_only_rmse_db` - Physics-only RMSE before correction.
/// * `parameters_tuned` - Whether physical parameters were tuned.
#[allow(clippy::too_many_arguments)]
pub fn export_full_calibration(
    antenna_id: &str,
    feed_id: &str,
    antenna_name: &str,
    data_source: String,
    physical: &ExportPhysicalParams,
    surface: &CorrectionSurface,
    measurements: &[MeasurementPoint],
    rmse_db: f64,
    r_squared: f64,
    physics_only_rmse_db: f64,
    parameters_tuned: bool,
) -> Result<AntennaCalibration> {
    let extents = measurement_extents(measurements)?;

    // Build the 4D correction surface over a flat temperature interval enclosing
    // the measured temperatures (with a 1 K pad to guarantee a nonzero interval).
    let (t_meas_lo, t_meas_hi) = extents.temperature_min_max;
    let t_lo = t_meas_lo - 1.0;
    let t_hi = t_meas_hi + 1.0;
    let correction = to_bspline_4d(surface, t_lo, t_hi)?;

    // Physical config.
    let reflector = DataReflectorGeometry {
        diameter_m: physical.diameter_m,
        focal_length_m: physical.focal_length_m,
        f_over_d_ratio: physical.f_over_d_ratio,
        surface_rms_mm: physical.surface_rms_mm,
    };
    let feed = DataFeedParameters {
        position: physical.feed_position_m,
        q_factor: physical.q_factor,
        phase_center_offset_m: physical.phase_center_offset_m,
        // deliberate defocus is service-config only; not exposed by the calibrate CLI
        axial_defocus_m: 0.0,
    };
    let mut config_builder = PhysicalAntennaConfigBuilder::default()
        .reflector(reflector)
        .feed(feed);
    if let Some((spacing, wire)) = physical.mesh {
        config_builder = config_builder.mesh(DataMeshParameters {
            mesh_spacing_mm: spacing,
            wire_diameter_mm: wire,
        });
    }
    let physical_config = config_builder
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "physical config".to_string(),
            reason: e,
        })?;

    // Validity ranges from measurement extents. Elevation (from e-cone) must lie
    // within [0, 90]; clamp defensively since the service validates it.
    let el_lo = extents.elevation_min_max.0.max(0.0);
    let el_hi = extents.elevation_min_max.1.min(90.0);
    let validity_ranges = ValidityRangesBuilder::default()
        .azimuth_range(extents.azimuth_min_max.0, extents.azimuth_min_max.1)
        .elevation_range(el_lo, el_hi)
        .frequency_range(extents.frequency_min_max.0, extents.frequency_min_max.1)
        .temperature(extents.temperature_mid)
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "validity ranges".to_string(),
            reason: e,
        })?;

    // Coverage from measurement extents.
    let coverage = CalibrationCoverageBuilder::default()
        .azimuth_range(extents.azimuth_min_max.0, extents.azimuth_min_max.1)
        .elevation_range(el_lo, el_hi)
        .frequency_range(extents.frequency_min_max.0, extents.frequency_min_max.1)
        .num_measurements(measurements.len())
        .has_correction_surface(true)
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "calibration coverage".to_string(),
            reason: e,
        })?;

    let calibration_status = CalibrationStatus::FullyCalibrated {
        accuracy_estimate_db: rmse_db,
    };

    let correction_improvement_db = physics_only_rmse_db - rmse_db;
    let metadata = CalibrationMetadataBuilder::default()
        .antenna_name(antenna_name.to_string())
        .calibration_date(chrono::Utc::now().to_rfc3339())
        .format_version("2.0".to_string())
        .data_source(data_source)
        .rmse_db(rmse_db)
        .r_squared(r_squared)
        .num_measurements(measurements.len())
        .physics_only_rmse_db(physics_only_rmse_db)
        .correction_improvement_db(correction_improvement_db)
        .parameters_tuned(parameters_tuned)
        .parameters_source(ParameterSource::FullGridTuning {
            num_measurements: measurements.len(),
        })
        .measurement_density(MeasurementDensity::Dense {
            points_per_beam: 0.0,
        })
        .physics_model_version(PHYSICS_MODEL_VERSION)
        .notes(format!(
            "Full calibration with 4D correction surface (shape {:?}), R²={:.6}",
            correction.shape, r_squared
        ))
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "calibration metadata".to_string(),
            reason: e,
        })?;

    let calibration = AntennaCalibrationBuilder::default()
        .antenna_id(antenna_id.to_string())
        .feed_id(feed_id.to_string())
        .metadata(metadata)
        .physical_config(physical_config)
        .correction_surface(correction)
        .validity_ranges(validity_ranges)
        .calibration_status(calibration_status)
        .calibration_coverage(coverage)
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "antenna calibration".to_string(),
            reason: e,
        })?;

    Ok(calibration)
}

/// Serialize an [`AntennaCalibration`] in the ANTC container format and write it to `path`.
///
/// **This is the only artifact writer in the tool.** Both producers — full-grid export
/// (`export_full_calibration`) and boresight export (`build_calibration_artifact`) — go
/// through it, so a boresight artifact and a full-mode artifact cannot disagree about
/// their framing. They diverged until 2026-07-30 (roadmap D2): boresight wrote a bare
/// `postcard::to_allocvec` with no magic, no version, and no CRC, which the service loader
/// accepted only via its legacy headerless fallback — so a boresight artifact carried no
/// container version stamp (a future framing change would have mis-decoded it silently
/// instead of being rejected) and no integrity check (truncation surfaced as a decode
/// error at best, wrong numbers at worst).
///
/// The layout is `[magic "ANTC"][version u32 LE][crc32 u32 LE][payload len u64 LE]`
/// followed by the postcard payload, matching the CRC-checked path in
/// [`antenna_model::data::loader::load_calibration_artifact`]. The magic, version, and
/// header length come from that module's public constants rather than being restated here,
/// so reader and writer share one definition of the container format.
///
/// Note this stamps the **container** axis only. The **schema** axis
/// (`metadata.format_version`) rides inside the payload and is set by whichever builder
/// produced `calibration`; see [`antenna_model::data::types::CALIBRATION_SCHEMA_VERSION`].
pub fn write_calibration_artifact(calibration: &AntennaCalibration, path: &Path) -> Result<()> {
    let payload =
        postcard::to_allocvec(calibration).map_err(|e| ArtifactExportError::SerializeFailed {
            reason: e.to_string(),
        })?;

    let crc = crc32fast::hash(&payload);
    let mut bytes = Vec::with_capacity(ANTC_HEADER_LEN + payload.len());
    bytes.extend_from_slice(ANTC_MAGIC);
    bytes.extend_from_slice(&ANTC_ARTIFACT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&payload);
    debug_assert_eq!(bytes.len() - payload.len(), ANTC_HEADER_LEN);

    std::fs::write(path, &bytes).map_err(|e| ArtifactExportError::WriteFailed {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction_surface::{fit_correction_surface, CorrectionSurfaceParams};
    use antenna_model::model::evaluate_correction;

    /// Smooth synthetic residual function over (clock, cone, freq).
    fn residual(clock_deg: f64, cone_deg: f64, freq_mhz: f64, freq0: f64) -> f64 {
        0.1 * (clock_deg * std::f64::consts::PI / 180.0).sin()
            + 0.02 * cone_deg
            + 0.01 * (freq_mhz - freq0)
    }

    /// Build a fitted 3D correction surface from synthetic residual data.
    fn make_test_surface() -> (CorrectionSurface, f64) {
        // 8 x 6 x 6 = 288 points against the 5x6x6 = 180 coefficients that 1/2/2 knots at
        // order 4 declare. Sized to the coefficient count, per roadmap D20 — the previous
        // 6 x 5 x 5 = 150 grid was underdetermined and the fitter now says so.
        let freq0 = 8000.0;
        let clocks = [0.0, 50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 350.0];
        let cones = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
        let freqs = [8000.0, 8080.0, 8160.0, 8240.0, 8320.0, 8400.0];

        let mut measurements = Vec::new();
        for &k in &clocks {
            for &c in &cones {
                for &f in &freqs {
                    // g_over_t is residual here; predictions are zero, so the
                    // fitted surface approximates `residual`.
                    let r = residual(k, c, f, freq0);
                    measurements.push(MeasurementPoint::new(k, c, f, r, 290.0));
                }
            }
        }
        let predictions = vec![0.0; measurements.len()];

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 1,
            num_knots_econe: 2,
            num_knots_eclock: 2,
            // Small regularization keeps the fitted coefficients well-conditioned
            // (O(1) instead of O(100)), so the round-trip comparison is not
            // dominated by basis-evaluation rounding amplified by huge coeffs.
            regularization: 1e-3,
            adaptive_knots: false,
            cross_validation_folds: 0,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 1.0,
            min_knot_spacing_eclock: 5.0,
        };

        let surface = fit_correction_surface(&measurements, &predictions, &params)
            .expect("surface fit should succeed");
        (surface, freq0)
    }

    #[test]
    fn test_to_bspline_4d_validates() {
        let (surface, _freq0) = make_test_surface();
        let model = to_bspline_4d(&surface, 289.0, 291.0).expect("conversion should succeed");
        assert!(
            model.validate().is_ok(),
            "exported 4D model failed validation: {:?}",
            model.validate()
        );

        // Shape mapping: spatial axes copy directly (no padding now that the
        // service's find_knot_span off-by-one is fixed); temperature axis has order+1 layers.
        let [n_freq, n_cone, n_clock] = surface.shape;
        assert_eq!(model.shape[0], n_clock); // azimuth <- clock, no pad
        assert_eq!(model.shape[1], n_cone); // elevation <- cone, no pad
        assert_eq!(model.shape[2], n_freq); // frequency, no pad
        assert_eq!(model.shape[3], surface.spline_order + 1);
    }

    #[test]
    fn test_round_trip_matches_3d_evaluation() {
        let (surface, _freq0) = make_test_surface();
        let t_lo = 289.0;
        let t_hi = 291.0;
        let t_mid = 0.5 * (t_lo + t_hi);
        let model = to_bspline_4d(&surface, t_lo, t_hi).expect("conversion should succeed");
        assert!(model.validate().is_ok());

        // Sample interior points AND the exact domain boundaries of every axis
        // (fitted ranges: clock [0, 350], cone [0, 10], freq [8000, 8400]) —
        // including the temperature boundaries of the synthetic 4D axis. Boundary
        // sampling added 2026-07-30 after the D15 endpoint fix: interior-only
        // sampling left the two implementations' boundary behavior uncompared (see
        // docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md,
        // "Why it went unnoticed"), so any future divergence at an edge would have
        // gone unseen here.
        let clocks = [
            0.0, 10.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0, 349.0, 350.0,
        ];
        let cones = [0.0, 0.5, 3.0, 5.0, 7.0, 9.5, 10.0];
        let freqs = [8000.0, 8050.0, 8200.0, 8350.0, 8400.0];
        let temps = [t_lo, t_mid, t_hi];

        let mut max_err = 0.0_f64;
        let mut samples = 0;
        for &k in &clocks {
            for &c in &cones {
                for &f in &freqs {
                    let expected = surface.evaluate(f, c, k).expect("3D evaluate");
                    for &t in &temps {
                        // 4D mapping: azimuth=clock, elevation=cone, frequency=f.
                        let got = evaluate_correction(&model, k, c, f, t)
                            .expect("4D evaluate")
                            .correction_db;
                        let err = (got - expected).abs();
                        max_err = max_err.max(err);
                        samples += 1;
                        assert!(
                            err < 1e-9,
                            "mismatch at clock={k}, cone={c}, freq={f}, temp={t}: \
                             expected={expected}, got={got}, err={err}"
                        );
                    }
                }
            }
        }
        assert!(samples >= 20, "expected >=20 samples, got {samples}");
        eprintln!("round-trip max error over {samples} samples: {max_err:e}");
    }

    #[test]
    fn test_temperature_axis_is_flat_not_zero() {
        // The flat temperature axis must NOT zero out the correction.
        let (surface, _freq0) = make_test_surface();
        let model = to_bspline_4d(&surface, 280.0, 300.0).expect("conversion");

        // A point with a clearly nonzero expected correction.
        let (k, c, f) = (90.0, 5.0, 8200.0);
        let expected = surface.evaluate(f, c, k).expect("3D evaluate");
        assert!(
            expected.abs() > 1e-3,
            "test point should have nonzero correction, got {expected}"
        );

        // Evaluating at several temperatures in-range must all match (flat axis).
        for &t in &[281.0, 290.0, 299.0] {
            let got = evaluate_correction(&model, k, c, f, t)
                .expect("4D evaluate")
                .correction_db;
            assert!(
                (got - expected).abs() < 1e-9,
                "temperature {t} should be flat: expected={expected}, got={got}"
            );
        }
    }

    #[test]
    fn test_export_full_calibration_assembles() {
        let (surface, _freq0) = make_test_surface();
        let measurements: Vec<MeasurementPoint> = {
            let mut v = Vec::new();
            for &k in &[0.0, 180.0, 350.0] {
                for &c in &[0.0, 5.0, 10.0] {
                    for &f in &[8000.0, 8400.0] {
                        v.push(MeasurementPoint::new(k, c, f, 0.0, 290.0));
                    }
                }
            }
            v
        };

        let physical = ExportPhysicalParams {
            diameter_m: 3.7,
            focal_length_m: 1.85,
            f_over_d_ratio: 0.5,
            surface_rms_mm: 1.2,
            feed_position_m: (0.0, 0.0, 0.0),
            q_factor: 8.0,
            phase_center_offset_m: 0.0,
            mesh: None,
        };

        let cal = export_full_calibration(
            "test_antenna",
            "x_band",
            "Test 3.7m",
            "file://test.csv".to_string(),
            &physical,
            &surface,
            &measurements,
            0.4,
            0.99,
            0.9,
            true,
        )
        .expect("export should succeed");

        cal.validate().expect("artifact must validate");
        assert_eq!(cal.antenna_id, "test_antenna");
        assert_eq!(cal.feed_id, "x_band");
        assert!(cal.correction_surface.is_some());
        assert!(matches!(
            cal.calibration_status,
            Some(CalibrationStatus::FullyCalibrated { .. })
        ));
        let cov = cal.calibration_coverage.expect("coverage present");
        assert!(cov.has_correction_surface);
        assert_eq!(cov.num_measurements, measurements.len());
    }
}
