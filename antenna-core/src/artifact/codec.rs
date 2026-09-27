use std::path::Path;

use tracing::{debug, info, warn};

use super::{validation, ArtifactError, FramingError, VersionError};
use crate::model::PHYSICS_MODEL_VERSION;
use crate::types::{AntennaCalibration, CALIBRATION_SCHEMA_VERSION};

/// Magic bytes opening every ANTC calibration artifact.
pub const ANTC_MAGIC: &[u8; 4] = b"ANTC";

/// The ANTC **container** version this build writes and the only one it decodes.
///
/// Bump it whenever existing files can no longer be decoded — a codec change or any payload
/// layout change — so they are rejected before a positional decode can misread them.
pub const ANTC_ARTIFACT_VERSION: u32 = 4;

/// Byte length of an ANTC header: magic 4 + version 4 + CRC32 4 + payload length 8.
pub const ANTC_HEADER_LEN: usize = 20;

/// Encodes an artifact as ANTC container bytes, stamping [`ANTC_ARTIFACT_VERSION`].
///
/// The schema stamp is whatever `metadata.format_version` already holds.
pub fn encode(calibration: &AntennaCalibration) -> Vec<u8> {
    // postcard into a growable Vec fails only for a sequence of unknown length or a custom
    // `Serialize` error; every calibration type derives `Serialize` over sized collections.
    #[allow(clippy::expect_used)]
    let payload =
        postcard::to_allocvec(calibration).expect("derived Serialize into a Vec is infallible");

    [
        ANTC_MAGIC.as_slice(),
        &ANTC_ARTIFACT_VERSION.to_le_bytes(),
        &crc32fast::hash(&payload).to_le_bytes(),
        &(payload.len() as u64).to_le_bytes(),
        &payload,
    ]
    .concat()
}

/// Decodes ANTC container bytes into a valid artifact — the entry point for any loader.
///
/// Checks run in order: ANTC framing, CRC32, container version, payload decode, schema
/// version, then artifact validation. Validation never runs on a foreign schema major. A
/// differing schema minor or physics-model version only warns.
///
/// # Example
/// ```
/// use antenna_core::artifact::{decode, ArtifactError, FramingError};
///
/// let err = decode(b"not an artifact").unwrap_err();
/// assert!(matches!(err, ArtifactError::Framing(FramingError::MissingHeader)));
/// ```
pub fn decode(bytes: &[u8]) -> Result<AntennaCalibration, ArtifactError> {
    let (version, payload) = unframe(bytes)?;
    if version != ANTC_ARTIFACT_VERSION {
        return Err(VersionError::Container { found: version }.into());
    }

    let calibration: AntennaCalibration =
        postcard::from_bytes(payload).map_err(FramingError::Undecodable)?;
    check_schema_version(&calibration.metadata.format_version)?;
    validation::validate(&calibration)?;

    info!(
        antenna_id = %calibration.antenna_id,
        feed_id = %calibration.feed_id,
        format_version = %calibration.metadata.format_version,
        has_correction_surface = calibration.correction_surface.is_some(),
        "decoded calibration artifact"
    );
    if calibration.metadata.physics_model_version != PHYSICS_MODEL_VERSION {
        // Correction surfaces are fitted to `measured − physics` residuals, so a stale
        // physics model degrades accuracy silently; it warns rather than errors (P1b).
        warn!(
            artifact_physics_model_version = calibration.metadata.physics_model_version,
            service_physics_model_version = PHYSICS_MODEL_VERSION,
            "calibration artifact was fitted against a different physics model; \
             residual corrections may be stale — recalibrate"
        );
    }

    Ok(calibration)
}

/// Reads and [`decode`]s the artifact file at `path`.
///
/// # Example
/// ```no_run
/// let calibration = antenna_core::artifact::read("calibration_data/antenna_1.bin")?;
/// println!("{} / {}", calibration.antenna_id, calibration.feed_id);
/// # Ok::<(), antenna_core::artifact::ArtifactError>(())
/// ```
pub fn read(path: impl AsRef<Path>) -> Result<AntennaCalibration, ArtifactError> {
    let path = path.as_ref();
    debug!(path = %path.display(), "reading calibration artifact");
    decode(&std::fs::read(path).map_err(|source| io_error(path, source))?)
}

/// [`encode`]s `calibration` and writes it to `path`.
pub fn write(
    path: impl AsRef<Path>,
    calibration: &AntennaCalibration,
) -> Result<(), ArtifactError> {
    let path = path.as_ref();
    std::fs::write(path, encode(calibration)).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: std::io::Error) -> ArtifactError {
    ArtifactError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Splits ANTC bytes into the container version and a CRC-checked payload.
fn unframe(bytes: &[u8]) -> Result<(u32, &[u8]), FramingError> {
    let (header, rest) = bytes
        .split_first_chunk::<ANTC_HEADER_LEN>()
        .filter(|(header, _)| header.starts_with(ANTC_MAGIC))
        .ok_or(FramingError::MissingHeader)?;
    let field = |at: usize| [header[at], header[at + 1], header[at + 2], header[at + 3]];
    let version = u32::from_le_bytes(field(4));
    let expected_crc = u32::from_le_bytes(field(8));
    let declared = u64::from_le_bytes([
        header[12], header[13], header[14], header[15], header[16], header[17], header[18],
        header[19],
    ]);

    let payload = usize::try_from(declared)
        .ok()
        .and_then(|len| rest.get(..len))
        .ok_or(FramingError::Truncated {
            declared,
            available: rest.len(),
        })?;
    let actual_crc = crc32fast::hash(payload);
    if actual_crc != expected_crc {
        return Err(FramingError::CrcMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }
    Ok((version, payload))
}

/// Splits a `MAJOR.MINOR` stamp; anything else is `None`, never "probably fine".
fn parse_schema_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    if minor.contains('.') {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Rejects a foreign or unreadable schema major; warns on a differing minor.
///
/// The supported version is parsed from [`CALIBRATION_SCHEMA_VERSION`] rather than restated,
/// so one constant governs writing and reading.
fn check_schema_version(artifact: &str) -> Result<(), VersionError> {
    let (major, minor) =
        parse_schema_version(artifact).ok_or_else(|| VersionError::UnreadableSchema {
            found: artifact.to_string(),
        })?;
    let supported = parse_schema_version(CALIBRATION_SCHEMA_VERSION);
    if supported.map(|(supported_major, _)| supported_major) != Some(major) {
        return Err(VersionError::SchemaMajor {
            found: artifact.to_string(),
        });
    }
    if supported.map(|(_, supported_minor)| supported_minor) != Some(minor) {
        warn!(
            artifact_schema = artifact,
            build_schema = CALIBRATION_SCHEMA_VERSION,
            "calibration schema differs in MINOR; layout is compatible, loading"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{fixtures, BSplineModel4D, CalibrationCoverage, CalibrationStatus};

    /// ANTC bytes around an arbitrary payload — the raw escape hatch for framing tests only.
    fn frame(payload: &[u8], version: u32, crc: u32) -> Vec<u8> {
        [
            ANTC_MAGIC.as_slice(),
            &version.to_le_bytes(),
            &crc.to_le_bytes(),
            &(payload.len() as u64).to_le_bytes(),
            payload,
        ]
        .concat()
    }

    fn valid() -> AntennaCalibration {
        fixtures::builder().build().unwrap()
    }

    fn payload(calibration: &AntennaCalibration) -> Vec<u8> {
        postcard::to_allocvec(calibration).unwrap()
    }

    fn major() -> u32 {
        parse_schema_version(CALIBRATION_SCHEMA_VERSION).unwrap().0
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let calibration = fixtures::calibrated().build().unwrap();
        assert_eq!(decode(&encode(&calibration)).unwrap(), calibration);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact.bin");
        let calibration = valid();
        write(&path, &calibration).unwrap();
        assert_eq!(read(&path).unwrap(), calibration);
    }

    #[test]
    fn read_of_a_missing_file_is_io_naming_the_path() {
        match read("/nonexistent/path/to/file.bin") {
            Err(ArtifactError::Io { path, .. }) => {
                assert!(path.ends_with("nonexistent/path/to/file.bin"))
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn write_to_an_unwritable_path_is_io() {
        assert!(matches!(
            write("/nonexistent/dir/file.bin", &valid()),
            Err(ArtifactError::Io { .. })
        ));
    }

    #[test]
    fn unframed_bytes_are_refused_for_framing() {
        assert!(matches!(
            decode(b"invalid binary data"),
            Err(ArtifactError::Framing(FramingError::MissingHeader))
        ));
    }

    /// Guards against reinstating the headerless fallback (D27).
    #[test]
    fn headerless_payload_is_refused_even_when_otherwise_valid() {
        let calibration = valid();
        decode(&encode(&calibration)).expect("control: the framed form must decode");
        assert!(matches!(
            decode(&payload(&calibration)),
            Err(ArtifactError::Framing(FramingError::MissingHeader))
        ));
    }

    #[test]
    fn declared_length_beyond_the_data_is_truncated() {
        let mut bytes = encode(&valid());
        let available = bytes.len() - ANTC_HEADER_LEN;
        let declared = available as u64 + 100;
        bytes[12..20].copy_from_slice(&declared.to_le_bytes());
        match decode(&bytes) {
            Err(ArtifactError::Framing(FramingError::Truncated {
                declared: d,
                available: a,
            })) => assert_eq!((d, a), (declared, available)),
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn corrupted_payload_is_a_crc_mismatch() {
        let mut bytes = encode(&valid());
        if let Some(last) = bytes.last_mut() {
            *last ^= 0xff;
        }
        assert!(matches!(
            decode(&bytes),
            Err(ArtifactError::Framing(FramingError::CrcMismatch { .. }))
        ));
    }

    #[test]
    fn foreign_container_version_is_rejected() {
        // Derived, never a literal, so it stays "unsupported" when the version moves.
        let unsupported = ANTC_ARTIFACT_VERSION + 1;
        let mut bytes = encode(&valid());
        bytes[4..8].copy_from_slice(&unsupported.to_le_bytes());
        match decode(&bytes) {
            Err(ArtifactError::Version(VersionError::Container { found })) => {
                assert_eq!(found, unsupported)
            }
            other => panic!("expected a container version error, got {other:?}"),
        }
    }

    /// A truncated real payload, not random bytes, keeps the failure deterministic.
    #[test]
    fn intact_but_undecodable_payload_is_refused() {
        let full = payload(&valid());
        let truncated = &full[..full.len() / 2];
        let bytes = frame(truncated, ANTC_ARTIFACT_VERSION, crc32fast::hash(truncated));
        assert!(matches!(
            decode(&bytes),
            Err(ArtifactError::Framing(FramingError::Undecodable(_)))
        ));
    }

    /// Validation rules must not be applied to fields whose meaning a foreign schema major
    /// may have changed: a payload that is both foreign and invalid reports the version.
    #[test]
    fn foreign_schema_major_is_rejected_before_validation() {
        let mut invalid = fixtures::calibrated().build().unwrap();
        invalid.calibration_coverage = None;
        assert!(
            matches!(decode(&encode(&invalid)), Err(ArtifactError::Validation(_))),
            "control: under this build's schema the payload must fail validation"
        );

        invalid.metadata.format_version = format!("{}.0", major() + 1);
        match decode(&encode(&invalid)) {
            Err(ArtifactError::Version(VersionError::SchemaMajor { found })) => {
                assert_eq!(found, invalid.metadata.format_version)
            }
            other => panic!("expected a schema major error, got {other:?}"),
        }
    }

    #[test]
    fn foreign_schema_major_above_or_below_is_rejected() {
        for foreign in [major() - 1, major() + 1] {
            let mut calibration = valid();
            calibration.metadata.format_version = format!("{foreign}.0");
            assert!(matches!(
                decode(&encode(&calibration)),
                Err(ArtifactError::Version(VersionError::SchemaMajor { .. }))
            ));
        }
    }

    #[test]
    fn unreadable_schema_stamp_is_rejected() {
        let mut calibration = valid();
        calibration.metadata.format_version = "garbage".to_string();
        assert!(matches!(
            decode(&encode(&calibration)),
            Err(ArtifactError::Version(
                VersionError::UnreadableSchema { .. }
            ))
        ));
    }

    #[test]
    fn differing_schema_minor_loads() {
        let (major, minor) = parse_schema_version(CALIBRATION_SCHEMA_VERSION).unwrap();
        let mut calibration = valid();
        calibration.metadata.format_version = format!("{major}.{}", minor + 9);
        assert_eq!(decode(&encode(&calibration)).unwrap(), calibration);
    }

    /// Guards against an unparseable constant making every artifact a schema-major error.
    #[test]
    fn supported_schema_version_constant_is_parseable() {
        assert!(parse_schema_version(CALIBRATION_SCHEMA_VERSION).is_some());
        assert!(check_schema_version(CALIBRATION_SCHEMA_VERSION).is_ok());
    }

    #[test]
    fn schema_version_parses_only_major_dot_minor() {
        assert_eq!(parse_schema_version("2.0"), Some((2, 0)));
        assert_eq!(parse_schema_version("10.37"), Some((10, 37)));
        for bad in [
            "2", "2.0.1", "", ".", "2.", ".0", "v2.0", "two.zero", "-1.0", "2.0 ",
        ] {
            assert_eq!(parse_schema_version(bad), None, "{bad:?} must not parse");
        }
    }

    /// Guards the #97 rejections on decode, where a value arrives without the builder.
    #[test]
    fn coverage_rejections_hold_on_decode() {
        let calibrated = fixtures::calibrated().build().unwrap();
        decode(&encode(&calibrated)).expect("control: the unmodified fixture must decode");

        let rejected = |mutate: fn(&mut AntennaCalibration), expected: &str| {
            let mut calibration = calibrated.clone();
            mutate(&mut calibration);
            match decode(&encode(&calibration)) {
                Err(ArtifactError::Validation(e)) => assert!(
                    e.to_string().contains(expected),
                    "rejection must say {expected:?}: {e}"
                ),
                other => panic!("expected a rejection naming {expected:?}, got {other:?}"),
            }
        };

        rejected(
            |c| c.calibration_coverage = None,
            "calibration_coverage is absent",
        );
        rejected(
            |c| {
                if let Some(coverage) = c.calibration_coverage.as_mut() {
                    coverage.elevation_range.1 = 45.0;
                }
            },
            "is not contained by the correction_surface's fitted support",
        );
        rejected(
            |c| {
                c.calibration_status = Some(CalibrationStatus::PartiallyCalibrated {
                    accuracy_estimate_db: 1.5,
                    coverage: CalibrationCoverage::boresight_cone((8_000.0, 8_500.0), 28, true),
                })
            },
            "calibration_status.coverage and calibration_coverage disagree",
        );
    }

    fn schema_5_0_with_surface(coefficients: Vec<f64>) -> AntennaCalibration {
        let mut calibration = valid();
        calibration.metadata.format_version = "5.0".to_string();
        calibration.correction_surface = Some(BSplineModel4D {
            coefficients,
            shape: [2, 2, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 20.0, 20.0],
            knots_frequency: vec![8_000.0, 8_000.0, 9_000.0, 9_000.0],
            knots_temperature: vec![280.0, 280.0, 300.0, 300.0],
            spline_order: 2,
        });
        calibration.calibration_coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 10.0)
                .elevation_range(0.0, 20.0)
                .frequency_range(8_000.0, 9_000.0)
                .num_measurements(1000)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );
        calibration
    }

    #[test]
    fn schema_5_0_flat_temperature_artifact_loads_under_minor_policy() {
        let calibration = schema_5_0_with_surface(vec![1.0; 16]);
        assert_eq!(decode(&encode(&calibration)).unwrap(), calibration);
    }

    #[test]
    fn schema_5_0_temperature_varying_artifact_is_rejected() {
        let calibration = schema_5_0_with_surface([vec![1.0; 8], vec![2.0; 8]].concat());
        match decode(&encode(&calibration)) {
            Err(ArtifactError::Validation(e)) => assert!(
                e.to_string().contains("temperature slab 1"),
                "rejection must name the unequal slab: {e}"
            ),
            other => panic!("temperature-varying surface must be rejected, got {other:?}"),
        }
    }

    /// A physics-model mismatch warns, never errors, and the stamp survives.
    #[test]
    fn mismatched_physics_model_version_loads() {
        let mut calibration = valid();
        calibration.metadata.physics_model_version = PHYSICS_MODEL_VERSION + 1;
        assert_eq!(decode(&encode(&calibration)).unwrap(), calibration);
    }
}
