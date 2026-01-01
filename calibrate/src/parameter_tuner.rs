//! Physical Parameter Tuning
//!
//! This module implements optional parameter optimization to fine-tune a small set
//! of physical parameters (surface RMS, mesh spacing, wire diameter) to improve the
//! fit between the physical optics model and measurements.
//!
//! # Philosophy
//!
//! - Only 2-3 parameters are tuned (not the full physical model)
//! - Fixed parameters: geometry (diameter, f/D), feed q-factor
//! - Tunable parameters: surface RMS, mesh spacing, (optionally) wire diameter
//! - This step is **optional** - if skipped, correction surfaces compensate
//! - Uses Nelder-Mead simplex optimization (derivative-free, robust)
//!
//! # References
//! - Implementation plan Sprint 4, Task 4.3
//! - Design doc Section 4.2 (Calibration Process)

use anyhow::{Context, Result};
use argmin::core::{CostFunction, Executor, State};
use argmin::solver::neldermead::NelderMead;
use ndarray::Array1;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::antenna_config::{AntennaClass, ParameterBounds, TunableParameters};
use crate::parser::MeasurementData;
use antenna_model::model::{
    compute_g_over_t, AntennaConfiguration as PhysicsConfig, AntennaConfigurationBuilder,
    EClockConeCoordinates, FeedParametersBuilder, IntegrationParams, MeshParametersBuilder,
    ReflectorGeometryBuilder,
};

/// Parameters to tune during optimization
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TuningMode {
    /// Tune only surface RMS
    SurfaceRmsOnly,
    /// Tune surface RMS and mesh spacing
    SurfaceAndMeshSpacing,
    /// Tune all three: surface RMS, mesh spacing, and wire diameter
    All,
}

impl TuningMode {
    /// Number of parameters being tuned
    pub fn num_parameters(&self) -> usize {
        match self {
            TuningMode::SurfaceRmsOnly => 1,
            TuningMode::SurfaceAndMeshSpacing => 2,
            TuningMode::All => 3,
        }
    }
}

/// Results from parameter tuning
#[derive(Debug, Clone)]
pub struct TuningResult {
    /// Tuned surface RMS (mm)
    pub surface_rms_mm: f64,
    /// Tuned mesh spacing (mm), if tuned
    pub mesh_spacing_mm: Option<f64>,
    /// Tuned wire diameter (mm), if tuned
    pub mesh_wire_diameter_mm: Option<f64>,
    /// Initial RMSE before tuning (dB)
    pub initial_rmse_db: f64,
    /// Final RMSE after tuning (dB)
    pub final_rmse_db: f64,
    /// Improvement in RMSE (dB)
    pub improvement_db: f64,
    /// Number of iterations
    pub iterations: usize,
    /// Number of function evaluations
    pub function_evaluations: usize,
}

impl TuningResult {
    /// Convert tuning result to TunableParameters
    pub fn to_tunable_parameters(&self) -> TunableParameters {
        TunableParameters {
            surface_rms_mm: Some(self.surface_rms_mm),
            mesh_spacing_mm: self.mesh_spacing_mm,
            mesh_wire_diameter_mm: self.mesh_wire_diameter_mm,
        }
    }
}

/// Objective function for parameter optimization
///
/// Computes weighted RMSE between measured and predicted G/T values.
/// Higher weights are given to main lobe measurements.
#[derive(Clone)]
struct ObjectiveFunction {
    /// Antenna class (fixed parameters)
    antenna_class: Arc<AntennaClass>,
    /// Measurement data
    measurements: Arc<MeasurementData>,
    /// Tuning mode (which parameters to optimize)
    mode: TuningMode,
    /// Parameter bounds for validation
    bounds: ParameterBounds,
    /// Integration parameters for physics model
    integration_params: IntegrationParams,
    /// Evaluation counter
    eval_counter: Arc<AtomicUsize>,
}

impl ObjectiveFunction {
    /// Create new objective function
    pub fn new(
        antenna_class: Arc<AntennaClass>,
        measurements: Arc<MeasurementData>,
        mode: TuningMode,
        integration_params: IntegrationParams,
    ) -> Self {
        let bounds = ParameterBounds::default();
        Self {
            antenna_class,
            measurements,
            mode,
            bounds,
            integration_params,
            eval_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Convert parameter vector to physical parameters
    fn params_to_physical(&self, params: &[f64]) -> (f64, Option<f64>, Option<f64>) {
        match self.mode {
            TuningMode::SurfaceRmsOnly => (params[0], None, None),
            TuningMode::SurfaceAndMeshSpacing => (params[0], Some(params[1]), None),
            TuningMode::All => (params[0], Some(params[1]), Some(params[2])),
        }
    }

    /// Build physics model configuration from tunable parameters
    fn build_physics_config(
        &self,
        surface_rms_mm: f64,
        mesh_spacing_mm: Option<f64>,
        wire_diameter_mm: Option<f64>,
    ) -> Result<PhysicsConfig> {
        // Get effective parameters
        let effective_surface_rms = surface_rms_mm;
        let effective_mesh_spacing = mesh_spacing_mm.unwrap_or(self.antenna_class.mesh.spacing_mm);
        let effective_wire_diameter =
            wire_diameter_mm.unwrap_or(self.antenna_class.mesh.wire_diameter_mm);

        // Build reflector geometry
        let reflector = ReflectorGeometryBuilder::default()
            .diameter(self.antenna_class.geometry.diameter_m)
            .focal_length(
                self.antenna_class.geometry.diameter_m * self.antenna_class.geometry.f_over_d,
            )
            .surface_rms(effective_surface_rms / 1000.0) // mm to m
            .build()
            .context("Failed to build reflector geometry")?;

        // Build feed parameters (at focal point for on-axis configuration)
        let focal_length =
            self.antenna_class.geometry.diameter_m * self.antenna_class.geometry.f_over_d;
        let feed = FeedParametersBuilder::default()
            .at_focus(focal_length)
            .q_factor(self.antenna_class.feed.q_factor)
            .phase_center_offset(self.antenna_class.feed.phase_center_offset_wavelengths)
            .asymmetry_factor(self.antenna_class.feed.asymmetry_factor)
            .build()
            .context("Failed to build feed parameters")?;

        // Build mesh parameters
        let mesh = MeshParametersBuilder::default()
            .spacing(effective_mesh_spacing / 1000.0) // mm to m
            .wire_diameter(effective_wire_diameter / 1000.0) // mm to m
            .build()
            .context("Failed to build mesh parameters")?;

        // Build complete configuration
        let config = AntennaConfigurationBuilder::default()
            .id("calibration_tuning")
            .name("Calibration Tuning Model")
            .reflector(reflector)
            .feed(feed)
            .mesh(mesh)
            .build()
            .context("Failed to build antenna configuration")?;

        Ok(config)
    }

    /// Compute weighted RMSE for given parameters
    fn compute_rmse(&self, params: &[f64]) -> Result<f64> {
        // Increment evaluation counter
        let eval_count = self.eval_counter.fetch_add(1, Ordering::SeqCst) + 1;

        // Extract parameters
        let (surface_rms_mm, mesh_spacing_mm, wire_diameter_mm) = self.params_to_physical(params);

        // Validate bounds
        if surface_rms_mm < self.bounds.surface_rms_mm.0
            || surface_rms_mm > self.bounds.surface_rms_mm.1
        {
            return Ok(1e10); // Penalty for out-of-bounds
        }
        if let Some(spacing) = mesh_spacing_mm {
            if spacing < self.bounds.mesh_spacing_mm.0 || spacing > self.bounds.mesh_spacing_mm.1 {
                return Ok(1e10);
            }
        }
        if let Some(diameter) = wire_diameter_mm {
            if diameter < self.bounds.wire_diameter_mm.0
                || diameter > self.bounds.wire_diameter_mm.1
            {
                return Ok(1e10);
            }
        }

        // Build physics configuration
        let physics_config =
            self.build_physics_config(surface_rms_mm, mesh_spacing_mm, wire_diameter_mm)?;

        // Compute predictions and errors
        let mut squared_errors = Vec::new();
        let mut weights = Vec::new();

        for point in &self.measurements.points {
            // Convert E-clock/E-cone to physics coordinates (radians)
            let coords = EClockConeCoordinates {
                e_clock: point.e_clock_deg.to_radians(),
                e_cone: point.e_cone_deg.to_radians(),
            };

            // Convert to far-field angles (θ, φ)
            let far_field = coords.to_far_field();

            // Compute predicted G/T from physics model
            let frequency_hz = point.frequency_mhz * 1e6;
            let temperature_k = self.antenna_class.system_noise_temperature_k;

            let predicted_g_over_t = compute_g_over_t(
                far_field.theta,
                far_field.phi,
                &physics_config,
                frequency_hz,
                temperature_k,
                &self.integration_params,
            );

            // Handle computation errors gracefully
            let predicted = match predicted_g_over_t {
                Ok(val) => val,
                Err(e) => {
                    warn!("Physics computation failed for eval {}: {}", eval_count, e);
                    return Ok(1e10); // Penalty for failed computation
                }
            };

            // Compute error
            let error = point.g_over_t_db - predicted;
            squared_errors.push(error * error);

            // Weight: higher for main lobe (within 3 beamwidths)
            // Rough beamwidth estimate: 70*λ/D degrees
            let wavelength_m = 3e8 / frequency_hz;
            let beamwidth_deg = 70.0 * wavelength_m / self.antenna_class.geometry.diameter_m;
            let weight = if point.is_main_lobe(beamwidth_deg) {
                3.0 // 3x weight for main lobe
            } else {
                1.0
            };
            weights.push(weight);
        }

        // Compute weighted RMSE
        let weighted_sum: f64 = squared_errors
            .iter()
            .zip(weights.iter())
            .map(|(err, w)| err * w)
            .sum();
        let total_weight: f64 = weights.iter().sum();
        let rmse = (weighted_sum / total_weight).sqrt();

        if eval_count.is_multiple_of(10) {
            debug!(
                "Evaluation {}: params={:?}, RMSE={:.3} dB",
                eval_count, params, rmse
            );
        }

        Ok(rmse)
    }
}

impl CostFunction for ObjectiveFunction {
    type Param = Array1<f64>;
    type Output = f64;

    fn cost(&self, params: &Self::Param) -> Result<Self::Output, argmin::core::Error> {
        self.compute_rmse(params.as_slice().unwrap())
            .map_err(|e| argmin::core::Error::msg(format!("Cost computation failed: {}", e)))
    }
}

/// Tune physical parameters to minimize RMSE between model and measurements
///
/// This is an **optional** calibration step that fine-tunes 2-3 key parameters.
/// If skipped, correction surfaces (Task 4.4) will compensate for parameter mismatch.
///
/// # Arguments
/// - `antenna_class`: Antenna class with nominal parameters
/// - `initial_tunable`: Initial tunable parameter values (if any)
/// - `measurements`: Measurement data for optimization
/// - `mode`: Which parameters to tune (surface RMS, mesh spacing, wire diameter)
/// - `max_iterations`: Maximum optimization iterations (default: 200)
///
/// # Returns
/// Tuning result with optimized parameters and RMSE improvement
///
/// # Examples
/// ```no_run
/// use calibrate::{tune_parameters, TuningMode, AntennaClass, MeasurementData, TunableParameters};
///
/// # fn example(antenna_class: AntennaClass, measurements: MeasurementData) {
/// // Tune surface RMS and mesh spacing
/// let result = tune_parameters(
///     antenna_class,
///     TunableParameters::default_from_class(),
///     measurements,
///     TuningMode::SurfaceAndMeshSpacing,
///     Some(200)
/// ).expect("Tuning failed");
///
/// println!("Initial RMSE: {:.2} dB", result.initial_rmse_db);
/// println!("Final RMSE: {:.2} dB", result.final_rmse_db);
/// println!("Improvement: {:.2} dB", result.improvement_db);
/// # }
/// ```
pub fn tune_parameters(
    antenna_class: AntennaClass,
    initial_tunable: TunableParameters,
    measurements: MeasurementData,
    mode: TuningMode,
    max_iterations: Option<u64>,
) -> Result<TuningResult> {
    info!(
        "Starting parameter tuning ({} parameters)",
        mode.num_parameters()
    );

    let max_iters = max_iterations.unwrap_or(200);

    // Use fast integration parameters for optimization (speed over accuracy)
    let integration_params = IntegrationParams::fast();

    // Initial parameter values from tunable parameters or class defaults
    let mut initial_params_vec = Vec::new();
    initial_params_vec.push(initial_tunable.effective_surface_rms(&antenna_class));

    if mode.num_parameters() >= 2 {
        initial_params_vec.push(initial_tunable.effective_mesh_spacing(&antenna_class));
    }

    if mode.num_parameters() >= 3 {
        initial_params_vec.push(initial_tunable.effective_wire_diameter(&antenna_class));
    }

    let initial_params = Array1::from_vec(initial_params_vec.clone());

    // Create objective function
    let antenna_class_arc = Arc::new(antenna_class);
    let measurements_arc = Arc::new(measurements);
    let objective = ObjectiveFunction::new(
        antenna_class_arc.clone(),
        measurements_arc.clone(),
        mode,
        integration_params,
    );

    // Compute initial RMSE
    let initial_rmse = objective
        .compute_rmse(&initial_params_vec)
        .context("Failed to compute initial RMSE")?;
    info!("Initial RMSE: {:.3} dB", initial_rmse);

    // Set up Nelder-Mead optimizer
    let solver = NelderMead::new(vec![initial_params]).with_sd_tolerance(1e-6)?;

    // Run optimization
    info!(
        "Running Nelder-Mead optimization (max {} iterations)",
        max_iters
    );
    let result = Executor::new(objective.clone(), solver)
        .configure(|state| state.max_iters(max_iters))
        .run()
        .context("Optimization failed")?;

    // Extract optimized parameters
    let best_params = result
        .state()
        .get_best_param()
        .ok_or_else(|| anyhow::anyhow!("No best parameter found"))?;

    let final_rmse = result.state().get_best_cost();

    let (surface_rms_mm, mesh_spacing_mm, wire_diameter_mm) =
        objective.params_to_physical(best_params.as_slice().unwrap());

    let improvement = initial_rmse - final_rmse;

    info!("Optimization completed:");
    info!("  Final RMSE: {:.3} dB", final_rmse);
    info!(
        "  Improvement: {:.3} dB ({:.1}%)",
        improvement,
        (improvement / initial_rmse) * 100.0
    );
    info!("  Surface RMS: {:.3} mm", surface_rms_mm);
    if let Some(spacing) = mesh_spacing_mm {
        info!("  Mesh spacing: {:.3} mm", spacing);
    }
    if let Some(diameter) = wire_diameter_mm {
        info!("  Wire diameter: {:.3} mm", diameter);
    }

    let tuning_result = TuningResult {
        surface_rms_mm,
        mesh_spacing_mm,
        mesh_wire_diameter_mm: wire_diameter_mm,
        initial_rmse_db: initial_rmse,
        final_rmse_db: final_rmse,
        improvement_db: improvement,
        iterations: result.state().get_iter() as usize,
        function_evaluations: objective.eval_counter.load(Ordering::SeqCst),
    };

    Ok(tuning_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antenna_config::{
        FeedParameters, MeshParameters, ReflectorGeometry, SurfaceParameters,
    };
    use crate::parser::MeasurementPoint;

    /// Create a simple test antenna class
    fn create_test_class() -> AntennaClass {
        AntennaClass {
            class_id: "TestAntenna".to_string(),
            description: "Test antenna".to_string(),
            geometry: ReflectorGeometry {
                diameter_m: 1.0,
                f_over_d: 0.5,
            },
            feed: FeedParameters {
                q_factor: 8.0,
                phase_center_offset_wavelengths: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: MeshParameters {
                spacing_mm: 2.0,
                wire_diameter_mm: 0.2,
            },
            surface: SurfaceParameters { rms_mm: 0.5 },
            system_noise_temperature_k: 50.0,
        }
    }

    /// Create synthetic measurement data
    fn create_synthetic_measurements() -> MeasurementData {
        // Create a few measurement points near boresight
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.0, 50.0), // On-axis
            MeasurementPoint::new(0.0, 0.5, 8400.0, 40.5, 50.0), // Slight off-axis
            MeasurementPoint::new(90.0, 0.5, 8400.0, 40.5, 50.0), // Different clock
            MeasurementPoint::new(0.0, 1.0, 8400.0, 39.8, 50.0), // Further off-axis
            MeasurementPoint::new(180.0, 1.5, 8400.0, 38.5, 50.0), // Sidelobe region
        ];

        MeasurementData::new(points, "synthetic test data".to_string())
    }

    #[test]
    fn test_tuning_mode_num_parameters() {
        assert_eq!(TuningMode::SurfaceRmsOnly.num_parameters(), 1);
        assert_eq!(TuningMode::SurfaceAndMeshSpacing.num_parameters(), 2);
        assert_eq!(TuningMode::All.num_parameters(), 3);
    }

    #[test]
    fn test_params_to_physical() {
        let class = create_test_class();
        let measurements = create_synthetic_measurements();
        let objective = ObjectiveFunction::new(
            Arc::new(class),
            Arc::new(measurements),
            TuningMode::All,
            IntegrationParams::fast(),
        );

        let (rms, spacing, diameter) = objective.params_to_physical(&[0.5, 2.0, 0.2]);
        assert_eq!(rms, 0.5);
        assert_eq!(spacing, Some(2.0));
        assert_eq!(diameter, Some(0.2));
    }

    #[test]
    fn test_tuning_result_to_tunable_parameters() {
        let result = TuningResult {
            surface_rms_mm: 0.6,
            mesh_spacing_mm: Some(2.2),
            mesh_wire_diameter_mm: Some(0.25),
            initial_rmse_db: 2.0,
            final_rmse_db: 1.0,
            improvement_db: 1.0,
            iterations: 50,
            function_evaluations: 200,
        };

        let tunable = result.to_tunable_parameters();
        assert_eq!(tunable.surface_rms_mm, Some(0.6));
        assert_eq!(tunable.mesh_spacing_mm, Some(2.2));
        assert_eq!(tunable.mesh_wire_diameter_mm, Some(0.25));
    }

    #[test]
    fn test_objective_function_bounds_validation() {
        let class = create_test_class();
        let measurements = create_synthetic_measurements();
        let objective = ObjectiveFunction::new(
            Arc::new(class),
            Arc::new(measurements),
            TuningMode::SurfaceRmsOnly,
            IntegrationParams::fast(),
        );

        // Out of bounds - should return large penalty
        let result = objective.compute_rmse(&[10.0]).unwrap();
        assert!(result > 1e9);

        // In bounds - should return reasonable RMSE
        let result = objective.compute_rmse(&[0.5]).unwrap();
        assert!(result < 100.0);
    }

    // Integration test with full optimization (marked as ignored for speed)
    #[test]
    #[ignore]
    fn test_parameter_tuning_integration() {
        // This test requires the full physics model and is slow
        let class = create_test_class();
        let measurements = create_synthetic_measurements();
        let initial_tunable = TunableParameters {
            surface_rms_mm: Some(0.5),
            mesh_spacing_mm: Some(2.0),
            mesh_wire_diameter_mm: None,
        };

        // Run tuning
        let result = tune_parameters(
            class,
            initial_tunable,
            measurements,
            TuningMode::SurfaceAndMeshSpacing,
            Some(50), // Limit iterations for test speed
        )
        .expect("Tuning failed");

        // Check that tuning completed
        assert!(result.final_rmse_db <= result.initial_rmse_db + 0.1); // Allow slight increase due to noise

        // Check that parameters are in reasonable range
        assert!(result.surface_rms_mm > 0.1 && result.surface_rms_mm < 2.0);
        if let Some(spacing) = result.mesh_spacing_mm {
            assert!(spacing > 1.0 && spacing < 10.0);
        }
    }
}
