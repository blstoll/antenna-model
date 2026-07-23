//! Error types for the Antenna Model Service
//!
//! This module defines a comprehensive error handling framework using `thiserror`
//! for ergonomic error propagation throughout the service.
//!
//! # Error Categories
//!
//! - [`DataError`]: Calibration data loading, parsing, and serialization issues
//! - [`ApiError`]: HTTP/API-level errors with status code mappings
//! - [`ValidationError`]: Input parameter validation failures
//! - [`ComputationError`]: Interpolation and mathematical computation errors
//! - [`AntennaModelError`]: Top-level error type encompassing all error categories

use std::io;
use thiserror::Error;

/// Top-level error type for the Antenna Model Service
///
/// This is the primary error type that encompasses all error categories.
/// It provides convenient conversions from specific error types and
/// preserves error context through the chain.
#[derive(Error, Debug)]
pub enum AntennaModelError {
    /// Data-related errors (loading, serialization, etc.)
    #[error("data error: {0}")]
    Data(#[from] DataError),

    /// API/HTTP-related errors
    #[error("API error: {0}")]
    Api(#[from] ApiError),

    /// Input validation errors
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),

    /// Computation/interpolation errors
    #[error("computation error: {0}")]
    Computation(#[from] ComputationError),

    /// Configuration errors
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Invalid coordinate error
    #[error("invalid coordinate for '{param}': {reason}")]
    InvalidCoordinate { param: String, reason: String },

    /// Coordinate transformation error
    #[error("coordinate transformation error: {details}")]
    CoordinateTransformError { details: String },

    /// Feed not found
    #[error("feed '{feed_id}' not found for antenna '{antenna_id}'")]
    FeedNotFound { antenna_id: String, feed_id: String },

    /// Feature not implemented
    #[error("feature not implemented: {feature}")]
    NotImplemented { feature: String },

    /// Generic error with context
    #[error("{0}")]
    Generic(String),
}

/// Errors related to calibration data management
///
/// These errors occur during data loading, parsing, serialization,
/// and validation of calibration artifacts.
#[derive(Error, Debug)]
pub enum DataError {
    /// Calibration file not found
    #[error("calibration file not found: {path}")]
    FileNotFound { path: String },

    /// Corrupted calibration data (checksum mismatch)
    #[error("corrupted calibration data in {path}: {reason}")]
    CorruptedData { path: String, reason: String },

    /// Unsupported calibration format version
    #[error("unsupported calibration format version {version} in {path} (expected {expected})")]
    UnsupportedVersion {
        path: String,
        version: u32,
        expected: u32,
    },

    /// Deserialization error
    #[error("deserialization error in {path}: {reason}")]
    Deserialization { path: String, reason: String },

    /// Invalid calibration data structure
    #[error("invalid calibration data in {path}: {reason}")]
    InvalidData { path: String, reason: String },

    /// Antenna not found in repository
    #[error("antenna not found: {antenna_id}")]
    AntennaNotFound { antenna_id: String },

    /// Duplicate antenna ID
    #[error("duplicate antenna ID: {antenna_id}")]
    DuplicateAntenna { antenna_id: String },

    /// Missing required field
    #[error("missing required field '{field}' in {path}")]
    MissingField { path: String, field: String },

    /// I/O error during data operations
    #[error("I/O error while accessing {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },

    /// General data loading error
    #[error("failed to load calibration from {path}: {reason}")]
    LoadError { path: String, reason: String },

    /// Data validation error
    #[error("validation failed for {path}: {reason}")]
    ValidationError { path: String, reason: String },

    /// Configuration error during data loading
    #[error("configuration error: {reason}")]
    ConfigurationError { reason: String },
}

/// Errors related to API/HTTP operations
///
/// These errors map to appropriate HTTP status codes and provide
/// structured error responses for API clients.
#[derive(Error, Debug)]
pub enum ApiError {
    /// Bad request (400) - malformed request
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Not found (404) - resource not found
    #[error("not found: {0}")]
    NotFound(String),

    /// Unprocessable entity (422) - validation failed
    #[error("unprocessable entity: {0}")]
    UnprocessableEntity(String),

    /// Internal server error (500)
    #[error("internal server error: {0}")]
    InternalError(String),

    /// Service unavailable (503)
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Request timeout (504) — server-side processing exceeded its wall-clock
    /// budget. 504 (a 5xx) keeps the fault server-side; see the `RequestTimeout`
    /// middleware for the full 504-vs-408-vs-503 rationale.
    #[error("request timeout: {0}")]
    Timeout(String),

    /// Payload too large (413)
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    /// Rate limit exceeded (429)
    #[error("rate limit exceeded: {0}")]
    RateLimitExceeded(String),
}

impl ApiError {
    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> u16 {
        match self {
            ApiError::BadRequest(_) => 400,
            ApiError::NotFound(_) => 404,
            ApiError::Timeout(_) => 504,
            ApiError::PayloadTooLarge(_) => 413,
            ApiError::UnprocessableEntity(_) => 422,
            ApiError::RateLimitExceeded(_) => 429,
            ApiError::InternalError(_) => 500,
            ApiError::ServiceUnavailable(_) => 503,
        }
    }

    /// Check if this error is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        let code = self.status_code();
        (400..500).contains(&code)
    }

    /// Check if this error is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        let code = self.status_code();
        (500..600).contains(&code)
    }
}

/// Errors related to input validation
///
/// These errors occur when request parameters fail validation checks
/// (range validation, type validation, etc.)
#[derive(Error, Debug)]
pub enum ValidationError {
    /// Parameter out of valid range
    #[error("parameter '{param}' value {value} is out of valid range [{min}, {max}]")]
    OutOfRange {
        param: String,
        value: f64,
        min: f64,
        max: f64,
    },

    /// Invalid parameter value
    #[error("invalid value for parameter '{param}': {reason}")]
    InvalidValue { param: String, reason: String },

    /// Missing required parameter
    #[error("missing required parameter: {param}")]
    MissingParameter { param: String },

    /// Invalid antenna ID format
    #[error("invalid antenna ID '{antenna_id}': {reason}")]
    InvalidAntennaId { antenna_id: String, reason: String },

    /// Invalid grid specification
    #[error("invalid grid specification for {dimension}: {reason}")]
    InvalidGrid { dimension: String, reason: String },

    /// Batch size limit exceeded
    #[error("batch size {size} exceeds limit of {limit}")]
    BatchSizeLimitExceeded { size: usize, limit: usize },

    /// Invalid frequency range
    #[error("frequency {frequency_mhz} MHz is outside supported range [100, 50000] MHz")]
    FrequencyOutOfRange { frequency_mhz: f64 },

    /// Invalid angle range
    #[error("{angle_type} {value} degrees is outside valid range [{min}, {max}]")]
    AngleOutOfRange {
        angle_type: String,
        value: f64,
        min: f64,
        max: f64,
    },
}

/// Errors related to model computation
///
/// These errors occur during interpolation, extrapolation, and
/// mathematical operations in the computation engine.
#[derive(Error, Debug)]
pub enum ComputationError {
    /// Numerical instability detected
    #[error("numerical instability in {operation}: {reason}")]
    NumericalInstability { operation: String, reason: String },

    /// Invalid knot vector
    #[error("invalid knot vector for dimension {dimension}: {reason}")]
    InvalidKnotVector {
        /// Dimension name (e.g., "azimuth", "elevation")
        dimension: String,
        /// Reason for invalidity
        reason: String,
    },

    /// Invalid coefficient dimensions
    #[error("coefficient dimensions mismatch: expected {expected:?}, got {actual:?}")]
    DimensionMismatch {
        /// Expected dimensions
        expected: Vec<usize>,
        /// Actual dimensions
        actual: Vec<usize>,
    },

    /// Spline order not supported
    #[error("spline order {order} not supported (must be between 1 and 5)")]
    UnsupportedSplineOrder {
        /// The unsupported spline order
        order: u8,
    },

    /// Insufficient data points
    #[error(
        "insufficient data points in dimension {dimension}: need at least {required}, got {actual}"
    )]
    InsufficientDataPoints {
        /// Dimension name (e.g., "azimuth", "elevation")
        dimension: String,
        /// Required number of data points
        required: usize,
        /// Actual number of data points
        actual: usize,
    },

    /// Matrix operation failed
    #[error("matrix operation failed in {operation}: {reason}")]
    MatrixOperationFailed {
        /// Operation that failed
        operation: String,
        /// Reason for failure
        reason: String,
    },

    /// Interpolation failed
    #[error("interpolation failed at point ({azimuth}, {elevation}, {frequency}, {temperature}): {reason}")]
    InterpolationFailed {
        /// Azimuth angle in degrees
        azimuth: f64,
        /// Elevation angle in degrees
        elevation: f64,
        /// Frequency in Hz
        frequency: f64,
        /// Temperature in Kelvin
        temperature: f64,
        /// Reason for failure
        reason: String,
    },

    /// Invalid model state
    #[error("invalid model state: {0}")]
    InvalidModelState(String),

    /// A single aperture integration exceeded its configured wall-clock budget (S3).
    ///
    /// Raised cooperatively inside the radial integration loop when one integral runs
    /// past `IntegrationParams::time_budget` (from `performance.integration_budget_ms`).
    /// The overrun is deterministic in the request payload — the same heavy grid re-costs
    /// the same — so it maps to `504` (not a transient `503`): the remedy is a smaller
    /// request, not a retry. This bounds ONE integral; the request wall-clock is S2's
    /// `RequestTimeout` and concurrency is S4.
    #[error(
        "computation exceeded time budget in {operation}: {elapsed_ms:.0} ms > {budget_ms} ms budget"
    )]
    TimeBudgetExceeded {
        /// Name of the integrator that hit the deadline.
        operation: String,
        /// Wall-clock time elapsed since the integration started, in milliseconds.
        elapsed_ms: f64,
        /// The configured budget that was exceeded, in milliseconds.
        budget_ms: u64,
    },
}

/// Errors related to configuration
///
/// These errors occur during configuration loading and parsing.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Configuration file not found
    #[error("configuration file not found: {path}")]
    FileNotFound {
        /// Path to the configuration file
        path: String,
    },

    /// Configuration parse error
    #[error("failed to parse configuration from {path}: {reason}")]
    ParseError {
        /// Path to the configuration file
        path: String,
        /// Reason for parse failure
        reason: String,
    },

    /// Invalid configuration value
    #[error("invalid configuration value for '{key}': {reason}")]
    InvalidValue {
        /// Configuration key
        key: String,
        /// Reason for invalidity
        reason: String,
    },

    /// Missing required configuration
    #[error("missing required configuration: {key}")]
    MissingRequired {
        /// Configuration key that is missing
        key: String,
    },

    /// Environment variable error
    #[error("invalid environment variable {var}: {reason}")]
    InvalidEnvironmentVariable {
        /// Environment variable name
        var: String,
        /// Reason for invalidity
        reason: String,
    },
}

// Conversion implementations for common error types

impl From<ValidationError> for ComputationError {
    fn from(err: ValidationError) -> Self {
        ComputationError::InvalidModelState(err.to_string())
    }
}

impl From<serde_yaml::Error> for ConfigError {
    fn from(err: serde_yaml::Error) -> Self {
        ConfigError::ParseError {
            path: "unknown".to_string(),
            reason: err.to_string(),
        }
    }
}

impl From<config::ConfigError> for ConfigError {
    fn from(err: config::ConfigError) -> Self {
        ConfigError::ParseError {
            path: "config".to_string(),
            reason: err.to_string(),
        }
    }
}

// Convert ConfigError to DataError for data loading operations
impl From<ConfigError> for DataError {
    fn from(err: ConfigError) -> Self {
        DataError::ConfigurationError {
            reason: err.to_string(),
        }
    }
}

// Convert ValidationError to ApiError for HTTP responses
impl From<ValidationError> for ApiError {
    fn from(err: ValidationError) -> Self {
        ApiError::UnprocessableEntity(err.to_string())
    }
}

// Convert DataError to ApiError for HTTP responses
impl From<DataError> for ApiError {
    fn from(err: DataError) -> Self {
        match err {
            DataError::AntennaNotFound { antenna_id } => {
                ApiError::NotFound(format!("antenna not found: {}", antenna_id))
            }
            DataError::FileNotFound { path } => {
                ApiError::NotFound(format!("file not found: {}", path))
            }
            _ => ApiError::InternalError(err.to_string()),
        }
    }
}

// Convert ComputationError to ApiError for HTTP responses
impl From<ComputationError> for ApiError {
    fn from(err: ComputationError) -> Self {
        match &err {
            // A blown wall-clock budget is a server-side processing timeout (504), mirroring
            // S2's RequestTimeout — deterministic in the payload, so not a transient 503.
            ComputationError::TimeBudgetExceeded { .. } => ApiError::Timeout(err.to_string()),
            _ => ApiError::InternalError(err.to_string()),
        }
    }
}

// Convert coordinate errors to ApiError
impl From<AntennaModelError> for ApiError {
    fn from(err: AntennaModelError) -> Self {
        match err {
            AntennaModelError::InvalidCoordinate { param, reason } => {
                ApiError::BadRequest(format!("invalid coordinate for '{}': {}", param, reason))
            }
            AntennaModelError::CoordinateTransformError { details } => {
                ApiError::InternalError(format!("coordinate transformation error: {}", details))
            }
            AntennaModelError::FeedNotFound {
                antenna_id,
                feed_id,
            } => ApiError::NotFound(format!(
                "feed '{}' not found for antenna '{}'",
                feed_id, antenna_id
            )),
            AntennaModelError::NotImplemented { feature } => {
                ApiError::UnprocessableEntity(format!("feature not implemented: {}", feature))
            }
            AntennaModelError::Data(data_err) => data_err.into(),
            AntennaModelError::Api(api_err) => api_err,
            AntennaModelError::Validation(val_err) => val_err.into(),
            AntennaModelError::Computation(comp_err) => comp_err.into(),
            AntennaModelError::Config(conf_err) => ApiError::InternalError(conf_err.to_string()),
            AntennaModelError::Io(io_err) => ApiError::InternalError(io_err.to_string()),
            AntennaModelError::Generic(msg) => ApiError::InternalError(msg),
        }
    }
}

/// Result type alias for operations that return AntennaModelError
pub type Result<T> = std::result::Result<T, AntennaModelError>;

/// Result type alias for data operations
pub type DataResult<T> = std::result::Result<T, DataError>;

/// Result type alias for API operations
pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// Result type alias for validation operations
pub type ValidationResult<T> = std::result::Result<T, ValidationError>;

/// Result type alias for computation operations
pub type ComputationResult<T> = std::result::Result<T, ComputationError>;

/// Result type alias for configuration operations
pub type ConfigResult<T> = std::result::Result<T, ConfigError>;

/// Extension trait for adding context to errors
pub trait ErrorContext<T> {
    /// Add context to an error
    fn context(self, context: &str) -> Result<T>;

    /// Add context to an error using a closure
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String;
}

impl<T, E> ErrorContext<T> for std::result::Result<T, E>
where
    E: Into<AntennaModelError>,
{
    fn context(self, context: &str) -> Result<T> {
        self.map_err(|e| {
            let err: AntennaModelError = e.into();
            AntennaModelError::Generic(format!("{}: {}", context, err))
        })
    }

    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| {
            let err: AntennaModelError = e.into();
            AntennaModelError::Generic(format!("{}: {}", f(), err))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_error_display() {
        let err = DataError::FileNotFound {
            path: "antenna_1.bin".to_string(),
        };
        assert_eq!(err.to_string(), "calibration file not found: antenna_1.bin");

        let err = DataError::CorruptedData {
            path: "antenna_1.bin".to_string(),
            reason: "checksum mismatch".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "corrupted calibration data in antenna_1.bin: checksum mismatch"
        );

        let err = DataError::AntennaNotFound {
            antenna_id: "antenna_1".to_string(),
        };
        assert_eq!(err.to_string(), "antenna not found: antenna_1");
    }

    #[test]
    fn test_api_error_status_codes() {
        assert_eq!(ApiError::BadRequest("test".to_string()).status_code(), 400);
        assert_eq!(ApiError::NotFound("test".to_string()).status_code(), 404);
        assert_eq!(ApiError::Timeout("test".to_string()).status_code(), 504);
        assert_eq!(
            ApiError::PayloadTooLarge("test".to_string()).status_code(),
            413
        );
        assert_eq!(
            ApiError::UnprocessableEntity("test".to_string()).status_code(),
            422
        );
        assert_eq!(
            ApiError::RateLimitExceeded("test".to_string()).status_code(),
            429
        );
        assert_eq!(
            ApiError::InternalError("test".to_string()).status_code(),
            500
        );
        assert_eq!(
            ApiError::ServiceUnavailable("test".to_string()).status_code(),
            503
        );
    }

    #[test]
    fn test_api_error_classification() {
        let client_err = ApiError::BadRequest("test".to_string());
        assert!(client_err.is_client_error());
        assert!(!client_err.is_server_error());

        let server_err = ApiError::InternalError("test".to_string());
        assert!(!server_err.is_client_error());
        assert!(server_err.is_server_error());
    }

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError::OutOfRange {
            param: "azimuth_deg".to_string(),
            value: 400.0,
            min: 0.0,
            max: 360.0,
        };
        assert_eq!(
            err.to_string(),
            "parameter 'azimuth_deg' value 400 is out of valid range [0, 360]"
        );

        let err = ValidationError::MissingParameter {
            param: "frequency_mhz".to_string(),
        };
        assert_eq!(err.to_string(), "missing required parameter: frequency_mhz");

        let err = ValidationError::BatchSizeLimitExceeded {
            size: 1500,
            limit: 1000,
        };
        assert_eq!(err.to_string(), "batch size 1500 exceeds limit of 1000");
    }

    #[test]
    fn test_computation_error_display() {
        let err = ComputationError::NumericalInstability {
            operation: "basis_function".to_string(),
            reason: "division by zero".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "numerical instability in basis_function: division by zero"
        );

        let err = ComputationError::DimensionMismatch {
            expected: vec![10, 10, 20, 1],
            actual: vec![10, 10, 15, 1],
        };
        assert_eq!(
            err.to_string(),
            "coefficient dimensions mismatch: expected [10, 10, 20, 1], got [10, 10, 15, 1]"
        );

        let err = ComputationError::UnsupportedSplineOrder { order: 7 };
        assert_eq!(
            err.to_string(),
            "spline order 7 not supported (must be between 1 and 5)"
        );
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::FileNotFound {
            path: "service.yaml".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "configuration file not found: service.yaml"
        );

        let err = ConfigError::InvalidValue {
            key: "server.port".to_string(),
            reason: "must be between 1024 and 65535".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "invalid configuration value for 'server.port': must be between 1024 and 65535"
        );
    }

    #[test]
    fn test_error_conversion_validation_to_api() {
        let validation_err = ValidationError::MissingParameter {
            param: "antenna_id".to_string(),
        };
        let api_err: ApiError = validation_err.into();
        assert_eq!(api_err.status_code(), 422);
        assert!(api_err.is_client_error());
    }

    #[test]
    fn test_error_conversion_data_to_api() {
        let data_err = DataError::AntennaNotFound {
            antenna_id: "antenna_1".to_string(),
        };
        let api_err: ApiError = data_err.into();
        assert_eq!(api_err.status_code(), 404);
        assert!(matches!(api_err, ApiError::NotFound(_)));

        let data_err = DataError::CorruptedData {
            path: "test.bin".to_string(),
            reason: "checksum failed".to_string(),
        };
        let api_err: ApiError = data_err.into();
        assert_eq!(api_err.status_code(), 500);
        assert!(matches!(api_err, ApiError::InternalError(_)));
    }

    #[test]
    fn test_error_conversion_computation_to_api() {
        let comp_err = ComputationError::NumericalInstability {
            operation: "interpolation".to_string(),
            reason: "overflow".to_string(),
        };
        let api_err: ApiError = comp_err.into();
        assert_eq!(api_err.status_code(), 500);
        assert!(matches!(api_err, ApiError::InternalError(_)));
    }

    #[test]
    fn test_time_budget_exceeded_maps_to_504() {
        // S3: a blown per-integration wall-clock budget maps to 504 (server-side timeout),
        // NOT the blanket 500 every other ComputationError takes.
        let comp_err = ComputationError::TimeBudgetExceeded {
            operation: "azimuthal_mode_field".to_string(),
            elapsed_ms: 31_000.0,
            budget_ms: 30_000,
        };
        let api_err: ApiError = comp_err.into();
        assert_eq!(api_err.status_code(), 504);
        assert!(matches!(api_err, ApiError::Timeout(_)));
    }

    #[test]
    fn test_error_chain_preservation() {
        let data_err = DataError::FileNotFound {
            path: "test.bin".to_string(),
        };
        let model_err: AntennaModelError = data_err.into();

        // Error chain should be preserved
        let display = format!("{}", model_err);
        assert!(display.contains("calibration file not found"));
        assert!(display.contains("test.bin"));
    }

    #[test]
    fn test_error_context_helper() {
        fn operation_that_fails() -> std::result::Result<(), DataError> {
            Err(DataError::FileNotFound {
                path: "test.bin".to_string(),
            })
        }

        let result = operation_that_fails().context("loading calibration data");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("loading calibration data"));
        assert!(msg.contains("calibration file not found"));
    }

    #[test]
    fn test_error_context_with_closure() {
        fn operation_that_fails() -> std::result::Result<(), DataError> {
            Err(DataError::FileNotFound {
                path: "test.bin".to_string(),
            })
        }

        let antenna_id = "antenna_1";
        let result =
            operation_that_fails().with_context(|| format!("loading antenna {}", antenna_id));
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("loading antenna antenna_1"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let model_err: AntennaModelError = io_err.into();
        assert!(matches!(model_err, AntennaModelError::Io(_)));
    }

    #[test]
    fn test_result_type_aliases() {
        fn data_operation() -> DataResult<String> {
            Ok("success".to_string())
        }

        fn api_operation() -> ApiResult<i32> {
            Ok(42)
        }

        fn validation_operation() -> ValidationResult<bool> {
            Ok(true)
        }

        fn computation_operation() -> ComputationResult<f64> {
            Ok(std::f64::consts::PI)
        }

        assert!(data_operation().is_ok());
        assert!(api_operation().is_ok());
        assert!(validation_operation().is_ok());
        assert!(computation_operation().is_ok());
    }

    #[test]
    fn test_error_debug_format() {
        let err = DataError::AntennaNotFound {
            antenna_id: "test".to_string(),
        };
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("AntennaNotFound"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_multiple_error_conversions() {
        // Test that we can convert through multiple layers
        let validation_err = ValidationError::OutOfRange {
            param: "test".to_string(),
            value: 100.0,
            min: 0.0,
            max: 50.0,
        };

        let api_err: ApiError = validation_err.into();
        assert_eq!(api_err.status_code(), 422);

        // Now convert to top-level error
        let model_err: AntennaModelError = api_err.into();
        assert!(matches!(model_err, AntennaModelError::Api(_)));
    }
}
