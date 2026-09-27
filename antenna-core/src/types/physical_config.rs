use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Physical parameters of the physics model: reflector, feed, and optional mesh.
///
/// Every parameter `calibrate` fits against must be carried here, or the service
/// evaluates a different antenna than the residual surface describes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhysicalAntennaConfig {
    /// Reflector geometry.
    pub reflector: ReflectorGeometry,

    /// Feed parameters.
    pub feed: FeedParameters,

    /// Mesh parameters; `None` for a solid reflector.
    pub mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfig {
    /// Creates a new builder for constructing a `PhysicalAntennaConfig`.
    pub fn builder() -> PhysicalAntennaConfigBuilder {
        PhysicalAntennaConfigBuilder::default()
    }

    /// Validates the reflector, feed, and mesh (if any).
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.reflector.validate()?;
        self.feed.validate()?;
        if let Some(ref mesh) = self.mesh {
            mesh.validate()?;
        }
        Ok(())
    }
}

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

/// Feed horn parameters for the aperture integration.
///
/// Invariants, checked by [`Self::validate`]: `q_factor` in `[0, 20]`;
/// `asymmetry_factor` finite and positive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedParameters {
    /// Feed **design offset from the focal point**, antenna-frame metres `(x, y, z)`.
    ///
    /// The origin is the focal point, not the reflector vertex: an on-axis feed is
    /// `(0, 0, 0)`, and `z` is positive away from the vertex. `x`/`y` are this feed's
    /// static lateral displacement from the optical axis. The service adds this to a
    /// vertex-origin steering position, so writing a vertex-relative position here
    /// (`(0, 0, f)`) places the feed at `z ≈ 2f` — a 27.3 dB boresight loss. See C13.
    pub position: (f64, f64, f64),

    /// Exponent `q` of the `cos^q` illumination pattern (typically 6–12).
    pub q_factor: f64,

    /// Phase-centre offset from the feed aperture, metres. Compensated by the model's
    /// auto-refocus, so it produces no defocus.
    pub phase_center_offset_m: f64,

    /// Deliberate axial defocus of the feed phase centre from the focal point, metres;
    /// `0` is focused.
    #[serde(default)]
    pub axial_defocus_m: f64,

    /// E-plane / H-plane illumination asymmetry: `1.0` is symmetric, values above 1.0
    /// broaden the E-plane. Modulates the effective q-factor by `cos 2φ'`, and a
    /// non-unity value selects the azimuthal-mode integrator branch.
    ///
    /// Producers must write the antenna class's design value. A symmetric default here
    /// serves a different illumination than the correction surface was fitted against —
    /// up to 1.20 dB off-axis while invisible at boresight. See D23.
    pub asymmetry_factor: f64,
}

impl FeedParameters {
    /// Creates a new builder for constructing `FeedParameters`.
    pub fn builder() -> FeedParametersBuilder {
        FeedParametersBuilder::default()
    }

    /// Checks the invariants listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.q_factor < 0.0 || self.q_factor > 20.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "q_factor".to_string(),
                value: self.q_factor,
                reason: "must be between 0 and 20".to_string(),
            });
        }

        // Mirrors `model::geometry::FeedParameters::validate`, but fails at artifact load
        // instead of when a request first reaches the integrator.
        if self.asymmetry_factor <= 0.0 || !self.asymmetry_factor.is_finite() {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "asymmetry_factor".to_string(),
                value: self.asymmetry_factor,
                reason: "must be positive (1.0 is a symmetric feed)".to_string(),
            });
        }

        Ok(())
    }
}

/// Wire-mesh reflector parameters.
///
/// Invariants, checked by [`Self::validate`]: both dimensions positive, and the wire
/// thinner than the mesh spacing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshParameters {
    /// Mesh spacing (hole size), millimetres.
    pub mesh_spacing_mm: f64,

    /// Wire diameter, millimetres.
    pub wire_diameter_mm: f64,
}

impl MeshParameters {
    /// Creates a new builder for constructing `MeshParameters`.
    pub fn builder() -> MeshParametersBuilder {
        MeshParametersBuilder::default()
    }

    /// Checks the invariants listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.mesh_spacing_mm <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "mesh_spacing_mm".to_string(),
                value: self.mesh_spacing_mm,
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "wire_diameter_mm".to_string(),
                value: self.wire_diameter_mm,
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm >= self.mesh_spacing_mm {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "wire_diameter_mm".to_string(),
                value: self.wire_diameter_mm,
                reason: "must be less than mesh_spacing_mm".to_string(),
            });
        }

        Ok(())
    }
}

/// Builder for [`PhysicalAntennaConfig`]. `reflector` and `feed` are required.
#[derive(Default)]
pub struct PhysicalAntennaConfigBuilder {
    reflector: Option<ReflectorGeometry>,
    feed: Option<FeedParameters>,
    mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfigBuilder {
    pub fn reflector(mut self, reflector: ReflectorGeometry) -> Self {
        self.reflector = Some(reflector);
        self
    }

    pub fn feed(mut self, feed: FeedParameters) -> Self {
        self.feed = Some(feed);
        self
    }

    pub fn mesh(mut self, mesh: MeshParameters) -> Self {
        self.mesh = Some(mesh);
        self
    }

    pub fn build(self) -> Result<PhysicalAntennaConfig, String> {
        Ok(PhysicalAntennaConfig {
            reflector: self.reflector.ok_or("reflector is required")?,
            feed: self.feed.ok_or("feed is required")?,
            mesh: self.mesh,
        })
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

/// Builder for [`FeedParameters`]. `position` and `q_factor` are required;
/// `phase_center_offset_m` and `axial_defocus_m` default to `0`, `asymmetry_factor` to
/// `1.0` (symmetric).
#[derive(Default)]
pub struct FeedParametersBuilder {
    position: Option<(f64, f64, f64)>,
    q_factor: Option<f64>,
    phase_center_offset_m: Option<f64>,
    axial_defocus_m: Option<f64>,
    asymmetry_factor: Option<f64>,
}

impl FeedParametersBuilder {
    pub fn position(mut self, x: f64, y: f64, z: f64) -> Self {
        self.position = Some((x, y, z));
        self
    }

    pub fn q_factor(mut self, q: f64) -> Self {
        self.q_factor = Some(q);
        self
    }

    pub fn phase_center_offset_m(mut self, offset: f64) -> Self {
        self.phase_center_offset_m = Some(offset);
        self
    }

    pub fn axial_defocus_m(mut self, defocus: f64) -> Self {
        self.axial_defocus_m = Some(defocus);
        self
    }

    /// E/H illumination asymmetry; `1.0` (the default) is a symmetric feed.
    pub fn asymmetry_factor(mut self, factor: f64) -> Self {
        self.asymmetry_factor = Some(factor);
        self
    }

    pub fn build(self) -> Result<FeedParameters, String> {
        Ok(FeedParameters {
            position: self.position.ok_or("position is required")?,
            q_factor: self.q_factor.ok_or("q_factor is required")?,
            phase_center_offset_m: self.phase_center_offset_m.unwrap_or(0.0),
            axial_defocus_m: self.axial_defocus_m.unwrap_or(0.0),
            // Physically meaningful default, but an artifact producer must pass the
            // class's value through rather than accept it; each has a test pinning that.
            asymmetry_factor: self.asymmetry_factor.unwrap_or(1.0),
        })
    }
}

/// Builder for [`MeshParameters`]. Both fields are required.
#[derive(Default)]
pub struct MeshParametersBuilder {
    mesh_spacing_mm: Option<f64>,
    wire_diameter_mm: Option<f64>,
}

impl MeshParametersBuilder {
    pub fn mesh_spacing_mm(mut self, spacing: f64) -> Self {
        self.mesh_spacing_mm = Some(spacing);
        self
    }

    pub fn wire_diameter_mm(mut self, diameter: f64) -> Self {
        self.wire_diameter_mm = Some(diameter);
        self
    }

    pub fn build(self) -> Result<MeshParameters, String> {
        Ok(MeshParameters {
            mesh_spacing_mm: self.mesh_spacing_mm.ok_or("mesh_spacing_mm is required")?,
            wire_diameter_mm: self
                .wire_diameter_mm
                .ok_or("wire_diameter_mm is required")?,
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
