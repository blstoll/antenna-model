use serde::{Deserialize, Serialize};

/// Spatial density of the measurement data relative to the antenna beamwidth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MeasurementDensity {
    /// No measurements: design specifications only.
    None,

    /// Boresight only: one spatial point, possibly many frequencies.
    BoresightOnly,

    /// 2–5 points per beamwidth: enough to tune parameters, limited correction surface.
    Sparse {
        /// Average measurement points per beamwidth.
        points_per_beam: f64,
    },

    /// More than 10 points per beamwidth: supports a high-quality correction surface.
    Dense {
        /// Average measurement points per beamwidth.
        points_per_beam: f64,
    },
}

impl MeasurementDensity {
    /// Average points per beamwidth, for the grid variants.
    pub fn points_per_beam(&self) -> Option<f64> {
        match self {
            MeasurementDensity::Sparse { points_per_beam }
            | MeasurementDensity::Dense { points_per_beam } => Some(*points_per_beam),
            _ => None,
        }
    }

    /// Whether any measurements were taken.
    pub fn has_measurements(&self) -> bool {
        !matches!(self, MeasurementDensity::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measurement_density_none() {
        let density = MeasurementDensity::None;
        assert_eq!(density.points_per_beam(), None);
        assert!(!density.has_measurements());
    }

    #[test]
    fn test_measurement_density_boresight_only() {
        let density = MeasurementDensity::BoresightOnly;
        assert_eq!(density.points_per_beam(), None);
        assert!(density.has_measurements());
    }

    #[test]
    fn test_measurement_density_sparse() {
        let density = MeasurementDensity::Sparse {
            points_per_beam: 3.5,
        };
        assert_eq!(density.points_per_beam(), Some(3.5));
        assert!(density.has_measurements());
    }

    #[test]
    fn test_measurement_density_dense() {
        let density = MeasurementDensity::Dense {
            points_per_beam: 15.0,
        };
        assert_eq!(density.points_per_beam(), Some(15.0));
        assert!(density.has_measurements());
    }
}
