use serde::{Deserialize, Serialize};

/// Parabolic reflector geometry.
///
/// Artifact invariants ([`crate::artifact`]): positive diameter and focal length;
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
