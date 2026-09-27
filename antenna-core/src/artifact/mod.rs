//! Calibration artifacts: the one path from bytes to a trusted [`AntennaCalibration`] and
//! back.
//!
//! An artifact is `[magic 4][version u32 LE][crc32 u32 LE][len u64 LE][payload]`, the
//! payload a postcard-encoded [`AntennaCalibration`]. [`decode`] is the extension point for
//! any loader — file, object store, HTTP — and runs every check in order; [`encode`] is its
//! inverse, and [`read`]/[`write`] wrap both for files. This is the only definition of the
//! ANTC framing (D27): do not lay the header out by hand.
//!
//! Every [`AntennaCalibration`] comes from [`AntennaCalibrationBuilder::build`] or
//! [`decode`], and both run the one validation this module owns, so consumers never
//! re-check. There is deliberately no public validate function. See
//! `docs/adr/0001-artifacts-are-validated-at-construction.md`.
//!
//! # The two version axes
//!
//! | Axis | Where | Guards |
//! |---|---|---|
//! | **Container** ([`ANTC_ARTIFACT_VERSION`]) | ANTC header, readable before decoding | How file bytes become a payload: framing and codec. |
//! | **Schema** ([`CALIBRATION_SCHEMA_VERSION`](crate::types::CALIBRATION_SCHEMA_VERSION)) | `metadata.format_version`, readable only after decoding | What the payload's fields are and mean. |
//!
//! A postcard layout change bumps both. A meaning-only change bumps the schema: MAJOR if an
//! existing field's meaning changed, MINOR if meaning is only documented or validation
//! tightened. A framing or codec change bumps the container only. A foreign container or
//! schema MAJOR is an error; a differing MINOR warns and loads. History:
//! `docs/calibration-workflow-guide.md` §10.5.1.
//! [`CalibrationMetadata::physics_model_version`](crate::types::CalibrationMetadata::physics_model_version)
//! is a third, orthogonal axis and only warns.

mod codec;
mod error;
mod validation;

pub use codec::{decode, encode, read, write, ANTC_ARTIFACT_VERSION, ANTC_HEADER_LEN, ANTC_MAGIC};
pub use error::{ArtifactError, FramingError, VersionError};

use crate::types::{AntennaCalibration, AntennaCalibrationBuilder, ValidationError};

impl AntennaCalibrationBuilder {
    /// Assembles and validates the artifact.
    ///
    /// Fails with [`ValidationError::MissingField`] naming the first required field left
    /// unset, or with the first artifact invariant the assembled value breaks.
    pub fn build(self) -> Result<AntennaCalibration, ValidationError> {
        let calibration = self.assemble()?;
        validation::validate(&calibration)?;
        Ok(calibration)
    }
}
