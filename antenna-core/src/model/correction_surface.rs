//! Correction-surface layout and evaluation (GitHub issue #92).
//!
//! This module is the mathematical owner of fitted residual surfaces. Callers use
//! domain coordinates (E-clock, E-cone, and frequency); schema 5's synthetic
//! temperature axis is confined to the wire adapter in this module.

use crate::data::types::{BSplineModel4D, ValidationError as DataValidationError};

/// One non-zero contribution to a correction-surface coefficient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasisStencilEntry {
    pub coefficient_index: usize,
    pub basis_weight: f64,
}

/// Sparse tensor-product basis values for one in-support query.
#[derive(Debug, Clone, PartialEq)]
pub struct BasisStencil {
    entries: Vec<BasisStencilEntry>,
}

impl BasisStencil {
    pub fn entries(&self) -> &[BasisStencilEntry] {
        &self.entries
    }
}

/// Outcome of asking an unfitted layout for its sparse basis stencil.
#[derive(Debug, Clone, PartialEq)]
pub enum BasisStencilOutcome {
    InSupport(BasisStencil),
    OutsideSupport,
}

/// Outcome of evaluating a fitted residual surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CorrectionEvaluation {
    Applied(f64),
    OutsideSupport,
}

impl CorrectionEvaluation {
    /// Return the evidence-backed correction, or `None` outside fitted support.
    pub fn correction_db(self) -> Option<f64> {
        match self {
            Self::Applied(correction_db) => Some(correction_db),
            Self::OutsideSupport => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct AxisLayout {
    knots: Vec<f64>,
    coefficient_count: usize,
}

impl AxisLayout {
    fn support(&self, order: usize) -> (f64, f64) {
        (self.knots[order - 1], self.knots[self.knots.len() - order])
    }

    fn active_basis(&self, value: f64, order: usize) -> Option<Vec<(usize, f64)>> {
        let (lower, upper) = self.support(order);
        if !value.is_finite() || !(lower..=upper).contains(&value) {
            return None;
        }

        let span = find_knot_span(&self.knots, value, order);
        let active = evaluate_basis_functions(&self.knots, span, value, order)
            .into_iter()
            .enumerate()
            .filter(|(_, weight)| *weight != 0.0)
            .map(|(local, weight)| {
                let coefficient_index = span + local - (order - 1);
                (coefficient_index < self.coefficient_count).then_some((coefficient_index, weight))
            })
            .collect::<Option<Vec<_>>>()?;
        (!active.is_empty()).then_some(active)
    }
}

/// Validated three-axis B-spline geometry in canonical coefficient order.
///
/// Coefficients are E-clock-fastest, then E-cone, then frequency:
///
/// ```text
/// i_e_clock + n_e_clock * (i_e_cone + n_e_cone * i_frequency)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionSurfaceLayout {
    axes: [AxisLayout; 3],
    shape: [usize; 3],
    order: usize,
}

impl CorrectionSurfaceLayout {
    pub fn new(
        shape: [usize; 3],
        knots_e_clock: Vec<f64>,
        knots_e_cone: Vec<f64>,
        knots_frequency: Vec<f64>,
        spline_order: u8,
    ) -> std::result::Result<Self, DataValidationError> {
        Self::new_with_axis_names(
            shape,
            [knots_e_clock, knots_e_cone, knots_frequency],
            ["E-clock", "E-cone", "frequency"],
            spline_order,
        )
    }

    fn new_with_axis_names(
        shape: [usize; 3],
        knots: [Vec<f64>; 3],
        names: [&'static str; 3],
        spline_order: u8,
    ) -> std::result::Result<Self, DataValidationError> {
        validate_order(spline_order)?;
        let order = spline_order as usize;
        let [knots_first, knots_second, knots_third] = knots;
        let axes = [
            validated_axis(names[0], shape[0], knots_first, order)?,
            validated_axis(names[1], shape[1], knots_second, order)?,
            validated_axis(names[2], shape[2], knots_third, order)?,
        ];

        coefficient_count(shape).ok_or_else(|| DataValidationError::InvalidKnotVector {
            dimension: "correction surface".to_string(),
            reason: format!("coefficient shape {shape:?} overflows usize"),
        })?;

        Ok(Self { axes, shape, order })
    }

    pub fn coefficient_count(&self) -> usize {
        // Construction proves this product fits in usize.
        self.shape[0] * self.shape[1] * self.shape[2]
    }

    pub fn shape(&self) -> [usize; 3] {
        self.shape
    }

    pub fn spline_order(&self) -> u8 {
        self.order as u8
    }

    /// The validated E-clock knot vector, in canonical axis order.
    pub fn knots_e_clock(&self) -> &[f64] {
        &self.axes[0].knots
    }

    /// The validated E-cone knot vector, in canonical axis order.
    pub fn knots_e_cone(&self) -> &[f64] {
        &self.axes[1].knots
    }

    /// The validated frequency knot vector, in canonical axis order.
    pub fn knots_frequency(&self) -> &[f64] {
        &self.axes[2].knots
    }

    /// Compute the sparse basis stencil for a domain query.
    ///
    /// Exact support boundaries are included. A query outside any axis returns
    /// [`BasisStencilOutcome::OutsideSupport`] before any basis is evaluated; no
    /// coordinate is clamped and no polynomial is extended beyond fitted support.
    pub fn basis_stencil(
        &self,
        e_clock_deg: f64,
        e_cone_deg: f64,
        frequency_mhz: f64,
    ) -> BasisStencilOutcome {
        let Some(clock_basis) = self.axes[0].active_basis(e_clock_deg, self.order) else {
            return BasisStencilOutcome::OutsideSupport;
        };
        let Some(cone_basis) = self.axes[1].active_basis(e_cone_deg, self.order) else {
            return BasisStencilOutcome::OutsideSupport;
        };
        let Some(frequency_basis) = self.axes[2].active_basis(frequency_mhz, self.order) else {
            return BasisStencilOutcome::OutsideSupport;
        };

        let mut entries = Vec::with_capacity(self.order.pow(3));
        for &(frequency_index, frequency_weight) in &frequency_basis {
            for &(cone_index, cone_weight) in &cone_basis {
                for &(clock_index, clock_weight) in &clock_basis {
                    entries.push(BasisStencilEntry {
                        coefficient_index: clock_index
                            + self.shape[0] * (cone_index + self.shape[1] * frequency_index),
                        basis_weight: clock_weight * cone_weight * frequency_weight,
                    });
                }
            }
        }

        BasisStencilOutcome::InSupport(BasisStencil { entries })
    }
}

/// A validated correction-surface layout plus fitted coefficients.
#[derive(Debug, Clone, PartialEq)]
pub struct FittedCorrectionSurface {
    layout: CorrectionSurfaceLayout,
    coefficients: Vec<f64>,
}

impl FittedCorrectionSurface {
    pub fn new(
        layout: CorrectionSurfaceLayout,
        coefficients: Vec<f64>,
    ) -> std::result::Result<Self, DataValidationError> {
        let expected = layout.coefficient_count();
        if coefficients.len() != expected {
            return Err(DataValidationError::InconsistentShape {
                expected,
                actual: coefficients.len(),
            });
        }
        Ok(Self {
            layout,
            coefficients,
        })
    }

    /// Adapt schema 5's byte-compatible 4D wire type into the executable 3D model.
    ///
    /// The synthetic temperature dimension is accepted only when every slab is
    /// identical. Validation and flattening happen here once; evaluation never scans
    /// the wire coefficients.
    pub fn from_model4d(model: &BSplineModel4D) -> std::result::Result<Self, DataValidationError> {
        let layout = schema5_layout(model)?;
        let coefficients = model.coefficients[..layout.coefficient_count()].to_vec();
        Self::new(layout, coefficients)
    }

    pub fn layout(&self) -> &CorrectionSurfaceLayout {
        &self.layout
    }

    /// The fitted coefficients, in the layout's canonical E-clock-fastest order.
    pub fn coefficients(&self) -> &[f64] {
        &self.coefficients
    }

    pub fn evaluate(
        &self,
        e_clock_deg: f64,
        e_cone_deg: f64,
        frequency_mhz: f64,
    ) -> CorrectionEvaluation {
        match self
            .layout
            .basis_stencil(e_clock_deg, e_cone_deg, frequency_mhz)
        {
            BasisStencilOutcome::OutsideSupport => CorrectionEvaluation::OutsideSupport,
            BasisStencilOutcome::InSupport(stencil) => CorrectionEvaluation::Applied(
                stencil
                    .entries()
                    .iter()
                    .map(|entry| self.coefficients[entry.coefficient_index] * entry.basis_weight)
                    .sum(),
            ),
        }
    }
}

fn validate_order(spline_order: u8) -> std::result::Result<(), DataValidationError> {
    if !(1..=10).contains(&spline_order) {
        return Err(DataValidationError::InvalidSplineOrder(spline_order));
    }
    Ok(())
}

fn validated_axis(
    name: &'static str,
    coefficient_count: usize,
    knots: Vec<f64>,
    order: usize,
) -> std::result::Result<AxisLayout, DataValidationError> {
    if coefficient_count == 0 {
        return Err(DataValidationError::InvalidKnotVector {
            dimension: name.to_string(),
            reason: "coefficient count must be non-zero".to_string(),
        });
    }
    let minimum_knots = coefficient_count.checked_add(order).ok_or_else(|| {
        DataValidationError::InvalidKnotVector {
            dimension: name.to_string(),
            reason: "coefficient count + spline order overflows usize".to_string(),
        }
    })?;
    if knots.len() < minimum_knots {
        return Err(DataValidationError::InvalidKnotVector {
            dimension: name.to_string(),
            reason: format!(
                "knot vector length {} < shape {} + order {}",
                knots.len(),
                coefficient_count,
                order
            ),
        });
    }
    if !knots.windows(2).all(|window| window[0] <= window[1]) {
        return Err(DataValidationError::InvalidKnotVector {
            dimension: name.to_string(),
            reason: "knot vector is not non-decreasing".to_string(),
        });
    }

    Ok(AxisLayout {
        knots,
        coefficient_count,
    })
}

/// Validate schema 5's wire representation through the same adapter used to prepare it.
pub(crate) fn validate_model4d(
    model: &BSplineModel4D,
) -> std::result::Result<(), DataValidationError> {
    schema5_layout(model).map(|_| ())
}

fn schema5_layout(
    model: &BSplineModel4D,
) -> std::result::Result<CorrectionSurfaceLayout, DataValidationError> {
    let layout = CorrectionSurfaceLayout::new_with_axis_names(
        [model.shape[0], model.shape[1], model.shape[2]],
        [
            model.knots_azimuth.clone(),
            model.knots_elevation.clone(),
            model.knots_frequency.clone(),
        ],
        ["azimuth", "elevation", "frequency"],
        model.spline_order,
    )?;
    let order = model.spline_order as usize;
    validated_axis(
        "temperature",
        model.shape[3],
        model.knots_temperature.clone(),
        order,
    )?;

    let slab_size = layout.coefficient_count();
    let expected = slab_size.checked_mul(model.shape[3]).ok_or_else(|| {
        DataValidationError::InvalidKnotVector {
            dimension: "correction surface".to_string(),
            reason: format!("coefficient shape {:?} overflows usize", model.shape),
        }
    })?;
    if model.coefficients.len() != expected {
        return Err(DataValidationError::InconsistentShape {
            expected,
            actual: model.coefficients.len(),
        });
    }

    let reference = &model.coefficients[..slab_size];
    for temperature_slab in 1..model.shape[3] {
        let start = temperature_slab * slab_size;
        let slab = &model.coefficients[start..start + slab_size];
        if let Some((coefficient_index, (&expected, &actual))) = reference
            .iter()
            .zip(slab)
            .enumerate()
            .find(|(_, (expected, actual))| expected != actual)
        {
            return Err(DataValidationError::TemperatureDependentCorrection {
                temperature_slab,
                coefficient_index,
                expected,
                actual,
            });
        }
    }

    Ok(layout)
}

fn coefficient_count(shape: [usize; 3]) -> Option<usize> {
    shape
        .into_iter()
        .try_fold(1usize, |count, axis| count.checked_mul(axis))
}

fn find_knot_span(knots: &[f64], value: f64, order: usize) -> usize {
    let mut low = order - 1;
    let mut high = knots.len() - order;
    if value >= knots[high] {
        return high - 1;
    }

    while high - low > 1 {
        let middle = (low + high) / 2;
        if value < knots[middle] {
            high = middle;
        } else {
            low = middle;
        }
    }
    low
}

fn evaluate_basis_functions(knots: &[f64], span: usize, value: f64, order: usize) -> Vec<f64> {
    let degree = order - 1;
    let mut basis = vec![0.0; order];
    let mut left = vec![0.0; order];
    let mut right = vec![0.0; order];
    basis[0] = 1.0;

    for level in 1..=degree {
        left[level] = value - knots[span + 1 - level];
        right[level] = knots[span + level] - value;
        let mut saved = 0.0;
        for basis_index in 0..level {
            let denominator = right[basis_index + 1] + left[level - basis_index];
            let term = if denominator.abs() > 1e-14 {
                basis[basis_index] / denominator
            } else {
                0.0
            };
            basis[basis_index] = saved + right[basis_index + 1] * term;
            saved = left[level - basis_index] * term;
        }
        basis[level] = saved;
    }

    basis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_surface_applies_inside_support_and_refuses_outside_support() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let surface = FittedCorrectionSurface::new(layout, vec![1.25; 8]).unwrap();

        assert_eq!(
            surface.evaluate(5.0, 10.0, 8_500.0),
            CorrectionEvaluation::Applied(1.25)
        );
        assert_eq!(
            surface.evaluate(10_000.0, 10.0, 8_500.0),
            CorrectionEvaluation::OutsideSupport
        );
    }

    #[test]
    fn sparse_stencil_uses_e_clock_fastest_canonical_order() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();

        let BasisStencilOutcome::InSupport(stencil) = layout.basis_stencil(5.0, 10.0, 8_500.0)
        else {
            panic!("midpoint must be in support");
        };
        assert_eq!(stencil.entries().len(), 8);
        for (expected_index, entry) in stencil.entries().iter().enumerate() {
            assert_eq!(entry.coefficient_index, expected_index);
            assert!((entry.basis_weight - 0.125).abs() < 1e-12);
        }
    }

    /// The canonical index law, pinned on a layout whose axes have *different* sizes.
    ///
    /// `[2, 2, 2]` cannot tell E-clock-fastest from any transposition of it — every stride
    /// is the same number. Here the strides are 1, 3 and 12, so a surface indexed in any
    /// other order reads a different coefficient. Each probe sits on one axis's upper
    /// boundary and the others' lower boundary, where exactly one basis function per axis is
    /// active, so the evaluated value *is* the coefficient index the law selects
    /// (GitHub issue #94).
    #[test]
    fn the_canonical_index_law_is_e_clock_fastest_then_e_cone_then_frequency() {
        let shape = [3, 4, 2];
        let layout = CorrectionSurfaceLayout::new(
            shape,
            vec![0.0, 0.0, 4.0, 8.0, 8.0],
            vec![0.0, 0.0, 5.0, 10.0, 15.0, 15.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let coefficients: Vec<f64> = (0..shape[0] * shape[1] * shape[2])
            .map(|index| index as f64)
            .collect();
        let surface = FittedCorrectionSurface::new(layout, coefficients).unwrap();

        let value = |clock, cone, frequency| {
            surface
                .evaluate(clock, cone, frequency)
                .correction_db()
                .expect("probe must be in support")
        };

        assert_eq!(value(0.0, 0.0, 8_000.0), 0.0, "origin is coefficient 0");
        assert_eq!(
            value(8.0, 0.0, 8_000.0),
            (shape[0] - 1) as f64,
            "E-clock has stride 1"
        );
        assert_eq!(
            value(0.0, 15.0, 8_000.0),
            (shape[0] * (shape[1] - 1)) as f64,
            "E-cone has stride n_e_clock"
        );
        assert_eq!(
            value(0.0, 0.0, 9_000.0),
            (shape[0] * shape[1] * (shape[2] - 1)) as f64,
            "frequency has stride n_e_clock * n_e_cone"
        );
    }

    #[test]
    fn every_axis_has_closed_support_and_far_outside_has_no_value() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let coefficients: Vec<f64> = (0..8).map(|index| index as f64).collect();
        let surface = FittedCorrectionSurface::new(layout, coefficients).unwrap();

        for query in [
            (-1.0, 10.0, 8_500.0),
            (11.0, 10.0, 8_500.0),
            (5.0, -1.0, 8_500.0),
            (5.0, 21.0, 8_500.0),
            (5.0, 10.0, 7_999.0),
            (5.0, 10.0, 9_001.0),
            (1.0e12, 10.0, 8_500.0),
        ] {
            assert_eq!(
                surface.evaluate(query.0, query.1, query.2),
                CorrectionEvaluation::OutsideSupport,
                "query {query:?}"
            );
        }

        for query in [
            (0.0, 10.0, 8_500.0),
            (10.0, 10.0, 8_500.0),
            (5.0, 0.0, 8_500.0),
            (5.0, 20.0, 8_500.0),
            (5.0, 10.0, 8_000.0),
            (5.0, 10.0, 9_000.0),
        ] {
            assert!(
                matches!(
                    surface.evaluate(query.0, query.1, query.2),
                    CorrectionEvaluation::Applied(_)
                ),
                "exact boundary {query:?}"
            );
        }
    }

    #[test]
    fn linear_surface_is_continuous_at_exact_boundaries() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let coefficients: Vec<f64> = (0..8)
            .map(|index| if index % 2 == 0 { 0.0 } else { 10.0 })
            .collect();
        let surface = FittedCorrectionSurface::new(layout, coefficients).unwrap();
        let value = |clock| match surface.evaluate(clock, 10.0, 8_500.0) {
            CorrectionEvaluation::Applied(value) => value,
            CorrectionEvaluation::OutsideSupport => panic!("query must be in support"),
        };

        assert_eq!(value(0.0), 0.0);
        assert_eq!(value(10.0), 10.0);
        assert!((value(1.0e-9) - value(0.0)).abs() < 1.0e-8);
        assert!((value(10.0 - 1.0e-9) - value(10.0)).abs() < 1.0e-8);
    }

    #[test]
    fn non_finite_query_is_outside_support_without_a_numeric_value() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let surface = FittedCorrectionSurface::new(layout, vec![0.0; 8]).unwrap();
        for query in [
            (f64::NAN, 10.0, 8_500.0),
            (5.0, f64::INFINITY, 8_500.0),
            (5.0, 10.0, f64::NEG_INFINITY),
        ] {
            assert_eq!(
                surface.evaluate(query.0, query.1, query.2),
                CorrectionEvaluation::OutsideSupport
            );
        }
    }

    #[test]
    fn surplus_knot_basis_positions_are_outside_fitted_support() {
        // Schema 5 historically admitted knot vectors longer than shape + order. The
        // mathematical basis then contains positions with no serialized coefficient.
        // Those regions are unsupported; they must not alias onto the final coefficient.
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 0.0, 10.0, 10.0, 10.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let surface = FittedCorrectionSurface::new(layout, vec![7.0; 8]).unwrap();

        assert_eq!(
            surface.evaluate(7.5, 10.0, 8_500.0),
            CorrectionEvaluation::OutsideSupport
        );
    }

    #[test]
    fn degenerate_executable_axis_has_no_fitted_value() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 20.0, 20.0],
            vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            2,
        )
        .unwrap();
        let surface = FittedCorrectionSurface::new(layout, vec![7.0; 8]).unwrap();

        assert_eq!(
            surface.evaluate(0.0, 10.0, 8_500.0),
            CorrectionEvaluation::OutsideSupport
        );
    }

    #[test]
    fn schema5_adapter_rejects_temperature_varying_coefficients() {
        let model = BSplineModel4D {
            coefficients: vec![1.0; 8].into_iter().chain(vec![2.0; 8]).collect(),
            shape: [2, 2, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 20.0, 20.0],
            knots_frequency: vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            knots_temperature: vec![280.0, 280.0, 300.0, 300.0],
            spline_order: 2,
        };

        let error = FittedCorrectionSurface::from_model4d(&model).unwrap_err();
        assert!(
            error.to_string().contains("temperature slab 1"),
            "error must identify the unequal slab: {error}"
        );
        assert_eq!(model.validate().unwrap_err(), error);
    }

    #[test]
    fn schema5_adapter_flattens_identical_temperature_slabs_once() {
        let model = BSplineModel4D {
            coefficients: vec![1.5; 16],
            shape: [2, 2, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 20.0, 20.0],
            knots_frequency: vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            knots_temperature: vec![280.0, 280.0, 300.0, 300.0],
            spline_order: 2,
        };

        model.validate().unwrap();
        let surface = FittedCorrectionSurface::from_model4d(&model).unwrap();
        assert_eq!(surface.layout().shape(), [2, 2, 2]);
        assert_eq!(
            surface.evaluate(5.0, 10.0, 8_500.0),
            CorrectionEvaluation::Applied(1.5)
        );
    }
}
