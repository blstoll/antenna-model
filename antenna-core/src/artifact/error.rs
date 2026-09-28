use std::io;
use std::path::PathBuf;

use thiserror::Error;

use super::codec::ANTC_ARTIFACT_VERSION;
use crate::types::{ValidationError, CALIBRATION_SCHEMA_VERSION};

/// Why bytes or a file could not become a trusted [`crate::types::AntennaCalibration`].
///
/// The variants separate a damaged or foreign file ([`Framing`](Self::Framing),
/// [`Version`](Self::Version)) from a sound file describing an invalid model
/// ([`Validation`](Self::Validation)).
#[derive(Debug, Error)]
pub enum ArtifactError {
    /// The bytes are not an intact ANTC container around a decodable payload.
    #[error("malformed calibration artifact: {0}")]
    Framing(#[from] FramingError),

    /// The container or schema version is one this build does not read.
    #[error("unsupported calibration artifact: {0}")]
    Version(#[from] VersionError),

    /// The artifact file could not be read or written.
    #[error("I/O error on calibration artifact {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The artifact decoded under a supported schema but breaks an artifact invariant.
    #[error("invalid calibration artifact: {0}")]
    Validation(#[from] ValidationError),
}

/// How the ANTC container or its payload is damaged.
#[derive(Debug, Error)]
pub enum FramingError {
    /// The bytes do not start with an ANTC header. A bare postcard payload has no CRC to
    /// check, so it is refused rather than decoded (D27).
    #[error(
        "missing ANTC container header; regenerate the artifact with a current `calibrate` build"
    )]
    MissingHeader,

    /// The header declares a longer payload than the bytes hold.
    #[error("ANTC header declares a {declared}-byte payload but only {available} bytes follow")]
    Truncated { declared: u64, available: usize },

    /// The payload does not match the header's CRC32.
    #[error("CRC32 mismatch — artifact corrupted (expected {expected:#010x}, got {actual:#010x})")]
    CrcMismatch { expected: u32, actual: u32 },

    /// The payload is intact but does not decode as an `AntennaCalibration`.
    #[error("payload does not decode: {0}")]
    Undecodable(postcard::Error),
}

/// Which version axis rejected the artifact. See [`super`] for the two axes.
#[derive(Debug, Error)]
pub enum VersionError {
    /// The ANTC container version differs from [`ANTC_ARTIFACT_VERSION`].
    #[error("ANTC container version {found} (this build reads version {ANTC_ARTIFACT_VERSION})")]
    Container { found: u32 },

    /// The schema stamp's MAJOR differs from [`CALIBRATION_SCHEMA_VERSION`]'s: the payload's
    /// fields may mean something else, so none of them is trusted.
    #[error(
        "calibration schema version {found} (this build reads schema {CALIBRATION_SCHEMA_VERSION}); \
         recalibrate with a matching `calibrate` build"
    )]
    SchemaMajor { found: String },

    /// The schema stamp is not `MAJOR.MINOR`.
    #[error("unreadable calibration schema version {found:?} (expected MAJOR.MINOR, e.g. {CALIBRATION_SCHEMA_VERSION:?})")]
    UnreadableSchema { found: String },
}
