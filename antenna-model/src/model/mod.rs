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

pub mod coordinates;
pub mod geometry;
pub mod illumination;
pub mod integration;
pub mod pattern;
pub mod phase;

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
    phase_total, wavelength_from_frequency, wavenumber, GaussianSurface, IdealSurface,
    SurfaceErrorModel, ZernikeSurface,
};
