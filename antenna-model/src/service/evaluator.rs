//! Gain Computation Service - Core Pipeline

use crate::api::schemas::{GainRequest, GainResponse, GeometryInfo, ComputationMetadata};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::model::{compute_emitter_direction, compute_feed_offset, evaluate_correction, EClockConeCoordinates};
use std::time::Instant;

pub fn compute_gain_from_request(
    request: &GainRequest,
    repository: &CalibrationRepository,
) -> Result<GainResponse> {
    let start = Instant::now();
    let mut warnings = Vec::new();

    let feed_offset = compute_feed_offset(
        &request.feed_position,
        &request.reflector_boresight,
        &request.vehicle_position,
        &request.vehicle_attitude,
    )?;

    let (emitter_az, emitter_el) = compute_emitter_direction(
        &request.emitter_position,
        &request.vehicle_position,
        &request.vehicle_attitude,
    )?;

    let calibration = repository
        .get_calibration(&request.antenna_id, &request.feed_id)
        .ok_or_else(|| AntennaModelError::FeedNotFound {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
        })?;

    let wavelength_m = 299.792458 / request.frequency_mhz;
    let diameter_m = calibration.physical_config.reflector.diameter_m;
    let max_gain_linear = 0.65 * (std::f64::consts::PI * diameter_m / wavelength_m).powi(2);
    let max_gain_db = 10.0 * max_gain_linear.log10();
    
    let e_cone = EClockConeCoordinates::from_degrees(emitter_az, emitter_el);
    let beamwidth_rad = 1.22 * wavelength_m / diameter_m;
    let pointing_loss_db = -12.0 * (e_cone.e_cone / beamwidth_rad).powi(2);
    let base_gain_db = max_gain_db + pointing_loss_db;

    let mut correction_db = 0.0;
    if let Some(ref correction_surface) = calibration.correction_surface {
        let result = evaluate_correction(correction_surface, emitter_az, emitter_el, request.frequency_mhz, 290.0)?;
        correction_db = result.correction_db;
        warnings.extend(result.warnings);
    }

    let final_gain_db = base_gain_db + correction_db;
    let (reference_gain_db, loss_db) = if request.include_reference {
        (Some(max_gain_db), Some(max_gain_db - final_gain_db))
    } else {
        (None, None)
    };

    Ok(GainResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        gain_db: final_gain_db,
        reference_gain_db,
        loss_db,
        geometry: GeometryInfo {
            feed_offset_meters: feed_offset,
            emitter_azimuth_deg: emitter_az,
            emitter_elevation_deg: emitter_el,
            beam_squint_deg: None,
        },
        warnings,
        metadata: ComputationMetadata {
            computation_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            coordinate_transform_ms: None,
            physics_model_ms: None,
            correction_surface_ms: None,
            extrapolated: false,
        },
    })
}
