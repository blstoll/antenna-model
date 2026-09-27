use serde::{Deserialize, Serialize};

use super::ValidationError;
use crate::model::correction_surface::CorrectionDomain;

/// Half-angle, in degrees, of the on-axis cone that counts as boresight coverage.
///
/// Boresight is the pole of the (azimuth, polar-angle) system, where azimuth is
/// degenerate, so boresight coverage is a small polar cone with azimuth unconstrained —
/// never a point in az/el space. 0.01° sits two orders above the polar-angle noise of
/// ECEF-derived queries (~10⁻⁵°) and one order below the narrowest realistic HPBW for
/// these antennas (~0.1°), so "boresight-calibrated" stays a true claim.
pub const BORESIGHT_COVERAGE_CONE_DEG: f64 = 0.01;

/// Where measurements justify applying the correction surface.
///
/// Coverage is contained by, never required to equal, the surface's fitted support:
/// boresight coverage is deliberately narrower than its flat support. Containment is
/// checked when the artifact is validated. See #97.
///
/// `elevation` throughout is the E-cone polar angle off boresight, never horizon
/// elevation; all bounds are inclusive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibrationCoverage {
    /// Azimuth (E-clock) coverage, degrees `(min, max)`. `(0, 360)` for boresight
    /// coverage: azimuth is unconstrained at the pole.
    pub azimuth_range: (f64, f64),

    /// Elevation (E-cone) coverage, degrees `(min, max)`.
    /// `(0, BORESIGHT_COVERAGE_CONE_DEG)` for boresight coverage.
    pub elevation_range: (f64, f64),

    /// Frequency coverage, MHz `(min, max)`.
    pub frequency_range: (f64, f64),

    /// Total number of measurement points.
    pub num_measurements: usize,

    /// Whether a correction surface was fitted; `false` means only physical parameters
    /// were tuned.
    pub has_correction_surface: bool,
}

impl CalibrationCoverage {
    /// Creates a new builder for constructing `CalibrationCoverage`.
    pub fn builder() -> CalibrationCoverageBuilder {
        CalibrationCoverageBuilder::default()
    }

    /// Coverage over a measured domain. Full mode passes the same domain the surface's
    /// support is clamped to.
    pub fn from_domain(
        domain: CorrectionDomain,
        num_measurements: usize,
        has_correction_surface: bool,
    ) -> Self {
        Self {
            azimuth_range: domain.e_clock_deg,
            elevation_range: domain.e_cone_deg,
            frequency_range: domain.frequency_mhz,
            num_measurements,
            has_correction_surface,
        }
    }

    /// Boresight coverage: the [`BORESIGHT_COVERAGE_CONE_DEG`] cone, azimuth
    /// unconstrained, over the measured frequency span.
    pub fn boresight_cone(
        frequency_range: (f64, f64),
        num_measurements: usize,
        has_correction_surface: bool,
    ) -> Self {
        Self::from_domain(
            CorrectionDomain {
                e_clock_deg: (0.0, 360.0),
                e_cone_deg: (0.0, BORESIGHT_COVERAGE_CONE_DEG),
                frequency_mhz: frequency_range,
            },
            num_measurements,
            has_correction_surface,
        )
    }

    /// The covered region in the correction surface's query coordinates, for comparison
    /// with fitted support.
    pub fn domain(&self) -> CorrectionDomain {
        CorrectionDomain {
            e_clock_deg: self.azimuth_range,
            e_cone_deg: self.elevation_range,
            frequency_mhz: self.frequency_range,
        }
    }

    /// Whether this is boresight-only coverage: an on-axis elevation cone no wider than
    /// [`BORESIGHT_COVERAGE_CONE_DEG`].
    ///
    /// Azimuth is ignored because it carries no information at the pole. The legacy
    /// zero-width `(0,0)/(0,0)` encoding also counts.
    pub fn is_boresight_only(&self) -> bool {
        self.elevation_range.0 == 0.0 && self.elevation_range.1 <= BORESIGHT_COVERAGE_CONE_DEG
    }

    /// Whether a direction lies in the covered **spatial** region, ignoring frequency.
    ///
    /// Use this to decide whether a query left the measured region
    /// (`WarningCode::OutOfCoverage`); a covered direction at an uncalibrated frequency
    /// must not raise that warning. Whether a correction may be applied is
    /// [`Self::contains_direction_at_frequency`]. Shares its pole limitation.
    pub fn contains_direction(&self, azimuth: f64, elevation: f64) -> bool {
        azimuth >= self.azimuth_range.0
            && azimuth <= self.azimuth_range.1
            && elevation >= self.elevation_range.0
            && elevation <= self.elevation_range.1
    }

    /// Whether a query point — direction **and** frequency (MHz) — is covered.
    ///
    /// The authority for whether a correction surface may be applied; the served path
    /// delegates here rather than repeating the range test (#60).
    ///
    /// # Known limitation: azimuth at the pole
    ///
    /// At `elevation ≈ 0` azimuth comes from `atan2` on float noise, so any coverage
    /// whose elevation range includes 0 but whose azimuth range is constrained can
    /// reject an exact-boresight query. Boresight artifacts avoid this by declaring
    /// azimuth `(0, 360)`. The general fix — skip the azimuth clause below a pole
    /// threshold — is recorded, unapplied, in the D13 entry; apply it here and in
    /// [`Self::contains_direction`] together.
    pub fn contains_direction_at_frequency(
        &self,
        azimuth: f64,
        elevation: f64,
        frequency: f64,
    ) -> bool {
        self.contains_direction(azimuth, elevation)
            && frequency >= self.frequency_range.0
            && frequency <= self.frequency_range.1
    }

    /// Checks that every range has `min <= max`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.azimuth_range.0 > self.azimuth_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "azimuth".to_string(),
                min: self.azimuth_range.0,
                max: self.azimuth_range.1,
            });
        }

        if self.elevation_range.0 > self.elevation_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_range.0,
                max: self.elevation_range.1,
            });
        }

        if self.frequency_range.0 > self.frequency_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "frequency".to_string(),
                min: self.frequency_range.0,
                max: self.frequency_range.1,
            });
        }

        Ok(())
    }
}

/// Builder for [`CalibrationCoverage`]. The three ranges and `num_measurements` are
/// required; `has_correction_surface` defaults to `false`.
#[derive(Default)]
pub struct CalibrationCoverageBuilder {
    azimuth_range: Option<(f64, f64)>,
    elevation_range: Option<(f64, f64)>,
    frequency_range: Option<(f64, f64)>,
    num_measurements: Option<usize>,
    has_correction_surface: Option<bool>,
}

impl CalibrationCoverageBuilder {
    pub fn azimuth_range(mut self, min: f64, max: f64) -> Self {
        self.azimuth_range = Some((min, max));
        self
    }

    pub fn elevation_range(mut self, min: f64, max: f64) -> Self {
        self.elevation_range = Some((min, max));
        self
    }

    pub fn frequency_range(mut self, min: f64, max: f64) -> Self {
        self.frequency_range = Some((min, max));
        self
    }

    pub fn num_measurements(mut self, num: usize) -> Self {
        self.num_measurements = Some(num);
        self
    }

    pub fn has_correction_surface(mut self, has_surface: bool) -> Self {
        self.has_correction_surface = Some(has_surface);
        self
    }

    pub fn build(self) -> Result<CalibrationCoverage, String> {
        Ok(CalibrationCoverage {
            azimuth_range: self.azimuth_range.ok_or("azimuth_range is required")?,
            elevation_range: self.elevation_range.ok_or("elevation_range is required")?,
            frequency_range: self.frequency_range.ok_or("frequency_range is required")?,
            num_measurements: self
                .num_measurements
                .ok_or("num_measurements is required")?,
            has_correction_surface: self.has_correction_surface.unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibration_coverage_builder() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        assert_eq!(coverage.azimuth_range, (0.0, 360.0));
        assert_eq!(coverage.elevation_range, (0.0, 90.0));
        assert_eq!(coverage.frequency_range, (7100.0, 8500.0));
        assert_eq!(coverage.num_measurements, 100);
        assert!(coverage.has_correction_surface);
    }

    #[test]
    fn test_calibration_coverage_is_boresight_only() {
        let boresight = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 28,
            has_correction_surface: false,
        };
        assert!(boresight.is_boresight_only());

        let limited = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (30.0, 60.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        };
        assert!(!limited.is_boresight_only());
    }

    /// The boresight encoding: azimuth unconstrained, elevation an on-axis cone.
    #[test]
    fn boresight_cone_coverage_is_boresight_only() {
        let cone = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, BORESIGHT_COVERAGE_CONE_DEG),
            frequency_range: (3700.0, 6425.0),
            num_measurements: 6,
            has_correction_surface: true,
        };
        assert!(
            cone.is_boresight_only(),
            "an on-axis cone with unconstrained azimuth IS boresight-only coverage; \
             the API surfaces this as coverage.is_boresight_only"
        );

        // A query aimed exactly at boresight: azimuth is atan2 on float noise, so
        // it can be anything. It must still be in coverage.
        assert!(
            cone.contains_direction_at_frequency(63.43, 0.0, 4000.0),
            "boresight coverage must not reject a boresight query over its \
             meaningless azimuth"
        );

        // Just outside the cone is genuinely off-axis and must fall out.
        assert!(!cone.contains_direction_at_frequency(
            63.43,
            10.0 * BORESIGHT_COVERAGE_CONE_DEG,
            4000.0
        ));
    }

    /// The legacy `(0,0)/(0,0)` encoding still reports boresight-only, though a nonzero
    /// noise azimuth falls outside it.
    #[test]
    fn legacy_degenerate_boresight_coverage_still_reports_boresight_only() {
        let legacy = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 5,
            has_correction_surface: true,
        };
        assert!(legacy.is_boresight_only());
        assert!(!legacy.contains_direction_at_frequency(63.43, 0.0, 8000.0));
    }

    /// A full-mode grid reaching the on-axis point is not boresight-only.
    #[test]
    fn a_full_grid_reaching_boresight_is_not_boresight_only() {
        let grid = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 25.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 512,
            has_correction_surface: true,
        };
        assert!(!grid.is_boresight_only());
    }

    #[test]
    fn test_calibration_coverage_containment() {
        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (30.0, 60.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        };

        assert!(coverage.contains_direction_at_frequency(45.0, 45.0, 8000.0));
        assert!(!coverage.contains_direction_at_frequency(45.0, 20.0, 8000.0)); // E-cone too low
        assert!(!coverage.contains_direction_at_frequency(45.0, 45.0, 9000.0)); // frequency too high
    }

    /// A coverage box whose bounds are all strictly interior to their axes, so a
    /// probe can sit one step outside any single bound without leaving the others.
    fn bounded_coverage() -> CalibrationCoverage {
        CalibrationCoverage {
            azimuth_range: (10.0, 350.0),
            elevation_range: (5.0, 60.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        }
    }

    /// Pins the closed-interval convention of the served correction gate (#60).
    #[test]
    fn full_containment_bounds_are_inclusive() {
        let coverage = bounded_coverage();

        // Both extreme corners of the closed box are in coverage.
        assert!(coverage.contains_direction_at_frequency(10.0, 5.0, 7100.0));
        assert!(coverage.contains_direction_at_frequency(350.0, 60.0, 8500.0));

        // One step outside each bound, one axis at a time.
        assert!(!coverage.contains_direction_at_frequency(9.9, 30.0, 8000.0));
        assert!(!coverage.contains_direction_at_frequency(350.1, 30.0, 8000.0));
        assert!(!coverage.contains_direction_at_frequency(180.0, 4.9, 8000.0));
        assert!(!coverage.contains_direction_at_frequency(180.0, 60.1, 8000.0));
        assert!(!coverage.contains_direction_at_frequency(180.0, 30.0, 7099.9));
        assert!(!coverage.contains_direction_at_frequency(180.0, 30.0, 8500.1));
    }

    /// The spatial predicate is the same closed box with the frequency axis dropped.
    #[test]
    fn spatial_containment_bounds_are_inclusive() {
        let coverage = bounded_coverage();

        assert!(coverage.contains_direction(10.0, 5.0));
        assert!(coverage.contains_direction(350.0, 60.0));

        assert!(!coverage.contains_direction(9.9, 30.0));
        assert!(!coverage.contains_direction(350.1, 30.0));
        assert!(!coverage.contains_direction(180.0, 4.9));
        assert!(!coverage.contains_direction(180.0, 60.1));
    }

    /// Guards against the out-of-coverage advisory firing for a covered direction at an
    /// uncalibrated frequency (#60).
    #[test]
    fn spatial_containment_ignores_frequency() {
        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 30.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        };

        assert!(coverage.contains_direction(45.0, 15.0));
        assert!(!coverage.contains_direction_at_frequency(45.0, 15.0, 12000.0));
    }

    /// Pins that both predicates share the pole limitation, so a fix moves both (#60).
    #[test]
    fn both_predicates_share_the_pole_limitation() {
        let legacy = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 5,
            has_correction_surface: true,
        };

        assert!(!legacy.contains_direction(63.43, 0.0));
        assert!(!legacy.contains_direction_at_frequency(63.43, 0.0, 8000.0));
    }

    #[test]
    fn test_calibration_coverage_validate() {
        let valid_coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 90.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 100,
            has_correction_surface: true,
        };
        assert!(valid_coverage.validate().is_ok());

        let invalid_coverage = CalibrationCoverage {
            azimuth_range: (360.0, 0.0), // Invalid: min > max
            elevation_range: (0.0, 90.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 100,
            has_correction_surface: true,
        };
        assert!(invalid_coverage.validate().is_err());
    }
}
