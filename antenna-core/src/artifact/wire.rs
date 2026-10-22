//! The schema-6 payload: what postcard reads and writes, and its conversion to and from the
//! domain [`AntennaCalibration`].
//!
//! The record mirrors [`AntennaCalibration`] field for field and in the same order, except
//! that the correction surface is a [`SurfaceRecord`] of plain values rather than the
//! validated [`FittedCorrectionSurface`]. Postcard is positional: reordering, adding, or
//! retyping a field here is a layout change (see [`super`] for the two version axes).

use serde::{Deserialize, Serialize};

use crate::model::{CorrectionSurfaceLayout, FittedCorrectionSurface};
use crate::types::{
    AntennaCalibration, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
    PhysicalAntennaConfig, ValidationError, ValidityRanges,
};

#[derive(Serialize, Deserialize)]
pub(super) struct CalibrationRecord {
    antenna_id: String,
    feed_id: String,
    pub(super) metadata: CalibrationMetadata,
    physical_config: PhysicalAntennaConfig,
    correction_surface: Option<SurfaceRecord>,
    validity_ranges: ValidityRanges,
    calibration_status: Option<CalibrationStatus>,
    calibration_coverage: Option<CalibrationCoverage>,
}

/// A correction surface on the wire: shape `[n_e_clock, n_e_cone, n_frequency]`,
/// coefficients indexed `i_e_clock + n_e_clock * (i_e_cone + n_e_cone * i_frequency)`.
#[derive(Serialize, Deserialize)]
struct SurfaceRecord {
    coefficients: Vec<f64>,
    shape: [usize; 3],
    knots_e_clock: Vec<f64>,
    knots_e_cone: Vec<f64>,
    knots_frequency: Vec<f64>,
    spline_order: u8,
}

impl From<&AntennaCalibration> for CalibrationRecord {
    fn from(calibration: &AntennaCalibration) -> Self {
        Self {
            antenna_id: calibration.antenna_id.clone(),
            feed_id: calibration.feed_id.clone(),
            metadata: calibration.metadata.clone(),
            physical_config: calibration.physical_config.clone(),
            correction_surface: calibration
                .correction_surface
                .as_ref()
                .map(SurfaceRecord::from),
            validity_ranges: calibration.validity_ranges.clone(),
            calibration_status: calibration.calibration_status.clone(),
            calibration_coverage: calibration.calibration_coverage.clone(),
        }
    }
}

impl CalibrationRecord {
    /// The domain value, with the correction surface held to the layout invariant set. The
    /// rest of the artifact is not validated here; [`super::decode`] does that next.
    pub(super) fn into_domain(self) -> Result<AntennaCalibration, ValidationError> {
        Ok(AntennaCalibration {
            antenna_id: self.antenna_id,
            feed_id: self.feed_id,
            metadata: self.metadata,
            physical_config: self.physical_config,
            correction_surface: self
                .correction_surface
                .map(SurfaceRecord::into_domain)
                .transpose()?,
            validity_ranges: self.validity_ranges,
            calibration_status: self.calibration_status,
            calibration_coverage: self.calibration_coverage,
        })
    }
}

impl From<&FittedCorrectionSurface> for SurfaceRecord {
    fn from(surface: &FittedCorrectionSurface) -> Self {
        let layout = surface.layout();
        Self {
            coefficients: surface.coefficients().to_vec(),
            shape: layout.shape(),
            knots_e_clock: layout.knots_e_clock().to_vec(),
            knots_e_cone: layout.knots_e_cone().to_vec(),
            knots_frequency: layout.knots_frequency().to_vec(),
            spline_order: layout.spline_order(),
        }
    }
}

impl SurfaceRecord {
    fn into_domain(self) -> Result<FittedCorrectionSurface, ValidationError> {
        let layout = CorrectionSurfaceLayout::new(
            self.shape,
            self.knots_e_clock,
            self.knots_e_cone,
            self.knots_frequency,
            self.spline_order,
        )?;
        FittedCorrectionSurface::new(layout, self.coefficients)
    }
}
