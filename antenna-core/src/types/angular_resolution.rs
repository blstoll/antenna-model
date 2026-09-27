use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Knots per lobe period below which a correction surface cannot follow the pattern's
/// lobe structure on an axis.
///
/// Derived, not fitted: representing a periodic feature needs at least two degrees of
/// freedom per period (Nyquist), and a B-spline's are placed by its knots. It is a
/// representability bound, not an accuracy target.
pub const MIN_KNOTS_PER_LOBE_PERIOD: f64 = 2.0;

/// How finely a fitted correction surface can vary in angle, against how finely the
/// antenna's own pattern does. See D21.
///
/// Recorded because in-sample RMSE cannot show this limitation: a surface whose knots
/// are coarser than the `λ/D` lobe period reproduces its own grid well while smoothing
/// away lobe-scale residual structure. Background:
/// `docs/findings-2026-08-02-correction-surface-angular-resolution.md`.
///
/// Spacings are the **widest gap between consecutive distinct knots actually placed**,
/// not the requested minimum. Lobe periods are the worst case in coverage: the highest
/// calibrated frequency and, for clock, the outermost `|cone|` angle.
///
/// Invariants, checked by [`Self::validate`]: both spacings and the cone period are
/// finite and positive; the clock period is positive and may be `INFINITY`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AngularResolution {
    /// Widest gap between consecutive distinct cone (polar) knots, degrees.
    pub cone_knot_spacing_deg: f64,

    /// Cone-angle lobe period at the highest calibrated frequency, degrees: `λ/D`.
    pub cone_lobe_period_deg: f64,

    /// Widest gap between consecutive distinct clock (azimuthal) knots, degrees.
    pub clock_knot_spacing_deg: f64,

    /// Clock-angle lobe period at the highest calibrated frequency and outermost
    /// calibrated `|cone|` angle `θ`, degrees: `(λ/D) / sin θ`.
    ///
    /// `f64::INFINITY` means there is no clock structure to resolve (`sin θ → 0`) and
    /// reads as fully resolved. No other field uses infinity, so no ratio is `INF/INF`.
    pub clock_lobe_period_deg: f64,
}

/// `period / spacing`, or `0.0` ("resolves nothing") when the spacing cannot be a
/// spacing.
///
/// A deserialized `0.0` spacing would otherwise give `finite / 0.0 = INFINITY` — the
/// best verdict from the worst input (D26 finding 5).
fn knots_per_lobe_period(period_deg: f64, spacing_deg: f64) -> f64 {
    if !(spacing_deg.is_finite() && spacing_deg > 0.0) || period_deg.is_nan() || period_deg < 0.0 {
        return 0.0;
    }
    // An infinite period over a finite spacing is INFINITY: correctly "fully resolved".
    period_deg / spacing_deg
}

impl AngularResolution {
    /// Knots per lobe period on the cone axis. Below [`MIN_KNOTS_PER_LOBE_PERIOD`] the
    /// surface carries the residual's envelope trend, not its lobe structure.
    pub fn cone_knots_per_lobe_period(&self) -> f64 {
        knots_per_lobe_period(self.cone_lobe_period_deg, self.cone_knot_spacing_deg)
    }

    /// Knots per lobe period on the clock axis, at the outermost calibrated cone angle.
    pub fn clock_knots_per_lobe_period(&self) -> f64 {
        knots_per_lobe_period(self.clock_lobe_period_deg, self.clock_knot_spacing_deg)
    }

    /// Checks the invariants listed on the type, so a deserialized assessment that
    /// cannot be interpreted is refused at load rather than reported on.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (name, spacing) in [
            ("cone_knot_spacing_deg", self.cone_knot_spacing_deg),
            ("clock_knot_spacing_deg", self.clock_knot_spacing_deg),
        ] {
            if !(spacing.is_finite() && spacing > 0.0) {
                return Err(ValidationError::InvalidAngularResolution {
                    field: name.to_string(),
                    value: spacing,
                    reason: "knot spacing must be finite and positive".to_string(),
                });
            }
        }
        if !(self.cone_lobe_period_deg.is_finite() && self.cone_lobe_period_deg > 0.0) {
            return Err(ValidationError::InvalidAngularResolution {
                field: "cone_lobe_period_deg".to_string(),
                value: self.cone_lobe_period_deg,
                reason: "lobe period must be finite and positive".to_string(),
            });
        }
        // Infinity is legal here, so the `is_finite() && > 0.0` form above cannot be
        // used; the explicit NaN test keeps visible what is being excluded.
        if self.clock_lobe_period_deg.is_nan() || self.clock_lobe_period_deg <= 0.0 {
            return Err(ValidationError::InvalidAngularResolution {
                field: "clock_lobe_period_deg".to_string(),
                value: self.clock_lobe_period_deg,
                reason: "lobe period must be positive (infinite is legal: no clock \
                         structure on axis)"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Whether **both** angular axes clear [`MIN_KNOTS_PER_LOBE_PERIOD`].
    ///
    /// A surface that does not is still useful and still written; it just cannot
    /// follow lobe-scale residual structure off the main beam.
    pub fn resolves_lobe_structure(&self) -> bool {
        self.cone_knots_per_lobe_period() >= MIN_KNOTS_PER_LOBE_PERIOD
            && self.clock_knots_per_lobe_period() >= MIN_KNOTS_PER_LOBE_PERIOD
    }

    /// One line naming both axes and both ratios, for logs and reports.
    pub fn summary(&self) -> String {
        format!(
            "cone {:.2}° knots vs {:.2}° lobe period ({:.2} knots/period); \
             clock {:.2}° knots vs {:.2}° lobe period ({:.2} knots/period); \
             minimum {MIN_KNOTS_PER_LOBE_PERIOD:.1}",
            self.cone_knot_spacing_deg,
            self.cone_lobe_period_deg,
            self.cone_knots_per_lobe_period(),
            self.clock_knot_spacing_deg,
            self.clock_lobe_period_deg,
            self.clock_knots_per_lobe_period(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against `resolves_lobe_structure` consulting only one axis (D21).
    #[test]
    fn resolution_is_bounded_by_the_worse_of_the_two_angular_axes() {
        let comfortable = AngularResolution {
            cone_knot_spacing_deg: 0.5,
            cone_lobe_period_deg: 5.0,
            clock_knot_spacing_deg: 2.0,
            clock_lobe_period_deg: 20.0,
        };
        assert_eq!(comfortable.cone_knots_per_lobe_period(), 10.0);
        assert_eq!(comfortable.clock_knots_per_lobe_period(), 10.0);
        assert!(comfortable.resolves_lobe_structure());

        let cone_starved = AngularResolution {
            cone_knot_spacing_deg: 4.0,
            ..comfortable.clone()
        };
        assert_eq!(cone_starved.cone_knots_per_lobe_period(), 1.25);
        assert!(
            cone_starved.clock_knots_per_lobe_period() >= MIN_KNOTS_PER_LOBE_PERIOD,
            "the clock axis must still pass, or this proves nothing about which axis bound it"
        );
        assert!(!cone_starved.resolves_lobe_structure());

        let clock_starved = AngularResolution {
            clock_knot_spacing_deg: 40.0,
            ..comfortable.clone()
        };
        assert!(
            clock_starved.cone_knots_per_lobe_period() >= MIN_KNOTS_PER_LOBE_PERIOD,
            "the cone axis must still pass, or this proves nothing about which axis bound it"
        );
        assert!(!clock_starved.resolves_lobe_structure());
    }

    /// Guards against the threshold comparison silently inverting (D21).
    #[test]
    fn the_minimum_knots_per_lobe_period_is_inclusive() {
        let at_bound = AngularResolution {
            cone_knot_spacing_deg: 1.0,
            cone_lobe_period_deg: MIN_KNOTS_PER_LOBE_PERIOD,
            clock_knot_spacing_deg: 1.0,
            clock_lobe_period_deg: MIN_KNOTS_PER_LOBE_PERIOD,
        };
        assert!(at_bound.resolves_lobe_structure());

        let just_under = AngularResolution {
            cone_lobe_period_deg: MIN_KNOTS_PER_LOBE_PERIOD * 0.999,
            ..at_bound
        };
        assert!(!just_under.resolves_lobe_structure());
    }

    /// Guards against an infinite (on-axis) clock period reading as NaN or a failure (D21).
    #[test]
    fn a_degenerate_clock_axis_reads_as_resolved_not_as_a_failure() {
        let on_axis = AngularResolution {
            cone_knot_spacing_deg: 0.1,
            cone_lobe_period_deg: 1.0,
            clock_knot_spacing_deg: 90.0,
            clock_lobe_period_deg: f64::INFINITY,
        };
        assert!(on_axis.clock_knots_per_lobe_period().is_infinite());
        assert!(on_axis.resolves_lobe_structure());
        assert!(
            on_axis.summary().contains("inf"),
            "the summary must not hide a degenerate axis: {}",
            on_axis.summary()
        );
    }

    /// Guards against a zero or non-finite deserialized spacing reading as infinitely
    /// resolved (D26 finding 5).
    #[test]
    fn a_zero_or_non_finite_knot_spacing_resolves_nothing() {
        for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let cone_bad = AngularResolution {
                cone_knot_spacing_deg: bad,
                cone_lobe_period_deg: 5.0,
                clock_knot_spacing_deg: 2.0,
                clock_lobe_period_deg: 20.0,
            };
            assert_eq!(
                cone_bad.cone_knots_per_lobe_period(),
                0.0,
                "cone spacing {bad} must yield no resolution, not infinite resolution"
            );
            assert!(
                !cone_bad.resolves_lobe_structure(),
                "cone spacing {bad} must not read as resolved"
            );
            assert!(
                cone_bad.validate().is_err(),
                "cone spacing {bad} must be refused by validate()"
            );

            let clock_bad = AngularResolution {
                cone_knot_spacing_deg: 0.5,
                clock_knot_spacing_deg: bad,
                ..cone_bad.clone()
            };
            assert_eq!(clock_bad.clock_knots_per_lobe_period(), 0.0);
            assert!(!clock_bad.resolves_lobe_structure());
            assert!(clock_bad.validate().is_err());
        }
    }

    /// Guards against `INF/INF = NaN` reaching the verdict, `summary()` or `PartialEq` (D26).
    #[test]
    fn no_input_produces_a_nan_verdict() {
        let both_degenerate = AngularResolution {
            cone_knot_spacing_deg: f64::INFINITY,
            cone_lobe_period_deg: f64::INFINITY,
            clock_knot_spacing_deg: f64::INFINITY,
            clock_lobe_period_deg: f64::INFINITY,
        };
        assert!(!both_degenerate.cone_knots_per_lobe_period().is_nan());
        assert!(!both_degenerate.clock_knots_per_lobe_period().is_nan());
        assert!(!both_degenerate.resolves_lobe_structure());
        assert!(both_degenerate.validate().is_err());
        // The struct still compares equal to itself, which a NaN field would not.
        assert_eq!(both_degenerate, both_degenerate.clone());
    }
}
