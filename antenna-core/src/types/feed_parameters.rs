use serde::{Deserialize, Serialize};

use super::ValidationError;

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
    /// Producers must write the antenna class's design value rather than accept
    /// [`FeedParametersBuilder`]'s symmetric default: a mismatch serves a different
    /// illumination than the correction surface was fitted against — up to 1.20 dB
    /// off-axis while invisible at boresight. See D23.
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
