//! Antenna Calibration Tool CLI
//!
//! Command-line interface for calibrating antenna models from measurement data.
//! Supports end-to-end workflow from measurement parsing to artifact generation.

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing::{info, warn, error, debug};
use tracing_subscriber::{fmt, EnvFilter};

use calibrate::{
    parse_measurements, tune_parameters, fit_correction_surface, validate_calibration,
    save_artifact, export_metadata_json, export_validation_json,
    AntennaClassRegistry, AntennaConfiguration, CorrectionSurfaceParams,
    ValidationConfig, TuningMode, TunableParameters, CalibrationArtifact, ArtifactMetadata,
    MeasurementPoint,
};

use antenna_model::model::{
    compute_g_over_t, AntennaConfigurationBuilder,
    FeedParametersBuilder, IntegrationParams,
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
    /// Input measurement CSV file path (or S3 URL)
    ///
    /// CSV format: e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
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

    /// Antenna name (human-readable description)
    #[arg(short = 'n', long, default_value = "Untitled Antenna")]
    antenna_name: String,

    /// Antenna class name (e.g., "DSN_34m", "GroundStation_13m")
    ///
    /// References shared parameters from antenna_classes.yaml
    #[arg(short = 'c', long, default_value = "DSN_34m")]
    antenna_class: String,

    /// Enable parameter tuning (optimizes 2-3 physical parameters)
    ///
    /// If not specified, uses nominal class parameters without tuning
    #[arg(short = 't', long)]
    tune_parameters: bool,

    /// Tuning mode: surface-only, surface-and-mesh, or all
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

/// Compute model predictions for all measurement points
fn compute_model_predictions(
    measurements: &[MeasurementPoint],
    antenna_class: &calibrate::AntennaClass,
    tunable_params: &TunableParameters,
) -> Result<Vec<f64>> {
    info!("Computing physics model predictions for {} points...", measurements.len());

    // Get effective parameters
    let surface_rms_mm = tunable_params.surface_rms_mm.unwrap_or(antenna_class.surface.rms_mm);
    let mesh_spacing_mm = tunable_params.mesh_spacing_mm.unwrap_or(antenna_class.mesh.spacing_mm);
    let wire_diameter_mm = tunable_params.mesh_wire_diameter_mm.unwrap_or(antenna_class.mesh.wire_diameter_mm);

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

    // Integration parameters (default settings for good accuracy)
    let integration_params = IntegrationParams::default();

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
        ).context(format!("Failed to compute G/T for point {}: freq={} MHz, e_cone={}, e_clock={}",
            idx, point.frequency_mhz, point.e_cone_deg, point.e_clock_deg))?;

        predictions.push(predicted_g_over_t);
    }

    info!("  ✓ Computed {} predictions", predictions.len());

    Ok(predictions)
}

/// Main calibration workflow
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

    let estimated_beamwidth_deg = 70.0 / measurements.points.first()
        .map(|p| p.frequency_mhz / 1000.0)
        .unwrap_or(1.0); // Estimate beamwidth for 1m diameter antenna
    let quality_report = measurements.quality_report(estimated_beamwidth_deg);

    info!("  ✓ Parsed {} measurements", measurements.points.len());
    info!("  Coverage: {} unique frequencies",
        quality_report.unique_frequencies
    );
    info!("  Frequency range: {:.1} - {:.1} MHz",
        quality_report.frequency_range.0,
        quality_report.frequency_range.1
    );
    info!("  E-cone range: {:.1} - {:.1} deg",
        quality_report.e_cone_range.0,
        quality_report.e_cone_range.1
    );
    info!("  Main lobe points: {}, sidelobe points: {}",
        quality_report.main_lobe_points,
        quality_report.sidelobe_points
    );

    if quality_report.outlier_count > 0 {
        warn!("  ⚠ Found {} outlier points", quality_report.outlier_count);
    }

    // Step 2: Load antenna class definition
    info!("Step 2/6: Loading antenna class definition...");
    let registry = AntennaClassRegistry::load_from_file(&args.classes_file)
        .map_err(|e| anyhow::anyhow!("Failed to load antenna classes: {}", e))?;

    let class = registry
        .get_class(&args.antenna_class)
        .context(format!("Antenna class '{}' not found", args.antenna_class))?;

    info!("  ✓ Loaded class: {}", class.description);
    info!("    Diameter: {:.1}m, f/D: {:.4}", class.geometry.diameter_m, class.geometry.f_over_d);

    // Step 3: Create antenna configuration with optional tuning
    let mut tunable_params = TunableParameters::default_from_class();

    if args.tune_parameters {
        info!("Step 3/6: Tuning physical parameters...");
        info!("  Running parameter optimization (max {} iterations)...", args.max_tuning_iterations);

        let tuning_mode = match args.tuning_mode.as_str() {
            "surface-only" => TuningMode::SurfaceRmsOnly,
            "surface-and-mesh" => TuningMode::SurfaceAndMeshSpacing,
            "all" => TuningMode::All,
            _ => {
                warn!("Unknown tuning mode '{}', using 'surface-only'", args.tuning_mode);
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
        info!("    Improvement: {:.4} dB ({:.1}%)",
            tuning_result.improvement_db,
            (tuning_result.improvement_db / tuning_result.initial_rmse_db) * 100.0
        );
        info!("    Iterations: {}", tuning_result.iterations);

        info!("    Tuned surface_rms: {:.3} mm", tuning_result.surface_rms_mm);
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

    // Create antenna configuration
    let antenna_config = AntennaConfiguration::new(
        args.antenna_id.clone(),
        args.antenna_name.clone(),
        class.class_id.clone(),
    );

    // Step 4: Compute model predictions
    info!("Step 4/6: Computing model predictions...");
    let model_predictions = compute_model_predictions(&measurements.points, class, &tunable_params)?;

    // Compute initial model-only RMSE
    let model_only_rmse = {
        let squared_errors: f64 = measurements.points.iter()
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

    let surface_params = CorrectionSurfaceParams {
        spline_order: 4,
        num_knots_frequency: 4,
        num_knots_econe: 6,
        num_knots_eclock: 8,
        regularization: 1e-3,
        adaptive_knots: true,
        cross_validation_folds: if args.validate { args.cv_folds } else { 0 },
        min_knot_spacing_frequency: 50.0,  // 50 MHz minimum spacing
        min_knot_spacing_econe: 2.0,       // 2 degrees minimum spacing
        min_knot_spacing_eclock: 5.0,      // 5 degrees minimum spacing
    };

    let correction_surface = fit_correction_surface(
        &measurements.points,
        &model_predictions,
        &surface_params,
    )?;

    info!("  ✓ Correction surface fitted");
    info!("    RMSE: {:.4} dB", correction_surface.fit_stats.rmse_db);
    info!("    Max residual: {:.4} dB", correction_surface.fit_stats.max_residual_db);
    info!("    R²: {:.6}", correction_surface.fit_stats.r_squared);
    info!("    Improvement: {:.1}%", correction_surface.fit_stats.improvement_percent);

    // Step 6: Validation
    info!("Step 6/6: Running validation...");

    let validation_config = ValidationConfig {
        num_folds: args.cv_folds,
        main_lobe_beamwidths: 1.0,
        first_sidelobe_max_deg: 5.0,
        frequency_bands: vec![],  // Use default bands
        main_lobe_target_db: 1.0,
        first_sidelobe_target_db: 1.0,
        outlier_threshold_db: 3.0,
        correction_params: CorrectionSurfaceParams::default(),
    };

    let validation_report = validate_calibration(
        &measurements.points,
        &model_predictions,
        &correction_surface,
        &validation_config,
    )?;

    info!("  ✓ Validation complete");
    info!("    Corrected RMSE: {:.4} dB", validation_report.corrected_rmse);
    info!("    Main lobe max error: {:.4} dB", validation_report.main_lobe_max_error);
    info!("    First sidelobe max error: {:.4} dB", validation_report.first_sidelobe_max_error);
    info!("    Outliers: {} ({:.1}%)",
        validation_report.outliers.len(),
        validation_report.outliers.len() as f64 / measurements.points.len() as f64 * 100.0
    );

    if !validation_report.main_lobe_meets_target {
        warn!("  ⚠ Main lobe accuracy target not met ({:.4} dB > {:.4} dB)",
            validation_report.main_lobe_max_error,
            validation_config.main_lobe_target_db
        );
    } else {
        info!("  ✓ Main lobe meets accuracy target");
    }

    if !validation_report.first_sidelobe_meets_target {
        warn!("  ⚠ First sidelobe accuracy target not met ({:.4} dB > {:.4} dB)",
            validation_report.first_sidelobe_max_error,
            validation_config.first_sidelobe_target_db
        );
    } else {
        info!("  ✓ First sidelobe meets accuracy target");
    }

    // Print cross-validation results if available
    if let Some(cv) = &validation_report.cross_validation {
        info!("  Cross-validation results:");
        info!("    Mean RMSE: {:.4} dB (± {:.4} dB)", cv.mean_rmse, cv.std_rmse);
        info!("    Range: [{:.4}, {:.4}] dB", cv.min_rmse, cv.max_rmse);
    }

    // Generate and save calibration artifact
    info!("Generating calibration artifact...");

    let artifact = CalibrationArtifact {
        antenna_config,
        correction_surface: correction_surface.clone(),
        validation_report,
        metadata: ArtifactMetadata {
            created_at: chrono::Utc::now().to_rfc3339(),
            measurement_source: measurements.source.clone(),
            parameters_tuned: args.tune_parameters,
            num_measurement_points: measurements.points.len(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            notes: Some(format!("Calibrated with class: {}, R²={:.6}", class.class_id, correction_surface.fit_stats.r_squared)),
            frequency_range: quality_report.frequency_range,
            angular_range: quality_report.e_cone_range,
        },
    };

    save_artifact(&artifact, &args.output)
        .context("Failed to save calibration artifact")?;

    let file_size = std::fs::metadata(&args.output)?.len();
    info!("  ✓ Artifact saved: {} ({:.2} KB)",
        args.output.display(),
        file_size as f64 / 1024.0
    );

    // Export metadata JSON (optional)
    if let Some(metadata_path) = args.metadata {
        info!("Exporting metadata to JSON...");
        export_metadata_json(&artifact, &metadata_path)?;
        info!("  ✓ Metadata saved: {}", metadata_path.display());
    }

    // Export validation report (optional)
    if let Some(report_path) = args.report {
        info!("Exporting validation report to JSON...");
        export_validation_json(&artifact, &report_path)?;
        info!("  ✓ Validation report saved: {}", report_path.display());
    }

    info!("");
    info!("✓ Calibration workflow complete!");
    info!("");
    info!("Summary:");
    info!("  Antenna ID: {}", args.antenna_id);
    info!("  Measurements: {}", measurements.points.len());
    info!("  Parameter tuning: {}", if args.tune_parameters { "yes" } else { "no" });
    info!("  Model-only RMSE: {:.4} dB", model_only_rmse);
    info!("  Corrected RMSE: {:.4} dB", artifact.validation_report.corrected_rmse);
    info!("  Improvement: {:.1}%", artifact.validation_report.rmse_improvement_percent);
    info!("  Main lobe target met: {}", if artifact.validation_report.main_lobe_meets_target { "yes" } else { "no" });
    info!("  First sidelobe target met: {}", if artifact.validation_report.first_sidelobe_meets_target { "yes" } else { "no" });
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

    // Run calibration workflow
    if let Err(e) = run_calibration(args).await {
        error!("Calibration failed: {:#}", e);
        std::process::exit(1);
    }
}
