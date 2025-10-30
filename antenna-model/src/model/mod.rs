//! Physical Optics Model Components
//!
//! This module contains the physical optics computation engine for parabolic
//! reflector antenna pattern modeling. It includes:
//!
//! - **Geometry**: Physical antenna parameters (reflector, feed, mesh)
//! - **Coordinates**: Coordinate system transformations
//! - **Phase**: Phase functions for aperture integration
//! - **Illumination**: Feed pattern models and aperture illumination
//! - **Integration**: Aperture integration engine for far-field patterns
//! - **Pattern**: Far-field pattern computation, gain, and G/T calculations
//! - **Surface**: Surface error modeling (Ruze equation, Zernike polynomials)
//! - **Mesh**: Wire mesh reflector physics (transparency, angle effects, polarization)
//! - **Edge Cases**: Detection and handling of edge cases (large offsets, near-boresight)
//! - **Ray Trace**: Ray tracing for large feed offset scenarios
//! - **Direct Path**: Near-boresight direct path interference modeling
//! - **Numerical Stability**: Adaptive integration, Kaiser windowing, noise floor management

pub mod coordinates;
pub mod direct_path;
pub mod edge_cases;
pub mod geometry;
pub mod illumination;
pub mod integration;
pub mod mesh;
pub mod numerical_stability;
pub mod pattern;
pub mod phase;
pub mod ray_trace;
pub mod surface;

// Re-export commonly used types
pub use coordinates::{
    normalize_angle, normalize_angle_symmetric, ApertureCoordinates, AzElCoordinates,
    EClockConeCoordinates, FarFieldCoordinates,
};

pub use geometry::{
    AntennaConfiguration, AntennaConfigurationBuilder, FeedParameters, FeedParametersBuilder,
    FeedPosition, MeshParameters, MeshParametersBuilder, MeshPattern, ReflectorGeometry,
    ReflectorGeometryBuilder,
};

pub use illumination::{
    cos_q_pattern, edge_taper_db, feed_angle, illumination_amplitude, phase_center_offset_phase,
    q_factor_from_taper,
};

pub use integration::{
    compute_far_field, far_field_normalization, integrate_aperture, IntegrationParams,
    IntegrationResult,
};

pub use pattern::{
    compute_beamwidth, compute_g_over_t, compute_gain, compute_gain_db, mesh_transparency,
    overall_efficiency, ruze_efficiency, theoretical_max_gain,
};

pub use phase::{
    angle_of_incidence, phase_feed_displacement, phase_mesh, phase_path, phase_surface_error,
    phase_total, wavelength_from_frequency, wavenumber,
};

pub use surface::{
    compute_surface_rms, ruze_efficiency as surface_ruze_efficiency,
    ruze_efficiency_from_frequency, zernike_polynomial, GaussianSurface, IdealSurface,
    SurfaceErrorModel, ZernikeIndex, ZernikeSurface,
};

pub use mesh::{
    angle_correction_factor, basic_transparency, cutoff_wavelength, effective_cutoff_wavelength,
    mesh_efficiency, mesh_efficiency_simple, mesh_reflection_coefficient,
    mesh_transparency_polarized, mesh_transparency_with_angle, transparency_with_diameter,
    Polarization,
};

pub use edge_cases::{
    analyze_edge_cases, apply_gain_floor, apply_gain_floor_db, higher_order_aberrations,
    needs_adaptive_integration, ComputationMode, EdgeCaseAnalysis, LARGE_OFFSET_THRESHOLD,
    MIN_GAIN_FLOOR, MIN_GAIN_FLOOR_DB, NEAR_BORESIGHT_THRESHOLD, SEVERE_OFFSET_THRESHOLD,
};

pub use ray_trace::{compute_gain_ray_trace, ray_trace_aperture, Ray, RayTraceResult};

pub use direct_path::{
    compute_with_direct_path, should_include_direct_path, DirectPathResult,
};

pub use numerical_stability::{
    adaptive_integration_params, apply_kaiser_taper, kaiser_window, smooth_to_floor,
    strongly_needs_adaptive, unwrap_phase,
};
