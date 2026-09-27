use serde::{Deserialize, Serialize};

/// Wire representation of a correction surface.
///
/// The field names are historical: `azimuth` is E-clock and `elevation` is E-cone.
/// Temperature is a fourth serialized axis kept for byte compatibility; every
/// temperature coefficient slab must be identical. Adapt it once into the executable
/// three-axis [`crate::model::FittedCorrectionSurface`] rather than evaluating it
/// directly.
///
/// Artifact invariants ([`crate::artifact`]): the coefficient count matches `shape`, and
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
}
