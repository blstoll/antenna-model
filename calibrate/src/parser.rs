/// Measurement data parser and validation
///
/// This module provides functionality to parse antenna measurement data from CSV files
/// (local or S3), validate the data quality, and generate coverage statistics.
use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::warn;

/// G/T range typical of a **boresight** figure for a realistic antenna, in dB/K.
///
/// Used only to flag points for operator attention in [`DataQualityReport`]. It is *not*
/// a validity test: off-axis measurements legitimately fall far below the lower bound.
pub const TYPICAL_G_OVER_T_RANGE_DB: std::ops::RangeInclusive<f64> = -20.0..=70.0;

/// Maximum number of individual parse/validation failures logged verbatim; beyond this
/// only the count is reported, so a large drop is loud rather than a wall of text.
const MAX_REPORTED_PARSE_ERRORS: usize = 10;

/// A single measurement point from calibration data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementPoint {
    /// E-clock angle in degrees (0-360)
    pub e_clock_deg: f64,
    /// E-cone angle in degrees (0-90 typical)
    pub e_cone_deg: f64,
    /// Frequency in MHz
    pub frequency_mhz: f64,
    /// G/T ratio in dB/K
    pub g_over_t_db: f64,
    /// System noise temperature in Kelvin
    pub temperature_k: f64,
}

impl MeasurementPoint {
    /// Create a new measurement point
    pub fn new(
        e_clock_deg: f64,
        e_cone_deg: f64,
        frequency_mhz: f64,
        g_over_t_db: f64,
        temperature_k: f64,
    ) -> Self {
        Self {
            e_clock_deg,
            e_cone_deg,
            frequency_mhz,
            g_over_t_db,
            temperature_k,
        }
    }

    /// Extract gain from G/T using the noise temperature
    ///
    /// Gain (dB) = G/T (dB/K) + 10*log10(T_sys)
    pub fn gain_db(&self) -> f64 {
        self.g_over_t_db + 10.0 * self.temperature_k.log10()
    }

    /// Validate that the measurement point has physically reasonable values
    ///
    /// This is a **physicality** check only: finite values, plus the angle, frequency and
    /// temperature bounds below. It deliberately does *not* range-check `g_over_t_db` —
    /// a real pattern falls tens of dB below its boresight figure off the main lobe, so
    /// any fixed G/T floor rejects legitimate sidelobe measurements as if they were
    /// malformed. Atypical G/T is surfaced as a *warning* on
    /// [`DataQualityReport`] instead, where an operator can see it without losing the
    /// data. See roadmap unit D11.
    pub fn validate(&self) -> Result<()> {
        if !self.e_clock_deg.is_finite()
            || !self.e_cone_deg.is_finite()
            || !self.frequency_mhz.is_finite()
            || !self.g_over_t_db.is_finite()
            || !self.temperature_k.is_finite()
        {
            anyhow::bail!(
                "Measurement contains a non-finite value \
                 (e_clock={}, e_cone={}, frequency={}, g_over_t={}, temperature={})",
                self.e_clock_deg,
                self.e_cone_deg,
                self.frequency_mhz,
                self.g_over_t_db,
                self.temperature_k
            );
        }
        if !(0.0..=360.0).contains(&self.e_clock_deg) {
            anyhow::bail!(
                "E-clock angle {} deg is out of valid range [0, 360]",
                self.e_clock_deg
            );
        }
        if !(-90.0..=90.0).contains(&self.e_cone_deg) {
            anyhow::bail!(
                "E-cone angle {} deg is out of valid range [-90, 90]",
                self.e_cone_deg
            );
        }
        if self.frequency_mhz <= 0.0 || self.frequency_mhz > 100_000.0 {
            anyhow::bail!(
                "Frequency {} MHz is out of valid range (0, 100000]",
                self.frequency_mhz
            );
        }
        if self.temperature_k <= 0.0 || self.temperature_k > 1000.0 {
            anyhow::bail!(
                "Temperature {} K is out of valid range (0, 1000]",
                self.temperature_k
            );
        }
        Ok(())
    }

    /// Whether this point's G/T falls outside the range typical of a *boresight* figure.
    ///
    /// Reported as a quality warning, never a rejection: values below the floor are the
    /// expected signature of off-axis measurements, not of bad data.
    pub fn has_atypical_g_over_t(&self) -> bool {
        !TYPICAL_G_OVER_T_RANGE_DB.contains(&self.g_over_t_db)
    }

    /// Check if this measurement is likely in the main lobe (near boresight)
    ///
    /// Main lobe is typically within ~3 beamwidths from boresight (0° E-cone)
    pub fn is_main_lobe(&self, beamwidth_deg: f64) -> bool {
        self.e_cone_deg.abs() < 3.0 * beamwidth_deg
    }
}

/// Collection of measurement points with validation and statistics
#[derive(Debug, Clone)]
pub struct MeasurementData {
    /// All measurement points
    pub points: Vec<MeasurementPoint>,
    /// Original source (file path or S3 URL)
    pub source: String,
}

impl MeasurementData {
    /// Create a new measurement data collection
    pub fn new(points: Vec<MeasurementPoint>, source: String) -> Self {
        Self { points, source }
    }

    /// Get the number of measurement points
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Check if the measurement data is empty
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Get frequency range covered by measurements
    pub fn frequency_range(&self) -> (f64, f64) {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for point in &self.points {
            min = min.min(point.frequency_mhz);
            max = max.max(point.frequency_mhz);
        }
        (min, max)
    }

    /// Get E-cone angular range covered by measurements
    pub fn e_cone_range(&self) -> (f64, f64) {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for point in &self.points {
            min = min.min(point.e_cone_deg);
            max = max.max(point.e_cone_deg);
        }
        (min, max)
    }

    /// Get E-clock angular range covered by measurements
    pub fn e_clock_range(&self) -> (f64, f64) {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for point in &self.points {
            min = min.min(point.e_clock_deg);
            max = max.max(point.e_clock_deg);
        }
        (min, max)
    }

    /// Count measurements at each unique frequency
    pub fn frequency_distribution(&self) -> HashMap<String, usize> {
        let mut dist = HashMap::new();
        for point in &self.points {
            // Round to 1 decimal for grouping
            let key = format!("{:.1}", point.frequency_mhz);
            *dist.entry(key).or_insert(0) += 1;
        }
        dist
    }

    /// Detect outliers using modified Z-score method
    ///
    /// Returns indices of measurements that are statistical outliers in G/T
    pub fn detect_outliers(&self, threshold: f64) -> Vec<usize> {
        if self.points.len() < 3 {
            return Vec::new();
        }

        // Calculate median and MAD (Median Absolute Deviation)
        let mut g_over_t_values: Vec<f64> = self.points.iter().map(|p| p.g_over_t_db).collect();
        g_over_t_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let median = if g_over_t_values.len().is_multiple_of(2) {
            let mid = g_over_t_values.len() / 2;
            (g_over_t_values[mid - 1] + g_over_t_values[mid]) / 2.0
        } else {
            g_over_t_values[g_over_t_values.len() / 2]
        };

        // Calculate MAD
        let mut deviations: Vec<f64> = self
            .points
            .iter()
            .map(|p| (p.g_over_t_db - median).abs())
            .collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mad = if deviations.len().is_multiple_of(2) {
            let mid = deviations.len() / 2;
            (deviations[mid - 1] + deviations[mid]) / 2.0
        } else {
            deviations[deviations.len() / 2]
        };

        // Modified Z-score = 0.6745 * (x - median) / MAD
        let mut outliers = Vec::new();
        if mad > 1e-10 {
            // Avoid division by zero
            for (i, point) in self.points.iter().enumerate() {
                let modified_z = 0.6745 * (point.g_over_t_db - median).abs() / mad;
                if modified_z > threshold {
                    outliers.push(i);
                }
            }
        }

        outliers
    }

    /// Generate a data quality report
    pub fn quality_report(&self, estimated_beamwidth_deg: f64) -> DataQualityReport {
        let (freq_min, freq_max) = self.frequency_range();
        let (cone_min, cone_max) = self.e_cone_range();
        let (clock_min, clock_max) = self.e_clock_range();

        let main_lobe_count = self
            .points
            .iter()
            .filter(|p| p.is_main_lobe(estimated_beamwidth_deg))
            .count();

        let outliers = self.detect_outliers(3.5); // 3.5 is common threshold

        let freq_dist = self.frequency_distribution();
        let unique_frequencies = freq_dist.len();

        let atypical_g_over_t_count = self
            .points
            .iter()
            .filter(|p| p.has_atypical_g_over_t())
            .count();
        let g_over_t_range = self
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.g_over_t_db), hi.max(p.g_over_t_db))
            });

        DataQualityReport {
            total_points: self.points.len(),
            frequency_range: (freq_min, freq_max),
            e_cone_range: (cone_min, cone_max),
            e_clock_range: (clock_min, clock_max),
            g_over_t_range,
            unique_frequencies,
            main_lobe_points: main_lobe_count,
            sidelobe_points: self.points.len() - main_lobe_count,
            atypical_g_over_t_count,
            outlier_count: outliers.len(),
            outlier_indices: outliers,
            frequency_distribution: freq_dist,
        }
    }
}

/// Data quality report summarizing measurement coverage and issues
#[derive(Debug, Clone, Serialize)]
pub struct DataQualityReport {
    pub total_points: usize,
    pub frequency_range: (f64, f64),
    pub e_cone_range: (f64, f64),
    pub e_clock_range: (f64, f64),
    /// Observed (min, max) G/T in dB/K across all points.
    pub g_over_t_range: (f64, f64),
    pub unique_frequencies: usize,
    pub main_lobe_points: usize,
    pub sidelobe_points: usize,
    /// Points whose G/T falls outside [`TYPICAL_G_OVER_T_RANGE_DB`].
    ///
    /// A warning, not a defect count: off-axis measurements are *expected* to sit below
    /// the boresight-typical floor. A high count on data claimed to be main-lobe-only, or
    /// values above the upper bound, is what warrants a second look.
    pub atypical_g_over_t_count: usize,
    pub outlier_count: usize,
    #[serde(skip)]
    pub outlier_indices: Vec<usize>,
    pub frequency_distribution: HashMap<String, usize>,
}

impl DataQualityReport {
    /// Format the report as a human-readable string
    pub fn format(&self) -> String {
        format!(
            r#"Data Quality Report
==================
Total Points: {}
Frequency Range: {:.1} - {:.1} MHz ({} unique frequencies)
E-Cone Range: {:.2}° - {:.2}°
E-Clock Range: {:.2}° - {:.2}°
G/T Range: {:.2} - {:.2} dB/K
Main Lobe Points: {} ({:.1}%)
Sidelobe Points: {} ({:.1}%)
Outliers Detected: {} ({:.1}%)
{}
Frequency Distribution:
{}"#,
            self.total_points,
            self.frequency_range.0,
            self.frequency_range.1,
            self.unique_frequencies,
            self.e_cone_range.0,
            self.e_cone_range.1,
            self.e_clock_range.0,
            self.e_clock_range.1,
            self.g_over_t_range.0,
            self.g_over_t_range.1,
            self.main_lobe_points,
            100.0 * self.main_lobe_points as f64 / self.total_points as f64,
            self.sidelobe_points,
            100.0 * self.sidelobe_points as f64 / self.total_points as f64,
            self.outlier_count,
            100.0 * self.outlier_count as f64 / self.total_points as f64,
            self.format_atypical_g_over_t(),
            self.format_frequency_distribution()
        )
    }

    /// Warning line for points outside the boresight-typical G/T range, or blank.
    fn format_atypical_g_over_t(&self) -> String {
        if self.atypical_g_over_t_count == 0 {
            return String::new();
        }
        format!(
            "\n⚠ G/T outside the boresight-typical range [{:.0}, {:.0}] dB/K: {} points \
             ({:.1}%). Expected for off-axis measurements; check the data if these are \
             meant to be main-lobe points.\n",
            TYPICAL_G_OVER_T_RANGE_DB.start(),
            TYPICAL_G_OVER_T_RANGE_DB.end(),
            self.atypical_g_over_t_count,
            100.0 * self.atypical_g_over_t_count as f64 / self.total_points as f64,
        )
    }

    fn format_frequency_distribution(&self) -> String {
        let mut freqs: Vec<_> = self.frequency_distribution.iter().collect();
        freqs.sort_by(|a, b| {
            let fa = a.0.parse::<f64>().unwrap_or(f64::NAN);
            let fb = b.0.parse::<f64>().unwrap_or(f64::NAN);
            fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
        });

        freqs
            .iter()
            .map(|(freq, count)| format!("  {} MHz: {} points", freq, count))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Parse measurements from a CSV file (local or S3)
///
/// Supports both local file paths and S3 URLs (s3://bucket/key)
pub async fn parse_measurements(source: &str) -> Result<MeasurementData> {
    let content = if source.starts_with("s3://") {
        fetch_from_s3(source).await?
    } else {
        std::fs::read_to_string(source).context("Failed to read local file")?
    };

    parse_csv_content(&content, source)
}

/// Synchronous version of parse_measurements for local files only
pub fn parse_measurements_sync(file_path: &str) -> Result<MeasurementData> {
    if file_path.starts_with("s3://") {
        anyhow::bail!("S3 URLs require async parse_measurements function");
    }

    let content = std::fs::read_to_string(file_path).context("Failed to read local file")?;
    parse_csv_content(&content, file_path)
}

/// Parse CSV content into measurement data
fn parse_csv_content(content: &str, source: &str) -> Result<MeasurementData> {
    let mut reader = csv::Reader::from_reader(content.as_bytes());
    let mut points = Vec::new();
    let mut errors = Vec::new();

    for (line_num, result) in reader.deserialize().enumerate() {
        let record: MeasurementPoint = match result {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("Line {}: {}", line_num + 2, e)); // +2 for header and 0-indexing
                continue;
            }
        };

        // Validate each point
        if let Err(e) = record.validate() {
            errors.push(format!("Line {}: {}", line_num + 2, e));
            continue;
        }

        points.push(record);
    }

    if points.is_empty() {
        if errors.is_empty() {
            anyhow::bail!("No valid measurement points found in file");
        } else {
            anyhow::bail!(
                "No valid measurement points found. Errors:\n{}",
                errors.join("\n")
            );
        }
    }

    // Warn about dropped rows but don't fail if we have some valid points. Routed through
    // `tracing` (not `eprintln!`) so it honors the configured log level, formatter and
    // sinks — a silent stderr write is how the pre-D11 G/T gate hid a 40% data loss.
    if !errors.is_empty() {
        let sample: Vec<&str> = errors
            .iter()
            .take(MAX_REPORTED_PARSE_ERRORS)
            .map(String::as_str)
            .collect();
        warn!(
            source = source,
            dropped = errors.len(),
            retained = points.len(),
            "Dropped {} of {} rows while parsing measurements; first {} reason(s): {}",
            errors.len(),
            errors.len() + points.len(),
            sample.len(),
            sample.join(" | ")
        );
        if errors.len() > MAX_REPORTED_PARSE_ERRORS {
            warn!(
                "... {} further parse failures not shown",
                errors.len() - MAX_REPORTED_PARSE_ERRORS
            );
        }
    }

    Ok(MeasurementData::new(points, source.to_string()))
}

/// Fetch content from S3
async fn fetch_from_s3(s3_url: &str) -> Result<String> {
    // Parse S3 URL: s3://bucket/key
    let url_parts: Vec<&str> = s3_url
        .strip_prefix("s3://")
        .context("Invalid S3 URL format")?
        .splitn(2, '/')
        .collect();

    if url_parts.len() != 2 {
        anyhow::bail!("Invalid S3 URL format. Expected s3://bucket/key");
    }

    let bucket = url_parts[0];
    let key = url_parts[1];

    // Create S3 client
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let client = S3Client::new(&config);

    // Fetch object
    let response = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .context("Failed to fetch object from S3")?;

    // Read body
    let body = response
        .body
        .collect()
        .await
        .context("Failed to read S3 object body")?;

    let content = String::from_utf8(body.to_vec()).context("S3 object is not valid UTF-8")?;

    Ok(content)
}

/// Create a sample CSV file for testing
pub fn create_sample_csv<P: AsRef<Path>>(path: P, num_points: usize) -> Result<()> {
    use std::fs::File;
    use std::io::Write;

    let mut file = File::create(path)?;

    // Write header
    writeln!(
        file,
        "e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k"
    )?;

    // Generate sample points
    // Main lobe: E-cone 0-5 degrees
    // First sidelobe: E-cone 5-10 degrees
    // Far sidelobes: E-cone 10-30 degrees

    let frequencies = [2200.0, 8400.0, 12000.0]; // S, X, Ku bands
    let temperature_k = 50.0; // Typical system temperature

    for i in 0..num_points {
        let e_clock = (i as f64 * 360.0 / num_points as f64) % 360.0;
        let e_cone = (i as f64 * 30.0 / num_points as f64) % 30.0;
        let freq = frequencies[i % frequencies.len()];

        // Simulate realistic G/T pattern
        // Peak at boresight, decreasing with angle
        let gain_loss = (e_cone / 5.0).powi(2); // Quadratic falloff
        let base_g_over_t = 41.5; // Peak G/T
        let g_over_t = base_g_over_t - gain_loss;

        writeln!(
            file,
            "{},{},{},{},{}",
            e_clock, e_cone, freq, g_over_t, temperature_k
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measurement_point_creation() {
        let point = MeasurementPoint::new(45.0, 10.0, 8400.0, 41.5, 50.0);
        assert_eq!(point.e_clock_deg, 45.0);
        assert_eq!(point.e_cone_deg, 10.0);
        assert_eq!(point.frequency_mhz, 8400.0);
        assert_eq!(point.g_over_t_db, 41.5);
        assert_eq!(point.temperature_k, 50.0);
    }

    #[test]
    fn test_gain_extraction() {
        let point = MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0);
        let gain = point.gain_db();
        // Gain = G/T + 10*log10(T) = 41.5 + 10*log10(50) = 41.5 + 16.99 ≈ 58.49
        assert!((gain - 58.49).abs() < 0.01, "Gain calculation incorrect");
    }

    #[test]
    fn test_measurement_validation_valid() {
        let point = MeasurementPoint::new(45.0, 10.0, 8400.0, 41.5, 50.0);
        assert!(point.validate().is_ok());
    }

    #[test]
    fn test_measurement_validation_invalid_clock() {
        let point = MeasurementPoint::new(-10.0, 10.0, 8400.0, 41.5, 50.0);
        assert!(point.validate().is_err());

        let point = MeasurementPoint::new(370.0, 10.0, 8400.0, 41.5, 50.0);
        assert!(point.validate().is_err());
    }

    #[test]
    fn test_measurement_validation_invalid_cone() {
        let point = MeasurementPoint::new(45.0, -100.0, 8400.0, 41.5, 50.0);
        assert!(point.validate().is_err());

        let point = MeasurementPoint::new(45.0, 100.0, 8400.0, 41.5, 50.0);
        assert!(point.validate().is_err());
    }

    #[test]
    fn test_measurement_validation_invalid_frequency() {
        let point = MeasurementPoint::new(45.0, 10.0, -100.0, 41.5, 50.0);
        assert!(point.validate().is_err());

        let point = MeasurementPoint::new(45.0, 10.0, 150_000.0, 41.5, 50.0);
        assert!(point.validate().is_err());
    }

    #[test]
    fn test_measurement_validation_invalid_temperature() {
        let point = MeasurementPoint::new(45.0, 10.0, 8400.0, 41.5, -10.0);
        assert!(point.validate().is_err());

        let point = MeasurementPoint::new(45.0, 10.0, 8400.0, 41.5, 2000.0);
        assert!(point.validate().is_err());
    }

    #[test]
    fn test_main_lobe_detection() {
        let beamwidth = 2.0; // 2 degree beamwidth

        let point = MeasurementPoint::new(0.0, 1.0, 8400.0, 41.5, 50.0);
        assert!(point.is_main_lobe(beamwidth));

        let point = MeasurementPoint::new(0.0, 5.0, 8400.0, 41.5, 50.0);
        assert!(point.is_main_lobe(beamwidth));

        let point = MeasurementPoint::new(0.0, 10.0, 8400.0, 41.5, 50.0);
        assert!(!point.is_main_lobe(beamwidth));
    }

    #[test]
    fn test_measurement_data_creation() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, 40.5, 50.0),
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        assert_eq!(data.len(), 2);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_frequency_range() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, 2200.0, 40.5, 50.0),
            MeasurementPoint::new(90.0, 15.0, 12000.0, 39.5, 50.0),
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        let (min, max) = data.frequency_range();
        assert_eq!(min, 2200.0);
        assert_eq!(max, 12000.0);
    }

    #[test]
    fn test_e_cone_range() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, 40.5, 50.0),
            MeasurementPoint::new(90.0, 5.0, 8400.0, 39.5, 50.0),
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        let (min, max) = data.e_cone_range();
        assert_eq!(min, 0.0);
        assert_eq!(max, 10.0);
    }

    #[test]
    fn test_frequency_distribution() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, 40.5, 50.0),
            MeasurementPoint::new(90.0, 5.0, 2200.0, 39.5, 50.0),
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        let dist = data.frequency_distribution();
        assert_eq!(dist.get("8400.0"), Some(&2));
        assert_eq!(dist.get("2200.0"), Some(&1));
    }

    #[test]
    fn test_outlier_detection() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.0, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, 41.2, 50.0),
            MeasurementPoint::new(90.0, 5.0, 8400.0, 41.1, 50.0),
            MeasurementPoint::new(135.0, 15.0, 8400.0, 41.3, 50.0),
            MeasurementPoint::new(180.0, 20.0, 8400.0, 60.0, 50.0), // Outlier
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        let outliers = data.detect_outliers(3.5);
        assert_eq!(outliers.len(), 1);
        assert_eq!(outliers[0], 4);
    }

    #[test]
    fn test_quality_report_generation() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 1.0, 8400.0, 41.3, 50.0),
            MeasurementPoint::new(90.0, 5.0, 8400.0, 40.5, 50.0),
            MeasurementPoint::new(135.0, 10.0, 2200.0, 39.5, 50.0),
        ];
        let data = MeasurementData::new(points, "test.csv".to_string());
        let report = data.quality_report(2.0);

        assert_eq!(report.total_points, 4);
        assert_eq!(report.unique_frequencies, 2);
        assert_eq!(report.main_lobe_points, 3); // E-cone < 6 degrees
        assert_eq!(report.sidelobe_points, 1);
    }

    #[test]
    fn test_csv_parsing_valid() {
        let csv_content = r#"e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,8400.0,41.5,50.0
45.0,10.0,8400.0,40.5,50.0
90.0,15.0,2200.0,39.5,50.0"#;

        let result = parse_csv_content(csv_content, "test.csv");
        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data.len(), 3);
    }

    #[test]
    fn test_csv_parsing_with_errors() {
        let csv_content = r#"e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,8400.0,41.5,50.0
invalid,10.0,8400.0,40.5,50.0
90.0,15.0,2200.0,39.5,50.0"#;

        let result = parse_csv_content(csv_content, "test.csv");
        assert!(result.is_ok()); // Should succeed with valid points
        let data = result.unwrap();
        assert_eq!(data.len(), 2); // Only 2 valid points
    }

    #[test]
    fn test_csv_parsing_empty() {
        let csv_content = r#"e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k"#;

        let result = parse_csv_content(csv_content, "test.csv");
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_parsing_all_invalid() {
        let csv_content = r#"e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
500.0,0.0,8400.0,41.5,50.0
0.0,200.0,8400.0,40.5,50.0"#;

        let result = parse_csv_content(csv_content, "test.csv");
        assert!(result.is_err());
    }

    // ========================================================================
    // D11 — sidelobe measurements must survive parsing
    // ========================================================================

    /// A realistic pattern: 41.5 dB/K at boresight falling well past −20 dB/K off-axis.
    /// Every one of these rows is legitimate data; before D11 the 8 below −20 were
    /// rejected as if malformed.
    const REALISTIC_PATTERN_CSV: &str =
        "e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,8400.0,41.5,50.0
0.0,1.0,8400.0,38.2,50.0
0.0,2.0,8400.0,25.7,50.0
0.0,4.0,8400.0,2.4,50.0
0.0,6.0,8400.0,-14.8,50.0
0.0,8.0,8400.0,-21.3,50.0
0.0,12.0,8400.0,-33.9,50.0
0.0,20.0,8400.0,-41.2,50.0
0.0,30.0,8400.0,-48.6,50.0
0.0,45.0,8400.0,-55.0,50.0
0.0,60.0,8400.0,-61.7,50.0
0.0,90.0,8400.0,-70.4,50.0";

    #[test]
    fn realistic_off_axis_measurements_are_not_dropped() {
        let data = parse_csv_content(REALISTIC_PATTERN_CSV, "pattern.csv")
            .expect("a realistic antenna pattern must parse");

        assert_eq!(
            data.len(),
            12,
            "every row of a realistic pattern must survive parsing"
        );
        let min = data
            .points
            .iter()
            .map(|p| p.g_over_t_db)
            .fold(f64::INFINITY, f64::min);
        assert!(
            min < -40.0,
            "deep sidelobe rows were dropped: minimum retained G/T is {min} dB/K"
        );
    }

    #[test]
    fn deep_sidelobe_points_are_individually_valid() {
        // −70 dB/K at 90° off-boresight is an ordinary far-sidelobe measurement.
        let point = MeasurementPoint::new(0.0, 90.0, 8400.0, -70.4, 50.0);
        assert!(point.validate().is_ok());
        assert!(point.has_atypical_g_over_t());
    }

    #[test]
    fn non_finite_values_are_rejected() {
        for point in [
            MeasurementPoint::new(f64::NAN, 10.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, f64::NAN, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, f64::NAN, 41.5, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, f64::NAN, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, 41.5, f64::NAN),
            MeasurementPoint::new(45.0, 10.0, 8400.0, f64::INFINITY, 50.0),
            MeasurementPoint::new(45.0, 10.0, 8400.0, f64::NEG_INFINITY, 50.0),
        ] {
            assert!(
                point.validate().is_err(),
                "non-finite measurement accepted: {point:?}"
            );
        }
    }

    #[test]
    fn quality_report_warns_about_atypical_g_over_t_without_dropping_it() {
        let data = parse_csv_content(REALISTIC_PATTERN_CSV, "pattern.csv").unwrap();
        let report = data.quality_report(2.0);

        assert_eq!(report.total_points, 12, "no point was discarded");
        assert_eq!(
            report.atypical_g_over_t_count, 7,
            "the 7 rows below -20 dB/K should be flagged, not removed"
        );
        assert!((report.g_over_t_range.0 - (-70.4)).abs() < 1e-9);
        assert!((report.g_over_t_range.1 - 41.5).abs() < 1e-9);

        let text = report.format();
        assert!(text.contains("G/T outside the boresight-typical range"));
        assert!(text.contains("G/T Range: -70.40 - 41.50 dB/K"));
    }

    #[test]
    fn quality_report_stays_quiet_when_all_points_are_typical() {
        let points = vec![
            MeasurementPoint::new(0.0, 0.0, 8400.0, 41.5, 50.0),
            MeasurementPoint::new(45.0, 1.0, 8400.0, 40.1, 50.0),
        ];
        let report = MeasurementData::new(points, "t.csv".to_string()).quality_report(2.0);

        assert_eq!(report.atypical_g_over_t_count, 0);
        assert!(!report
            .format()
            .contains("outside the boresight-typical range"));
    }

    /// Captures `tracing` output so the drop report can be asserted on.
    #[derive(Clone, Default)]
    struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl CapturedLogs {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn parse_capturing_logs(content: &str) -> (Result<MeasurementData>, String) {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();
        let result =
            tracing::subscriber::with_default(subscriber, || parse_csv_content(content, "t.csv"));
        (result, logs.text())
    }

    /// Genuinely malformed rows are still dropped, and the drop is reported through
    /// `tracing` at WARN with an accurate count — not a silent `eprintln!`.
    #[test]
    fn malformed_rows_are_dropped_and_reported_through_tracing() {
        let csv = "e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,8400.0,41.5,50.0
not-a-number,1.0,8400.0,40.0,50.0
0.0,2.0,8400.0,-45.0,50.0
500.0,3.0,8400.0,39.0,50.0
0.0,4.0,8400.0,38.0,-5.0
0.0,5.0,8400.0,-55.0,50.0";

        let (result, logs) = parse_capturing_logs(csv);
        let data = result.expect("the valid rows should still parse");

        assert_eq!(data.len(), 3, "3 valid rows (two of them deep sidelobes)");
        assert!(
            logs.contains("WARN"),
            "drop must be reported at WARN: {logs}"
        );
        assert!(
            logs.contains("Dropped 3 of 6 rows"),
            "drop report must carry an accurate count: {logs}"
        );
    }

    #[test]
    fn a_clean_parse_reports_no_warning() {
        let (result, logs) = parse_capturing_logs(REALISTIC_PATTERN_CSV);
        assert_eq!(result.unwrap().len(), 12);
        assert!(
            logs.is_empty(),
            "a clean parse must not warn, but logged: {logs}"
        );
    }

    /// A large drop is summarized rather than dumped line by line.
    #[test]
    fn the_reported_failure_sample_is_bounded() {
        let mut csv =
            String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
        csv.push_str("0.0,0.0,8400.0,41.5,50.0\n");
        for i in 0..40 {
            csv.push_str(&format!("500.0,{}.0,8400.0,40.0,50.0\n", i));
        }

        let (result, logs) = parse_capturing_logs(&csv);
        assert_eq!(result.unwrap().len(), 1);
        assert!(logs.contains("Dropped 40 of 41 rows"));
        assert!(
            logs.contains("30 further parse failures not shown"),
            "the sample should be capped at {MAX_REPORTED_PARSE_ERRORS}: {logs}"
        );
    }

    #[test]
    fn test_sample_csv_creation() {
        let temp_dir = std::env::temp_dir();
        let csv_path = temp_dir.join("test_sample.csv");

        let result = create_sample_csv(&csv_path, 100);
        assert!(result.is_ok());

        // Parse it back
        let data = parse_measurements_sync(csv_path.to_str().unwrap());
        assert!(data.is_ok());
        let data = data.unwrap();
        assert_eq!(data.len(), 100);

        // Cleanup
        std::fs::remove_file(csv_path).ok();
    }
}
