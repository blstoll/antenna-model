//! Gain Computation Service - Core Pipeline
//!
//! This module implements the end-to-end gain computation workflow, orchestrating
//! coordinate transformations, physics modeling, and correction surface evaluation.
//!
//! # Pipeline Overview
//!
//! ```text
//! ┌─────────────────┐
//! │ GainRequest     │  Input: 3D positions (ECEF/Geodetic), frequencies, antenna ID
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 1: Load Calibration Data                                       │
//! │ - Retrieve antenna configuration (reflector, feed, mesh)            │
//! │ - Load correction surface (B-spline) if calibrated                  │
//! │ - Get validity ranges and calibration status                        │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 2: Coordinate Transformations                                  │
//! │ - Convert emitter/feed/vehicle positions to antenna frame           │
//! │ - Compute azimuth/elevation angles (θ, φ)                           │
//! │ - Apply beam squint correction for frequency offset                 │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 3: Physics Model Computation                                   │
//! │ - Aperture integration (physical optics) or ray tracing             │
//! │ - Phase accumulation: path + coma + surface + mesh                  │
//! │ - Apply Ruze efficiency (surface RMS) and mesh transparency         │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 4: Correction Surface Evaluation                               │
//! │ - Interpolate B-spline correction (if calibrated)                   │
//! │ - Add correction to physics model: Gain_final = Gain_phys + ΔG      │
//! │ - Generate warnings for extrapolation outside calibrated range      │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 5: G/T Computation (if temperature provided)                   │
//! │ - Compute G/T ratio from gain and system temperature                │
//! │ - Apply overall efficiency (Ruze × mesh)                            │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 6: Loss Calculation (if reference gain provided)               │
//! │ - Compute loss = reference_gain - actual_gain (dB)                  │
//! │ - Used for link budget analysis                                     │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │ GainResponse    │  Output: gain_db, loss, warnings, metadata
//! └─────────────────┘
//! ```
//!
//! # Error Handling
//! - Missing antenna/feed → FeedNotFound error
//! - Invalid coordinates → ValidationError
//! - Computation failures → ComputationError with context
//! - Out-of-range queries → Warning (not error)
//!
//! # Where each step lives
//!
//! This module owns step 1 and the *request adaptation* half of step 2: coordinate
//! transformation, repository lookup, and converting `feed_pointing_location` (an aim
//! point) into a feed steering displacement. Everything from beam squint onward — the
//! rest of step 2, and steps 3 through 6 — is the **served-gain law**, which lives in
//! `service::served_gain` (issue #61, parent #59). Read that module's docs for the
//! authoritative ordering of squint, physics, correction, disposition, warnings, and the
//! ideal reference. What remains here is request adaptation, response DTO construction,
//! and response timing.
//!
//! (`served_gain` is crate-private, so the reference above is deliberately not an
//! intra-doc link — linking a public module's docs to a private item is a rustdoc
//! warning.)

use crate::api::schemas::{
    CalibrationStatusInfo, ComputationMetadata, CorrectionApplication, GainRequest, GainResponse,
    GeometryInfo, Vector3D,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::model::integration::DEFAULT_INTEGRATION_BUDGET;
use crate::model::{compute_emitter_direction_with_attitude, compute_feed_position_from_pointing};
use crate::service::served_gain::{
    CorrectionDisposition, FeedSteering, PreSquintDirection, PreparedServedGain,
    ReferenceGainRequest, ServedFrequencies,
};
use std::time::{Duration, Instant};

/// Compute antenna gain from a gain request
///
/// This is the main entry point for gain computation, transforming 3D positions
/// into antenna frame coordinates and evaluating the physics model.
///
/// # Arguments
///
/// * `request` - The gain request containing vehicle position, reflector boresight, and feed pointing location
/// * `repository` - The calibration data repository
///
/// # Returns
///
/// A `GainResponse` containing the computed gain and metadata
pub fn compute_gain_from_request(
    request: &GainRequest,
    repository: &CalibrationRepository,
) -> Result<GainResponse> {
    // Thin wrapper: the served handlers call `_with_budget` with the configured value; this
    // 2-arg form keeps every existing (mostly test) call-site compiling by passing the
    // generous model-layer default. See `compute_gain_from_request_with_budget`.
    compute_gain_from_request_with_budget(request, repository, DEFAULT_INTEGRATION_BUDGET)
}

/// Compute antenna gain from a request, bounding each aperture integration to `time_budget`
/// (roadmap S3). Identical to [`compute_gain_from_request`] except the per-integration
/// wall-clock budget is caller-supplied — the served path threads
/// `performance.integration_budget_ms` here so the config knob is live. A single over-budget
/// integration returns `ComputationError::TimeBudgetExceeded` (→ 504).
pub fn compute_gain_from_request_with_budget(
    request: &GainRequest,
    repository: &CalibrationRepository,
    time_budget: Duration,
) -> Result<GainResponse> {
    let start = Instant::now();

    // Coordinate transformation runs BEFORE the repository lookup. That ordering is
    // observable: a request that is both geometrically invalid and names an unknown
    // antenna reports the coordinate fault, not `FeedNotFound`. Keep it here — moving it
    // behind the served-gain seam would silently reverse an established error precedence
    // (#59, "single-gain coordinate errors retain their current precedence").
    let (emitter_az, emitter_el) = compute_emitter_direction_with_attitude(
        &request.emitter_position,
        &request.vehicle_position,
        &request.reflector_boresight,
        request.vehicle_attitude,
    )?;

    let calibration = repository
        .get_calibration(&request.antenna_id, &request.feed_id)
        .ok_or_else(|| AntennaModelError::FeedNotFound {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
        })?;

    // Request adaptation: `feed_pointing_location` is an aim point — a location on Earth
    // the feed is pointed at — not a physical feed coordinate (see
    // `docs/domain-contract.md`). Convert it into the steering displacement that
    // preparation combines with this feed's design offset.
    //
    // This now runs BEFORE the reflector is built, where it used to run after. The
    // precedence that matters — a coordinate fault beating `FeedNotFound` — is preserved
    // above, and the only pair this could reorder is unreachable on the served path:
    // `data::loader` validates every artifact as it loads it (`AntennaCalibration::validate`
    // → `ReflectorGeometry::validate`), rejecting non-positive diameter or focal length,
    // negative surface RMS, and out-of-band f/D — a superset of what
    // `model::ReflectorGeometry::new` can fail on. A loaded artifact therefore cannot fail
    // reflector construction, so no request can reach a reflector error at all, in either
    // order.
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_pointing_location,
        &request.reflector_boresight,
        &request.vehicle_position,
        calibration.physical_config.reflector.focal_length_m,
        calibration.physical_config.reflector.diameter_m,
        request.vehicle_attitude,
    )?;

    tracing::debug!(
        emitter_az = %emitter_az,
        emitter_el = %emitter_el,
        "Computed emitter direction in antenna frame"
    );

    let prepared = PreparedServedGain::prepare(
        calibration,
        FeedSteering::new(steer_x, steer_y, steer_z),
        ServedFrequencies::new(request.frequency_mhz, request.pointing_frequency_mhz),
        time_budget,
    )?;

    // Everything from here to the response DTO is the served-gain law, and it lives in
    // `service::served_gain` — this endpoint no longer knows how to build the
    // physical-optics model, gate coverage, evaluate a correction surface, or decide which
    // warnings a direction earns.
    let served = prepared.evaluate_direct(
        PreSquintDirection::new(emitter_az, emitter_el),
        if request.include_reference {
            ReferenceGainRequest::Include
        } else {
            ReferenceGainRequest::Omit
        },
    )?;

    // Issue #64: both public correction fields come from the served result's authoritative
    // disposition, never from calibration status or an open-coded surface predicate.
    let calibration_status_info = prepared.calibration_status().map(|status| {
        let mut info = CalibrationStatusInfo::from(status);
        let application = match served.correction {
            CorrectionDisposition::Unavailable => CorrectionApplication::Unavailable,
            CorrectionDisposition::OutsideCoverage => CorrectionApplication::None,
            CorrectionDisposition::Applied { .. } => CorrectionApplication::All,
        };
        info.set_correction_application(application);
        info
    });

    Ok(GainResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        gain_db: served.gain_db,
        reference_gain_db: served.reference_gain_db,
        loss_db: served.loss_db,
        geometry: GeometryInfo {
            physical_feed_offset_m: Vector3D::new(
                served.physical_feed_offset.x_m,
                served.physical_feed_offset.y_m,
                served.physical_feed_offset.z_m,
            ),
            emitter_azimuth_deg: served.direction.e_clock_deg,
            emitter_elevation_deg: served.direction.e_cone_deg,
            beam_squint_deg: served.reported_beam_squint_deg(),
        },
        // A single-gain failure is an HTTP error, never a 200 body carrying a
        // reason — only `service::batch` populates this field.
        error: None,
        metadata: ComputationMetadata {
            computation_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            coordinate_transform_ms: None,
            physics_model_ms: None,
            correction_surface_ms: None,
            extrapolated: served.correction.extrapolated(),
            spillover_loss_db: served.spillover_loss_db,
        },
        warnings: served.warnings,
        calibration_status: calibration_status_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schemas::Position3D;
    use crate::data::types::{
        AntennaCalibration, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
        FeedParameters, MeshParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
    };
    use crate::service::test_support::{create_test_calibration, dummy_correction_surface};
    use crate::warnings::WarningCode;

    /// The calibration-status warning codes the served-gain law can emit
    /// (`service::served_gain::generate_calibration_warnings`).
    ///
    /// Tests that assert "this response carries no calibration-status warnings"
    /// select on this set. Before C8 stage 3 they instead *excluded* the two known
    /// convergence phrases by substring, which passed for any warning class that
    /// was neither — including new ones nobody had considered.
    const CALIBRATION_WARNING_CODES: &[WarningCode] = &[
        WarningCode::Uncalibrated,
        WarningCode::PartiallyCalibrated,
        WarningCode::OutOfCoverage,
        WarningCode::CorrectionNotApplied,
    ];

    fn create_test_request() -> GainRequest {
        // Emitter is a LEO satellite at 400 km geodetic altitude.
        let emitter = Position3D::geodetic(-117.0, 35.0, 400_000.0);
        GainRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: Position3D::geodetic(-118.0, 34.0, 100.0),
            reflector_boresight: Position3D::geodetic(-117.99, 34.01, 110.0), // 10m from vehicle
            feed_pointing_location: Position3D::geodetic(-117.99, 34.01, 123.6), // Feed at focal point
            emitter_position: emitter,
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        }
    }

    /// The correction surface must be evaluated at the calibration's
    /// temperature_const, not a hardcoded 290 K. This artifact is calibrated
    /// at 300 K; with the old hardcoded 290 K the temperature dimension
    /// extrapolated and emitted a warning.
    #[test]
    fn test_correction_uses_calibration_temperature() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.validity_ranges.temperature_const = 300.0;
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![300.0, 300.0, 300.0, 300.0, 300.0, 300.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            !response
                .warnings
                .iter()
                .any(|w| w.message.contains("temperature")),
            "no temperature extrapolation warning expected, got: {:?}",
            response.warnings
        );
        assert!(!response.metadata.extrapolated);
    }

    #[test]
    fn test_compute_gain_uncalibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed (physics model)
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "uncalibrated");
        assert_eq!(status.accuracy_estimate_db, 3.0);
        assert_eq!(status.loss_accuracy_estimate_db, Some(2.0));
        assert_eq!(
            status.correction_application,
            CorrectionApplication::Unavailable
        );
        assert!(!status.correction_applied);

        // Should have warning about uncalibrated
        assert!(!response.warnings.is_empty());
        assert!(response
            .warnings
            .iter()
            .any(|w| w.is(WarningCode::Uncalibrated)));
    }

    /// Evaluate a boresight-pointed request (emitter and feed both aimed along the
    /// reflector boresight, as in `test_loss_near_zero_for_boresight_focused_feed`)
    /// against an uncalibrated fixture whose feed has been mutated by `mutate` —
    /// returns the served gain_db. Boresight pointing is used (rather than the
    /// default off-axis `create_test_request()` geometry) so that gain changes are
    /// attributable to the feed mutation rather than to which sidelobe the default
    /// off-axis direction happens to land in.
    fn gain_with_feed_mutation(mutate: impl FnOnce(&mut FeedParameters)) -> f64 {
        let mut calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        mutate(&mut calibration.physical_config.feed);
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(calibration);
        let mut request = create_test_request();
        request.emitter_position = request.reflector_boresight.clone();
        request.feed_pointing_location = request.reflector_boresight.clone();
        compute_gain_from_request(&request, &repo).unwrap().gain_db
    }

    /// P7 auto-refocus, end-to-end: a config-level phase_center_offset_m must not
    /// change the served gain (it is a compensated feed property).
    #[test]
    fn test_phase_center_offset_m_is_inert_at_service_level() {
        let g_zero = gain_with_feed_mutation(|_| {});
        let g_pco = gain_with_feed_mutation(|feed| feed.phase_center_offset_m = 0.02);
        // Same deterministic code path, same physics inputs -> bit-identical.
        assert_eq!(
            g_zero, g_pco,
            "phase_center_offset_m must be inert (auto-refocus, P7)"
        );
    }

    /// P7: axial_defocus_m is the live deliberate-defocus knob, end-to-end.
    #[test]
    fn test_axial_defocus_m_reduces_gain_at_service_level() {
        let g_focused = gain_with_feed_mutation(|_| {});
        let g_defocused = gain_with_feed_mutation(|feed| feed.axial_defocus_m = 0.05);
        assert!(
            g_focused - g_defocused > 0.5,
            "5 cm axial_defocus_m at 8.4 GHz must cost measurable gain: \
             focused={g_focused:.2}, defocused={g_defocused:.2}"
        );
    }

    #[test]
    fn test_loss_near_zero_for_boresight_focused_feed() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let mut request = create_test_request();
        // Aim emitter along the boresight direction (on-axis) and feed at boresight (focused):
        request.emitter_position = request.reflector_boresight.clone();
        request.feed_pointing_location = request.reflector_boresight.clone();
        request.include_reference = true;
        let response = compute_gain_from_request(&request, &repo).unwrap();
        let loss = response.loss_db.expect("reference requested");
        assert!(
            loss.abs() < 0.6,
            "boresight focused-feed loss should be ~0 dB, got {loss}"
        );
    }

    #[test]
    fn test_compute_gain_uncalibrated_with_reference() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        repo.add_calibration(calibration);

        let mut request = create_test_request();
        request.include_reference = true;

        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have reference gain and loss
        assert!(response.reference_gain_db.is_some());
        assert!(response.loss_db.is_some());

        // Loss should be positive (gain < reference)
        let loss = response.loss_db.unwrap();
        assert!(loss >= 0.0);
    }

    /// Uncalibrated antennas (no correction surface) should have physical
    /// spillover folded in: `apply_spillover` gated on, and the applied loss
    /// (small negative dB, per P1 magnitude finding) surfaced on metadata.
    #[test]
    fn test_spillover_applied_for_uncalibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        let spillover = response
            .metadata
            .spillover_loss_db
            .expect("spillover should be applied and reported for an uncalibrated antenna");
        assert!(
            spillover < 0.0,
            "spillover loss must be negative, got {spillover}"
        );
    }

    /// Calibrated antennas (correction surface present) must NOT have physical
    /// spillover folded in — the surface already absorbs it empirically. The
    /// flag must be off, so `spillover_loss_db` is `None`.
    #[test]
    fn test_spillover_not_applied_for_calibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        // Valid order-3 B-spline knot vectors (>=4 knots each), matching the pattern used by
        // `test_correction_uses_calibration_temperature`, so the surface actually evaluates.
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            response.metadata.spillover_loss_db.is_none(),
            "calibrated antenna must not report spillover_loss_db, got {:?}",
            response.metadata.spillover_loss_db
        );
    }

    /// Reference invariant: the ideal reference computation shares the same
    /// `integration_params` (and thus the same `apply_spillover` gate) as the
    /// actual gain computation, so the *base* spillover applied to both should
    /// cancel out of `loss_db` entirely, leaving only the physics baseline
    /// delta that already exists for the calibrated path (surface-RMS Ruze
    /// loss on the real antenna vs. a perfect ideal reference — see the sibling
    /// `test_loss_near_zero_for_boresight_focused_feed`, tolerance 0.6 dB).
    ///
    /// We assert this directly by comparing the uncalibrated (spillover ON)
    /// loss against the calibrated (spillover OFF) loss for the *same*
    /// physical geometry: if spillover truly cancels, the two losses must be
    /// numerically identical (not just both "small").
    #[test]
    fn test_loss_near_zero_for_boresight_focused_feed_uncalibrated() {
        let mut boresight_request = create_test_request();
        // Aim emitter along the boresight direction (on-axis) and feed at boresight (focused):
        boresight_request.emitter_position = boresight_request.reflector_boresight.clone();
        boresight_request.feed_pointing_location = boresight_request.reflector_boresight.clone();
        boresight_request.include_reference = true;

        // Baseline: calibrated (correction surface present) -> apply_spillover is off.
        // `correction_surface` is a distinct field from `calibration_status`, so it must
        // be attached explicitly even for `FullyCalibrated` (see other tests in this module).
        let mut repo_calibrated = CalibrationRepository::new();
        let mut calibration_with_surface =
            create_test_calibration(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            });
        calibration_with_surface.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        assert!(calibration_with_surface.correction_surface.is_some());
        repo_calibrated.add_calibration(calibration_with_surface);
        let response_calibrated =
            compute_gain_from_request(&boresight_request, &repo_calibrated).unwrap();
        assert!(response_calibrated.metadata.spillover_loss_db.is_none());
        let loss_calibrated = response_calibrated
            .loss_db
            .expect("reference requested (calibrated baseline)");

        // Uncalibrated: no correction surface -> apply_spillover is on for both
        // the actual and ideal-reference computations.
        let mut repo_uncalibrated = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo_uncalibrated.add_calibration(calibration);
        let response_uncalibrated =
            compute_gain_from_request(&boresight_request, &repo_uncalibrated).unwrap();

        // Confirm spillover was actually applied on this path (otherwise the
        // invariant would hold trivially and prove nothing about cancellation).
        let spillover = response_uncalibrated
            .metadata
            .spillover_loss_db
            .expect("spillover should be applied on the uncalibrated path");
        assert!(spillover < 0.0);

        let loss_uncalibrated = response_uncalibrated
            .loss_db
            .expect("reference requested (uncalibrated)");

        // The base spillover efficiency applies identically to the actual and
        // ideal-reference computations (same q-factor, f/D, and on-axis/zero
        // feed offset in both), so it must cancel exactly out of loss_db.
        assert!(
            (loss_uncalibrated - loss_calibrated).abs() < 1e-6,
            "spillover should cancel out of loss_db entirely: calibrated (no spillover) = {loss_calibrated}, \
             uncalibrated (spillover applied to both actual and reference) = {loss_uncalibrated}"
        );

        // Sanity: still within the same loose bound as the calibrated sibling test.
        assert!(
            loss_uncalibrated.abs() < 0.6,
            "boresight focused-feed loss should be ~0 dB, got {loss_uncalibrated}"
        );
    }

    /// Build a request whose feed is steered far off boresight, so the actual
    /// gain routes to a non-StandardPhysicalOptics mode (large feed offset) and
    /// the model layer applies no spillover. Mirrors the ECEF geometry the
    /// integration tests use for their large-offset cases (feed near the vehicle,
    /// boresight and emitter coincident at a 400 km satellite, so the emitter sits
    /// essentially ON boresight — this is a large FEED offset, not a large pointing
    /// angle).
    fn create_large_offset_request() -> GainRequest {
        use crate::model::coordinates_3d::geodetic_to_ecef;
        let (veh_x, veh_y, veh_z) = geodetic_to_ecef(-118.1234, 34.5678, 100.0).unwrap();
        let (emit_x, emit_y, emit_z) = geodetic_to_ecef(-117.0, 35.0, 400_000.0).unwrap();
        let (feed_x, feed_y, feed_z) = geodetic_to_ecef(-118.124, 34.568, 105.0).unwrap();

        let ecef = |x: f64, y: f64, z: f64| Position3D::ecef(x, y, z);

        GainRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: ecef(veh_x, veh_y, veh_z),
            reflector_boresight: ecef(emit_x, emit_y, emit_z),
            feed_pointing_location: ecef(feed_x, feed_y, feed_z),
            emitter_position: ecef(emit_x, emit_y, emit_z),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: true,
            vehicle_attitude: None,
        }
    }

    /// A request whose emitter sits tens of degrees off the boresight axis with
    /// the feed AT focus (small offset → StandardPhysicalOptics), so the physics
    /// pattern is deep in the sidelobes — far below any plausible Ruze floor.
    /// Mirrors the P8 off-axis integration geometry: boresight aims at satellite
    /// A (−117, 35, 400 km); emitter is at satellite B (−120, 30, 400 km), tens of
    /// degrees away. (Contrast `create_large_offset_request`, where emitter ==
    /// boresight so θ ≈ 0 — a large *feed* offset, not a large pointing angle.)
    fn create_deep_offaxis_request() -> GainRequest {
        use crate::model::coordinates_3d::geodetic_to_ecef;
        let ecef = |lon: f64, lat: f64, alt: f64| {
            let (x, y, z) = geodetic_to_ecef(lon, lat, alt).unwrap();
            Position3D::ecef(x, y, z)
        };
        let mut request = create_large_offset_request();
        // Feed aimed at the boresight target → feed at focus → StandardPhysicalOptics.
        request.feed_pointing_location = request.reflector_boresight.clone();
        // Emitter to a far-off satellite: ~69 deg off the boresight axis (FORWARD
        // hemisphere — verified empirically via `geometry.emitter_elevation_deg`).
        request.emitter_position = ecef(-120.0, 30.0, 400_000.0);
        request.include_reference = false;
        request
    }

    /// Regression test for the reference/actual spillover asymmetry: for an
    /// uncalibrated antenna at a LARGE feed offset, the actual gain routes to a
    /// non-standard-PO mode and gets NO spillover. The ideal reference (always a
    /// focused feed → standard PO) must therefore also skip spillover, tracking
    /// the actual's state — otherwise `loss_db` carries a one-sided spillover bias
    /// while `metadata.spillover_loss_db` reports `None`.
    ///
    /// We prove consistency by comparing the ideal REFERENCE gain against a
    /// CALIBRATED antenna (correction surface → no spillover on either side) for
    /// the SAME geometry: with the reference tracking the actual's (absent)
    /// spillover, the two ideal-boresight reference gains must be numerically
    /// identical. (We compare the references directly rather than `loss_db`,
    /// because after the F7 redesign 2026-07-16 the uncalibrated ACTUAL gain
    /// additionally carries the statistical floor via the power sum — a separate,
    /// correct uncorrected-physics behavior gated by the same predicate as
    /// spillover — which would confound a raw `loss_db` comparison. The reference
    /// is an ideal reflector (surface_rms = 0.0 ⇒ floor = 0) at boresight, so it
    /// is floor-independent and isolates the spillover-tracking invariant.)
    #[test]
    fn test_large_offset_uncalibrated_reference_has_no_spillover_bias() {
        let request = create_large_offset_request();

        // Uncalibrated: apply_spillover flag on, but actual is large-offset → no spillover.
        let mut repo_uncalibrated = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo_uncalibrated.add_calibration(calibration);
        let response_uncalibrated =
            compute_gain_from_request(&request, &repo_uncalibrated).unwrap();

        // The actual got no spillover (non-standard-PO mode gate in the model layer).
        assert!(
            response_uncalibrated.metadata.spillover_loss_db.is_none(),
            "large-offset actual should not report spillover, got {:?}",
            response_uncalibrated.metadata.spillover_loss_db
        );
        let reference_uncalibrated = response_uncalibrated
            .reference_gain_db
            .expect("reference requested (uncalibrated)");

        // Calibrated baseline: correction surface present → apply_spillover off on both sides.
        let mut repo_calibrated = CalibrationRepository::new();
        let mut calibration_with_surface =
            create_test_calibration(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            });
        calibration_with_surface.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo_calibrated.add_calibration(calibration_with_surface);
        let response_calibrated = compute_gain_from_request(&request, &repo_calibrated).unwrap();
        assert!(response_calibrated.metadata.spillover_loss_db.is_none());
        let reference_calibrated = response_calibrated
            .reference_gain_db
            .expect("reference requested (calibrated baseline)");

        // With the reference tracking the actual's (absent) spillover, the two ideal
        // boresight reference gains must be numerically identical — no one-sided spillover
        // bias. (Floor-independent: the ideal reflector has surface_rms = 0.0, so its floor
        // is identically zero whether the flag is on or off.)
        assert!(
            (reference_uncalibrated - reference_calibrated).abs() < 1e-6,
            "reference spillover must track the actual: uncalibrated large-offset reference = \
             {reference_uncalibrated}, calibrated (no spillover) reference = {reference_calibrated}"
        );
    }

    /// Spillover keys on correction-surface *presence* (whole-antenna gate), NOT
    /// on per-query coverage. An antenna WITH a surface whose coverage excludes the
    /// query (so `correction_applied` is false but `correction_surface.is_some()`)
    /// must still get NO spillover.
    #[test]
    fn test_spillover_not_applied_when_surface_present_but_out_of_coverage() {
        let mut repo = CalibrationRepository::new();

        // Coverage restricted to a narrow region the default request falls outside.
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 5.0)
            .elevation_range(0.0, 5.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        });
        // Attach a valid surface so `correction_surface.is_some()` even though the query
        // is out of coverage (→ correction not applied).
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 5.0, 5.0, 5.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 5.0, 5.0, 5.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Surface exists but wasn't applied here (out of coverage → extrapolated).
        assert!(response.metadata.extrapolated);
        let status = response.calibration_status.as_ref().unwrap();
        assert_eq!(status.correction_application, CorrectionApplication::None);
        assert!(!status.correction_applied);
        // Whole-antenna gate: presence of a surface suppresses spillover regardless.
        assert!(
            response.metadata.spillover_loss_db.is_none(),
            "spillover must key on surface presence, not coverage; got {:?}",
            response.metadata.spillover_loss_db
        );
    }

    #[test]
    fn test_compute_gain_fully_calibrated() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        // Corrected physics ⇒ a correction surface is present. Without it the P11
        // predicate treats the served gain as raw physics and (correctly) fires the
        // off-axis honesty warning; a fully-calibrated antenna carries a surface.
        calibration.correction_surface = Some(dummy_correction_surface());
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "fully_calibrated");
        assert_eq!(status.accuracy_estimate_db, 1.0);
        assert_eq!(status.correction_application, CorrectionApplication::All);
        assert!(status.correction_applied);

        // Should NOT have calibration-related warnings (fully calibrated).
        // Integration convergence warnings are acceptable and unrelated to
        // calibration status. Post-C8-stage-3 this selects the calibration class by
        // code rather than excluding two convergence phrases by substring — the
        // old form also passed for any *new* unrelated warning class.
        let calibration_warnings: Vec<_> = response
            .warnings
            .iter()
            .filter(|w| CALIBRATION_WARNING_CODES.contains(&w.code))
            .collect();
        assert!(
            calibration_warnings.is_empty(),
            "Unexpected calibration warnings: {:?}",
            calibration_warnings
        );
    }

    /// End-to-end proof that a boresight artifact's frequency correction actually
    /// reaches the served gain.
    ///
    /// This is the assertion that catches a *silent skip*: before 2026-07-31 the
    /// artifact loaded, carried its correction, reported `PartiallyCalibrated`, and
    /// served raw physics — every observable except this one looked healthy. It
    /// asserts `correction_applied` and the gain shift, not just a tolerance band.
    #[test]
    fn a_boresight_aimed_query_gets_the_boresight_correction_applied() {
        const CORRECTION_DB: f64 = 1.5;

        // Emitter placed exactly at the boresight aim point: the query IS boresight,
        // so its azimuth is atan2 on float noise and its elevation is ~0.
        let target = Position3D::geodetic(-117.0, 35.0, 400_000.0);
        let request = GainRequest {
            emitter_position: target.clone(),
            reflector_boresight: target,
            ..create_test_request()
        };

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, crate::data::types::BORESIGHT_COVERAGE_CONE_DEG)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(5)
            .has_correction_surface(true)
            .build()
            .unwrap();

        // A flat-axis frequency correction shaped exactly as calibrate's
        // `fit_frequency_correction` writes it: order + 1 identical layers on
        // azimuth/elevation/temperature over their full queryable spans.
        let surface = |value: f64| {
            let order = 3usize;
            let layers = order + 1;
            let n_freq = 4;
            crate::data::types::BSplineModel4D {
                coefficients: vec![value; layers * layers * n_freq * layers],
                shape: [layers, layers, n_freq, layers],
                knots_azimuth: vec![0.0, 0.0, 0.0, 180.0, 360.0, 360.0, 360.0],
                knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 180.0, 180.0, 180.0],
                knots_frequency: vec![8000.0, 8000.0, 8000.0, 8300.0, 9000.0, 9000.0, 9000.0],
                knots_temperature: vec![0.0, 0.0, 0.0, 500.0, 1000.0, 1000.0, 1000.0],
                spline_order: order as u8,
            }
        };

        let serve = |correction_db: f64| {
            let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db: 1.5,
                coverage: coverage.clone(),
            });
            calibration.correction_surface = Some(surface(correction_db));
            let mut repo = CalibrationRepository::new();
            repo.add_calibration(calibration);
            compute_gain_from_request(&request, &repo).unwrap()
        };

        // The baseline carries a ZERO-valued surface rather than no surface at all:
        // `physics_is_uncorrected()` gates spillover and the F7 sidelobe floor on
        // surface *presence*, so a no-surface baseline would compute different
        // physics and the difference below would not isolate the correction.
        let baseline = serve(0.0);
        let corrected = serve(CORRECTION_DB);

        let status = corrected
            .calibration_status
            .as_ref()
            .expect("partially calibrated artifact must report status");
        assert_eq!(status.correction_application, CorrectionApplication::All);
        assert!(
            status.correction_applied,
            "the boresight correction was silently skipped — the coverage gate is \
             rejecting a boresight query again"
        );

        assert!(
            (corrected.gain_db - baseline.gain_db - CORRECTION_DB).abs() < 1e-9,
            "served gain should shift by exactly the correction: {} - {} != {CORRECTION_DB}",
            corrected.gain_db,
            baseline.gain_db
        );
    }

    #[test]
    fn test_compute_gain_partially_calibrated_in_coverage() {
        let mut repo = CalibrationRepository::new();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(false)
            .build()
            .unwrap();

        let calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "partially_calibrated");
        assert_eq!(status.accuracy_estimate_db, 1.5);

        // Should have warning about partial calibration
        assert!(!response.warnings.is_empty());
        assert!(response
            .warnings
            .iter()
            .any(|w| w.is(WarningCode::PartiallyCalibrated)));
    }

    #[test]
    fn test_compute_gain_antenna_not_found() {
        let repo = CalibrationRepository::new();
        let request = create_test_request();
        let result = compute_gain_from_request(&request, &repo);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AntennaModelError::FeedNotFound { .. }
        ));
    }

    #[test]
    fn test_extrapolated_flag_out_of_coverage() {
        // When a correction surface exists but the query is outside coverage,
        // the extrapolated flag should be set to true.
        let mut repo = CalibrationRepository::new();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 10.0) // narrow range
            .elevation_range(0.0, 10.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });
        // Add a correction surface so out-of-coverage detection triggers
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 10.0],
            knots_elevation: vec![0.0, 10.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        // Use the standard test request — emitter direction lands outside the narrow coverage
        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Since the correction surface exists but coverage doesn't include the emitter direction,
        // the extrapolated flag should be set.
        assert!(
            response.metadata.extrapolated,
            "Expected extrapolated=true when correction surface exists but query is out-of-coverage"
        );
    }

    #[test]
    fn test_extrapolated_flag_no_correction_surface() {
        // When there's no correction surface, extrapolated should be false
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            !response.metadata.extrapolated,
            "Expected extrapolated=false when no correction surface"
        );
    }

    /// Test that `physical_feed_offset_m` is ~zero when the feed is aimed at the same Earth
    /// target as the reflector boresight (focused/on-axis configuration).
    ///
    /// When `feed_pointing_location == reflector_boresight`, `compute_feed_position_from_pointing`
    /// returns (0, 0, focal_length), so the physical offset from the focal point is
    /// (0, 0, focal_length - focal_length) = (0, 0, 0).
    ///
    /// The OLD (angular) code also returned ~(0, 0, 0) for this case because
    /// `feed_az - refl_az ≈ 0` — so this test alone does not discriminate.
    /// See `test_feed_offset_is_meters_not_degrees` for the discriminating case.
    ///
    /// It *does* discriminate on the frame of the artifact's design offset, and that is
    /// the second thing it now guards (roadmap **C13**, closed 2026-08-02):
    /// `create_test_calibration` writes `position: (0, 0, 0)` for an on-axis feed —
    /// focus-relative, the convention `antennas.yaml` and the boresight producer use —
    /// against a 5 m focal length, so a vertex-relative artifact would land here at
    /// `z = 5.0` and blow the 0.05 m bound. This is the design-spec producer's half of
    /// C13's "one test per producer"; the `calibrate` half is
    /// `exported_feed_position_is_focus_relative_not_vertex_relative` (unit) and the
    /// served assertion in `calibrate/tests/cli_full_mode_real_data_e2e.rs` (end to end).
    #[test]
    fn test_feed_offset_reported_in_meters_zero_for_boresight() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let mut request = create_test_request();
        // Aim the feed at the same Earth point as the reflector boresight → on-axis feed,
        // so the physical feed offset from the focal point should be ~zero.
        request.feed_pointing_location = request.reflector_boresight.clone();
        let response = compute_gain_from_request(&request, &repo).unwrap();
        let off = &response.geometry.physical_feed_offset_m;
        assert!(
            off.x.abs() < 0.05 && off.y.abs() < 0.05 && off.z.abs() < 0.05,
            "expected ~zero physical offset in meters for boresight-aimed feed, got ({}, {}, {})",
            off.x,
            off.y,
            off.z
        );
    }

    /// Discriminating test: verifies `physical_feed_offset_m` contains physical meters,
    /// not angular degrees.
    ///
    /// Strategy: call `compute_feed_position_from_pointing` directly to get the
    /// expected physical feed position (x, y, z) in the antenna frame, then assert
    /// that `response.geometry.physical_feed_offset_m` equals (x, y, z - focal_length_m).
    ///
    /// The default `create_test_request()` has `feed_pointing_location` at a different altitude
    /// than `reflector_boresight` (123.6 m vs 110.0 m at the same lon/lat), giving a
    /// non-zero angular offset and therefore a non-zero physical feed displacement.
    ///
    /// The OLD code stored angular degrees (feed_az - refl_az, feed_el - refl_el),
    /// which for this geometry differ from the physical meters values — so this test
    /// WOULD HAVE FAILED against the old implementation.
    #[test]
    fn test_feed_offset_is_meters_not_degrees() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let request = create_test_request();

        // Compute the expected physical feed position directly using the same helper
        // the evaluator uses. focal_length_m = 5.0 (from create_test_calibration).
        let focal_length_m = 5.0_f64;
        let diameter_m = 10.0_f64;
        let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
            &request.feed_pointing_location,
            &request.reflector_boresight,
            &request.vehicle_position,
            focal_length_m,
            diameter_m,
            None,
        )
        .expect("compute_feed_position_from_pointing failed in test");
        // Design offset from create_test_calibration is (0, 0, 0), so total = steer
        let expected_x = steer_x;
        let expected_y = steer_y;
        let expected_z_offset = steer_z - focal_length_m;

        let response = compute_gain_from_request(&request, &repo).unwrap();
        let off = &response.geometry.physical_feed_offset_m;

        assert!(
            (off.x - expected_x).abs() < 1e-9,
            "physical_feed_offset_m.x should be {expected_x} m (physical), got {}",
            off.x
        );
        assert!(
            (off.y - expected_y).abs() < 1e-9,
            "physical_feed_offset_m.y should be {expected_y} m (physical), got {}",
            off.y
        );
        assert!(
            (off.z - expected_z_offset).abs() < 1e-9,
            "physical_feed_offset_m.z should be {expected_z_offset} m (z - focal_length), got {}",
            off.z
        );

        // Also verify the magnitude is physically plausible (sub-meter for the small
        // angular offset in this test geometry, certainly < focal_length = 5 m).
        let mag = (off.x * off.x + off.y * off.y + off.z * off.z).sqrt();
        assert!(
            mag.is_finite() && mag < focal_length_m,
            "feed offset magnitude {mag} m is not physically plausible (should be < focal_length {focal_length_m} m)"
        );
    }

    #[test]
    fn test_backward_compatibility_no_calibration_status() {
        let mut repo = CalibrationRepository::new();

        // Create calibration without calibration_status (old format)
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-01T00:00:00Z")
            .format_version("1.0")
            .data_source("test")
            .rmse_db(0.5)
            .r_squared(0.99)
            .num_measurements(1000)
            .build()
            .unwrap();

        let mut calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(PhysicalAntennaConfig {
                reflector: ReflectorGeometry {
                    diameter_m: 10.0,
                    focal_length_m: 5.0,
                    f_over_d_ratio: 0.5,
                    surface_rms_mm: 0.5,
                },
                feed: FeedParameters {
                    // Feed at focal point - zero offset from optical axis
                    position: (0.0, 0.0, 0.0),
                    q_factor: 8.0,
                    phase_center_offset_m: 0.0,
                    axial_defocus_m: 0.0,
                    asymmetry_factor: 1.0,
                },
                mesh: Some(MeshParameters {
                    mesh_spacing_mm: 5.0,
                    wire_diameter_mm: 0.5,
                }),
            })
            .validity_ranges(ValidityRanges {
                azimuth_min_max: (0.0, 360.0),
                elevation_min_max: (0.0, 90.0),
                frequency_min_max: (1000.0, 10000.0),
                temperature_const: 290.0,
            })
            .build()
            .unwrap();
        // Old-format calibrated .bin: no calibration_status field, but a correction
        // surface IS present (corrected physics). This test pins the status-None
        // backward-compat path; the surface keeps the P11 off-axis warning silent, as
        // it must for a corrected antenna.
        calibration.correction_surface = Some(dummy_correction_surface());

        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should still compute gain
        assert!(!response.gain_db.is_nan());

        // calibration_status should be None for backward compatibility
        assert!(response.calibration_status.is_none());

        // Should not have calibration warnings (treated as fully calibrated).
        // Integration convergence warnings are acceptable and unrelated to
        // calibration status. Post-C8-stage-3 this selects the calibration class by
        // code rather than excluding two convergence phrases by substring — the
        // old form also passed for any *new* unrelated warning class.
        let calibration_warnings: Vec<_> = response
            .warnings
            .iter()
            .filter(|w| CALIBRATION_WARNING_CODES.contains(&w.code))
            .collect();
        assert!(
            calibration_warnings.is_empty(),
            "Unexpected calibration warnings: {:?}",
            calibration_warnings
        );
    }

    // Served-path tests below stay at the service/API boundary. The physical floor formula,
    // ideal-reference construction, and model projection are owned and tested below that seam.

    /// The public service seam must preserve the uncorrected-physics preparation policy. A
    /// zero-dB correction surface changes no interpolation value; its presence disables the
    /// floor and spillover terms that a real correction surface empirically absorbs. Their
    /// individual formulas are tested by `antenna-core/src/model/pattern.rs` symbols
    /// `compute_gain` and `sidelobe_floor_gain`.
    #[test]
    fn served_uncorrected_physics_policy_changes_deep_offaxis_gain() {
        let mut uncorrected_repository = CalibrationRepository::new();
        let mut uncorrected = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        uncorrected.physical_config.reflector.surface_rms_mm = 1.5;
        uncorrected_repository.add_calibration(uncorrected.clone());

        let mut corrected_repository = CalibrationRepository::new();
        let mut corrected = uncorrected;
        corrected.calibration_status = Some(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        corrected.correction_surface = Some(dummy_correction_surface());
        corrected_repository.add_calibration(corrected);

        let request = create_deep_offaxis_request();
        let uncorrected_response =
            compute_gain_from_request(&request, &uncorrected_repository).unwrap();
        let corrected_response =
            compute_gain_from_request(&request, &corrected_repository).unwrap();

        assert!(
            uncorrected_response.geometry.emitter_elevation_deg.abs() <= 90.0,
            "deep-offaxis premise broken: expected forward hemisphere"
        );
        assert!(
            uncorrected_response.gain_db - corrected_response.gain_db > 1.0,
            "the uncorrected-physics policy must materially raise deep-off-axis gain: \
             uncorrected={} dBi, corrected={} dBi",
            uncorrected_response.gain_db,
            corrected_response.gain_db
        );
    }

    /// **Roadmap D23, the served half.** Distinct artifact asymmetry factors must produce
    /// distinct results through the public service seam. This catches substitution of the
    /// symmetric default without rebuilding the model that `PreparedServedGain` owns.
    #[test]
    fn served_gain_uses_the_artifacts_asymmetry_factor() {
        let repository_for = |asymmetry_factor| {
            let mut repository = CalibrationRepository::new();
            let mut calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
                accuracy_estimate_db: 3.0,
                loss_accuracy_estimate_db: 2.0,
            });
            calibration.physical_config.feed.asymmetry_factor = asymmetry_factor;
            repository.add_calibration(calibration);
            repository
        };

        let request = create_deep_offaxis_request();
        let asymmetric = compute_gain_from_request(&request, &repository_for(1.1)).unwrap();
        let symmetric = compute_gain_from_request(&request, &repository_for(1.0)).unwrap();

        assert!(
            (asymmetric.gain_db - symmetric.gain_db).abs() > 1e-6,
            "negative control failed: asymmetry factors 1.1 and 1.0 produced indistinguishable \
             served gains ({} and {} dBi)",
            asymmetric.gain_db,
            symmetric.gain_db
        );
    }

    /// Endpoint coverage: the batch path must preserve the single-gain result instead of
    /// reconstructing any part of the served-gain law.
    #[test]
    fn batch_matches_single_gain_for_deep_offaxis_query() {
        use crate::api::schemas::BatchGainRequest;
        use crate::service::batch::evaluate_batch;

        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        calibration.physical_config.reflector.surface_rms_mm = 1.5;
        assert!(calibration.correction_surface.is_none());
        repo.add_calibration(calibration);

        let single_response =
            compute_gain_from_request(&create_deep_offaxis_request(), &repo).unwrap();
        assert!(
            single_response.geometry.emitter_elevation_deg.abs() <= 90.0,
            "deep-offaxis premise broken: expected forward hemisphere"
        );
        let single = single_response.gain_db;

        let request = BatchGainRequest {
            evaluations: vec![create_deep_offaxis_request()],
        };
        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 1);

        assert!(
            (response.results[0].gain_db - single).abs() < 1e-12,
            "batch item {} must equal the single-path gain {single}",
            response.results[0].gain_db
        );
    }
}
