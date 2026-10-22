//! Full-calibration artifact export.
//!
//! This module assembles a complete [`AntennaCalibration`] artifact that the antenna-model
//! service can load; `antenna_core::artifact::write` writes it.
//!
//! # The correction surface is constructed in core
//!
//! The fitted surface's knot vectors and shape belong to
//! `antenna_core::model::correction_surface` (GitHub issues #94, #95), and its wire form to
//! `antenna_core::artifact` (#98). This module stores the fitted
//! [`FittedCorrectionSurface`](antenna_core::model::FittedCorrectionSurface) as is.
//!
//! `validity_ranges` and `calibration_coverage` still carry the same legacy names
//! (`azimuth_range`/`elevation_range`) for the E-clock and E-cone extents, which is why the
//! builders below translate them.

use antenna_core::types::{
    AngularResolution, AntennaCalibration, AntennaCalibrationBuilder, CalibrationCoverage,
    CalibrationMetadataBuilder, CalibrationStatus, FeedParameters as DataFeedParameters,
    MeasurementDensity, MeshParameters as DataMeshParameters, ParameterSource,
    PhysicalAntennaConfigBuilder, ReflectorGeometry as DataReflectorGeometry,
    ValidityRangesBuilder, CALIBRATION_SCHEMA_VERSION,
};

use antenna_core::model::PHYSICS_MODEL_VERSION;

use crate::correction_surface::{assess_angular_resolution, CorrectionSurface};
use crate::parser::{closed_extent, MeasurementPoint};

/// Errors that can occur while exporting a full-calibration artifact.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactExportError {
    /// The source correction surface had an unexpected shape (e.g. zero in a dimension).
    #[error("invalid correction surface: {0}")]
    InvalidSurface(String),

    /// The assembled artifact broke an artifact invariant.
    #[error("invalid calibration artifact: {0}")]
    InvalidArtifact(antenna_core::types::ValidationError),

    /// A builder for one of the artifact sub-structures failed.
    #[error("failed to build {what}: {reason}")]
    BuildFailed {
        /// Which structure failed to build.
        what: String,
        /// The underlying reason.
        reason: String,
    },
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, ArtifactExportError>;

/// The measured temperature extent `(min, max)`.
///
/// Temperature is the only extent export reads from `measurements`. The E-clock, E-cone
/// and frequency extents come from the fitted surface's measured domain instead
/// ([`CorrectionSurface::measured_domain`], issue #97): recomputing them here could state
/// coverage the surface was never fitted over.
fn temperature_extent(measurements: &[MeasurementPoint]) -> Result<(f64, f64)> {
    if measurements.is_empty() {
        return Err(ArtifactExportError::InvalidSurface(
            "no measurements provided for extent computation".to_string(),
        ));
    }
    Ok(closed_extent(measurements.iter().map(|p| p.temperature_k)))
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
/// * `measurements` - All measurement points (for the count and temperature extent).
/// * `rmse_db` / `r_squared` - Combined-model quality metrics (from validation).
/// * `physics_only_rmse_db` - Physics-only RMSE before correction.
/// * `parameters_tuned` - Whether physical parameters were tuned.
///
/// # Coverage and support are one measured domain
///
/// Validity ranges and calibration coverage are written from `surface`'s measured domain —
/// the value the fit clamped its knot bounds to — so full-mode coverage equals the
/// surface's fitted support by construction (issue #97).
///
/// # The angular-resolution assessment is derived here, not passed in
///
/// What `surface`'s knots can resolve against this antenna's own `λ/D` (roadmap D21) is
/// computed inside this function, from `physical.diameter_m` — the *same* field it stamps
/// into the artifact's `reflector.diameter_m` a few lines below — so the two cannot describe
/// different antennas. Do not make it a parameter again. See D26.
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
    let (t_meas_lo, t_meas_hi) = temperature_extent(measurements)?;
    let domain = surface.measured_domain();

    // Derived from the diameter this function stamps, so the two cannot disagree.
    let angular_resolution: AngularResolution =
        assess_angular_resolution(surface, physical.diameter_m).map_err(|e| {
            ArtifactExportError::BuildFailed {
                what: "angular-resolution assessment".to_string(),
                reason: e.to_string(),
            }
        })?;

    let correction = surface.fitted().clone();

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

    // Validity ranges from the measured domain.
    //
    // The served elevation is a **polar angle from boresight** and is never negative
    // (`compute_emitter_direction_with_attitude`), so the E-cone axis reaching this point
    // must already be in that convention — which is why the parser reflects a negative-cone
    // row onto `(clock + 180°, |cone|)` on the way in (`MeasurementPoint::to_polar_convention`).
    //
    // Refuse rather than clamp: a clamp cannot tell "already in the polar convention" from
    // "silently truncated", and a truncated cone extent serves no correction at all while
    // every health signal reads normal. If this fires, the input skipped the normalization
    // above. See D26.
    let (cone_lo, cone_hi) = domain.e_cone_deg;
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
        .azimuth_range(domain.e_clock_deg.0, domain.e_clock_deg.1)
        .elevation_range(cone_lo, cone_hi)
        .frequency_range(domain.frequency_mhz.0, domain.frequency_mhz.1)
        .temperature(0.5 * (t_meas_lo + t_meas_hi))
        .build()
        .map_err(|e| ArtifactExportError::BuildFailed {
            what: "validity ranges".to_string(),
            reason: e,
        })?;

    // Coverage from the same measured domain as the surface's support (issue #97).
    let coverage = CalibrationCoverage::from_domain(domain, measurements.len(), true);

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
            "Full calibration with correction surface (shape {:?}), R²={:.6}",
            correction.layout().shape(),
            r_squared
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
        .map_err(ArtifactExportError::InvalidArtifact)?;

    Ok(calibration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction_surface::{fit_correction_surface, CorrectionSurfaceParams};
    use antenna_core::model::CoveredCorrectionSurface;

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

        assert_eq!(cal.antenna_id, "test_antenna");
        assert_eq!(cal.feed_id, "x_band");
        assert_eq!(
            cal.correction_surface.as_ref(),
            Some(surface.fitted()),
            "the artifact stores the fitted surface exactly"
        );
        assert!(matches!(
            cal.calibration_status,
            Some(CalibrationStatus::FullyCalibrated { .. })
        ));
        let cov = cal.calibration_coverage.expect("coverage present");
        assert!(cov.has_correction_surface);
        assert_eq!(cov.num_measurements, measurements.len());
    }

    /// Issue #97: full mode states surface bounds and coverage from ONE measured-domain
    /// value — the one the fit placed its knot bounds from — so the exported coverage equals
    /// the exported surface's support exactly. It is not re-derived from the `measurements`
    /// argument: here those extend past the fitted data on every axis, and an independent
    /// recomputation would write coverage the surface cannot evaluate over.
    #[test]
    fn full_mode_coverage_and_support_are_one_measured_domain() {
        let (surface, _freq0) = make_test_surface();
        let measured = surface.measured_domain();
        assert_eq!(
            measured,
            surface.layout().support(),
            "the fit places each axis's bounds at the measured domain's"
        );

        let wider: Vec<MeasurementPoint> = [
            MeasurementPoint::new(measured.e_clock_deg.0, 0.0, 7_900.0, 0.0, 290.0),
            MeasurementPoint::new(359.0, 12.0, 8_500.0, 0.0, 290.0),
        ]
        .into_iter()
        .collect();
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
            &wider,
            0.4,
            0.99,
            0.9,
            true,
        )
        .expect("export should succeed");

        let covered = CoveredCorrectionSurface::from_artifact(&cal)
            .expect("coverage is contained by support")
            .expect("full mode writes a surface");
        assert_eq!(covered.coverage().domain(), measured);
        assert_eq!(covered.surface().layout().support(), measured);
    }
}
