//! Antenna Calibration Tool CLI
//!
//! Command-line interface for calibrating antenna models from measurement data.
//! Supports end-to-end workflow from measurement parsing to artifact generation.

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use calibrate::artifact_export::{
    export_full_calibration, write_calibration_artifact, ExportPhysicalParams,
};
use calibrate::{
    build_calibration_artifact,
    // Boresight calibration imports
    calibrate_boresight,
    export_metadata_json,
    export_validation_json,
    fit_correction_surface,
    parse_measurements,
    tune_parameters,
    validate_calibration,
    AntennaClassRegistry,
    ArtifactMetadata,
    BoresightMeasurements,
    CorrectionSurfaceParams,
    DesignSpecs,
    MeasurementPoint,
    TunableParameters,
    TuningMode,
    ValidationConfig,
};

use antenna_model::model::{
    compute_g_over_t, AntennaConfigurationBuilder, FeedParametersBuilder, IntegrationParams,
    MeshParametersBuilder, ReflectorGeometryBuilder,
};

/// Antenna Calibration Tool
///
/// Generate calibration artifacts from measurement data for antenna models.
#[derive(Parser, Debug)]
#[command(name = "calibrate")]
#[command(version = "0.1.0")]
#[command(about = "Antenna calibration tool - generate calibration artifacts from measurements", long_about = None)]
struct Args {
    /// Calibration mode: full or boresight
    ///
    /// - full: Full grid calibration from dense measurements (default)
    /// - boresight: Boresight-only calibration from frequency sweep at az=0, el=0
    #[arg(long, default_value = "full")]
    calibration_mode: String,

    /// Input measurement CSV file path (or S3 URL)
    ///
    /// CSV format depends on calibration mode:
    /// - full: e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
    /// - boresight: frequency_mhz,g_over_t_db,temperature_k
    #[arg(short, long)]
    input: PathBuf,

    /// Output calibration artifact path
    ///
    /// Binary artifact file that will be generated (typically .bin extension)
    #[arg(short, long)]
    output: PathBuf,

    /// Antenna identifier (unique ID for this specific antenna)
    #[arg(short, long)]
    antenna_id: String,

    /// Feed identifier (e.g., "x_band", "s_band")
    ///
    /// Required for boresight calibration mode
    #[arg(long)]
    feed_id: Option<String>,

    /// Antenna name (human-readable description)
    #[arg(short = 'n', long, default_value = "Untitled Antenna")]
    antenna_name: String,

    /// Design specifications file path (YAML)
    ///
    /// Required for boresight calibration mode. Provides initial parameter estimates.
    #[arg(long)]
    design_specs: Option<PathBuf>,

    /// Antenna class name (e.g., "DSN_34m", "GroundStation_13m")
    ///
    /// References shared parameters from antenna_classes.yaml
    /// Only used for full calibration mode
    #[arg(short = 'c', long, default_value = "DSN_34m")]
    antenna_class: String,

    /// Enable parameter tuning (optimizes 2-3 physical parameters)
    ///
    /// If not specified, uses nominal class parameters without tuning
    /// Only applicable to full calibration mode
    #[arg(short = 't', long)]
    tune_parameters: bool,

    /// Tuning mode: surface-only, surface-and-mesh, or all
    ///
    /// Only applicable to full calibration mode
    #[arg(long, default_value = "surface-only")]
    tuning_mode: String,

    /// Run cross-validation after fitting
    #[arg(long)]
    validate: bool,

    /// Generate validation report JSON file
    #[arg(short = 'r', long)]
    report: Option<PathBuf>,

    /// Generate metadata JSON file
    #[arg(short = 'm', long)]
    metadata: Option<PathBuf>,

    /// Path to antenna classes definition file
    ///
    /// Only used for full calibration mode
    #[arg(long, default_value = "calibrate/antenna_classes.yaml")]
    classes_file: PathBuf,

    /// Verbose logging output
    #[arg(short, long)]
    verbose: bool,

    /// Number of cross-validation folds (if --validate is enabled)
    #[arg(long, default_value = "5")]
    cv_folds: usize,

    /// Maximum iterations for parameter tuning
    #[arg(long, default_value = "100")]
    max_tuning_iterations: u64,
}

/// Parameters the shipped correction surface is fitted with (full-mode step 5).
///
/// Deliberately sparser and more strongly regularized than
/// [`CorrectionSurfaceParams::default`]: this is the model family the artifact ships, and
/// [`validation_config`] must score *this* family, not the default one.
fn surface_fitting_params(validate: bool, cv_folds: usize) -> CorrectionSurfaceParams {
    CorrectionSurfaceParams {
        spline_order: 4,
        num_knots_frequency: 4,
        num_knots_econe: 6,
        num_knots_eclock: 8,
        regularization: 1e-3,
        adaptive_knots: true,
        cross_validation_folds: if validate { cv_folds } else { 0 },
        min_knot_spacing_frequency: 50.0, // 50 MHz minimum spacing
        min_knot_spacing_econe: 2.0,      // 2 degrees minimum spacing
        min_knot_spacing_eclock: 5.0,     // 5 degrees minimum spacing
    }
}

/// Validation settings for full-mode step 6.
///
/// `correction_params` **must** be the params the surface being validated was fitted with.
/// Passing `CorrectionSurfaceParams::default()` here (the pre-D10 behavior) made every
/// cross-validation fold refit a markedly more flexible surface — roughly double the knots
/// at 1000× weaker regularization — so the reported CV RMSE described a model family more
/// prone to overfit than the artifact being blessed.
///
/// `num_folds = 0` disables cross-validation only; every other check in step 6 (RMSE,
/// main-lobe and first-sidelobe statistics, outliers, band analysis) runs regardless.
/// Gating it on `--validate` matches the flag's documented meaning ("Run cross-validation
/// after fitting") and step 5, which already honors it — before this, `--cv-folds`' clap
/// default of 5 meant every full-mode run cross-validated whether asked to or not.
fn validation_config(
    validate: bool,
    cv_folds: usize,
    surface_params: &CorrectionSurfaceParams,
) -> ValidationConfig {
    ValidationConfig {
        num_folds: if validate { cv_folds } else { 0 },
        main_lobe_beamwidths: 1.0,
        first_sidelobe_max_deg: 5.0,
        frequency_bands: vec![], // Use default bands
        main_lobe_target_db: 1.0,
        first_sidelobe_target_db: 1.0,
        outlier_threshold_db: 3.0,
        correction_params: surface_params.clone(),
    }
}

/// The physical parameters stamped into a full-mode artifact.
///
/// Tuned values where the tuner produced one, the class nominal otherwise — the same
/// `unwrap_or` chain [`compute_model_predictions`] uses, so the artifact describes the
/// configuration the residuals were fitted against.
///
/// Split out of `run_calibration` so the one field whose *frame* is not recoverable from
/// its value — `feed_position_m` — can be pinned by a unit test (roadmap C13).
///
/// # What it cannot carry: `asymmetry_factor` (roadmap D23)
///
/// [`compute_model_predictions`] fits residuals against a model built with the class's
/// `feed.asymmetry_factor`, but [`ExportPhysicalParams`] — and the `FeedParameters` it writes
/// into the artifact — have no such field, so the service rebuilds the feed with the model
/// default of 1.0. On a class with a non-unity factor the correction surface is therefore
/// fitted against an **asymmetric** model and applied on top of a **symmetric** one. Two
/// shipped classes are affected (`GroundStation_13m` at 1.05, `UHF_Array_Element` at 1.1).
/// It is the same calibrate/service seam as C13 two lines below, and it cannot be closed here:
/// adding the field changes the postcard layout, so it needs a schema **and** container
/// version bump of its own. Filed as **D23**; the warning below is the interim honesty.
fn export_physical_params(
    class: &calibrate::AntennaClass,
    tunable_params: &TunableParameters,
) -> ExportPhysicalParams {
    if class.feed.asymmetry_factor != 1.0 {
        warn!(
            asymmetry_factor = class.feed.asymmetry_factor,
            class = %class.class_id,
            "this antenna class has a non-symmetric feed, but the artifact format has no field \
             for it: the residuals below are fitted against an asymmetric illumination and the \
             service will serve a symmetric one. See roadmap unit D23 — until it lands, prefer \
             a class with asymmetry_factor = 1.0 for any artifact that will actually be served."
        );
    }

    let focal_length_m = class.geometry.diameter_m * class.geometry.f_over_d;
    let surface_rms_mm = tunable_params
        .surface_rms_mm
        .unwrap_or(class.surface.rms_mm);
    let mesh_spacing_mm = tunable_params
        .mesh_spacing_mm
        .unwrap_or(class.mesh.spacing_mm);
    let wire_diameter_mm = tunable_params
        .mesh_wire_diameter_mm
        .unwrap_or(class.mesh.wire_diameter_mm);

    ExportPhysicalParams {
        diameter_m: class.geometry.diameter_m,
        focal_length_m,
        f_over_d_ratio: class.geometry.f_over_d,
        surface_rms_mm,
        // On-axis configuration: feed at the focal point.
        //
        // `FeedParameters.position` is the feed's **design offset from the focal point**,
        // not its vertex-origin position — see the field's doc comment in
        // `antenna_model::data::types`. "At the focus" is therefore the origin, and this
        // must NOT be `(0, 0, focal_length_m)`: the service adds this offset to a steering
        // position that is *already* vertex-origin (`compute_feed_position_from_pointing`
        // → `to_feed_position_with_bdf` returns `(dx, dy, f + dz)`), so writing the focal
        // length here placed a full-mode artifact's feed at z ≈ 2f. Measured cost on the
        // roadmap D14 fixture (1.22 m, f/D 0.375, 12.1 GHz): boresight gain 41.09 → 13.83
        // dBi, a 27.3 dB phantom axial defocus on every request. Roadmap unit **C13**,
        // fixed 2026-08-02 under D14 — the unit that first served a full-mode artifact.
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: class.feed.q_factor,
        phase_center_offset_m: 0.0,
        mesh: Some((mesh_spacing_mm, wire_diameter_mm)),
    }
}

/// Compute physics-model G/T predictions for all measurement points.
fn compute_model_predictions(
    measurements: &[MeasurementPoint],
    antenna_class: &calibrate::AntennaClass,
    tunable_params: &TunableParameters,
) -> Result<Vec<f64>> {
    info!(
        "Computing physics model predictions for {} points...",
        measurements.len()
    );

    // Get effective parameters
    let surface_rms_mm = tunable_params
        .surface_rms_mm
        .unwrap_or(antenna_class.surface.rms_mm);
    let mesh_spacing_mm = tunable_params
        .mesh_spacing_mm
        .unwrap_or(antenna_class.mesh.spacing_mm);
    let wire_diameter_mm = tunable_params
        .mesh_wire_diameter_mm
        .unwrap_or(antenna_class.mesh.wire_diameter_mm);

    // Build reflector geometry
    let reflector = ReflectorGeometryBuilder::default()
        .diameter(antenna_class.geometry.diameter_m)
        .focal_length(antenna_class.geometry.diameter_m * antenna_class.geometry.f_over_d)
        .surface_rms(surface_rms_mm / 1000.0) // mm to m
        .build()
        .context("Failed to build reflector geometry")?;

    // Build feed parameters (at focal point for on-axis configuration)
    let focal_length = antenna_class.geometry.diameter_m * antenna_class.geometry.f_over_d;
    let feed = FeedParametersBuilder::default()
        .at_focus(focal_length)
        .q_factor(antenna_class.feed.q_factor)
        .phase_center_offset(antenna_class.feed.phase_center_offset_wavelengths)
        .asymmetry_factor(antenna_class.feed.asymmetry_factor)
        .build()
        .context("Failed to build feed parameters")?;

    // Build mesh parameters
    let mesh = MeshParametersBuilder::default()
        .spacing(mesh_spacing_mm / 1000.0) // mm to m
        .wire_diameter(wire_diameter_mm / 1000.0) // mm to m
        .build()
        .context("Failed to build mesh parameters")?;

    // Build complete configuration
    let physics_config = AntennaConfigurationBuilder::default()
        .id(&antenna_class.class_id)
        .name(&antenna_class.description)
        .reflector(reflector)
        .feed(feed)
        .mesh(mesh)
        .build()
        .context("Failed to build antenna configuration")?;

    // Integration parameters (default settings for good accuracy).
    //
    // The uncorrected-physics gates are OFF because full mode always attaches a correction
    // surface — `run_full_calibration` propagates a fit failure rather than shipping an
    // artifact without one — so the service will always evaluate a full-mode artifact with
    // these terms off (`AntennaCalibration::physics_is_uncorrected()` is false for it).
    // Stated through the shared setter rather than left implicit in `default()`, so this
    // reads as the decision it is; see roadmap D17 for what happens when the two sides of
    // that decision disagree. `false` here is load-bearing on the invariant above: if full
    // mode ever learns to ship a correction-free artifact, this has to become conditional
    // the way `calibrate_boresight` already is.
    let integration_params = IntegrationParams::default().with_uncorrected_physics_gates(false);

    // Compute predictions for all measurement points
    let mut predictions = Vec::with_capacity(measurements.len());
    let temperature_k = antenna_class.system_noise_temperature_k;

    for (idx, point) in measurements.iter().enumerate() {
        if idx % 100 == 0 && idx > 0 {
            debug!("  Computed {}/{} predictions", idx, measurements.len());
        }

        // Convert E-clock/E-cone to far-field coordinates (in radians)
        // E-cone is the polar angle (theta) and E-clock is the azimuthal angle (phi)
        let theta = point.e_cone_deg.to_radians();
        let phi = point.e_clock_deg.to_radians();

        // Compute G/T from physics model
        let frequency_hz = point.frequency_mhz * 1e6;
        let predicted_g_over_t = compute_g_over_t(
            theta,
            phi,
            &physics_config,
            frequency_hz,
            temperature_k,
            &integration_params,
        )
        .context(format!(
            "Failed to compute G/T for point {}: freq={} MHz, e_cone={}, e_clock={}",
            idx, point.frequency_mhz, point.e_cone_deg, point.e_clock_deg
        ))?;

        predictions.push(predicted_g_over_t);
    }

    info!("  ✓ Computed {} predictions", predictions.len());

    Ok(predictions)
}

/// Boresight calibration workflow
async fn run_boresight_calibration(args: Args) -> Result<()> {
    info!("Starting boresight calibration workflow");
    info!("Antenna ID: {}", args.antenna_id);

    // Validate required parameters
    let feed_id = args
        .feed_id
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--feed-id is required for boresight calibration mode"))?;

    let design_specs_path = args.design_specs.as_ref().ok_or_else(|| {
        anyhow::anyhow!("--design-specs is required for boresight calibration mode")
    })?;

    info!("Feed ID: {}", feed_id);
    info!("Input: {}", args.input.display());
    info!("Design specs: {}", design_specs_path.display());
    info!("Output: {}", args.output.display());

    // Step 1: Load design specifications
    info!("Step 1/4: Loading design specifications...");
    let design_specs = DesignSpecs::load_from_file(design_specs_path)
        .context("Failed to load design specifications")?;

    info!("  ✓ Loaded design specs for {}", design_specs.antenna_name);
    info!(
        "    Diameter: {:.1}m, f/D: {:.4}",
        design_specs.reflector.diameter_m,
        design_specs.f_over_d_ratio()
    );
    info!(
        "    Initial surface RMS: {:.3} mm",
        design_specs.reflector.surface_rms_mm
    );

    // Step 2: Parse boresight measurements
    info!("Step 2/4: Parsing boresight measurements...");
    let csv_content = std::fs::read_to_string(&args.input)
        .with_context(|| format!("Failed to read input file: {}", args.input.display()))?;

    let measurements = BoresightMeasurements::from_csv(&csv_content)
        .context("Failed to parse boresight measurements")?;

    let (freq_min, freq_max) = measurements.frequency_range();
    info!("  ✓ Parsed {} measurements", measurements.points.len());
    info!("  Frequency range: {:.1} - {:.1} MHz", freq_min, freq_max);

    // Step 3: Run boresight calibration (parameter tuning)
    info!("Step 3/4: Running boresight calibration...");
    let calibration_result = calibrate_boresight(
        &design_specs,
        feed_id,
        &measurements,
        Some(args.max_tuning_iterations),
    )
    .context("Boresight calibration failed")?;

    info!("  ✓ Boresight calibration complete");

    // Step 4: Build calibration artifact
    info!("Step 4/4: Building calibration artifact...");
    let data_source = format!("file://{}", args.input.display());

    let calibration = build_calibration_artifact(
        &design_specs,
        feed_id,
        &measurements,
        &calibration_result,
        data_source,
    )
    .context("Failed to build calibration artifact")?;

    // Validate the artifact
    calibration
        .validate()
        .context("Calibration artifact failed validation")?;

    // Serialize and save. Same ANTC container framing as full mode — one writer, so the
    // two producers cannot drift apart on version stamping or CRC (roadmap D2).
    write_calibration_artifact(&calibration, &args.output)
        .context("Failed to write calibration artifact")?;

    let file_size = std::fs::metadata(&args.output)?.len();
    info!(
        "  ✓ Artifact saved: {} ({:.2} KB)",
        args.output.display(),
        file_size as f64 / 1024.0
    );

    // Export metadata if requested
    if let Some(metadata_path) = args.metadata {
        info!("Exporting metadata to JSON...");
        let metadata_json = serde_json::to_string_pretty(&calibration.metadata)?;
        std::fs::write(&metadata_path, metadata_json)?;
        info!("  ✓ Metadata saved: {}", metadata_path.display());
    }

    info!("");
    info!("✓ Boresight calibration workflow complete!");
    info!("");
    info!("Summary:");
    info!("  Antenna ID: {}", args.antenna_id);
    info!("  Feed ID: {}", feed_id);
    info!("  Calibration mode: Boresight (PartiallyCalibrated)");
    info!("  Measurements: {}", measurements.points.len());
    info!("  Frequency range: {:.1} - {:.1} MHz", freq_min, freq_max);
    info!(
        "  Initial RMSE: {:.4} dB (design specs)",
        calibration_result.initial_rmse_db
    );
    info!(
        "  Final RMSE: {:.4} dB (tuned)",
        calibration_result.final_rmse_db
    );
    info!(
        "  Improvement: {:.4} dB ({:.1}%)",
        calibration_result.improvement_db,
        (calibration_result.improvement_db / calibration_result.initial_rmse_db) * 100.0
    );
    info!("  Tuned parameters:");
    info!(
        "    surface_rms: {:.3} mm",
        calibration_result.tuned_params.surface_rms_mm
    );
    info!(
        "    q_factor: {:.2}",
        calibration_result.tuned_params.q_factor
    );
    if let Some(spacing) = calibration_result.tuned_params.mesh_spacing_mm {
        info!("    mesh_spacing: {:.2} mm", spacing);
    }
    info!("  Expected accuracy:");
    info!("    Boresight: ±1 dB");
    info!("    Off-axis: ±2-3 dB (physics extrapolation)");
    info!("    Loss (relative): ±1-2 dB");
    info!("  Output artifact: {}", args.output.display());

    Ok(())
}

/// Full calibration workflow (original)
async fn run_calibration(args: Args) -> Result<()> {
    info!("Starting antenna calibration workflow");
    info!("Antenna ID: {}", args.antenna_id);
    info!("Antenna class: {}", args.antenna_class);
    info!("Input: {}", args.input.display());
    info!("Output: {}", args.output.display());

    // Step 1: Parse measurement data
    info!("Step 1/6: Parsing measurement data...");
    let measurements = parse_measurements(args.input.to_str().context("Invalid input path")?)
        .await
        .context("Failed to parse measurement data")?;

    let estimated_beamwidth_deg = 70.0
        / measurements
            .points
            .first()
            .map(|p| p.frequency_mhz / 1000.0)
            .unwrap_or(1.0); // Estimate beamwidth for 1m diameter antenna
    let quality_report = measurements.quality_report(estimated_beamwidth_deg);

    info!("  ✓ Parsed {} measurements", measurements.points.len());
    info!(
        "  Coverage: {} unique frequencies",
        quality_report.unique_frequencies
    );
    info!(
        "  Frequency range: {:.1} - {:.1} MHz",
        quality_report.frequency_range.0, quality_report.frequency_range.1
    );
    info!(
        "  E-cone range: {:.1} - {:.1} deg",
        quality_report.e_cone_range.0, quality_report.e_cone_range.1
    );
    info!(
        "  Main lobe points: {}, sidelobe points: {}",
        quality_report.main_lobe_points, quality_report.sidelobe_points
    );

    info!(
        "  G/T range: {:.1} - {:.1} dB/K",
        quality_report.g_over_t_range.0, quality_report.g_over_t_range.1
    );

    if quality_report.outlier_count > 0 {
        warn!("  ⚠ Found {} outlier points", quality_report.outlier_count);
    }

    if quality_report.atypical_g_over_t_count > 0 {
        warn!(
            "  ⚠ {} points outside the boresight-typical G/T range [{:.0}, {:.0}] dB/K \
             (expected for off-axis measurements; all are retained and fitted)",
            quality_report.atypical_g_over_t_count,
            calibrate::parser::TYPICAL_G_OVER_T_RANGE_DB.start(),
            calibrate::parser::TYPICAL_G_OVER_T_RANGE_DB.end(),
        );
    }

    // Step 2: Load antenna class definition
    info!("Step 2/6: Loading antenna class definition...");
    let registry = AntennaClassRegistry::load_from_file(&args.classes_file)
        .map_err(|e| anyhow::anyhow!("Failed to load antenna classes: {}", e))?;

    let class = registry
        .get_class(&args.antenna_class)
        .context(format!("Antenna class '{}' not found", args.antenna_class))?;

    info!("  ✓ Loaded class: {}", class.description);
    info!(
        "    Diameter: {:.1}m, f/D: {:.4}",
        class.geometry.diameter_m, class.geometry.f_over_d
    );

    // Step 3: Create antenna configuration with optional tuning
    let mut tunable_params = TunableParameters::default_from_class();

    if args.tune_parameters {
        info!("Step 3/6: Tuning physical parameters...");
        info!(
            "  Running parameter optimization (max {} iterations)...",
            args.max_tuning_iterations
        );

        let tuning_mode = match args.tuning_mode.as_str() {
            "surface-only" => TuningMode::SurfaceRmsOnly,
            "surface-and-mesh" => TuningMode::SurfaceAndMeshSpacing,
            "all" => TuningMode::All,
            _ => {
                warn!(
                    "Unknown tuning mode '{}', using 'surface-only'",
                    args.tuning_mode
                );
                TuningMode::SurfaceRmsOnly
            }
        };

        let tuning_result = tune_parameters(
            class.clone(),
            tunable_params.clone(),
            measurements.clone(),
            tuning_mode,
            Some(args.max_tuning_iterations),
        )?;

        tunable_params = tuning_result.to_tunable_parameters();

        info!("  ✓ Parameter tuning complete");
        info!("    Initial RMSE: {:.4} dB", tuning_result.initial_rmse_db);
        info!("    Final RMSE: {:.4} dB", tuning_result.final_rmse_db);
        info!(
            "    Improvement: {:.4} dB ({:.1}%)",
            tuning_result.improvement_db,
            (tuning_result.improvement_db / tuning_result.initial_rmse_db) * 100.0
        );
        info!("    Iterations: {}", tuning_result.iterations);

        info!(
            "    Tuned surface_rms: {:.3} mm",
            tuning_result.surface_rms_mm
        );
        if let Some(spacing) = tuning_result.mesh_spacing_mm {
            info!("    Tuned mesh_spacing: {:.2} mm", spacing);
        }
        if let Some(diameter) = tuning_result.mesh_wire_diameter_mm {
            info!("    Tuned wire_diameter: {:.3} mm", diameter);
        }
    } else {
        info!("Step 3/6: Using nominal class parameters (no tuning)");
        info!("  ✓ Configuration ready with default parameters");
    }

    // Step 4: Compute model predictions
    info!("Step 4/6: Computing model predictions...");
    let model_predictions =
        compute_model_predictions(&measurements.points, class, &tunable_params)?;

    // Compute initial model-only RMSE
    let model_only_rmse = {
        let squared_errors: f64 = measurements
            .points
            .iter()
            .zip(model_predictions.iter())
            .map(|(m, p)| {
                let error = m.g_over_t_db - p;
                error * error
            })
            .sum();
        (squared_errors / measurements.points.len() as f64).sqrt()
    };

    info!("  ✓ Model predictions computed");
    info!("    Model-only RMSE: {:.4} dB", model_only_rmse);

    // Step 5: Fit correction surface to residuals
    info!("Step 5/6: Fitting correction surface to residuals...");

    let surface_params = surface_fitting_params(args.validate, args.cv_folds);

    let correction_surface =
        fit_correction_surface(&measurements.points, &model_predictions, &surface_params)?;

    info!("  ✓ Correction surface fitted");
    info!("    RMSE: {:.4} dB", correction_surface.fit_stats.rmse_db);
    info!(
        "    Max residual: {:.4} dB",
        correction_surface.fit_stats.max_residual_db
    );
    info!("    R²: {:.6}", correction_surface.fit_stats.r_squared);
    info!(
        "    Improvement: {:.1}%",
        correction_surface.fit_stats.improvement_percent
    );

    // Step 6: Validation
    info!("Step 6/6: Running validation...");

    let validation_config = validation_config(args.validate, args.cv_folds, &surface_params);

    let validation_report = validate_calibration(
        &measurements.points,
        &model_predictions,
        &correction_surface,
        &validation_config,
    )?;

    info!("  ✓ Validation complete");
    info!(
        "    Corrected RMSE: {:.4} dB",
        validation_report.corrected_rmse
    );
    info!(
        "    Main lobe max error: {:.4} dB",
        validation_report.main_lobe_max_error
    );
    info!(
        "    First sidelobe max error: {:.4} dB",
        validation_report.first_sidelobe_max_error
    );
    info!(
        "    Outliers: {} ({:.1}%)",
        validation_report.outliers.len(),
        validation_report.outliers.len() as f64 / measurements.points.len() as f64 * 100.0
    );

    if !validation_report.main_lobe_meets_target {
        warn!(
            "  ⚠ Main lobe accuracy target not met ({:.4} dB > {:.4} dB)",
            validation_report.main_lobe_max_error, validation_config.main_lobe_target_db
        );
    } else {
        info!("  ✓ Main lobe meets accuracy target");
    }

    if !validation_report.first_sidelobe_meets_target {
        warn!(
            "  ⚠ First sidelobe accuracy target not met ({:.4} dB > {:.4} dB)",
            validation_report.first_sidelobe_max_error, validation_config.first_sidelobe_target_db
        );
    } else {
        info!("  ✓ First sidelobe meets accuracy target");
    }

    // Print cross-validation results if available
    if let Some(cv) = &validation_report.cross_validation {
        info!("  Cross-validation results:");
        info!(
            "    Mean RMSE: {:.4} dB (± {:.4} dB)",
            cv.mean_rmse, cv.std_rmse
        );
        info!("    Range: [{:.4}, {:.4}] dB", cv.min_rmse, cv.max_rmse);
    }

    // Generate and save calibration artifact
    info!("Generating calibration artifact...");

    let artifact_metadata = ArtifactMetadata {
        created_at: chrono::Utc::now().to_rfc3339(),
        measurement_source: measurements.source.clone(),
        parameters_tuned: args.tune_parameters,
        num_measurement_points: measurements.points.len(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        notes: Some(format!(
            "Calibrated with class: {}, R²={:.6}",
            class.class_id, correction_surface.fit_stats.r_squared
        )),
        frequency_range: quality_report.frequency_range,
        angular_range: quality_report.e_cone_range,
    };

    // Build a service-loadable AntennaCalibration (4D B-spline correction
    // surface) and write it as the binary artifact. `artifact_metadata` above
    // and `validation_report` only drive the optional `--metadata`/`--report`
    // JSON sidecars below; neither is part of the on-disk binary format.
    let export_physical = export_physical_params(class, &tunable_params);

    let feed_id = args.feed_id.as_deref().unwrap_or("primary");
    let service_calibration = export_full_calibration(
        &args.antenna_id,
        feed_id,
        &args.antenna_name,
        format!("file://{}", args.input.display()),
        &export_physical,
        &correction_surface,
        &measurements.points,
        validation_report.corrected_rmse,
        correction_surface.fit_stats.r_squared,
        model_only_rmse,
        args.tune_parameters,
    )
    .context("Failed to build service-loadable calibration artifact")?;

    service_calibration
        .validate()
        .map_err(|e| anyhow::anyhow!("Service calibration failed validation: {}", e))?;

    write_calibration_artifact(&service_calibration, &args.output)
        .context("Failed to write service calibration artifact")?;

    let file_size = std::fs::metadata(&args.output)?.len();
    info!(
        "  ✓ Artifact saved: {} ({:.2} KB)",
        args.output.display(),
        file_size as f64 / 1024.0
    );

    // Export metadata JSON (optional)
    if let Some(metadata_path) = args.metadata {
        info!("Exporting metadata to JSON...");
        export_metadata_json(&artifact_metadata, &metadata_path)?;
        info!("  ✓ Metadata saved: {}", metadata_path.display());
    }

    // Export validation report (optional)
    if let Some(report_path) = args.report {
        info!("Exporting validation report to JSON...");
        export_validation_json(&validation_report, &report_path)?;
        info!("  ✓ Validation report saved: {}", report_path.display());
    }

    info!("");
    info!("✓ Calibration workflow complete!");
    info!("");
    info!("Summary:");
    info!("  Antenna ID: {}", args.antenna_id);
    info!("  Measurements: {}", measurements.points.len());
    info!(
        "  Parameter tuning: {}",
        if args.tune_parameters { "yes" } else { "no" }
    );
    info!("  Model-only RMSE: {:.4} dB", model_only_rmse);
    info!(
        "  Corrected RMSE: {:.4} dB",
        validation_report.corrected_rmse
    );
    info!(
        "  Improvement: {:.1}%",
        validation_report.rmse_improvement_percent
    );
    info!(
        "  Main lobe target met: {}",
        if validation_report.main_lobe_meets_target {
            "yes"
        } else {
            "no"
        }
    );
    info!(
        "  First sidelobe target met: {}",
        if validation_report.first_sidelobe_meets_target {
            "yes"
        } else {
            "no"
        }
    );
    info!("  Output artifact: {}", args.output.display());

    Ok(())
}

#[tokio::main]
async fn main() {
    // Parse command-line arguments
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("calibrate={},warn", log_level)));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    // Dispatch to appropriate calibration workflow based on mode
    let result = match args.calibration_mode.as_str() {
        "full" => run_calibration(args).await,
        "boresight" => run_boresight_calibration(args).await,
        mode => {
            error!("Unknown calibration mode: {}", mode);
            error!("Valid modes: full, boresight");
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        error!("Calibration failed: {:#}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D10 defect (a): the validation config must carry the params the artifact was
    /// actually fitted with. Before the fix this was `CorrectionSurfaceParams::default()`,
    /// so every fold refit fitted 8/8/12 knots at 1e-6 regularization while the shipped
    /// surface was 4/6/8 at 1e-3.
    #[test]
    fn validation_config_scores_the_surface_that_ships() {
        let surface_params = surface_fitting_params(true, 5);
        let config = validation_config(true, 5, &surface_params);

        assert_eq!(
            config.correction_params.num_knots_frequency,
            surface_params.num_knots_frequency
        );
        assert_eq!(
            config.correction_params.num_knots_econe,
            surface_params.num_knots_econe
        );
        assert_eq!(
            config.correction_params.num_knots_eclock,
            surface_params.num_knots_eclock
        );
        assert_eq!(
            config.correction_params.regularization,
            surface_params.regularization
        );
        assert_eq!(
            config.correction_params.spline_order,
            surface_params.spline_order
        );

        // Guard the specific regression: these are the default's values, not ours.
        let default = CorrectionSurfaceParams::default();
        assert_ne!(
            config.correction_params.num_knots_frequency, default.num_knots_frequency,
            "fixture no longer distinguishes the artifact params from the default"
        );
        assert_ne!(
            config.correction_params.regularization, default.regularization,
            "fixture no longer distinguishes the artifact params from the default"
        );
    }

    /// `--validate` is documented as "Run cross-validation after fitting". Step 5 honors
    /// it; step 6 did not, so every full-mode run cross-validated whether asked or not.
    #[test]
    fn cross_validation_is_gated_on_the_validate_flag() {
        let params = surface_fitting_params(false, 5);
        assert_eq!(
            validation_config(false, 5, &params).num_folds,
            0,
            "without --validate, step 6 must not cross-validate"
        );

        let params = surface_fitting_params(true, 5);
        assert_eq!(
            validation_config(true, 5, &params).num_folds,
            5,
            "with --validate, --cv-folds still sets the fold count"
        );
    }

    /// Gating CV must not disable the rest of step 6.
    #[test]
    fn gating_cross_validation_leaves_the_other_validation_settings_intact() {
        let params = surface_fitting_params(false, 5);
        let ungated = validation_config(false, 5, &params);
        let gated = validation_config(true, 5, &params);

        assert_eq!(ungated.main_lobe_target_db, gated.main_lobe_target_db);
        assert_eq!(
            ungated.first_sidelobe_target_db,
            gated.first_sidelobe_target_db
        );
        assert_eq!(ungated.outlier_threshold_db, gated.outlier_threshold_db);
        assert_eq!(ungated.main_lobe_beamwidths, gated.main_lobe_beamwidths);
        assert_eq!(ungated.first_sidelobe_max_deg, gated.first_sidelobe_max_deg);
    }

    /// **Roadmap C13.** The artifact's feed position is an offset **from the focal
    /// point**, so an on-axis feed is the origin — not `(0, 0, f)`.
    ///
    /// This is the assertion that had no home: the value lived inline in
    /// `run_calibration`, and no test served a full-mode artifact, so a frame that
    /// disagreed with every consumer went unnoticed from the day the exporter was
    /// written. The service adds this offset to an already-vertex-origin steering
    /// position, so the old value put the feed a full focal length behind the focus —
    /// 27.3 dB of boresight gain on the D14 fixture.
    ///
    /// The second assertion is the one with teeth: it fails for *exactly* the old value
    /// on a class whose focal length is non-zero, which a bare `== (0,0,0)` would too,
    /// but it says why, and it keeps working if the fixture class changes.
    #[test]
    fn exported_feed_position_is_focus_relative_not_vertex_relative() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("antenna_classes.yaml");
        let registry = AntennaClassRegistry::load_from_file(&path).expect("load antenna classes");
        let class = registry.get_class("DSN_34m").expect("DSN_34m class");

        let physical = export_physical_params(class, &TunableParameters::default_from_class());

        assert_eq!(
            physical.feed_position_m,
            (0.0, 0.0, 0.0),
            "an on-axis feed's design offset from the focal point is the origin"
        );
        assert_ne!(
            physical.feed_position_m.2, physical.focal_length_m,
            "feed_position_m is being written vertex-relative again (roadmap C13): the \
             service adds it to a vertex-origin steering position, so this places the feed \
             at z = 2f"
        );
        assert!(
            physical.focal_length_m > 1.0,
            "this test only has power on a class with a non-trivial focal length; \
             DSN_34m's is {} m",
            physical.focal_length_m
        );
    }

    /// `--cv-folds N` reaches both the surface fit and the validation fold count, and
    /// cross-validation stays off entirely without `--validate`.
    #[test]
    fn cv_folds_reaches_the_fit_and_the_validator() {
        let with_validate = surface_fitting_params(true, 7);
        assert_eq!(with_validate.cross_validation_folds, 7);
        assert_eq!(validation_config(true, 7, &with_validate).num_folds, 7);

        let without_validate = surface_fitting_params(false, 7);
        assert_eq!(without_validate.cross_validation_folds, 0);
        assert_eq!(validation_config(false, 7, &without_validate).num_folds, 0);
    }
}
