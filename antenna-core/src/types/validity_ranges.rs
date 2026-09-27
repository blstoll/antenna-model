use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Nominal parameter ranges, reported in antenna metadata.
///
/// Informational only: queries outside them are neither rejected nor warned about.
/// Correction application and extrapolation warnings are governed by
/// [`super::CalibrationCoverage`] and the correction surface's fitted support.
///
/// Invariants, checked by [`Self::validate`]: every range has `min <= max`; elevation
/// (a polar angle off boresight) lies in `[0, 90]`; temperature is positive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidityRanges {
    /// Azimuth (E-clock) range, degrees: `(min, max)`.
    pub azimuth_min_max: (f64, f64),

    /// Elevation (E-cone) range, degrees: `(min, max)`.
    pub elevation_min_max: (f64, f64),

    /// Frequency range, MHz: `(min, max)`.
    pub frequency_min_max: (f64, f64),

    /// Constant physical temperature, kelvin.
    pub temperature_const: f64,
}

impl ValidityRanges {
    /// Creates a new builder for constructing `ValidityRanges`.
    pub fn builder() -> ValidityRangesBuilder {
        ValidityRangesBuilder::default()
    }

    /// Checks the invariants listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.azimuth_min_max.0 > self.azimuth_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "azimuth".to_string(),
                min: self.azimuth_min_max.0,
                max: self.azimuth_min_max.1,
            });
        }

        if self.elevation_min_max.0 > self.elevation_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_min_max.0,
                max: self.elevation_min_max.1,
            });
        }

        if self.frequency_min_max.0 > self.frequency_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "frequency".to_string(),
                min: self.frequency_min_max.0,
                max: self.frequency_min_max.1,
            });
        }

        if self.elevation_min_max.0 < 0.0 || self.elevation_min_max.1 > 90.0 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_min_max.0,
                max: self.elevation_min_max.1,
            });
        }

        if self.temperature_const <= 0.0 {
            return Err(ValidationError::InvalidTemperature(self.temperature_const));
        }

        Ok(())
    }

    /// Whether a query point lies within all three ranges, bounds inclusive.
    pub fn contains(&self, azimuth: f64, elevation: f64, frequency: f64) -> bool {
        azimuth >= self.azimuth_min_max.0
            && azimuth <= self.azimuth_min_max.1
            && elevation >= self.elevation_min_max.0
            && elevation <= self.elevation_min_max.1
            && frequency >= self.frequency_min_max.0
            && frequency <= self.frequency_min_max.1
    }
}

/// Builder for [`ValidityRanges`]. Every field is required.
#[derive(Default)]
pub struct ValidityRangesBuilder {
    azimuth_min_max: Option<(f64, f64)>,
    elevation_min_max: Option<(f64, f64)>,
    frequency_min_max: Option<(f64, f64)>,
    temperature_const: Option<f64>,
}

impl ValidityRangesBuilder {
    pub fn azimuth_range(mut self, min: f64, max: f64) -> Self {
        self.azimuth_min_max = Some((min, max));
        self
    }

    pub fn elevation_range(mut self, min: f64, max: f64) -> Self {
        self.elevation_min_max = Some((min, max));
        self
    }

    pub fn frequency_range(mut self, min: f64, max: f64) -> Self {
        self.frequency_min_max = Some((min, max));
        self
    }

    pub fn temperature(mut self, temp: f64) -> Self {
        self.temperature_const = Some(temp);
        self
    }

    pub fn build(self) -> Result<ValidityRanges, String> {
        Ok(ValidityRanges {
            azimuth_min_max: self.azimuth_min_max.ok_or("azimuth_min_max is required")?,
            elevation_min_max: self
                .elevation_min_max
                .ok_or("elevation_min_max is required")?,
            frequency_min_max: self
                .frequency_min_max
                .ok_or("frequency_min_max is required")?,
            temperature_const: self
                .temperature_const
                .ok_or("temperature_const is required")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validity_ranges_builder() {
        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        assert_eq!(ranges.azimuth_min_max, (0.0, 360.0));
        assert_eq!(ranges.elevation_min_max, (0.0, 90.0));
        assert_eq!(ranges.frequency_min_max, (8000.0, 8500.0));
        assert_eq!(ranges.temperature_const, 290.0);
    }

    #[test]
    fn test_validity_ranges_validate() {
        let valid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(valid_ranges.validate().is_ok());

        // Invalid: min > max
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (360.0, 0.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(invalid_ranges.validate().is_err());

        // Invalid: elevation out of range
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (-10.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(invalid_ranges.validate().is_err());

        // Invalid: negative temperature
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: -10.0,
        };
        assert!(invalid_ranges.validate().is_err());
    }

    #[test]
    fn test_validity_ranges_contains() {
        let ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (10.0, 80.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };

        assert!(ranges.contains(45.0, 30.0, 8200.0));
        assert!(!ranges.contains(45.0, 5.0, 8200.0)); // elevation too low
        assert!(!ranges.contains(45.0, 30.0, 7000.0)); // frequency too low
    }
}
