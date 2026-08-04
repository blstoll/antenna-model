//! Calibration artifact loader
//!
//! This module provides functionality for loading and validating calibration artifacts
//! from binary files.
//!
//! # The two version axes
//!
//! A `.bin` calibration artifact carries **two** independent version stamps. They are easy
//! to confuse, so state plainly what each one guards (a third axis,
//! [`crate::data::types::CalibrationMetadata::physics_model_version`], is about physics
//! staleness rather than decoding — see `docs/calibration-workflow-guide.md` §10.5):
//!
//! | Axis | Where | Type | Guards |
//! |---|---|---|---|
//! | **Container** ([`ANTC_ARTIFACT_VERSION`]) | ANTC file header, *outside* the payload | `u32` | How file bytes become a payload byte string: the `[magic][version][crc32][len]` framing and which codec decodes the payload. |
//! | **Schema** ([`crate::data::types::CALIBRATION_SCHEMA_VERSION`]) | `metadata.format_version`, *inside* the payload | `String` `"MAJOR.MINOR"` | What the decoded [`AntennaCalibration`] means: which fields exist, in what order, meaning what. |
//!
//! **Why both are needed, and why neither subsumes the other.** The container stamp is
//! readable *before* decoding, so it is the only thing that can reject a file this build
//! cannot parse at all — a pre-2026-07-18 bincode payload, say. It cannot see inside the
//! payload, so it says nothing about field meanings. The schema stamp is the reverse: it
//! is only readable *after* a successful decode, so it cannot protect the decode itself,
//! but it catches the class the container stamp structurally cannot — a payload that
//! decodes cleanly and means something different. That class is real here because postcard
//! is positional and non-self-describing: swapping two `f64` fields, or redefining what an
//! existing field measures, produces bytes that decode without complaint into wrong
//! numbers.
//!
//! **Bump policy.** Any change to the postcard byte layout (adding, removing, reordering,
//! or retyping a field reachable from [`AntennaCalibration`]) bumps **both**: the schema
//! MAJOR, because the meaning changed, and the container version, because existing files
//! can no longer be decoded. A change confined to *documented meaning* with the layout
//! untouched bumps the schema MINOR only. A change to framing or codec alone — the
//! bincode → postcard migration, which is why the container version is 2 — bumps the
//! container version only.
//!
//! **Enforcement.** Container mismatch and schema-MAJOR mismatch are hard errors; a
//! differing schema MINOR warns and loads. Both producers in `calibrate` write the ANTC
//! header via one shared writer, so every artifact this repo produces carries a container
//! stamp. The headerless reader below survives only for artifacts written before that was
//! true; it is deliberately *not* a supported output format.

use crate::data::types::{AntennaCalibration, CALIBRATION_SCHEMA_VERSION};
use crate::error::DataError;
use crate::model::PHYSICS_MODEL_VERSION;
use std::path::Path;
use tracing::{debug, info, warn};

/// Magic bytes identifying an ANTC-format calibration artifact.
pub const ANTC_MAGIC: &[u8; 4] = b"ANTC";

/// The ANTC **container** version this build writes and the only one it can decode.
///
/// Covers the on-disk framing (`[magic 4][version u32 LE][crc32 u32 LE][len u64 LE]`
/// followed by the payload) and the codec that decodes the payload — not the payload's
/// schema, which is [`CALIBRATION_SCHEMA_VERSION`]. See the module docs for the split.
///
/// Bumped 1 → 2 on the bincode → postcard migration (2026-07-18): the payload
/// encoding changed, so any pre-migration ANTC file is rejected loudly rather than
/// risking a garbled decode.
///
/// Writers use this constant through
/// `calibrate::artifact_export::write_calibration_artifact`, so the reader and both
/// producers cannot disagree about the framing.
pub const ANTC_ARTIFACT_VERSION: u32 = 2;

/// Byte length of an ANTC header: 4 (magic) + 4 (version) + 4 (crc) + 8 (len) = 20.
pub const ANTC_HEADER_LEN: usize = 20;

/// Load a calibration artifact from a binary file
///
/// Deserializes and validates a calibration artifact from a .bin file.
///
/// # Arguments
/// * `path` - Path to the calibration binary file
///
/// # Returns
/// * `Ok(AntennaCalibration)` - Successfully loaded and validated calibration
/// * `Err(DataError)` - Failed to load or validate
///
/// # Example
/// ```no_run
/// use antenna_model::data::loader::load_calibration_artifact;
///
/// let calibration = load_calibration_artifact("calibration_data/antenna_1.bin")?;
/// println!("Loaded antenna: {}, feed: {}", calibration.antenna_id, calibration.feed_id);
/// # Ok::<(), antenna_model::error::DataError>(())
/// ```
pub fn load_calibration_artifact<P: AsRef<Path>>(path: P) -> Result<AntennaCalibration, DataError> {
    let path = path.as_ref();

    debug!("Loading calibration artifact from: {}", path.display());

    // Read the binary file
    let bytes = std::fs::read(path).map_err(|e| DataError::LoadError {
        path: path.display().to_string(),
        reason: format!("Failed to read file: {}", e),
    })?;

    // Detect ANTC header format or fall back to legacy headerless format.
    let payload: &[u8] = if bytes.len() >= ANTC_HEADER_LEN && &bytes[0..4] == ANTC_MAGIC {
        // Parse ANTC header: [magic 4][version u32 LE][crc u32 LE][len u64 LE][payload]
        let version_bytes: [u8; 4] = bytes[4..8].try_into().map_err(|_| DataError::LoadError {
            path: path.display().to_string(),
            reason: "ANTC header truncated (version)".to_string(),
        })?;
        let crc_bytes: [u8; 4] = bytes[8..12].try_into().map_err(|_| DataError::LoadError {
            path: path.display().to_string(),
            reason: "ANTC header truncated (crc)".to_string(),
        })?;
        let len_bytes: [u8; 8] = bytes[12..20].try_into().map_err(|_| DataError::LoadError {
            path: path.display().to_string(),
            reason: "ANTC header truncated (len)".to_string(),
        })?;

        let version = u32::from_le_bytes(version_bytes);
        let expected_crc = u32::from_le_bytes(crc_bytes);
        let payload_len = u64::from_le_bytes(len_bytes) as usize;

        debug!(
            antc_version = version,
            payload_len = payload_len,
            "Detected ANTC-format artifact"
        );

        if version != ANTC_ARTIFACT_VERSION {
            return Err(DataError::LoadError {
                path: path.display().to_string(),
                reason: format!(
                    "unsupported ANTC artifact version {version} (this build supports version {ANTC_ARTIFACT_VERSION})"
                ),
            });
        }

        let payload_slice = bytes
            .get(ANTC_HEADER_LEN..ANTC_HEADER_LEN.saturating_add(payload_len))
            .filter(|p| p.len() == payload_len)
            .ok_or_else(|| DataError::LoadError {
                path: path.display().to_string(),
                reason: "ANTC header length exceeds file size".to_string(),
            })?;

        let actual_crc = crc32fast::hash(payload_slice);
        if actual_crc != expected_crc {
            return Err(DataError::LoadError {
                path: path.display().to_string(),
                reason: format!(
                    "CRC32 mismatch — artifact corrupted (expected {:#010x}, got {:#010x})",
                    expected_crc, actual_crc
                ),
            });
        }

        debug!("ANTC CRC32 verified successfully");
        payload_slice
    } else {
        debug!("No ANTC magic detected; using legacy headerless format");
        &bytes
    };

    // Deserialize using postcard
    let calibration: AntennaCalibration =
        postcard::from_bytes(payload).map_err(|e| DataError::LoadError {
            path: path.display().to_string(),
            reason: format!("Failed to deserialize calibration data: {}", e),
        })?;

    // Validate the schema axis before anything reads a field. The payload decoded, but a
    // foreign MAJOR means its fields do not necessarily mean what this build thinks they
    // mean — so do not apply this build's validation rules to them, and do not log them as
    // if they were understood.
    check_schema_version(&calibration.metadata.format_version).map_err(|reason| {
        DataError::LoadError {
            path: path.display().to_string(),
            reason,
        }
    })?;

    // Validate the calibration
    calibration
        .validate()
        .map_err(|e| DataError::ValidationError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    // Log summary
    info!(
        "Loaded calibration: antenna_id={}, feed_id={}, format_version={}",
        calibration.antenna_id, calibration.feed_id, calibration.metadata.format_version
    );

    // Log physical parameters
    debug!(
        "Physical config: diameter={:.1}m, f/D={:.2}, surface_rms={:.2}mm",
        calibration.physical_config.reflector.diameter_m,
        calibration.physical_config.reflector.f_over_d_ratio,
        calibration.physical_config.reflector.surface_rms_mm
    );

    // Log correction surface presence
    if let Some(ref correction) = calibration.correction_surface {
        debug!(
            "Correction surface: shape={:?}, {} coefficients",
            correction.shape,
            correction.num_coefficients()
        );
    } else {
        debug!("No correction surface (physics-only model)");
    }

    // Log validity ranges
    debug!(
        "Validity ranges: az=[{:.1}, {:.1}]°, el=[{:.1}, {:.1}]°, freq=[{:.1}, {:.1}] MHz",
        calibration.validity_ranges.azimuth_min_max.0,
        calibration.validity_ranges.azimuth_min_max.1,
        calibration.validity_ranges.elevation_min_max.0,
        calibration.validity_ranges.elevation_min_max.1,
        calibration.validity_ranges.frequency_min_max.0,
        calibration.validity_ranges.frequency_min_max.1
    );

    // Warn if this artifact was fitted against a different physics-model version.
    if let Some(msg) = physics_model_version_mismatch(
        calibration.metadata.physics_model_version,
        PHYSICS_MODEL_VERSION,
    ) {
        warn!("{}", msg);
    }

    Ok(calibration)
}

/// Split a `MAJOR.MINOR` schema stamp into its two numeric components.
///
/// Returns `None` for anything that is not exactly two non-negative integers separated by
/// a single `.` — an unparseable stamp is not "probably fine", it is a stamp whose meaning
/// cannot be reasoned about at all, and the caller treats it as a hard failure.
fn parse_schema_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    if minor.contains('.') {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Enforce the schema axis of an artifact's version stamp.
///
/// A foreign MAJOR (or an unparseable stamp) is `Err`: postcard decodes positionally, so a
/// payload from a different schema major can decode without error and still mean something
/// else entirely. A differing MINOR is `Ok` with a `warn!` — by the bump policy on
/// [`CALIBRATION_SCHEMA_VERSION`], a minor bump leaves the layout and every existing
/// field's meaning intact.
///
/// The supported major/minor are derived from [`CALIBRATION_SCHEMA_VERSION`] itself rather
/// than restated, so the constant is the single source of truth for both writing and
/// reading (pinned by `supported_schema_version_constant_is_parseable`).
fn check_schema_version(artifact: &str) -> Result<(), String> {
    let (supported_major, supported_minor) = parse_schema_version(CALIBRATION_SCHEMA_VERSION)
        .ok_or_else(|| {
            format!(
                "internal error: CALIBRATION_SCHEMA_VERSION {CALIBRATION_SCHEMA_VERSION:?} is not MAJOR.MINOR"
            )
        })?;

    let Some((major, minor)) = parse_schema_version(artifact) else {
        return Err(format!(
            "unreadable calibration schema version {artifact:?} (expected MAJOR.MINOR, \
             e.g. {CALIBRATION_SCHEMA_VERSION:?}); refusing to interpret the artifact"
        ));
    };

    if major != supported_major {
        return Err(format!(
            "incompatible calibration schema version {artifact} (this build reads schema \
             major {supported_major}, i.e. {CALIBRATION_SCHEMA_VERSION}); the payload's \
             field layout or field meanings differ, so decoded values cannot be trusted — \
             recalibrate with a matching `calibrate` build"
        ));
    }

    if minor != supported_minor {
        warn!(
            "Calibration schema version {} differs in MINOR from this build's {} — the \
             field layout is compatible, but the artifact was authored against a different \
             minor revision of the schema",
            artifact, CALIBRATION_SCHEMA_VERSION
        );
    }

    Ok(())
}

/// Warning to emit when an artifact was fitted against a different physics-model
/// version than this service computes with. Correction surfaces are fitted to
/// `measured − physics` residuals, so a mismatch can silently degrade accuracy;
/// this is a warning, not an error (roadmap P1b policy).
fn physics_model_version_mismatch(artifact: u32, current: u32) -> Option<String> {
    (artifact != current).then(|| {
        format!(
            "Calibration artifact physics_model_version {} does not match the service's \
             physics model version {}; the correction surface was fitted against a \
             different physics model and residual corrections may be stale — recalibrate",
            artifact, current
        )
    })
}

/// Validate a calibration artifact's internal consistency
///
/// Performs deep validation beyond the basic checks in `AntennaCalibration::validate()`.
///
/// # Arguments
/// * `calibration` - The calibration to validate
///
/// # Returns
/// * `Ok(())` - Calibration is valid
/// * `Err(DataError)` - Validation failed
pub fn validate_calibration(calibration: &AntennaCalibration) -> Result<(), DataError> {
    // Basic validation (already done in load, but can be called separately)
    calibration
        .validate()
        .map_err(|e| DataError::ValidationError {
            path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
            reason: e.to_string(),
        })?;

    // Additional validation checks

    // Check that validity ranges are reasonable
    let freq_range = calibration.validity_ranges.frequency_min_max;
    if freq_range.0 < 100.0 || freq_range.1 > 50000.0 {
        warn!(
            "Frequency range [{:.1}, {:.1}] MHz is outside typical range [100, 50000] MHz",
            freq_range.0, freq_range.1
        );
    }

    // Check elevation range is physically reasonable
    let el_range = calibration.validity_ranges.elevation_min_max;
    if el_range.0 < 0.0 || el_range.1 > 90.0 {
        return Err(DataError::ValidationError {
            path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
            reason: format!(
                "Elevation range [{:.1}, {:.1}]° is outside physical bounds [0, 90]°",
                el_range.0, el_range.1
            ),
        });
    }

    // Check mesh parameters if present
    if let Some(ref mesh) = calibration.physical_config.mesh {
        if mesh.wire_diameter_mm >= mesh.mesh_spacing_mm {
            return Err(DataError::ValidationError {
                path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
                reason: format!(
                    "Wire diameter ({:.2} mm) must be less than mesh spacing ({:.2} mm)",
                    mesh.wire_diameter_mm, mesh.mesh_spacing_mm
                ),
            });
        }
    }

    // Check correction surface dimensions if present
    if let Some(ref correction) = calibration.correction_surface {
        let total_coeffs = correction.num_coefficients();
        if total_coeffs > 1_000_000 {
            warn!(
                "Correction surface has {} coefficients, which may impact performance",
                total_coeffs
            );
        }
    }

    // Check metadata quality metrics
    if calibration.metadata.rmse_db > 1.0 {
        warn!(
            "Calibration RMSE ({:.2} dB) exceeds 1 dB accuracy target",
            calibration.metadata.rmse_db
        );
    }

    if calibration.metadata.r_squared < 0.95 {
        warn!(
            "Calibration R² ({:.3}) is below 0.95, indicating poor fit quality",
            calibration.metadata.r_squared
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::types::{
        BSplineModel4D, CalibrationMetadata, FeedParameters, PhysicalAntennaConfig,
        ReflectorGeometry, ValidityRanges,
    };
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Build a minimal ANTC-format byte vector from a serialized payload.
    fn make_antc_bytes(payload: &[u8], version: u32, crc_override: Option<u32>) -> Vec<u8> {
        let crc = crc_override.unwrap_or_else(|| crc32fast::hash(payload));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ANTC");
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn create_test_calibration() -> AntennaCalibration {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(34.0)
            .focal_length_m(13.6)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.1)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(10.0, 80.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap()
    }

    #[test]
    fn test_load_calibration_artifact_success() {
        let calibration = create_test_calibration();

        // Serialize to a temporary file (headerless legacy format)
        let mut temp_file = NamedTempFile::new().unwrap();
        let encoded = postcard::to_allocvec(&calibration).unwrap();
        temp_file.write_all(&encoded).unwrap();
        temp_file.flush().unwrap();

        // Load it back
        let loaded = load_calibration_artifact(temp_file.path()).unwrap();

        assert_eq!(loaded.antenna_id, "test_antenna");
        assert_eq!(loaded.feed_id, "x_band");
        assert_eq!(loaded.metadata.antenna_name, "Test Antenna");
    }

    #[test]
    fn test_load_calibration_artifact_file_not_found() {
        let result = load_calibration_artifact("/nonexistent/path/to/file.bin");
        assert!(result.is_err());
        match result {
            Err(DataError::LoadError { path, .. }) => {
                assert!(path.contains("nonexistent"));
            }
            _ => panic!("Expected LoadError"),
        }
    }

    #[test]
    fn test_load_calibration_artifact_invalid_data() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"invalid binary data").unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        assert!(result.is_err());
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(reason.contains("deserialize"));
            }
            _ => panic!("Expected LoadError with deserialization failure"),
        }
    }

    #[test]
    fn test_validate_calibration_success() {
        let calibration = create_test_calibration();
        assert!(validate_calibration(&calibration).is_ok());
    }

    #[test]
    fn test_validate_calibration_invalid_elevation_range() {
        let mut calibration = create_test_calibration();
        calibration.validity_ranges.elevation_min_max = (-10.0, 100.0);

        let result = validate_calibration(&calibration);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_calibration_with_correction_surface() {
        let mut calibration = create_test_calibration();

        let correction = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        calibration.correction_surface = Some(correction);

        assert!(validate_calibration(&calibration).is_ok());
    }

    #[test]
    fn test_load_antc_headered_artifact() {
        let calibration = create_test_calibration();

        let payload = postcard::to_allocvec(&calibration).unwrap();
        let bytes = make_antc_bytes(&payload, ANTC_ARTIFACT_VERSION, None);

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let loaded = load_calibration_artifact(temp_file.path()).unwrap();

        assert_eq!(loaded.antenna_id, "test_antenna");
        assert_eq!(loaded.feed_id, "x_band");
        assert_eq!(loaded.metadata.antenna_name, "Test Antenna");
    }

    #[test]
    fn test_load_antc_bad_crc_rejected() {
        let calibration = create_test_calibration();

        let mut payload = postcard::to_allocvec(&calibration).unwrap();

        // Build header with correct CRC, then corrupt a payload byte.
        let correct_crc = crc32fast::hash(&payload);
        // Flip the last byte of the payload after computing the CRC.
        if let Some(last) = payload.last_mut() {
            *last ^= 0xff;
        }
        let bytes = make_antc_bytes(&payload, ANTC_ARTIFACT_VERSION, Some(correct_crc));

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        assert!(result.is_err(), "Expected Err for bad CRC, got Ok");
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(
                    reason.contains("CRC32 mismatch"),
                    "Expected CRC32 mismatch message, got: {}",
                    reason
                );
            }
            other => panic!("Expected LoadError, got {:?}", other),
        }
    }

    #[test]
    fn test_load_antc_truncated_length_rejected() {
        let calibration = create_test_calibration();

        let payload = postcard::to_allocvec(&calibration).unwrap();

        // Claim the payload is 100 bytes longer than it actually is.
        let inflated_len = payload.len() as u64 + 100;
        let crc = crc32fast::hash(&payload);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ANTC");
        bytes.extend_from_slice(&ANTC_ARTIFACT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&inflated_len.to_le_bytes());
        bytes.extend_from_slice(&payload); // actual payload is shorter than claimed

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        assert!(
            result.is_err(),
            "Expected Err for truncated payload, got Ok"
        );
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(
                    reason.contains("length exceeds file size"),
                    "Expected 'length exceeds file size' message, got: {}",
                    reason
                );
            }
            other => panic!("Expected LoadError, got {:?}", other),
        }
    }

    #[test]
    fn test_load_antc_unsupported_version_rejected() {
        let calibration = create_test_calibration();

        let payload = postcard::to_allocvec(&calibration).unwrap();
        // Build a valid ANTC artifact but with version = 3 (unsupported).
        let bytes = make_antc_bytes(&payload, 3, None);

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        assert!(
            result.is_err(),
            "Expected Err for unsupported ANTC version, got Ok"
        );
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(
                    reason.contains("version 3"),
                    "Expected message to mention 'version 3', got: {}",
                    reason
                );
            }
            other => panic!("Expected LoadError, got {:?}", other),
        }
    }

    /// The loader derives the major/minor it supports from `CALIBRATION_SCHEMA_VERSION`
    /// rather than restating them, so an unparseable constant would turn every load into
    /// an "internal error". Pin that it parses.
    #[test]
    fn supported_schema_version_constant_is_parseable() {
        let parsed = parse_schema_version(CALIBRATION_SCHEMA_VERSION);
        assert!(
            parsed.is_some(),
            "CALIBRATION_SCHEMA_VERSION {CALIBRATION_SCHEMA_VERSION:?} must be MAJOR.MINOR"
        );
        assert!(check_schema_version(CALIBRATION_SCHEMA_VERSION).is_ok());
    }

    #[test]
    fn test_parse_schema_version() {
        assert_eq!(parse_schema_version("2.0"), Some((2, 0)));
        assert_eq!(parse_schema_version("10.37"), Some((10, 37)));
        // Not MAJOR.MINOR: no separator, three components, empty parts, non-numeric,
        // signed. Each must be unparseable rather than silently coerced.
        for bad in [
            "2", "2.0.1", "", ".", "2.", ".0", "v2.0", "two.zero", "-1.0", "2.0 ",
        ] {
            assert_eq!(parse_schema_version(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn schema_major_mismatch_is_an_error() {
        // One major below and one above: neither can be interpreted by this build's field
        // layout. **Derived from the constant, not written out.** These were the literals
        // "1.0" and "3.0", which stopped testing what they claim the moment roadmap C13 moved
        // the schema to 3.0 — "3.0" became this build's own version and the test failed. A
        // version test that has to be edited whenever the version moves is a version test that
        // will eventually be edited wrongly.
        let (major, _) = parse_schema_version(CALIBRATION_SCHEMA_VERSION)
            .expect("this build's schema version parses");
        let below = format!("{}.0", major - 1);
        let above = format!("{}.0", major + 1);
        for foreign in [below.as_str(), above.as_str()] {
            let err = check_schema_version(foreign)
                .expect_err("a foreign schema major must be rejected, not warned about");
            assert!(
                err.contains(foreign) && err.contains(CALIBRATION_SCHEMA_VERSION),
                "the error must name both the artifact's version and this build's: {err}"
            );
        }
    }

    #[test]
    fn schema_minor_mismatch_loads() {
        // A minor bump leaves the byte layout and every field's meaning intact, so it
        // warns rather than failing (see CALIBRATION_SCHEMA_VERSION's bump policy).
        // Derived from the constant for the reason given in the test above.
        let (major, minor) = parse_schema_version(CALIBRATION_SCHEMA_VERSION)
            .expect("this build's schema version parses");
        assert!(check_schema_version(&format!("{major}.{}", minor + 7)).is_ok());
        assert!(check_schema_version(CALIBRATION_SCHEMA_VERSION).is_ok());
    }

    #[test]
    fn unreadable_schema_version_is_an_error() {
        let err = check_schema_version("garbage")
            .expect_err("an unparseable schema stamp must not be treated as compatible");
        assert!(
            err.contains("garbage"),
            "the error must quote the offending stamp: {err}"
        );
    }

    /// The wrong-version fixture required by roadmap unit D2, driven through the real
    /// loading path: a well-framed, CRC-clean, decodable artifact whose *schema* stamp
    /// this build cannot interpret must be rejected — the container check cannot catch
    /// this class, because the container is perfectly valid.
    #[test]
    fn test_load_rejects_foreign_schema_version() {
        let mut calibration = create_test_calibration();
        calibration.metadata.format_version = "1.0".to_string();

        let payload = postcard::to_allocvec(&calibration).unwrap();
        let bytes = make_antc_bytes(&payload, ANTC_ARTIFACT_VERSION, None);

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(
                    reason.contains("1.0"),
                    "expected the message to name the artifact's schema version, got: {reason}"
                );
            }
            other => panic!("expected LoadError for a foreign schema major, got {other:?}"),
        }
    }

    /// The legacy headerless path is guarded by the schema axis too — it is the *only*
    /// version check such a file gets, since it carries no container stamp at all.
    #[test]
    fn test_load_rejects_foreign_schema_version_headerless() {
        let mut calibration = create_test_calibration();
        calibration.metadata.format_version = "1.0".to_string();

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file
            .write_all(&postcard::to_allocvec(&calibration).unwrap())
            .unwrap();
        temp_file.flush().unwrap();

        assert!(
            load_calibration_artifact(temp_file.path()).is_err(),
            "a headerless artifact with a foreign schema major must still be rejected"
        );
    }

    #[test]
    fn test_load_accepts_differing_schema_minor() {
        let mut calibration = create_test_calibration();
        // Same major, a minor this build does not carry — derived, not a literal, so it stays
        // a MINOR test when the schema version moves (see `schema_major_mismatch_is_an_error`).
        let (major, minor) = parse_schema_version(CALIBRATION_SCHEMA_VERSION)
            .expect("this build's schema version parses");
        let differing_minor = format!("{major}.{}", minor + 9);
        calibration.metadata.format_version = differing_minor.clone();

        let payload = postcard::to_allocvec(&calibration).unwrap();
        let bytes = make_antc_bytes(&payload, ANTC_ARTIFACT_VERSION, None);

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&bytes).unwrap();
        temp_file.flush().unwrap();

        let loaded = load_calibration_artifact(temp_file.path())
            .expect("a differing schema MINOR must warn and load, not fail");
        assert_eq!(loaded.metadata.format_version, differing_minor);
    }

    #[test]
    fn test_physics_model_version_mismatch_message() {
        let msg = physics_model_version_mismatch(999, 1).expect("mismatch must warn");
        assert!(
            msg.contains("999") && msg.contains('1'),
            "must name both versions: {msg}"
        );
        assert!(physics_model_version_mismatch(1, 1).is_none());
        // 0 = unknown / pre-stamp artifact: still a mismatch worth warning about
        assert!(physics_model_version_mismatch(0, 1).is_some());
    }

    #[test]
    fn test_load_artifact_with_mismatched_physics_model_version() {
        let mut calibration = create_test_calibration();
        calibration.metadata.physics_model_version = 999;

        let mut temp_file = NamedTempFile::new().unwrap();
        let encoded = postcard::to_allocvec(&calibration).unwrap();
        temp_file.write_all(&encoded).unwrap();
        temp_file.flush().unwrap();

        // Mismatch must WARN, not error: load succeeds and preserves the stamp.
        let loaded = load_calibration_artifact(temp_file.path()).unwrap();
        assert_eq!(loaded.metadata.physics_model_version, 999);
    }
}
