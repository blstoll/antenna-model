use serde::{Deserialize, Serialize};

/// How an artifact's physical parameters (surface RMS, q-factor, mesh) were determined,
/// and so how far to trust them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterSource {
    /// Design specifications (vendor data, CAD). Typically ±20–30% per parameter.
    DesignSpecifications,

    /// Tuned from boresight measurements only. Typically ±5–10% per parameter.
    BoresightTuning {
        /// Number of boresight measurements used for tuning.
        num_measurements: usize,
    },

    /// Tuned from a partial measurement grid. Typically ±5–10% in coverage.
    PartialGridTuning {
        /// Number of grid measurements used for tuning.
        num_measurements: usize,
    },

    /// Tuned from a full measurement grid. Typically ±3–5% per parameter.
    FullGridTuning {
        /// Number of grid measurements used for tuning.
        num_measurements: usize,
    },
}

impl ParameterSource {
    /// The number of measurements used for tuning, or `None` for design specifications.
    pub fn num_measurements(&self) -> Option<usize> {
        match self {
            ParameterSource::DesignSpecifications => None,
            ParameterSource::BoresightTuning { num_measurements }
            | ParameterSource::PartialGridTuning { num_measurements }
            | ParameterSource::FullGridTuning { num_measurements } => Some(*num_measurements),
        }
    }

    /// Whether the parameters were tuned from measurements.
    pub fn is_tuned(&self) -> bool {
        !matches!(self, ParameterSource::DesignSpecifications)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_source_design_specifications() {
        let source = ParameterSource::DesignSpecifications;
        assert_eq!(source.num_measurements(), None);
        assert!(!source.is_tuned());
    }

    #[test]
    fn test_parameter_source_boresight_tuning() {
        let source = ParameterSource::BoresightTuning {
            num_measurements: 28,
        };
        assert_eq!(source.num_measurements(), Some(28));
        assert!(source.is_tuned());
    }

    #[test]
    fn test_parameter_source_partial_grid_tuning() {
        let source = ParameterSource::PartialGridTuning {
            num_measurements: 324,
        };
        assert_eq!(source.num_measurements(), Some(324));
        assert!(source.is_tuned());
    }

    #[test]
    fn test_parameter_source_full_grid_tuning() {
        let source = ParameterSource::FullGridTuning {
            num_measurements: 3312,
        };
        assert_eq!(source.num_measurements(), Some(3312));
        assert!(source.is_tuned());
    }
}
