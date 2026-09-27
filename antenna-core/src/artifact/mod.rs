//! Calibration artifacts come into existence valid.
//!
//! Every [`AntennaCalibration`] is created by [`AntennaCalibrationBuilder::build`] or by
//! decoding an artifact file ([`crate::data::loader`]), and both run the one validation
//! this module owns: the data-only checks, the correction-surface knot-layout rules,
//! calibration coverage ⊆ correction-surface support, and the single-coverage-claim rule.
//! Consumers take any artifact as valid without re-checking. There is deliberately no
//! public validate function. See `docs/adr/0001-artifacts-are-validated-at-construction.md`.

mod validation;

// Crate-visible only for the loader, until #106 moves it into this module.
pub(crate) use validation::validate;

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
