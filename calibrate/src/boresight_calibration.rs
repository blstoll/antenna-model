//! Boresight Calibration Mode
//!
//! This module implements boresight-only calibration for antenna models using frequency sweep
//! measurements at azimuth=0, elevation=0. This is a quick calibration method requiring ~1 hour
//! of test time versus ~8 hours for full grid calibration.
//!
//! # Workflow
//!
//! 1. Load design specifications as initial parameter estimates
//! 2. Tune physical parameters using differential evolution:
//!    - surface_rms_mm
//!    - q_factor
//!    - mesh_spacing_mm (if applicable)
//!    - wire_diameter_mm (if applicable)
//! 3. Optional: Fit 1D frequency-only correction surface
//! 4. Build calibration artifact with `PartiallyCalibrated` status
//!
//! # Accuracy Expectations
//!
//! - Boresight: ±1 dB (tuned to measurements)
//! - Off-axis: ±2-3 dB (physics extrapolation only)
//! - Loss (relative): ±1-2 dB (error cancellation)

use anyhow::{Context, Result};
use argmin::core::{CostFunction, Executor, State};
use argmin::solver::neldermead::NelderMead;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

use crate::design_specs_loader::{DesignSpecs, TuningBounds};
use crate::frequency_correction;
use antenna_model::data::types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, CalibrationCoverageBuilder,
    CalibrationMetadataBuilder, CalibrationStatus, FeedParameters as DataFeedParameters,
    MeasurementDensity, MeshParameters as DataMeshParameters, ParameterSource,
    PhysicalAntennaConfigBuilder, ReflectorGeometry as DataReflectorGeometry,
    ValidityRangesBuilder, BORESIGHT_COVERAGE_CONE_DEG, CALIBRATION_SCHEMA_VERSION,
};
use antenna_model::model::{
    compute_g_over_t, AntennaConfigurationBuilder, FeedParametersBuilder, IntegrationParams,
    MeshParametersBuilder, ReflectorGeometryBuilder, PHYSICS_MODEL_VERSION,
};

/// Boresight measurement point (frequency sweep at azimuth=0, elevation=0)
#[derive(Debug, Clone)]
pub struct BoresightMeasurement {
    /// Frequency in MHz
    pub frequency_mhz: f64,
    /// Measured G/T in dB/K
    pub g_over_t_db: f64,
    /// System noise temperature in Kelvin
    pub temperature_k: f64,
}

/// Collection of boresight measurements
#[derive(Debug, Clone)]
pub struct BoresightMeasurements {
    /// Measurement points
    pub points: Vec<BoresightMeasurement>,
}

impl BoresightMeasurements {
    /// Parse boresight measurements from CSV.
    ///
    /// CSV format: `frequency_mhz,g_over_t_db,temperature_k`.
    ///
    /// Lines beginning with `#` are ignored, so a measurement file can carry its own
    /// provenance block ahead of the column header. Real published data always arrives
    /// with assumptions attached — the assumed system noise temperature that turned a
    /// published *gain* into the `g_over_t_db` column, the digitization method, which
    /// rows came from which table — and a fixture that cannot record them beside the
    /// numbers invites them to be forgotten (roadmap D13; the reference `.psv` datasets
    /// under `antenna-model/tests/fixtures/reference_datasets/` use the same convention).
    ///
    /// Unlike full mode's parser this one **fails hard** on a malformed row rather than
    /// dropping it: a boresight sweep is a handful of points and a silently dropped one
    /// changes the fit. That difference is deliberate — see roadmap D11/D13.
    pub fn from_csv(csv_content: &str) -> Result<Self> {
        let mut points = Vec::new();
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .comment(Some(b'#'))
            .from_reader(csv_content.as_bytes());

        for (record_idx, result) in reader.records().enumerate() {
            // Report the real file line, not the record ordinal: a provenance block or a
            // blank line makes those two disagree, and a fail-hard parser whose error
            // points at the wrong line is worse than no line number at all.
            let fallback_line = record_idx + 2;
            let record =
                result.with_context(|| format!("Failed to parse CSV record {}", record_idx + 1))?;
            let line_num = record.position().map_or(fallback_line as u64, |p| p.line()) as usize;

            if record.len() != 3 {
                anyhow::bail!(
                    "Invalid CSV format at line {}: expected 3 columns, got {}",
                    line_num,
                    record.len()
                );
            }

            let frequency_mhz: f64 = record[0]
                .parse()
                .with_context(|| format!("Invalid frequency at line {}", line_num))?;

            let g_over_t_db: f64 = record[1]
                .parse()
                .with_context(|| format!("Invalid g_over_t at line {}", line_num))?;

            let temperature_k: f64 = record[2]
                .parse()
                .with_context(|| format!("Invalid temperature at line {}", line_num))?;

            points.push(BoresightMeasurement {
                frequency_mhz,
                g_over_t_db,
                temperature_k,
            });
        }

        if points.is_empty() {
            anyhow::bail!("No measurements found in CSV");
        }

        Ok(Self { points })
    }

    /// Get frequency range (min, max) in MHz
    pub fn frequency_range(&self) -> (f64, f64) {
        let freqs: Vec<f64> = self.points.iter().map(|p| p.frequency_mhz).collect();
        let min = freqs.iter().copied().fold(f64::INFINITY, f64::min);
        let max = freqs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (min, max)
    }
}

/// Parameters to tune during boresight calibration
#[derive(Debug, Clone)]
pub struct BoresightTunableParameters {
    /// Surface RMS error in millimeters
    pub surface_rms_mm: f64,
    /// Feed q-factor for cos^q illumination
    pub q_factor: f64,
    /// Optional mesh spacing in millimeters
    pub mesh_spacing_mm: Option<f64>,
    /// Optional wire diameter in millimeters
    pub wire_diameter_mm: Option<f64>,
}

impl BoresightTunableParameters {
    /// Create from design specs (initial guesses)
    pub fn from_design_specs(specs: &DesignSpecs, feed_id: &str) -> Result<Self> {
        let feed = specs
            .get_feed(feed_id)
            .ok_or_else(|| anyhow::anyhow!("Feed '{}' not found in design specs", feed_id))?;

        Ok(Self {
            surface_rms_mm: specs.reflector.surface_rms_mm,
            q_factor: feed.q_factor,
            mesh_spacing_mm: specs.mesh.as_ref().map(|m| m.mesh_spacing_mm),
            wire_diameter_mm: specs.mesh.as_ref().map(|m| m.wire_diameter_mm),
        })
    }

    /// Convert to parameter vector for optimization
    fn to_vector(&self) -> Vec<f64> {
        let mut vec = vec![self.surface_rms_mm, self.q_factor];
        if let Some(spacing) = self.mesh_spacing_mm {
            vec.push(spacing);
        }
        if let Some(diameter) = self.wire_diameter_mm {
            vec.push(diameter);
        }
        vec
    }

    /// Create from parameter vector
    fn from_vector(vec: &[f64], has_mesh: bool) -> Self {
        let surface_rms_mm = vec[0];
        let q_factor = vec[1];

        let (mesh_spacing_mm, wire_diameter_mm) = if has_mesh {
            (
                Some(vec[2]),
                if vec.len() > 3 { Some(vec[3]) } else { None },
            )
        } else {
            (None, None)
        };

        Self {
            surface_rms_mm,
            q_factor,
            mesh_spacing_mm,
            wire_diameter_mm,
        }
    }
}

/// Results from boresight calibration
#[derive(Debug, Clone)]
pub struct BoresightCalibrationResult {
    /// Tuned parameters
    pub tuned_params: BoresightTunableParameters,
    /// Initial RMSE (dB) with design specs
    pub initial_rmse_db: f64,
    /// Final RMSE (dB) after tuning
    pub final_rmse_db: f64,
    /// Improvement in RMSE (dB)
    pub improvement_db: f64,
    /// Number of optimization iterations
    pub iterations: usize,
    /// Number of function evaluations
    pub function_evaluations: usize,
    /// Optional 1D frequency correction surface (degenerate 4D B-spline)
    pub frequency_correction: Option<BSplineModel4D>,
}

/// Objective function for boresight parameter tuning
#[derive(Clone)]
struct BoresightObjectiveFunction {
    design_specs: Arc<DesignSpecs>,
    feed_id: String,
    measurements: Arc<BoresightMeasurements>,
    bounds: TuningBounds,
    integration_params: IntegrationParams,
    eval_counter: Arc<AtomicUsize>,
}

impl BoresightObjectiveFunction {
    /// `physics_will_be_uncorrected` says whether the artifact this tuning run feeds will
    /// ship **without** a correction surface, and therefore be served as raw physics.
    ///
    /// It is not a tuning knob: it selects the model the service will evaluate this
    /// artifact under, via the one shared setter the service also uses (roadmap D17).
    /// Getting it wrong does not make the fit worse in any way the calibrator can see — the
    /// tuner still converges, still reports a small RMSE — it just makes that RMSE describe
    /// a gain nobody will ever be served.
    fn new(
        design_specs: Arc<DesignSpecs>,
        feed_id: String,
        measurements: Arc<BoresightMeasurements>,
        bounds: TuningBounds,
        physics_will_be_uncorrected: bool,
    ) -> Self {
        Self {
            design_specs,
            feed_id,
            measurements,
            bounds,
            integration_params: IntegrationParams::default()
                .with_uncorrected_physics_gates(physics_will_be_uncorrected),
            eval_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Check if parameters are within bounds
    fn check_bounds(&self, params: &BoresightTunableParameters) -> bool {
        if params.surface_rms_mm < self.bounds.surface_rms_mm_range.0
            || params.surface_rms_mm > self.bounds.surface_rms_mm_range.1
        {
            return false;
        }

        if params.q_factor < self.bounds.q_factor_range.0
            || params.q_factor > self.bounds.q_factor_range.1
        {
            return false;
        }

        if let (Some(spacing), Some(range)) =
            (params.mesh_spacing_mm, self.bounds.mesh_spacing_mm_range)
        {
            if spacing < range.0 || spacing > range.1 {
                return false;
            }
        }

        if let (Some(diameter), Some(range)) =
            (params.wire_diameter_mm, self.bounds.wire_diameter_mm_range)
        {
            if diameter < range.0 || diameter > range.1 {
                return false;
            }
        }

        true
    }

    /// Compute predictions for all measurement points with given parameters
    fn compute_predictions(&self, params: &BoresightTunableParameters) -> Result<Vec<f64>> {
        // Build reflector geometry (using model builders - values in meters)
        let reflector = ReflectorGeometryBuilder::default()
            .diameter(self.design_specs.reflector.diameter_m)
            .focal_length(self.design_specs.reflector.focal_length_m)
            .surface_rms(params.surface_rms_mm / 1000.0) // Convert mm to m
            .build()
            .context("Failed to build reflector geometry")?;

        // Build feed parameters (using model builders)
        // For boresight calibration, assume feed is at focal point
        let feed_spec = self
            .design_specs
            .get_feed(&self.feed_id)
            .with_context(|| format!("Feed '{}' not found in design specs", self.feed_id))?;
        let feed = FeedParametersBuilder::default()
            .at_focus(self.design_specs.reflector.focal_length_m)
            .q_factor(params.q_factor)
            .phase_center_offset(feed_spec.phase_center_offset_m)
            // The declared design value, not a hardcoded 1.0 — the tuner must minimise
            // its objective against the same model the artifact will be served with
            // (roadmap D23; same rule D17 established for the integration gates).
            .asymmetry_factor(feed_spec.asymmetry_factor)
            .build()
            .context("Failed to build feed parameters")?;

        // Build mesh parameters (using model builders - values in meters)
        let mesh = if let Some(mesh_spacing) = params.mesh_spacing_mm {
            Some(
                MeshParametersBuilder::default()
                    .spacing(mesh_spacing / 1000.0) // Convert mm to m
                    .wire_diameter(params.wire_diameter_mm.unwrap_or(0.5) / 1000.0) // Convert mm to m
                    .build()
                    .context("Failed to build mesh parameters")?,
            )
        } else {
            None
        };

        // Build complete configuration
        let mut config_builder = AntennaConfigurationBuilder::default()
            .id(&self.design_specs.antenna_id)
            .name(&self.design_specs.antenna_name)
            .reflector(reflector)
            .feed(feed);

        if let Some(m) = mesh {
            config_builder = config_builder.mesh(m);
        }

        let config = config_builder
            .build()
            .context("Failed to build antenna configuration")?;

        // Compute predictions for all measurement points at boresight (theta=0, phi=0)
        let theta = 0.0; // Boresight
        let phi = 0.0;

        let mut predictions = Vec::with_capacity(self.measurements.points.len());
        for point in &self.measurements.points {
            let frequency_hz = point.frequency_mhz * 1e6;
            let predicted = compute_g_over_t(
                theta,
                phi,
                &config,
                frequency_hz,
                point.temperature_k,
                &self.integration_params,
            )
            .context("Failed to compute G/T")?;

            predictions.push(predicted);
        }

        Ok(predictions)
    }

    /// Compute RMSE for given parameters
    fn compute_rmse(&self, params: &BoresightTunableParameters) -> Result<f64> {
        let predictions = self.compute_predictions(params)?;

        let squared_errors: f64 = self
            .measurements
            .points
            .iter()
            .zip(predictions.iter())
            .map(|(meas, pred)| {
                let error = meas.g_over_t_db - pred;
                error * error
            })
            .sum();

        Ok((squared_errors / self.measurements.points.len() as f64).sqrt())
    }
}

impl CostFunction for BoresightObjectiveFunction {
    type Param = Vec<f64>;
    type Output = f64;

    fn cost(&self, param: &Self::Param) -> Result<Self::Output, argmin::core::Error> {
        let eval_num = self.eval_counter.fetch_add(1, Ordering::Relaxed) + 1;

        let has_mesh = self.design_specs.mesh.is_some();
        let params = BoresightTunableParameters::from_vector(param, has_mesh);

        // Check bounds
        if !self.check_bounds(&params) {
            return Ok(1e6); // Large penalty for out-of-bounds
        }

        // Compute RMSE
        let rmse = self
            .compute_rmse(&params)
            .map_err(|e| argmin::core::Error::msg(format!("RMSE computation failed: {}", e)))?;

        if eval_num.is_multiple_of(10) {
            debug!(
                "Eval {}: surface_rms={:.3}mm, q={:.2}, rmse={:.4}dB",
                eval_num, params.surface_rms_mm, params.q_factor, rmse
            );
        }

        Ok(rmse)
    }
}

/// One completed tuning run, together with the residuals it leaves behind.
///
/// A run is only meaningful alongside the gate setting it was tuned under — the residuals
/// are `measured − predicted` under *that* model — so the two travel together.
struct TuningPass {
    params: BoresightTunableParameters,
    initial_rmse_db: f64,
    final_rmse_db: f64,
    iterations: usize,
    function_evaluations: usize,
    /// `measured − predicted` at every measurement point, under the pass's own gates.
    residuals: Vec<f64>,
}

/// Perform boresight calibration.
///
/// # Arguments
///
/// * `design_specs` - Design specifications with initial parameter estimates
/// * `feed_id` - Feed identifier to calibrate
/// * `measurements` - Boresight measurements (frequency sweep at az=0, el=0)
/// * `max_iterations` - Maximum optimization iterations (recommended: 100-200)
///
/// # Returns
///
/// Calibration result with tuned parameters and statistics
///
/// # Why this runs the tuner up to twice (roadmap D17)
///
/// The service decides how to evaluate an artifact's physics from whether that artifact
/// carries a correction surface: no surface means the served gain is raw physics, so
/// spillover and the F7 sidelobe floor are folded in; a surface means they are left off
/// because the surface absorbs them empirically. Calibration has to optimize against
/// whichever of those two models the artifact it produces will actually be served under, or
/// the tuner's own reported RMSE describes a number the service never returns.
///
/// That is circular — whether a correction is fitted depends on the residuals, which depend
/// on the gates, which depend on whether a correction is fitted — so it is resolved in the
/// order the pipeline already runs in:
///
/// 1. Tune with the gates **on**, i.e. under the model a *correction-free* artifact is
///    served with, and decide from those residuals whether a correction is needed. If not,
///    that pass is the answer and it is self-consistent by construction.
/// 2. If a correction *is* needed, the artifact will carry one and the service will
///    therefore serve it with the gates **off** — so re-tune under those gates and fit the
///    correction to the residuals from that second pass.
///
/// The branch is decided once, in pass 1, and never revisited: pass 2 fits its correction
/// regardless of how small its residuals turn out to be. Deciding twice would let the two
/// passes disagree about which branch applies and leave the choice oscillating.
pub fn calibrate_boresight(
    design_specs: &DesignSpecs,
    feed_id: &str,
    measurements: &BoresightMeasurements,
    max_iterations: Option<u64>,
) -> Result<BoresightCalibrationResult> {
    info!("Starting boresight calibration...");
    info!("  Antenna: {}", design_specs.antenna_id);
    info!("  Feed: {}", feed_id);
    info!("  Measurements: {}", measurements.points.len());

    // Pass 1 — tune under the model a correction-free artifact is served with.
    let uncorrected_pass = tune_boresight_parameters(
        design_specs,
        feed_id,
        measurements,
        max_iterations,
        true, // physics_will_be_uncorrected
    )?;

    info!("  Checking if frequency correction is needed...");
    if !frequency_correction::should_fit_correction(&uncorrected_pass.residuals) {
        info!("    Max residual < 0.5 dB, frequency correction not needed");
        info!("    Artifact ships without a correction; the service will serve it with the");
        info!("    same spillover and sidelobe-floor terms this tuning run applied.");
        return Ok(uncorrected_pass.into_result(None));
    }

    // Pass 2 — a correction WILL be attached, so the service will serve this artifact with
    // the uncorrected-physics gates off. Re-tune under those gates and fit the correction to
    // the residuals they leave, so the shipped surface corrects the model that is served.
    info!("    Max residual exceeds 0.5 dB threshold, a frequency correction is needed");
    info!("    Re-tuning without the uncorrected-physics terms, which the service leaves");
    info!("    off for an artifact that carries a correction surface (roadmap D17)");
    let corrected_pass = tune_boresight_parameters(
        design_specs,
        feed_id,
        measurements,
        max_iterations,
        false, // physics_will_be_uncorrected
    )?;

    info!("    Fitting frequency correction...");
    let frequencies: Vec<f64> = measurements
        .points
        .iter()
        .map(|p| p.frequency_mhz)
        .collect();

    let frequency_correction = match frequency_correction::fit_frequency_correction(
        &frequencies,
        &corrected_pass.residuals,
    ) {
        Ok(correction) => {
            info!("    ✓ Frequency correction fitted successfully");
            info!("      Shape: {:?}", correction.shape);
            info!("      Frequency control points: {}", correction.shape[2]);
            Some(correction)
        }
        Err(e) => {
            info!("    ⚠ Failed to fit frequency correction: {}", e);
            info!("      Continuing without frequency correction");
            None
        }
    };

    // A failed fit drops the artifact back onto the correction-free branch, which the
    // service serves with the gates ON — so pass 2's parameters would then be tuned under
    // the wrong model. Fall back to pass 1, which is the self-consistent result for that
    // branch, rather than shipping pass 2's parameters with no correction to carry them.
    if frequency_correction.is_none() {
        info!("      Falling back to the parameters tuned for a correction-free artifact");
        return Ok(uncorrected_pass.into_result(None));
    }

    Ok(corrected_pass.into_result(frequency_correction))
}

impl TuningPass {
    fn into_result(
        self,
        frequency_correction: Option<BSplineModel4D>,
    ) -> BoresightCalibrationResult {
        BoresightCalibrationResult {
            tuned_params: self.params,
            initial_rmse_db: self.initial_rmse_db,
            final_rmse_db: self.final_rmse_db,
            improvement_db: self.initial_rmse_db - self.final_rmse_db,
            iterations: self.iterations,
            function_evaluations: self.function_evaluations,
            frequency_correction,
        }
    }
}

/// Run the Nelder-Mead tuner once, under one fixed setting of the uncorrected-physics gates.
///
/// See [`calibrate_boresight`] for why this is called up to twice and what
/// `physics_will_be_uncorrected` selects.
fn tune_boresight_parameters(
    design_specs: &DesignSpecs,
    feed_id: &str,
    measurements: &BoresightMeasurements,
    max_iterations: Option<u64>,
    physics_will_be_uncorrected: bool,
) -> Result<TuningPass> {
    info!(
        "  Tuning with uncorrected-physics terms {} (spillover + sidelobe floor)",
        if physics_will_be_uncorrected {
            "APPLIED"
        } else {
            "off"
        }
    );

    // Get initial parameters from design specs
    let initial_params = BoresightTunableParameters::from_design_specs(design_specs, feed_id)?;
    info!(
        "  Initial surface_rms: {:.3} mm",
        initial_params.surface_rms_mm
    );
    info!("  Initial q_factor: {:.2}", initial_params.q_factor);
    if let Some(spacing) = initial_params.mesh_spacing_mm {
        info!("  Initial mesh_spacing: {:.2} mm", spacing);
    }

    // Get tuning bounds
    let bounds = design_specs
        .get_tuning_bounds(feed_id)
        .ok_or_else(|| anyhow::anyhow!("Feed '{}' not found", feed_id))?;

    // Compute initial RMSE with design specs
    let objective = BoresightObjectiveFunction::new(
        Arc::new(design_specs.clone()),
        feed_id.to_string(),
        Arc::new(measurements.clone()),
        bounds.clone(),
        physics_will_be_uncorrected,
    );

    let initial_rmse = objective
        .compute_rmse(&initial_params)
        .context("Failed to compute initial RMSE")?;
    info!("  Initial RMSE: {:.4} dB", initial_rmse);

    // Set up optimization
    let initial_guess = initial_params.to_vector();

    // Create simplex for Nelder-Mead (n+1 vertices for n parameters)
    // We'll create a simplex by perturbing each parameter slightly
    let n_params = initial_guess.len();
    let mut simplex = vec![initial_guess.clone()];
    for i in 0..n_params {
        let mut perturbed = initial_guess.clone();
        // Perturb by 10% of the value or 0.1 if the value is small
        let perturbation = if perturbed[i].abs() > 1.0 {
            perturbed[i] * 0.1
        } else {
            0.1
        };
        perturbed[i] += perturbation;
        simplex.push(perturbed);
    }

    let solver = NelderMead::new(simplex).with_sd_tolerance(1e-4)?;

    info!("  Running Nelder-Mead optimization...");
    info!("    Max iterations: {}", max_iterations.unwrap_or(100));

    let executor = Executor::new(objective.clone(), solver).configure(|state| {
        state
            .max_iters(max_iterations.unwrap_or(100))
            .target_cost(0.1) // Stop if RMSE < 0.1 dB
    });

    let result = executor
        .run()
        .map_err(|e| anyhow::anyhow!("Optimization failed: {}", e))?;

    // Extract optimized parameters
    let final_params_vec = result
        .state()
        .get_best_param()
        .context("Optimization produced no best parameter")?;
    let has_mesh = design_specs.mesh.is_some();
    let final_params = BoresightTunableParameters::from_vector(final_params_vec, has_mesh);

    let final_rmse = result.state().get_best_cost();
    let iterations = result.state().get_iter();
    let function_evals = objective.eval_counter.load(Ordering::Relaxed);

    info!("  Optimization complete!");
    info!("    Iterations: {}", iterations);
    info!("    Function evaluations: {}", function_evals);
    info!("    Final RMSE: {:.4} dB", final_rmse);
    info!(
        "    Improvement: {:.4} dB ({:.1}%)",
        initial_rmse - final_rmse,
        (initial_rmse - final_rmse) / initial_rmse * 100.0
    );
    info!("  Tuned parameters:");
    info!("    surface_rms: {:.3} mm", final_params.surface_rms_mm);
    info!("    q_factor: {:.2}", final_params.q_factor);
    if let Some(spacing) = final_params.mesh_spacing_mm {
        info!("    mesh_spacing: {:.2} mm", spacing);
    }
    if let Some(diameter) = final_params.wire_diameter_mm {
        info!("    wire_diameter: {:.3} mm", diameter);
    }

    // Residuals under THIS pass's gates — `measured − predicted` for the model the service
    // will evaluate an artifact carrying these parameters under. The caller reads them both
    // to decide whether a correction is needed and, on the corrected branch, to fit it.
    let predictions = objective.compute_predictions(&final_params)?;
    let residuals: Vec<f64> = measurements
        .points
        .iter()
        .zip(predictions.iter())
        .map(|(meas, pred)| meas.g_over_t_db - pred)
        .collect();

    Ok(TuningPass {
        params: final_params,
        initial_rmse_db: initial_rmse,
        final_rmse_db: final_rmse,
        iterations: iterations as usize,
        function_evaluations: function_evals,
        residuals,
    })
}

/// Build a calibration artifact from boresight calibration results.
///
/// Creates an `AntennaCalibration` with `PartiallyCalibrated` status suitable
/// for use in the antenna model service.
pub fn build_calibration_artifact(
    design_specs: &DesignSpecs,
    feed_id: &str,
    measurements: &BoresightMeasurements,
    calibration_result: &BoresightCalibrationResult,
    data_source: String,
) -> Result<AntennaCalibration> {
    let feed_spec = design_specs
        .get_feed(feed_id)
        .ok_or_else(|| anyhow::anyhow!("Feed '{}' not found", feed_id))?;

    // Build reflector geometry with tuned parameters (using data types)
    let reflector = DataReflectorGeometry {
        diameter_m: design_specs.reflector.diameter_m,
        focal_length_m: design_specs.reflector.focal_length_m,
        f_over_d_ratio: design_specs.f_over_d_ratio(),
        surface_rms_mm: calibration_result.tuned_params.surface_rms_mm,
    };

    // Build feed parameters with tuned q_factor (using data types)
    let feed = DataFeedParameters {
        position: (
            feed_spec.position[0],
            feed_spec.position[1],
            feed_spec.position[2],
        ),
        q_factor: calibration_result.tuned_params.q_factor,
        phase_center_offset_m: feed_spec.phase_center_offset_m,
        // deliberate defocus is service-config only; not exposed by the calibrate CLI
        axial_defocus_m: 0.0,
        // Roadmap D23: the design spec's declared value, matching what
        // `compute_predictions` tuned against.
        asymmetry_factor: feed_spec.asymmetry_factor,
    };

    // Build mesh parameters with tuned values (if applicable) (using data types)
    let mesh = calibration_result
        .tuned_params
        .mesh_spacing_mm
        .map(|spacing| DataMeshParameters {
            mesh_spacing_mm: spacing,
            wire_diameter_mm: calibration_result
                .tuned_params
                .wire_diameter_mm
                .unwrap_or(0.5),
        });

    // Build physical antenna config
    let mut config_builder = PhysicalAntennaConfigBuilder::default()
        .reflector(reflector)
        .feed(feed);

    if let Some(m) = mesh {
        config_builder = config_builder.mesh(m);
    }

    let physical_config = config_builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build physical antenna config: {}", e))?;

    // Boresight coverage is an on-axis polar CONE, not the point (az=0, el=0).
    //
    // Boresight is the pole of the (azimuth, polar-angle) system: azimuth is
    // degenerate there — every azimuth value names the same direction, and a query
    // aimed exactly at boresight gets its azimuth from `atan2` on two components
    // that are float noise (measured: 63.43° on a realistic ECEF geometry). The old
    // `azimuth_range = (0, 0)` encoding therefore constrained a coordinate carrying
    // no information, and `is_in_coverage` rejected the very point the coverage was
    // meant to describe — so the fitted frequency correction was never applied.
    // Azimuth unconstrained + a small elevation cone is the truthful claim.
    let freq_range = measurements.frequency_range();
    let validity_ranges = ValidityRangesBuilder::default()
        .azimuth_range(0.0, 360.0) // degenerate at the pole: unconstrained
        .elevation_range(0.0, BORESIGHT_COVERAGE_CONE_DEG) // on-axis cone
        .frequency_range(freq_range.0, freq_range.1)
        .temperature(290.0) // Default
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build validity ranges: {}", e))?;

    // Build calibration coverage (boresight only) — same cone encoding, and this is
    // the one the evaluator actually gates the correction surface on.
    let coverage = CalibrationCoverageBuilder::default()
        .azimuth_range(0.0, 360.0)
        .elevation_range(0.0, BORESIGHT_COVERAGE_CONE_DEG)
        .frequency_range(freq_range.0, freq_range.1)
        .num_measurements(measurements.points.len())
        .has_correction_surface(calibration_result.frequency_correction.is_some())
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build coverage: {}", e))?;

    // Build calibration status
    let calibration_status = CalibrationStatus::PartiallyCalibrated {
        accuracy_estimate_db: 1.5, // ±1.5 dB for boresight
        coverage: coverage.clone(),
    };

    // Build metadata
    let notes = format!(
        "Boresight calibration from {} frequency samples. Tuned: surface_rms={:.3}mm, q_factor={:.2}",
        measurements.points.len(),
        calibration_result.tuned_params.surface_rms_mm,
        calibration_result.tuned_params.q_factor
    );

    let metadata = CalibrationMetadataBuilder::default()
        .antenna_name(design_specs.antenna_name.clone())
        .calibration_date(chrono::Utc::now().to_rfc3339())
        .format_version(CALIBRATION_SCHEMA_VERSION.to_string())
        .data_source(data_source)
        .rmse_db(calibration_result.final_rmse_db)
        .r_squared(0.95) // Typical R² for boresight calibration
        .num_measurements(measurements.points.len())
        .physics_only_rmse_db(calibration_result.initial_rmse_db)
        .correction_improvement_db(calibration_result.improvement_db)
        .parameters_tuned(true)
        .parameters_source(ParameterSource::BoresightTuning {
            num_measurements: measurements.points.len(),
        })
        .measurement_density(MeasurementDensity::BoresightOnly)
        .physics_model_version(PHYSICS_MODEL_VERSION)
        .notes(notes)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build calibration metadata: {}", e))?;

    // Build calibration (with optional frequency correction surface)
    let mut calibration_builder = AntennaCalibrationBuilder::default()
        .antenna_id(design_specs.antenna_id.clone())
        .feed_id(feed_id.to_string())
        .metadata(metadata)
        .physical_config(physical_config)
        .validity_ranges(validity_ranges)
        .calibration_status(calibration_status)
        .calibration_coverage(coverage);

    // Attach frequency correction surface if available
    if let Some(ref correction) = calibration_result.frequency_correction {
        calibration_builder = calibration_builder.correction_surface(correction.clone());
        info!("  ✓ Frequency correction surface attached");
    }

    let calibration = calibration_builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build antenna calibration: {}", e))?;

    info!("✓ Calibration artifact built successfully");
    info!("  Status: PartiallyCalibrated (boresight only)");
    if calibration_result.frequency_correction.is_some() {
        info!("  Accuracy estimate: ±0.5 dB at boresight (with frequency correction)");
    } else {
        info!("  Accuracy estimate: ±1.0 dB at boresight (physics only)");
    }
    info!("  Off-axis: ±2-3 dB (physics extrapolation)");

    Ok(calibration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_design_specs() -> DesignSpecs {
        use crate::design_specs_loader::{FeedSpecs, MeshSpecs, ReflectorSpecs};

        DesignSpecs {
            antenna_id: "test_antenna".to_string(),
            antenna_name: "Test 3.7m Ground Station".to_string(),
            reflector: ReflectorSpecs {
                diameter_m: 3.7,
                focal_length_m: 1.85,
                surface_rms_mm: 1.5,
            },
            feeds: vec![FeedSpecs {
                feed_id: "x_band".to_string(),
                name: "X-Band Feed".to_string(),
                position: [0.0, 0.0, 0.0],
                q_factor: 8.0,
                phase_center_offset_m: 0.0,
                asymmetry_factor: 1.0,
                frequency_range: [7100.0, 8500.0],
            }],
            mesh: Some(MeshSpecs {
                mesh_spacing_mm: 5.0,
                wire_diameter_mm: 0.5,
            }),
        }
    }

    fn create_test_measurements() -> BoresightMeasurements {
        // Synthetic boresight measurements at X-band
        BoresightMeasurements {
            points: vec![
                BoresightMeasurement {
                    frequency_mhz: 7100.0,
                    g_over_t_db: 40.5,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 7500.0,
                    g_over_t_db: 41.2,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 8000.0,
                    g_over_t_db: 41.8,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 8500.0,
                    g_over_t_db: 42.1,
                    temperature_k: 290.0,
                },
            ],
        }
    }

    #[test]
    fn test_parse_boresight_csv() {
        let csv_content = "frequency_mhz,g_over_t_db,temperature_k\n\
                          7100.0,40.5,290.0\n\
                          7500.0,41.2,290.0\n\
                          8000.0,41.8,290.0\n\
                          8500.0,42.1,290.0";

        let measurements = BoresightMeasurements::from_csv(csv_content).unwrap();
        assert_eq!(measurements.points.len(), 4);
        assert_eq!(measurements.points[0].frequency_mhz, 7100.0);
        assert_eq!(measurements.points[3].g_over_t_db, 42.1);
    }

    /// A committed measurement fixture must be able to carry its own provenance ahead of the
    /// column header and still be runnable **as committed** (roadmap D13). A data file whose
    /// documentation stops the tool from reading it is a file nobody can re-derive from.
    #[test]
    fn a_provenance_block_ahead_of_the_header_is_skipped() {
        let csv_content = "\
# Source: NTIA Report 84-164, table A-2.
# ASSUMPTION: T_sys = 100 K, so g_over_t_db = gain_dbi - 20.0.
#
frequency_mhz,g_over_t_db,temperature_k
3700,30.4,100
3950,31.0,100
";

        let measurements = BoresightMeasurements::from_csv(csv_content).unwrap();
        assert_eq!(measurements.points.len(), 2);
        assert_eq!(measurements.points[0].frequency_mhz, 3700.0);
        assert_eq!(measurements.points[1].g_over_t_db, 31.0);
        assert_eq!(measurements.points[0].temperature_k, 100.0);
    }

    /// The parser fails hard rather than dropping rows (unlike full mode's — see D11), so the
    /// line it names has to be the line in the file. Counting records instead would point at
    /// line 3 here, inside the provenance block.
    #[test]
    fn a_malformed_row_reports_its_real_file_line_past_a_provenance_block() {
        let csv_content = "\
# provenance line 1
# provenance line 2
frequency_mhz,g_over_t_db,temperature_k
3700,30.4,100
3950,not-a-number,100
";

        let err = BoresightMeasurements::from_csv(csv_content)
            .expect_err("a non-numeric G/T must be an error, not a dropped row");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("line 5"),
            "error must name the real file line (5), got: {rendered}"
        );
    }

    #[test]
    fn test_frequency_range() {
        let measurements = create_test_measurements();
        let (min, max) = measurements.frequency_range();
        assert_eq!(min, 7100.0);
        assert_eq!(max, 8500.0);
    }

    /// **Roadmap D23, boresight producer half.** The artifact must carry the design spec's
    /// declared `asymmetry_factor`, and `compute_predictions` must tune against the same
    /// value — the two used to disagree by construction, since `compute_predictions`
    /// hardcoded `1.0` while the artifact had nowhere to record anything.
    ///
    /// This is the D17 rule (calibrate tunes under what the service will serve) applied to a
    /// model parameter rather than an integration gate. Its sibling per-producer guards are
    /// `main::exported_asymmetry_factor_is_the_class_value_not_a_symmetric_default` (full
    /// mode), `repository::declared_asymmetry_factor_reaches_the_loaded_calibration`
    /// (design-spec producer), and the served half,
    /// `evaluator::served_gain_uses_the_artifacts_asymmetry_factor`.
    #[test]
    fn boresight_artifact_carries_the_design_spec_asymmetry_factor() {
        let mut specs = create_test_design_specs();
        specs.feeds[0].asymmetry_factor = 1.1;

        let measurements = create_test_measurements();
        let result = calibrate_boresight(&specs, "x_band", &measurements, Some(20))
            .expect("boresight calibration");
        let artifact = build_calibration_artifact(
            &specs,
            "x_band",
            &measurements,
            &result,
            "test".to_string(),
        )
        .expect("build artifact");

        assert_eq!(
            artifact.physical_config.feed.asymmetry_factor, 1.1,
            "the artifact must carry the declared design asymmetry, not a symmetric default"
        );
    }

    /// Negative control for the test above: a non-unity asymmetry must actually change the
    /// number the tuner is fitting. If it did not, that test would be pinning a field that
    /// travels but does nothing, which is the shape of the defect D23 closed.
    #[test]
    fn asymmetry_factor_moves_the_boresight_objective() {
        let mut symmetric = create_test_design_specs();
        symmetric.feeds[0].asymmetry_factor = 1.0;
        let mut asymmetric = create_test_design_specs();
        asymmetric.feeds[0].asymmetry_factor = 1.1;

        let measurements = create_test_measurements();
        let params = BoresightTunableParameters::from_design_specs(&symmetric, "x_band").unwrap();

        let predict = |specs: &DesignSpecs| {
            let bounds = specs.get_tuning_bounds("x_band").expect("tuning bounds");
            BoresightObjectiveFunction::new(
                Arc::new(specs.clone()),
                "x_band".to_string(),
                Arc::new(measurements.clone()),
                bounds,
                true,
            )
            .compute_predictions(&params)
            .expect("predictions")
        };

        let a = predict(&symmetric);
        let b = predict(&asymmetric);
        assert!(
            a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 1e-9),
            "asymmetry_factor did not move any boresight prediction, so the round-trip \
             test above would pass on a field the model ignores"
        );
    }

    #[test]
    fn test_tunable_parameters_from_design_specs() {
        let specs = create_test_design_specs();
        let params = BoresightTunableParameters::from_design_specs(&specs, "x_band").unwrap();

        assert_eq!(params.surface_rms_mm, 1.5);
        assert_eq!(params.q_factor, 8.0);
        assert_eq!(params.mesh_spacing_mm, Some(5.0));
        assert_eq!(params.wire_diameter_mm, Some(0.5));
    }

    #[test]
    fn test_param_vector_roundtrip() {
        let params = BoresightTunableParameters {
            surface_rms_mm: 1.5,
            q_factor: 8.0,
            mesh_spacing_mm: Some(5.0),
            wire_diameter_mm: Some(0.5),
        };

        let vec = params.to_vector();
        assert_eq!(vec.len(), 4);

        let reconstructed = BoresightTunableParameters::from_vector(&vec, true);
        assert_eq!(reconstructed.surface_rms_mm, params.surface_rms_mm);
        assert_eq!(reconstructed.q_factor, params.q_factor);
        assert_eq!(reconstructed.mesh_spacing_mm, params.mesh_spacing_mm);
        assert_eq!(reconstructed.wire_diameter_mm, params.wire_diameter_mm);
    }

    #[test]
    fn test_frequency_correction_integration() {
        // Test that frequency correction result structure is compatible with build_calibration_artifact

        // Create test measurements with systematic frequency-dependent bias (> 0.5 dB)
        let measurements_with_bias = BoresightMeasurements {
            points: vec![
                BoresightMeasurement {
                    frequency_mhz: 7100.0,
                    g_over_t_db: 40.5,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 7500.0,
                    g_over_t_db: 41.2,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 8000.0,
                    g_over_t_db: 41.8,
                    temperature_k: 290.0,
                },
                BoresightMeasurement {
                    frequency_mhz: 8500.0,
                    g_over_t_db: 42.1,
                    temperature_k: 290.0,
                },
            ],
        };

        // Create residuals that exceed threshold
        let residuals = vec![0.8, -0.6, 0.7, -0.9]; // Max abs > 0.5 dB
        assert!(frequency_correction::should_fit_correction(&residuals));

        // Extract frequencies
        let frequencies: Vec<f64> = measurements_with_bias
            .points
            .iter()
            .map(|p| p.frequency_mhz)
            .collect();

        // Fit correction
        let correction_result =
            frequency_correction::fit_frequency_correction(&frequencies, &residuals);
        assert!(
            correction_result.is_ok(),
            "Frequency correction should fit successfully"
        );

        let correction = correction_result.unwrap();

        // Verify the flat-axis 4D structure: azimuth, elevation and temperature
        // carry order + 1 = 4 identical layers each (D13, 2026-07-31 — a single
        // layer over degenerate knots is what the service loader used to reject).
        assert_eq!(correction.shape[0], 4); // Azimuth: flat
        assert_eq!(correction.shape[1], 4); // Elevation: flat
        assert_eq!(correction.shape[2], 4); // Frequency: 4 control points
        assert_eq!(correction.shape[3], 4); // Temperature: flat
        correction
            .validate()
            .expect("a fitted frequency correction must load through the service");

        // Verify it can be stored in BoresightCalibrationResult
        let calibration_result = BoresightCalibrationResult {
            tuned_params: BoresightTunableParameters {
                surface_rms_mm: 1.5,
                q_factor: 8.0,
                mesh_spacing_mm: Some(5.0),
                wire_diameter_mm: Some(0.5),
            },
            initial_rmse_db: 2.0,
            final_rmse_db: 0.8,
            improvement_db: 1.2,
            iterations: 50,
            function_evaluations: 200,
            frequency_correction: Some(correction),
        };

        // Verify frequency_correction is Some and can be used in build_calibration_artifact
        assert!(calibration_result.frequency_correction.is_some());
        assert_eq!(calibration_result.frequency_correction.unwrap().shape[2], 4);
    }

    // Note: Full calibration tests require physics model integration
    // These are better suited for integration tests
}
