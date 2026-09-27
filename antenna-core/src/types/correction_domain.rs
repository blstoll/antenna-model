use super::ValidationError;

/// A closed box in the correction surface's query coordinates: E-clock and E-cone in
/// degrees, frequency in MHz. Both bounds of every axis are inclusive.
///
/// Two different claims are stated in this shape, and they must not be confused:
///
/// - **support** ([`crate::model::CorrectionSurfaceLayout::support`]) — where the spline
///   *can* be evaluated, a property of its knots;
/// - **coverage** ([`super::CalibrationCoverage::domain`]) — where measurements *justify*
///   applying the correction, an empirical claim.
///
/// The artifact invariant between them is containment, not equality. See #97.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorrectionDomain {
    pub e_clock_deg: (f64, f64),
    pub e_cone_deg: (f64, f64),
    pub frequency_mhz: (f64, f64),
}

impl CorrectionDomain {
    /// The axes under the artifact's wire names (`azimuth_range`/`elevation_range` are the
    /// E-clock and E-cone extents), so a validation error names the field a reader can find.
    fn named_axes(&self) -> [(&'static str, (f64, f64)); 3] {
        [
            ("azimuth (E-clock)", self.e_clock_deg),
            ("elevation (E-cone)", self.e_cone_deg),
            ("frequency", self.frequency_mhz),
        ]
    }

    /// Whether `inner` lies within `self` on every axis, bounds inclusive; the error names
    /// the first axis that escapes. A NaN bound in `inner` is never contained.
    pub fn check_contains(&self, inner: &CorrectionDomain) -> Result<(), ValidationError> {
        self.named_axes()
            .into_iter()
            .zip(inner.named_axes())
            // Written positively so that a NaN comparison fails containment.
            .find(|((_, outer), (_, inner))| !(inner.0 >= outer.0 && inner.1 <= outer.1))
            .map_or(Ok(()), |((dimension, support), (_, coverage))| {
                Err(ValidationError::CoverageExceedsSupport {
                    dimension: dimension.to_string(),
                    coverage,
                    support,
                })
            })
    }
}
