use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Wire representation of a correction surface.
///
/// The field names are historical: `azimuth` is E-clock and `elevation` is E-cone.
/// Temperature is a fourth serialized axis kept for byte compatibility; every
/// temperature coefficient slab must be identical. Adapt it once into the executable
/// three-axis [`crate::model::FittedCorrectionSurface`] rather than evaluating it
/// directly.
///
/// Invariants, checked by [`Self::validate`]: the coefficient count matches `shape`, and
/// every axis satisfies the knot-vector invariant set owned by
/// [`crate::model::correction_surface`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BSplineModel4D {
    /// Flattened coefficients, indexed
    /// `i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp))`.
    pub coefficients: Vec<f64>,

    /// Coefficient array shape: `[n_azimuth, n_elevation, n_frequency, n_temperature]`.
    pub shape: [usize; 4],

    /// Knot vector for the azimuth (E-clock) axis, degrees.
    pub knots_azimuth: Vec<f64>,

    /// Knot vector for the elevation (E-cone) axis, degrees.
    pub knots_elevation: Vec<f64>,

    /// Knot vector for the frequency axis, MHz.
    pub knots_frequency: Vec<f64>,

    /// Knot vector for the temperature axis, kelvin.
    pub knots_temperature: Vec<f64>,

    /// B-spline order (degree + 1): 4 is cubic, 3 is quadratic. Full-mode fits write 4;
    /// boresight frequency corrections write 3.
    pub spline_order: u8,
}

impl BSplineModel4D {
    /// Creates a new builder for constructing a `BSplineModel4D`.
    pub fn builder() -> BSplineModel4DBuilder {
        BSplineModel4DBuilder::default()
    }

    /// Checks the invariants listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::model::correction_surface::validate_model4d(self)
    }

    /// Returns the total number of coefficients.
    pub fn num_coefficients(&self) -> usize {
        self.coefficients.len()
    }
}

/// Builder for [`BSplineModel4D`]. Every field is required except `spline_order`, which
/// defaults to 3 (quadratic).
#[derive(Default)]
pub struct BSplineModel4DBuilder {
    coefficients: Option<Vec<f64>>,
    shape: Option<[usize; 4]>,
    knots_azimuth: Option<Vec<f64>>,
    knots_elevation: Option<Vec<f64>>,
    knots_frequency: Option<Vec<f64>>,
    knots_temperature: Option<Vec<f64>>,
    spline_order: Option<u8>,
}

impl BSplineModel4DBuilder {
    pub fn coefficients(mut self, coeffs: Vec<f64>) -> Self {
        self.coefficients = Some(coeffs);
        self
    }

    pub fn shape(mut self, shape: [usize; 4]) -> Self {
        self.shape = Some(shape);
        self
    }

    pub fn knots_azimuth(mut self, knots: Vec<f64>) -> Self {
        self.knots_azimuth = Some(knots);
        self
    }

    pub fn knots_elevation(mut self, knots: Vec<f64>) -> Self {
        self.knots_elevation = Some(knots);
        self
    }

    pub fn knots_frequency(mut self, knots: Vec<f64>) -> Self {
        self.knots_frequency = Some(knots);
        self
    }

    pub fn knots_temperature(mut self, knots: Vec<f64>) -> Self {
        self.knots_temperature = Some(knots);
        self
    }

    pub fn spline_order(mut self, order: u8) -> Self {
        self.spline_order = Some(order);
        self
    }

    pub fn build(self) -> Result<BSplineModel4D, String> {
        Ok(BSplineModel4D {
            coefficients: self.coefficients.ok_or("coefficients are required")?,
            shape: self.shape.ok_or("shape is required")?,
            knots_azimuth: self.knots_azimuth.ok_or("knots_azimuth is required")?,
            knots_elevation: self.knots_elevation.ok_or("knots_elevation is required")?,
            knots_frequency: self.knots_frequency.ok_or("knots_frequency is required")?,
            knots_temperature: self
                .knots_temperature
                .ok_or("knots_temperature is required")?,
            spline_order: self.spline_order.unwrap_or(3),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bspline_model_builder() {
        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 1.0, 1.0])
            .spline_order(3)
            .build()
            .unwrap();

        assert_eq!(model.coefficients.len(), 24);
        assert_eq!(model.shape, [2, 3, 2, 2]);
        assert_eq!(model.spline_order, 3);
        assert_eq!(model.num_coefficients(), 24);
    }

    #[test]
    fn test_bspline_model_validate() {
        // Valid model: every axis clamped at order 3 with exactly shape + order knots.
        let valid_model = BSplineModel4D {
            coefficients: vec![1.0; 108],
            shape: [3, 4, 3, 3],
            knots_azimuth: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 3,
        };
        assert!(valid_model.validate().is_ok());

        // Invalid: coefficient size doesn't match shape
        let invalid_model = BSplineModel4D {
            coefficients: vec![1.0; 100],
            ..valid_model.clone()
        };
        assert!(matches!(
            invalid_model.validate(),
            Err(ValidationError::InconsistentShape {
                expected: 108,
                actual: 100
            })
        ));

        // Invalid: knot vector too short
        let invalid_model = BSplineModel4D {
            knots_azimuth: vec![0.0, 1.0],
            ..valid_model.clone()
        };
        assert!(matches!(
            invalid_model.validate(),
            Err(ValidationError::InvalidKnotVector { ref dimension, .. }) if dimension == "azimuth"
        ));

        // Invalid: knot vector not non-decreasing
        let invalid_model = BSplineModel4D {
            knots_azimuth: vec![0.0, 0.0, 0.0, 1.0, 0.5, 1.0],
            ..valid_model.clone()
        };
        assert!(matches!(
            invalid_model.validate(),
            Err(ValidationError::InvalidKnotVector { ref reason, .. })
                if reason.contains("non-decreasing")
        ));

        // Invalid: spline order out of range
        let invalid_model = BSplineModel4D {
            spline_order: 0,
            ..valid_model
        };
        assert_eq!(
            invalid_model.validate(),
            Err(ValidationError::InvalidSplineOrder(0))
        );
    }

    /// A minimal valid order-3 model, shape `[3, 3, 3, 3]`.
    fn make_valid_bspline() -> BSplineModel4D {
        BSplineModel4D {
            coefficients: vec![0.0; 81],
            shape: [3, 3, 3, 3],
            // knot vector length must be exactly shape[i] + spline_order
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![200.0, 200.0, 200.0, 350.0, 350.0, 350.0],
            spline_order: 3,
        }
    }

    #[test]
    fn test_bspline_validate_valid_model() {
        let model = make_valid_bspline();
        assert!(
            model.validate().is_ok(),
            "Expected valid model to pass, got: {:?}",
            model.validate().err()
        );
    }

    #[test]
    fn test_bspline_validate_rejects_short_knots() {
        // knots_azimuth has only 2 elements but order=3 requires shape[0]+order = 2+3 = 5 elements
        let model = BSplineModel4D {
            coefficients: vec![0.0; 8],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0], // too short for order 3 (needs >= 5)
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![200.0, 200.0, 200.0, 350.0, 350.0, 350.0],
            spline_order: 3,
        };
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for too-short knot vector"
        );
        match model.validate().unwrap_err() {
            ValidationError::InvalidKnotVector { dimension, .. } => {
                assert_eq!(dimension, "azimuth");
            }
            other => panic!("Expected InvalidKnotVector, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_non_monotonic_knots() {
        let mut model = make_valid_bspline();
        // Break monotonicity in elevation knots
        model.knots_elevation = vec![0.0, 0.0, 90.0, 50.0, 90.0, 90.0]; // 90 then 50 is decreasing
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for non-monotonic knot vector"
        );
        match model.validate().unwrap_err() {
            ValidationError::InvalidKnotVector { dimension, .. } => {
                assert_eq!(dimension, "elevation");
            }
            other => panic!("Expected InvalidKnotVector, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_coefficient_shape_mismatch() {
        let mut model = make_valid_bspline();
        // shape says 3*3*3*3 = 81 coefficients, but we give 80
        model.coefficients = vec![0.0; 80];
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for coefficient/shape mismatch"
        );
        match model.validate().unwrap_err() {
            ValidationError::InconsistentShape { expected, actual } => {
                assert_eq!(expected, 81);
                assert_eq!(actual, 80);
            }
            other => panic!("Expected InconsistentShape, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_zero_spline_order() {
        let mut model = make_valid_bspline();
        model.spline_order = 0;
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for spline_order=0"
        );
    }
}
