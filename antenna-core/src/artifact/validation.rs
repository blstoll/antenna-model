//! The artifact invariants. Private: reached only through the builder and the loader.

use crate::model::geometry::{F_OVER_D_MAX, F_OVER_D_MIN};
use crate::model::CoveredCorrectionSurface;
use crate::types::{
    AngularResolution, AntennaCalibration, CalibrationCoverage, FeedParameters, MeshParameters,
    PhysicalAntennaConfig, ReflectorGeometry, ValidationError, ValidityRanges,
};

type Checked = Result<(), ValidationError>;

/// Largest `cos^q` feed-illumination exponent an artifact may carry; real feeds sit at 6–12.
const Q_FACTOR_MAX: f64 = 20.0;

/// Checks every artifact invariant, failing on the first one broken.
pub(super) fn validate(calibration: &AntennaCalibration) -> Checked {
    non_empty("antenna_id", &calibration.antenna_id)?;
    non_empty("feed_id", &calibration.feed_id)?;
    physical_config(&calibration.physical_config)?;
    validity_ranges(&calibration.validity_ranges)?;
    calibration
        .calibration_coverage
        .as_ref()
        .map_or(Ok(()), coverage_ranges)?;
    // The single coverage claim holds with or without a surface.
    calibration.coverage()?;
    // The knot-layout rules and coverage ⊆ support.
    CoveredCorrectionSurface::from_artifact(calibration)?;
    calibration
        .metadata
        .angular_resolution
        .as_ref()
        .map_or(Ok(()), angular_resolution)
}

fn non_empty(field: &str, value: &str) -> Checked {
    if value.is_empty() {
        return Err(ValidationError::EmptyField(field.to_string()));
    }
    Ok(())
}

fn invalid_parameter(parameter: &str, value: f64, reason: impl Into<String>) -> ValidationError {
    ValidationError::InvalidPhysicalParameter {
        parameter: parameter.to_string(),
        value,
        reason: reason.into(),
    }
}

fn physical_config(config: &PhysicalAntennaConfig) -> Checked {
    reflector(&config.reflector)?;
    feed(&config.feed)?;
    config.mesh.as_ref().map_or(Ok(()), mesh)
}

/// Positive diameter and focal length; `f_over_d_ratio` within the model's supported range
/// and within 1% of `focal_length_m / diameter_m`; non-negative surface RMS.
fn reflector(reflector: &ReflectorGeometry) -> Checked {
    if reflector.diameter_m <= 0.0 {
        return Err(invalid_parameter(
            "diameter_m",
            reflector.diameter_m,
            "must be positive",
        ));
    }
    if reflector.focal_length_m <= 0.0 {
        return Err(invalid_parameter(
            "focal_length_m",
            reflector.focal_length_m,
            "must be positive",
        ));
    }
    if !(F_OVER_D_MIN..=F_OVER_D_MAX).contains(&reflector.f_over_d_ratio) {
        return Err(invalid_parameter(
            "f_over_d_ratio",
            reflector.f_over_d_ratio,
            format!("must be between {F_OVER_D_MIN} and {F_OVER_D_MAX}"),
        ));
    }
    let implied_f_over_d = reflector.focal_length_m / reflector.diameter_m;
    if (reflector.f_over_d_ratio - implied_f_over_d).abs() > 0.01 * implied_f_over_d {
        return Err(invalid_parameter(
            "f_over_d_ratio",
            reflector.f_over_d_ratio,
            format!("inconsistent with focal_length_m/diameter_m = {implied_f_over_d:.4}"),
        ));
    }
    if reflector.surface_rms_mm < 0.0 {
        return Err(invalid_parameter(
            "surface_rms_mm",
            reflector.surface_rms_mm,
            "must be non-negative",
        ));
    }
    Ok(())
}

/// `q_factor` in `[0, Q_FACTOR_MAX]`; `asymmetry_factor` finite and positive.
fn feed(feed: &FeedParameters) -> Checked {
    if feed.q_factor < 0.0 || feed.q_factor > Q_FACTOR_MAX {
        return Err(invalid_parameter(
            "q_factor",
            feed.q_factor,
            format!("must be between 0 and {Q_FACTOR_MAX}"),
        ));
    }
    // Mirrors `model::geometry::FeedParameters::validate`, but fails at construction
    // instead of when a request first reaches the integrator.
    if !(feed.asymmetry_factor.is_finite() && feed.asymmetry_factor > 0.0) {
        return Err(invalid_parameter(
            "asymmetry_factor",
            feed.asymmetry_factor,
            "must be positive (1.0 is a symmetric feed)",
        ));
    }
    Ok(())
}

/// Both dimensions positive, and the wire thinner than the mesh spacing.
fn mesh(mesh: &MeshParameters) -> Checked {
    if mesh.mesh_spacing_mm <= 0.0 {
        return Err(invalid_parameter(
            "mesh_spacing_mm",
            mesh.mesh_spacing_mm,
            "must be positive",
        ));
    }
    if mesh.wire_diameter_mm <= 0.0 {
        return Err(invalid_parameter(
            "wire_diameter_mm",
            mesh.wire_diameter_mm,
            "must be positive",
        ));
    }
    if mesh.wire_diameter_mm >= mesh.mesh_spacing_mm {
        return Err(invalid_parameter(
            "wire_diameter_mm",
            mesh.wire_diameter_mm,
            "must be less than mesh_spacing_mm",
        ));
    }
    Ok(())
}

fn ordered(dimension: &str, (min, max): (f64, f64)) -> Checked {
    if min > max {
        return Err(ValidationError::InvalidRange {
            dimension: dimension.to_string(),
            min,
            max,
        });
    }
    Ok(())
}

/// Every range ordered; elevation (a polar angle off boresight) within `[0, 90]`;
/// temperature positive.
fn validity_ranges(ranges: &ValidityRanges) -> Checked {
    ordered("azimuth", ranges.azimuth_min_max)?;
    ordered("elevation", ranges.elevation_min_max)?;
    ordered("frequency", ranges.frequency_min_max)?;
    let (elevation_min, elevation_max) = ranges.elevation_min_max;
    if elevation_min < 0.0 || elevation_max > 90.0 {
        return Err(ValidationError::InvalidRange {
            dimension: "elevation".to_string(),
            min: elevation_min,
            max: elevation_max,
        });
    }
    if ranges.temperature_const <= 0.0 {
        return Err(ValidationError::InvalidTemperature(
            ranges.temperature_const,
        ));
    }
    Ok(())
}

fn coverage_ranges(coverage: &CalibrationCoverage) -> Checked {
    ordered("azimuth", coverage.azimuth_range)?;
    ordered("elevation", coverage.elevation_range)?;
    ordered("frequency", coverage.frequency_range)
}

/// Both spacings and the cone period finite and positive; the clock period positive and
/// possibly `INFINITY`. Refuses a recorded assessment that cannot be interpreted rather
/// than reporting on it (D26 finding 5).
fn angular_resolution(resolution: &AngularResolution) -> Checked {
    let invalid = |field: &str, value: f64, reason: &str| {
        Err(ValidationError::InvalidAngularResolution {
            field: field.to_string(),
            value,
            reason: reason.to_string(),
        })
    };
    for (field, spacing) in [
        ("cone_knot_spacing_deg", resolution.cone_knot_spacing_deg),
        ("clock_knot_spacing_deg", resolution.clock_knot_spacing_deg),
    ] {
        if !(spacing.is_finite() && spacing > 0.0) {
            return invalid(field, spacing, "knot spacing must be finite and positive");
        }
    }
    let cone_period = resolution.cone_lobe_period_deg;
    if !(cone_period.is_finite() && cone_period > 0.0) {
        return invalid(
            "cone_lobe_period_deg",
            cone_period,
            "lobe period must be finite and positive",
        );
    }
    // Infinity is legal here, so the `is_finite() && > 0.0` form cannot be used; the
    // explicit NaN test keeps visible what is excluded.
    let clock_period = resolution.clock_lobe_period_deg;
    if clock_period.is_nan() || clock_period <= 0.0 {
        return invalid(
            "clock_lobe_period_deg",
            clock_period,
            "lobe period must be positive (infinite is legal: no clock structure on axis)",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::types::fixtures::{self, DOMAIN};
    use crate::types::{
        AngularResolution, AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D,
        CalibrationCoverage, CalibrationMetadata, CalibrationStatus, CorrectionDomain,
        MeshParameters, PhysicalAntennaConfig, ValidationError, ValidityRanges,
    };

    fn rejection(builder: AntennaCalibrationBuilder) -> ValidationError {
        builder
            .build()
            .expect_err("the builder must refuse this artifact")
    }

    fn with_physical(edit: impl FnOnce(&mut PhysicalAntennaConfig)) -> AntennaCalibrationBuilder {
        let mut config = fixtures::physical_config();
        edit(&mut config);
        fixtures::builder().physical_config(config)
    }

    fn with_ranges(edit: impl FnOnce(&mut ValidityRanges)) -> AntennaCalibrationBuilder {
        let mut ranges = fixtures::validity_ranges();
        edit(&mut ranges);
        fixtures::builder().validity_ranges(ranges)
    }

    fn rejected_parameter(builder: AntennaCalibrationBuilder) -> String {
        match rejection(builder) {
            ValidationError::InvalidPhysicalParameter { parameter, .. } => parameter,
            other => panic!("expected InvalidPhysicalParameter, got {other:?}"),
        }
    }

    #[test]
    fn the_fixtures_build() {
        fixtures::builder().build().unwrap();
        fixtures::calibrated().build().unwrap();
    }

    #[test]
    fn build_names_the_first_missing_required_field() {
        assert_eq!(
            rejection(AntennaCalibration::builder()),
            ValidationError::MissingField("antenna_id")
        );
        let message = rejection(
            AntennaCalibration::builder()
                .antenna_id("a")
                .feed_id("f")
                .metadata(fixtures::metadata())
                .physical_config(fixtures::physical_config()),
        )
        .to_string();
        assert_eq!(message, "validity_ranges is required");
    }

    #[test]
    fn empty_ids_are_rejected() {
        assert_eq!(
            rejection(fixtures::builder().antenna_id("")),
            ValidationError::EmptyField("antenna_id".to_string())
        );
        assert_eq!(
            rejection(fixtures::builder().feed_id("")),
            ValidationError::EmptyField("feed_id".to_string())
        );
    }

    #[test]
    fn reflector_f_over_d_must_be_consistent_and_in_range() {
        let inconsistent = with_physical(|c| c.reflector.f_over_d_ratio = 0.5);
        assert_eq!(rejected_parameter(inconsistent), "f_over_d_ratio");

        for (focal_length_m, f_over_d_ratio) in [(51.0, 1.5), (3.4, 0.1)] {
            let out_of_range = with_physical(|c| {
                c.reflector.focal_length_m = focal_length_m;
                c.reflector.f_over_d_ratio = f_over_d_ratio;
            });
            assert_eq!(rejected_parameter(out_of_range), "f_over_d_ratio");
        }
    }

    #[test]
    fn reflector_dimensions_must_be_physical() {
        assert_eq!(
            rejected_parameter(with_physical(|c| c.reflector.diameter_m = 0.0)),
            "diameter_m"
        );
        assert_eq!(
            rejected_parameter(with_physical(|c| c.reflector.surface_rms_mm = -0.1)),
            "surface_rms_mm"
        );
    }

    #[test]
    fn feed_parameters_must_be_in_domain() {
        assert_eq!(
            rejected_parameter(with_physical(
                |c| c.feed.q_factor = super::Q_FACTOR_MAX + 1.0
            )),
            "q_factor"
        );
        for asymmetry in [0.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                rejected_parameter(with_physical(|c| c.feed.asymmetry_factor = asymmetry)),
                "asymmetry_factor"
            );
        }
    }

    #[test]
    fn mesh_wire_must_be_thinner_than_its_spacing() {
        let mesh = |mesh_spacing_mm, wire_diameter_mm| {
            with_physical(move |c| {
                c.mesh = Some(MeshParameters {
                    mesh_spacing_mm,
                    wire_diameter_mm,
                })
            })
        };
        mesh(5.0, 0.5).build().expect("a valid mesh builds");
        assert_eq!(rejected_parameter(mesh(0.0, 0.5)), "mesh_spacing_mm");
        assert_eq!(rejected_parameter(mesh(5.0, 5.0)), "wire_diameter_mm");
    }

    #[test]
    fn validity_ranges_must_be_ordered_and_physical() {
        let dimension = |builder| match rejection(builder) {
            ValidationError::InvalidRange { dimension, .. } => dimension,
            other => panic!("expected InvalidRange, got {other:?}"),
        };
        assert_eq!(
            dimension(with_ranges(|r| r.azimuth_min_max = (360.0, 0.0))),
            "azimuth"
        );
        assert_eq!(
            dimension(with_ranges(|r| r.elevation_min_max = (-10.0, 90.0))),
            "elevation"
        );
        assert_eq!(
            rejection(with_ranges(|r| r.temperature_const = -10.0)),
            ValidationError::InvalidTemperature(-10.0)
        );
    }

    #[test]
    fn coverage_ranges_must_be_ordered() {
        let mut coverage = CalibrationCoverage::from_domain(DOMAIN, 10, false);
        coverage.azimuth_range = (360.0, 0.0);
        assert!(matches!(
            rejection(fixtures::builder().calibration_coverage(coverage)),
            ValidationError::InvalidRange { ref dimension, .. } if dimension == "azimuth"
        ));
    }

    /// Guards against an uninterpretable angular-resolution assessment building (D26).
    #[test]
    fn an_uninterpretable_angular_resolution_is_refused() {
        let with_resolution = |resolution| {
            fixtures::builder().metadata(CalibrationMetadata {
                angular_resolution: Some(resolution),
                ..fixtures::metadata()
            })
        };
        let well_formed = AngularResolution {
            cone_knot_spacing_deg: 2.0,
            cone_lobe_period_deg: 1.16,
            clock_knot_spacing_deg: 40.0,
            // The legal infinite clock period: no clock structure on axis.
            clock_lobe_period_deg: f64::INFINITY,
        };
        with_resolution(well_formed.clone())
            .build()
            .expect("a well-formed assessment builds");

        let refused_field = |resolution| match rejection(with_resolution(resolution)) {
            ValidationError::InvalidAngularResolution { field, .. } => field,
            other => panic!("expected InvalidAngularResolution, got {other:?}"),
        };
        for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                refused_field(AngularResolution {
                    cone_knot_spacing_deg: bad,
                    ..well_formed.clone()
                }),
                "cone_knot_spacing_deg"
            );
            assert_eq!(
                refused_field(AngularResolution {
                    clock_knot_spacing_deg: bad,
                    ..well_formed.clone()
                }),
                "clock_knot_spacing_deg"
            );
        }
        assert_eq!(
            refused_field(AngularResolution {
                cone_lobe_period_deg: f64::INFINITY,
                ..well_formed.clone()
            }),
            "cone_lobe_period_deg"
        );
        assert_eq!(
            refused_field(AngularResolution {
                clock_lobe_period_deg: f64::NAN,
                ..well_formed
            }),
            "clock_lobe_period_deg"
        );
    }

    /// The knot-layout rules hold for a surface arriving through the builder (#95).
    mod correction_surface_layout {
        use super::*;

        fn with_surface(edit: impl FnOnce(&mut BSplineModel4D)) -> AntennaCalibrationBuilder {
            let mut surface = fixtures::surface_over(DOMAIN, 0.5);
            edit(&mut surface);
            fixtures::calibrated().correction_surface(surface)
        }

        #[test]
        fn coefficient_count_must_match_shape() {
            assert_eq!(
                rejection(with_surface(|s| {
                    s.coefficients.pop();
                })),
                ValidationError::InconsistentShape {
                    expected: 8,
                    actual: 7
                }
            );
        }

        #[test]
        fn knot_vectors_must_fit_the_shape_and_not_decrease() {
            let dimension = |builder| match rejection(builder) {
                ValidationError::InvalidKnotVector { dimension, reason } => (dimension, reason),
                other => panic!("expected InvalidKnotVector, got {other:?}"),
            };
            assert_eq!(
                dimension(with_surface(|s| s.knots_azimuth = vec![0.0, 360.0])).0,
                "azimuth"
            );
            let (axis, reason) = dimension(with_surface(|s| {
                s.knots_elevation = vec![0.0, 30.0, 0.0, 30.0]
            }));
            assert_eq!(axis, "elevation");
            assert!(reason.contains("non-decreasing"), "{reason}");
        }

        #[test]
        fn spline_order_must_be_in_range() {
            assert_eq!(
                rejection(with_surface(|s| s.spline_order = 0)),
                ValidationError::InvalidSplineOrder(0)
            );
        }

        #[test]
        fn temperature_slabs_must_be_identical() {
            let surface = BSplineModel4D {
                coefficients: [vec![0.5; 8], vec![0.6; 8]].concat(),
                shape: [2, 2, 2, 2],
                knots_temperature: vec![280.0, 280.0, 300.0, 300.0],
                ..fixtures::surface_over(DOMAIN, 0.5)
            };
            assert!(matches!(
                rejection(fixtures::calibrated().correction_surface(surface)),
                ValidationError::TemperatureDependentCorrection {
                    temperature_slab: 1,
                    ..
                }
            ));
        }
    }

    /// Coverage must lie within, not equal, the surface's fitted support (#97).
    mod coverage_containment {
        use super::*;

        fn full_mode(coverage: CorrectionDomain) -> AntennaCalibrationBuilder {
            fixtures::calibrated()
                .calibration_coverage(CalibrationCoverage::from_domain(coverage, 100, true))
        }

        fn assert_exceeds(builder: AntennaCalibrationBuilder, axis: &str) {
            match builder.build() {
                Err(ValidationError::CoverageExceedsSupport { ref dimension, .. })
                    if dimension == axis => {}
                other => panic!("expected coverage to exceed support on {axis}, got {other:?}"),
            }
        }

        #[test]
        fn full_mode_coverage_equal_to_support_builds() {
            let calibration = full_mode(DOMAIN).build().unwrap();
            let covered = crate::model::CoveredCorrectionSurface::from_artifact(&calibration)
                .unwrap()
                .expect("a surface is present");
            assert_eq!(covered.surface().layout().support(), DOMAIN);
            assert_eq!(covered.coverage().domain(), DOMAIN);
        }

        #[test]
        fn boresight_coverage_is_a_strict_subset_of_its_flat_support() {
            let support = CorrectionDomain {
                e_clock_deg: (0.0, 360.0),
                e_cone_deg: (0.0, 180.0),
                frequency_mhz: (7_100.0, 8_500.0),
            };
            let coverage = CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, true);
            assert!(coverage.is_boresight_only());
            assert_ne!(
                coverage.domain(),
                support,
                "boresight coverage is deliberately narrower"
            );

            let calibration = fixtures::builder()
                .correction_surface(fixtures::surface_over(support, 0.5))
                .partially_calibrated(1.5, coverage.clone())
                .build()
                .unwrap();
            assert_eq!(calibration.coverage(), Ok(Some(&coverage)));
        }

        /// Guards against containment going exclusive at a support bound (#97).
        #[test]
        fn containment_is_inclusive_at_every_support_bound() {
            type AxisOf = fn(&mut CorrectionDomain) -> &mut (f64, f64);
            let axes: [(&str, AxisOf); 3] = [
                ("azimuth (E-clock)", |d| &mut d.e_clock_deg),
                ("elevation (E-cone)", |d| &mut d.e_cone_deg),
                ("frequency", |d| &mut d.frequency_mhz),
            ];
            for (name, axis) in axes {
                let mut below = DOMAIN;
                axis(&mut below).0 = axis(&mut below).0.next_down();
                assert_exceeds(full_mode(below), name);

                let mut above = DOMAIN;
                axis(&mut above).1 = axis(&mut above).1.next_up();
                assert_exceeds(full_mode(above), name);
            }
        }

        #[test]
        fn non_finite_coverage_is_not_contained() {
            let mut coverage = DOMAIN;
            coverage.frequency_mhz.1 = f64::NAN;
            assert_exceeds(full_mode(coverage), "frequency");
        }

        #[test]
        fn a_correction_surface_without_coverage_is_rejected() {
            let error = rejection(
                fixtures::builder()
                    .correction_surface(fixtures::surface_over(DOMAIN, 0.5))
                    .calibration_status(CalibrationStatus::FullyCalibrated {
                        accuracy_estimate_db: 0.5,
                    }),
            );
            assert_eq!(error, ValidationError::MissingCoverage);
            let message = error.to_string();
            assert!(
                message.contains("calibration_coverage") && message.contains("correction_surface"),
                "the error must name the fields involved: {message}"
            );
        }

        #[test]
        fn coverage_is_not_required_without_a_correction_surface() {
            let calibration = fixtures::builder()
                .calibration_status(CalibrationStatus::Uncalibrated {
                    accuracy_estimate_db: 3.0,
                    loss_accuracy_estimate_db: 2.0,
                })
                .build()
                .unwrap();
            assert_eq!(
                crate::model::CoveredCorrectionSurface::from_artifact(&calibration),
                Ok(None)
            );
        }

        #[test]
        fn the_exceeds_error_names_the_axis_and_both_intervals() {
            let mut coverage = DOMAIN;
            coverage.e_cone_deg.1 = 45.0;
            let message = rejection(full_mode(coverage)).to_string();
            for expected in ["elevation (E-cone)", "45", "30", "calibration_coverage"] {
                assert!(
                    message.contains(expected),
                    "{expected:?} missing from: {message}"
                );
            }
        }

        #[test]
        fn disagreeing_duplicate_partial_coverage_is_rejected() {
            let status_coverage =
                CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, false);
            let partially = |coverage| CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db: 1.5,
                coverage,
            };

            assert_eq!(
                rejection(
                    fixtures::builder()
                        .calibration_status(partially(status_coverage.clone()))
                        .calibration_coverage(CalibrationCoverage::from_domain(DOMAIN, 28, false))
                ),
                ValidationError::CoverageRecordsDisagree
            );

            let error =
                rejection(fixtures::builder().calibration_status(partially(status_coverage)));
            assert_eq!(error, ValidationError::CoverageRecordsDisagree);
            assert!(error.to_string().contains("calibration_status"));
        }
    }
}
