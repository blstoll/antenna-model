//! Full-calibration artifact export.
//!
//! This module assembles a complete [`AntennaCalibration`] artifact that the antenna-model
//! service can load via `load_calibration_artifact`.
//!
//! # The correction surface is constructed in core
//!
//! The fitted surface's knot vectors, shape, and schema-5 wire form all belong to
//! `antenna_core::model::correction_surface` (GitHub issues #94, #95). This module asks
//! [`to_model4d`](antenna_core::model::FittedCorrectionSurface::to_model4d) for the wire
//! type and supplies only the synthetic temperature interval, which it derives from the
//! measured temperatures. The legacy wire
//! names (`knots_azimuth` for E-clock, `knots_elevation` for E-cone) and the flat temperature
//! axis are that adapter's business; neither appears in this crate.
//!
//! `validity_ranges` and `calibration_coverage` still carry the same legacy names
//! (`azimuth_range`/`elevation_range`) for the E-clock and E-cone extents, which is why the
//! builders below translate them.

use antenna_core::data::loader::encode_calibration_artifact;
use antenna_core::data::types::{
    AngularResolution, AntennaCalibration, AntennaCalibrationBuilder, CalibrationCoverageBuilder,
    CalibrationMetadataBuilder, CalibrationStatus, FeedParameters as DataFeedParameters,
    MeasurementDensity, MeshParameters as DataMeshParameters, ParameterSource,
    PhysicalAntennaConfigBuilder, ReflectorGeometry as DataReflectorGeometry,
    ValidityRangesBuilder, CALIBRATION_SCHEMA_VERSION,
};

use antenna_core::model::PHYSICS_MODEL_VERSION;

use crate::correction_surface::{assess_angular_resolution, CorrectionSurface};
use crate::parser::MeasurementPoint;
use std::path::Path;

/// Errors that can occur while exporting a full-calibration artifact.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactExportError {
    /// The source correction surface had an unexpected shape (e.g. zero in a dimension).
    #[error("invalid correction surface: {0}")]
    InvalidSurface(String),

    /// The core wire adapter refused to construct the correction surface's wire form.
    /// Carries the typed core error, which names the axis (GitHub issue #95).
    #[error("invalid correction surface: {0}")]
    InvalidCorrectionSurface(#[from] antenna_core::data::types::ValidationError),

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

/// Extents of the measurement set used to populate validity ranges and coverage.
#[derive(Debug, Clone, Copy)]
struct MeasurementExtents {
    e_clock_min_max: (f64, f64),
    e_cone_min_max: (f64, f64),
    frequency_min_max: (f64, f64),
    temperature_mid: f64,
    temperature_min_max: (f64, f64),
}

/// Compute measurement extents (E-clock/E-cone/frequency/temperature) from the points.
fn measurement_extents(measurements: &[MeasurementPoint]) -> Result<MeasurementExtents> {
    if measurements.is_empty() {
        return Err(ArtifactExportError::InvalidSurface(
            "no measurements provided for extent computation".to_string(),
        ));
    }

    let mut e_clock = (f64::INFINITY, f64::NEG_INFINITY);
    let mut e_cone = (f64::INFINITY, f64::NEG_INFINITY);
    let mut freq = (f64::INFINITY, f64::NEG_INFINITY);
    let mut temp = (f64::INFINITY, f64::NEG_INFINITY);

    for p in measurements {
        e_clock.0 = e_clock.0.min(p.e_clock_deg);
        e_clock.1 = e_clock.1.max(p.e_clock_deg);
        e_cone.0 = e_cone.0.min(p.e_cone_deg);
        e_cone.1 = e_cone.1.max(p.e_cone_deg);
        freq.0 = freq.0.min(p.frequency_mhz);
        freq.1 = freq.1.max(p.frequency_mhz);
        temp.0 = temp.0.min(p.temperature_k);
        temp.1 = temp.1.max(p.temperature_k);
    }

    let temperature_mid = 0.5 * (temp.0 + temp.1);

    Ok(MeasurementExtents {
        e_clock_min_max: e_clock,
        e_cone_min_max: e_cone,
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
    /// E/H illumination asymmetry the residuals were fitted against (1.0 = symmetric).
    ///
    /// Roadmap **D23**: this must be the value `compute_model_predictions` built its model
    /// with. Anything else — including this struct simply not carrying it, which was the
    /// defect — means the correction surface is applied on top of a different illumination
    /// than it was fitted against.
    pub asymmetry_factor: f64,
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
///
/// # The angular-resolution assessment is derived here, not passed in
///
/// What `surface`'s knots can resolve against this antenna's own `λ/D` (roadmap D21) is
/// computed inside this function, from `physical.diameter_m` — the *same* field it stamps
/// into the artifact's `reflector.diameter_m` a few lines below.
///
/// It used to be a parameter, justified by a doc comment claiming the diameter "lives on the
/// antenna class, which this function only sees the already-flattened
/// [`ExportPhysicalParams`] view of". That was simply false — `ExportPhysicalParams` carries
/// `diameter_m` and this function stamps it — and the caller assessed against a second,
/// independent read (`class.geometry.diameter_m`), so an artifact could describe one antenna
/// in `diameter_m` and a different one in `angular_resolution`. That is the invariant C13 and
/// D23 established two lines from here: **every parameter the fitting model uses must be in
/// the artifact, or the artifact describes something other than what it serves.** Roadmap
/// **D26** finding 2; taking the parameter away is what makes the two agree by construction
/// rather than by a caller's care.
///
/// `served_behavior_rmse_db` describes the value the service returns at every validation point:
/// physics plus correction in support, and physics-only outside support. The artifact's generic
/// `accuracy_estimate_db` and `rmse_db` fields already describe served accuracy, so issue #93
/// changes neither their meaning nor the wire layout. Issue #96 will add a separate in-support
/// correction metric to the validation report.
#[allow(clippy::too_many_arguments)]
pub fn export_full_calibration(
    antenna_id: &str,
    feed_id: &str,
    antenna_name: &str,
    data_source: String,
    physical: &ExportPhysicalParams,
    surface: &CorrectionSurface,
    measurements: &[MeasurementPoint],
    served_behavior_rmse_db: f64,
    r_squared: f64,
    physics_only_rmse_db: f64,
    parameters_tuned: bool,
) -> Result<AntennaCalibration> {
    let extents = measurement_extents(measurements)?;

    // Derived from the diameter this function stamps, so the two cannot disagree.
    let angular_resolution: AngularResolution =
        assess_angular_resolution(surface, physical.diameter_m).map_err(|e| {
            ArtifactExportError::BuildFailed {
                what: "angular-resolution assessment".to_string(),
                reason: e.to_string(),
            }
        })?;

    // Build the 4D correction surface over a flat temperature interval enclosing
    // the measured temperatures (with a 1 K pad to guarantee a nonzero interval).
    let (t_meas_lo, t_meas_hi) = extents.temperature_min_max;
    let t_lo = t_meas_lo - 1.0;
    let t_hi = t_meas_hi + 1.0;
    let correction = surface.fitted().to_model4d(t_lo, t_hi)?;

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
        asymmetry_factor: physical.asymmetry_factor,
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

    // Validity ranges from measurement extents.
    //
    // The served elevation is a **polar angle from boresight** and is never negative
    // (`compute_emitter_direction_with_attitude`), so the E-cone axis reaching this point
    // must already be in that convention — which is why the parser reflects a negative-cone
    // row onto `(clock + 180°, |cone|)` on the way in (`MeasurementPoint::to_polar_convention`).
    //
    // This *was* `.max(0.0)` / `.min(90.0)`, a silent clamp, and negative E-cone is legal
    // validated input (`MeasurementPoint::validate` admits [-90, 90]). For a one-sided cut
    // recorded as -14°…0° the clamp collapsed the range to `(0.0, 0.0)`: the artifact then
    // reported `is_boresight_only()` over thousands of measurements, and `contains()` admitted
    // no elevation but exactly 0.0 — so the service applied **no correction at all** while
    // every health signal read normal. A wholly-negative span such as -14°…-1° produced the
    // inverted `(0.0, -1.0)`, rejecting everything by construction. Roadmap **D26** finding 1.
    //
    // Failing loudly here rather than clamping is deliberate: a clamp cannot distinguish
    // "already in the right convention" from "silently truncated", which is exactly how this
    // went unseen. If it fires, the input never went through the normalization above.
    let (cone_lo, cone_hi) = extents.e_cone_min_max;
    if !(0.0..=90.0).contains(&cone_lo) || !(0.0..=90.0).contains(&cone_hi) || cone_lo > cone_hi {
        return Err(ArtifactExportError::BuildFailed {
            what: "validity ranges".to_string(),
            reason: format!(
                "measured E-cone extent [{cone_lo}, {cone_hi}]° is not a polar-angle range in \
                 [0, 90]; measurements must be in the polar convention before export \
                 (see MeasurementPoint::to_polar_convention)"
            ),
        });
    }
    // `azimuth_range`/`elevation_range` are the artifact's wire names for the E-clock and
    // E-cone extents, the same translation the core wire adapter makes for the knot vectors.
    let validity_ranges = ValidityRangesBuilder::default()
        .azimuth_range(extents.e_clock_min_max.0, extents.e_clock_min_max.1)
        .elevation_range(cone_lo, cone_hi)
        .frequency_range(extents.frequency_min_max.0, extents.frequency_min_max.1)
        .temperature(extents.temperature_mid)
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "validity ranges".to_string(),
            reason: e,
        })?;

    // Coverage from measurement extents.
    let coverage = CalibrationCoverageBuilder::default()
        .azimuth_range(extents.e_clock_min_max.0, extents.e_clock_min_max.1)
        .elevation_range(cone_lo, cone_hi)
        .frequency_range(extents.frequency_min_max.0, extents.frequency_min_max.1)
        .num_measurements(measurements.len())
        .has_correction_surface(true)
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "calibration coverage".to_string(),
            reason: e,
        })?;

    let calibration_status = CalibrationStatus::FullyCalibrated {
        accuracy_estimate_db: served_behavior_rmse_db,
    };

    let correction_improvement_db = physics_only_rmse_db - served_behavior_rmse_db;
    let metadata = CalibrationMetadataBuilder::default()
        .antenna_name(antenna_name.to_string())
        .calibration_date(chrono::Utc::now().to_rfc3339())
        .format_version(CALIBRATION_SCHEMA_VERSION.to_string())
        .data_source(data_source)
        .rmse_db(served_behavior_rmse_db)
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
        .angular_resolution(angular_resolution)
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
/// The framing itself is [`antenna_core::data::loader::encode_calibration_artifact`], the
/// counterpart of the loader that reads it — so reader and writer share one definition of
/// the container format rather than agreeing by inspection. This function contributes the
/// file I/O and this tool's error vocabulary, nothing more. (It used to lay the header out
/// itself from the loader's public constants, which is closer but still a copy: D23 found a
/// *fourth* hand-rolled writer in a test carrying a hardcoded container version, which would
/// have sailed past its own bump.)
///
/// Note this stamps the **container** axis only. The **schema** axis
/// (`metadata.format_version`) rides inside the payload and is set by whichever builder
/// produced `calibration`; see [`antenna_core::data::types::CALIBRATION_SCHEMA_VERSION`].
pub fn write_calibration_artifact(calibration: &AntennaCalibration, path: &Path) -> Result<()> {
    let bytes = encode_calibration_artifact(calibration).map_err(|e| {
        ArtifactExportError::SerializeFailed {
            reason: e.to_string(),
        }
    })?;

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
    use antenna_core::model::FittedCorrectionSurface;

    fn applied_value(
        surface: &FittedCorrectionSurface,
        e_clock_deg: f64,
        e_cone_deg: f64,
        frequency_mhz: f64,
    ) -> f64 {
        surface
            .evaluate(e_clock_deg, e_cone_deg, frequency_mhz)
            .correction_db()
            .expect("test query left fitted support")
    }

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
    fn the_constructed_wire_model_validates() {
        let (surface, _freq0) = make_test_surface();
        let model = surface
            .fitted()
            .to_model4d(289.0, 291.0)
            .expect("wire construction should succeed");
        assert!(
            model.validate().is_ok(),
            "exported 4D model failed validation: {:?}",
            model.validate()
        );

        // Shape mapping: spatial axes copy directly (no padding now that the
        // service's find_knot_span off-by-one is fixed); temperature axis has order+1 layers.
        let [n_clock, n_cone, n_freq] = surface.shape();
        assert_eq!(model.shape[0], n_clock); // azimuth <- clock, no pad
        assert_eq!(model.shape[1], n_cone); // elevation <- cone, no pad
        assert_eq!(model.shape[2], n_freq); // frequency, no pad
        assert_eq!(model.shape[3], surface.spline_order() + 1);
    }

    #[test]
    fn test_round_trip_matches_3d_evaluation() {
        let (surface, _freq0) = make_test_surface();
        let t_lo = 289.0;
        let t_hi = 291.0;
        let model = surface
            .fitted()
            .to_model4d(t_lo, t_hi)
            .expect("wire construction should succeed");
        assert!(model.validate().is_ok());
        let fitted = FittedCorrectionSurface::from_model4d(&model).unwrap();

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
        let mut max_err = 0.0_f64;
        let mut samples = 0;
        for &k in &clocks {
            for &c in &cones {
                for &f in &freqs {
                    let expected = surface.evaluate(f, c, k).expect("3D evaluate");
                    let got = applied_value(&fitted, k, c, f);
                    let err = (got - expected).abs();
                    max_err = max_err.max(err);
                    samples += 1;
                    assert!(
                        err < 1e-9,
                        "mismatch at clock={k}, cone={c}, freq={f}: \
                         expected={expected}, got={got}, err={err}"
                    );
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
        let model = surface
            .fitted()
            .to_model4d(280.0, 300.0)
            .expect("wire construction");
        let fitted = FittedCorrectionSurface::from_model4d(&model).unwrap();

        // A point with a clearly nonzero expected correction.
        let (k, c, f) = (90.0, 5.0, 8200.0);
        let expected = surface.evaluate(f, c, k).expect("3D evaluate");
        assert!(
            expected.abs() > 1e-3,
            "test point should have nonzero correction, got {expected}"
        );

        // Preparation accepts the schema-5 adapter only because every synthetic
        // temperature slab is identical; runtime evaluation has no temperature input.
        let got = applied_value(&fitted, k, c, f);
        assert!(
            (got - expected).abs() < 1e-9,
            "flat-temperature adapter changed the correction: expected={expected}, got={got}"
        );
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
            asymmetry_factor: 1.0,
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
