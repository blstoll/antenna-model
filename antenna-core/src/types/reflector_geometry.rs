use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Parabolic reflector geometry.
///
/// Invariants, checked by [`Self::validate`]: positive diameter and focal length;
/// `f_over_d_ratio` within the model's supported range and within 1% of
/// `focal_length_m / diameter_m`; non-negative surface RMS.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReflectorGeometry {
    /// Dish diameter, metres.
    pub diameter_m: f64,

    /// Focal length, metres.
    pub focal_length_m: f64,

    /// f/D ratio; redundant with `focal_length_m / diameter_m` and must agree with it.
    pub f_over_d_ratio: f64,

    /// Surface RMS error, millimetres (Ruze efficiency).
    pub surface_rms_mm: f64,
}

impl ReflectorGeometry {
    /// Creates a new builder for constructing `ReflectorGeometry`.
    pub fn builder() -> ReflectorGeometryBuilder {
        ReflectorGeometryBuilder::default()
    }

    /// Checks the invariants listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        use crate::model::geometry::{F_OVER_D_MAX, F_OVER_D_MIN};

        if self.diameter_m <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "diameter_m".to_string(),
                value: self.diameter_m,
                reason: "must be positive".to_string(),
            });
        }

        if self.focal_length_m <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "focal_length_m".to_string(),
                value: self.focal_length_m,
                reason: "must be positive".to_string(),
            });
        }

        if !(F_OVER_D_MIN..=F_OVER_D_MAX).contains(&self.f_over_d_ratio) {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "f_over_d_ratio".to_string(),
                value: self.f_over_d_ratio,
                reason: format!("must be between {F_OVER_D_MIN} and {F_OVER_D_MAX}"),
            });
        }

        let implied_f_over_d = self.focal_length_m / self.diameter_m;
        if (self.f_over_d_ratio - implied_f_over_d).abs() > 0.01 * implied_f_over_d {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "f_over_d_ratio".to_string(),
                value: self.f_over_d_ratio,
                reason: format!(
                    "inconsistent with focal_length_m/diameter_m = {implied_f_over_d:.4}"
                ),
            });
        }

        if self.surface_rms_mm < 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "surface_rms_mm".to_string(),
                value: self.surface_rms_mm,
                reason: "must be non-negative".to_string(),
            });
        }

        Ok(())
    }
}

/// Builder for [`ReflectorGeometry`]. Every field is required.
#[derive(Default)]
pub struct ReflectorGeometryBuilder {
    diameter_m: Option<f64>,
    focal_length_m: Option<f64>,
    f_over_d_ratio: Option<f64>,
    surface_rms_mm: Option<f64>,
}

impl ReflectorGeometryBuilder {
    pub fn diameter_m(mut self, diameter: f64) -> Self {
        self.diameter_m = Some(diameter);
        self
    }

    pub fn focal_length_m(mut self, focal_length: f64) -> Self {
        self.focal_length_m = Some(focal_length);
        self
    }

    pub fn f_over_d_ratio(mut self, ratio: f64) -> Self {
        self.f_over_d_ratio = Some(ratio);
        self
    }

    pub fn surface_rms_mm(mut self, rms: f64) -> Self {
        self.surface_rms_mm = Some(rms);
        self
    }

    pub fn build(self) -> Result<ReflectorGeometry, String> {
        Ok(ReflectorGeometry {
            diameter_m: self.diameter_m.ok_or("diameter_m is required")?,
            focal_length_m: self.focal_length_m.ok_or("focal_length_m is required")?,
            f_over_d_ratio: self.f_over_d_ratio.ok_or("f_over_d_ratio is required")?,
            surface_rms_mm: self.surface_rms_mm.ok_or("surface_rms_mm is required")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflector_geometry_rejects_inconsistent_f_over_d() {
        let geom = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.6, // truth is 0.5
            surface_rms_mm: 0.5,
        };
        assert!(geom.validate().is_err());

        let consistent = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.5,
            surface_rms_mm: 0.5,
        };
        assert!(consistent.validate().is_ok());
    }

    #[test]
    fn test_reflector_geometry_rejects_out_of_range_f_over_d() {
        // Ratios consistent with focal_length_m/diameter_m but outside [0.2, 1.0].
        let too_high = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 15.0,
            f_over_d_ratio: 1.5,
            surface_rms_mm: 0.5,
        };
        assert!(too_high.validate().is_err());

        let too_low = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 1.0,
            f_over_d_ratio: 0.1,
            surface_rms_mm: 0.5,
        };
        assert!(too_low.validate().is_err());
    }
}
