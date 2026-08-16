//! Optional JSON sidecar exporters (`--metadata` / `--report`)
//!
//! Full calibration mode writes one service-loadable binary artifact (an
//! `AntennaCalibration` built by [`crate::artifact_export`], framed by
//! `crate::main::write_antc_artifact`). This module holds the *optional*
//! human-readable JSON files written alongside it: provenance
//! ([`ArtifactMetadata`]) and the validation report.
//!
//! # The artifact and this sidecar do not have the same numeric domain (roadmap D25)
//!
//! The `.bin` artifact is postcard-encoded, and postcard carries `f64::INFINITY` and
//! `f64::NAN` exactly. **JSON has no non-finite numbers at all**, and `serde_json`
//! silently writes one as `null` — which a plain `f64` field then refuses to read back,
//! so the sidecar this module writes would not parse.
//!
//! That is not hypothetical: `AngularResolution::clock_lobe_period_deg` is
//! `f64::INFINITY` for the `sin θ → 0` case, and it is *meaningful* there — it says there
//! is no clock structure to resolve, which is the **best** case, deliberately chosen over
//! `0.0` so a degenerate axis cannot read as infinitely well resolved (roadmap D26).
//! Encoding it as `null` would erase exactly the distinction D26 established.
//!
//! So the non-finite values are **named**, not dropped: [`json_f64`] writes a finite value
//! as a JSON number and a non-finite one as the string `"Infinity"` / `"-Infinity"` /
//! `"NaN"`, and reads both forms back. The artifact keeps its own encoding untouched —
//! `AngularResolution` gains no serde attribute, which would corrupt the positional
//! postcard format (see the note atop `antenna_core::data::types`).
//!
//! `ValidationReport` solved the same problem the other way and correctly so: its
//! cross-validation aggregates are `Option<f64>` because there the meaning *is* absence —
//! no fold scored, so there is no mean (roadmap D22). Absence is `null`; a defined but
//! non-finite value is a name. Do not collapse the two.
//!
//! History: this file was `serializer.rs` and additionally defined a
//! `CalibrationArtifact` type with a `save_artifact`/`load_artifact` binary
//! path — an ANTC-framed bincode blob of a *3D* correction surface that the
//! service could never load. The binary path was removed on the
//! bincode → postcard migration (2026-07-18); the `CalibrationArtifact`
//! wrapper that outlived it was removed by roadmap unit D1 (2026-07-29), since
//! neither exporter ever read its `antenna_config` or `correction_surface`
//! fields. Nothing here writes a binary artifact.

use crate::validator::ValidationReport;
use antenna_core::data::types::AngularResolution;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum SerializationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid artifact: {reason}")]
    InvalidArtifact { reason: String },
}

pub type Result<T> = std::result::Result<T, SerializationError>;

// ============================================================================
// Data Structures
// ============================================================================

/// Provenance for a calibration run, written by `--metadata`.
///
/// This is inspection output only — it is not part of the binary artifact the
/// service loads (that carries its own `CalibrationMetadata`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    /// Creation timestamp (ISO 8601 format)
    pub created_at: String,

    /// Measurement data source (file path or S3 URL)
    pub measurement_source: String,

    /// Was parameter tuning performed?
    pub parameters_tuned: bool,

    /// Number of measurement points used for calibration
    pub num_measurement_points: usize,

    /// Calibration tool version
    pub tool_version: String,

    /// Additional notes
    pub notes: Option<String>,

    /// Frequency range covered (MHz)
    pub frequency_range: (f64, f64),

    /// Angular range covered (E-cone degrees)
    pub angular_range: (f64, f64),

    /// What the fitted correction surface's knots can resolve against this antenna's own
    /// `λ/D` lobe period (roadmap D21). `None` for a mode that fits no angular surface.
    ///
    /// The same value the artifact carries in `CalibrationMetadata.angular_resolution`, put
    /// here so it is readable without decoding the `.bin`. It is a limitation the fit's own
    /// RMSE — two fields up in this same file — structurally cannot express.
    ///
    /// Serialized through [`angular_resolution_json`] because `clock_lobe_period_deg` can
    /// legitimately be `f64::INFINITY` and JSON cannot express that as a number — see the
    /// module docs (roadmap D25).
    #[serde(default, with = "angular_resolution_json")]
    pub angular_resolution: Option<AngularResolution>,
}

/// Serializes an `f64` that may be non-finite, which JSON cannot represent as a number.
///
/// Finite values are ordinary JSON numbers. Non-finite values become the strings
/// `"Infinity"`, `"-Infinity"` and `"NaN"` — the spelling JavaScript's `Number()` and
/// Python's `float()` both accept, so a reader outside Rust can recover the value. Both
/// forms are accepted on the way back in, along with Rust's own `inf` / `-inf` spelling,
/// so a file hand-edited from `{:?}` output still parses.
///
/// Without this, `serde_json` writes `null` for a non-finite `f64` and then fails to read
/// it back into a plain `f64` field — the sidecar would be written and be unparseable
/// (roadmap D25).
mod json_f64 {
    use serde::de::{Error as DeError, Unexpected};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else if value.is_nan() {
            serializer.serialize_str("NaN")
        } else if *value > 0.0 {
            serializer.serialize_str("Infinity")
        } else {
            serializer.serialize_str("-Infinity")
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NumberOrName {
            Number(f64),
            Name(String),
        }

        match NumberOrName::deserialize(deserializer)? {
            NumberOrName::Number(v) => Ok(v),
            NumberOrName::Name(name) => match name.as_str() {
                "Infinity" | "inf" | "+Infinity" => Ok(f64::INFINITY),
                "-Infinity" | "-inf" => Ok(f64::NEG_INFINITY),
                "NaN" | "nan" => Ok(f64::NAN),
                other => Err(D::Error::invalid_value(
                    Unexpected::Str(other),
                    &"a JSON number, or \"Infinity\" / \"-Infinity\" / \"NaN\"",
                )),
            },
        }
    }
}

/// Applies [`json_f64`] to every field of an [`AngularResolution`] behind an `Option`.
///
/// A mirror struct rather than attributes on `AngularResolution` itself: that type is
/// postcard-encoded into the artifact, where serde attributes silently corrupt the
/// positional format. The mirror is private and structurally identical, so adding a field
/// to `AngularResolution` is a compile error here rather than a silently dropped field.
mod angular_resolution_json {
    use antenna_core::data::types::AngularResolution;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Mirror {
        #[serde(with = "super::json_f64")]
        cone_knot_spacing_deg: f64,
        #[serde(with = "super::json_f64")]
        cone_lobe_period_deg: f64,
        #[serde(with = "super::json_f64")]
        clock_knot_spacing_deg: f64,
        #[serde(with = "super::json_f64")]
        clock_lobe_period_deg: f64,
    }

    impl From<&AngularResolution> for Mirror {
        fn from(a: &AngularResolution) -> Self {
            let AngularResolution {
                cone_knot_spacing_deg,
                cone_lobe_period_deg,
                clock_knot_spacing_deg,
                clock_lobe_period_deg,
            } = *a;
            Self {
                cone_knot_spacing_deg,
                cone_lobe_period_deg,
                clock_knot_spacing_deg,
                clock_lobe_period_deg,
            }
        }
    }

    impl From<Mirror> for AngularResolution {
        fn from(m: Mirror) -> Self {
            Self {
                cone_knot_spacing_deg: m.cone_knot_spacing_deg,
                cone_lobe_period_deg: m.cone_lobe_period_deg,
                clock_knot_spacing_deg: m.clock_knot_spacing_deg,
                clock_lobe_period_deg: m.clock_lobe_period_deg,
            }
        }
    }

    pub fn serialize<S: Serializer>(
        value: &Option<AngularResolution>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_ref().map(Mirror::from).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<AngularResolution>, D::Error> {
        Ok(Option::<Mirror>::deserialize(deserializer)?.map(AngularResolution::from))
    }
}

// ============================================================================
// JSON Sidecar Exporters
// ============================================================================

/// Export calibration metadata to JSON for inspection (`--metadata`).
pub fn export_metadata_json<P: AsRef<Path>>(metadata: &ArtifactMetadata, path: P) -> Result<()> {
    write_json(metadata, path)
}

/// Export the validation report to JSON (`--report`).
pub fn export_validation_json<P: AsRef<Path>>(report: &ValidationReport, path: P) -> Result<()> {
    write_json(report, path)
}

fn write_json<T: Serialize, P: AsRef<Path>>(value: &T, path: P) -> Result<()> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| SerializationError::InvalidArtifact {
            reason: format!("JSON serialization failed: {}", e),
        })?;

    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata() -> ArtifactMetadata {
        ArtifactMetadata {
            created_at: "2026-07-29T00:00:00Z".to_string(),
            measurement_source: "test_measurements.csv".to_string(),
            parameters_tuned: true,
            num_measurement_points: 10,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            notes: Some("calibrated with class: TestClass".to_string()),
            frequency_range: (2000.0, 8000.0),
            angular_range: (0.0, 5.0),
            angular_resolution: None,
        }
    }

    fn test_validation_report() -> ValidationReport {
        ValidationReport {
            num_points: 10,
            model_only_rmse: 1.0,
            model_only_max_error: 2.0,
            model_only_r_squared: 0.8,
            corrected_rmse: 0.5,
            corrected_max_error: 1.0,
            corrected_r_squared: 0.95,
            rmse_improvement_percent: 50.0,
            max_error_improvement_percent: 50.0,
            main_lobe_num_points: 5,
            main_lobe_max_error: 0.8,
            main_lobe_rmse: 0.4,
            main_lobe_meets_target: true,
            first_sidelobe_num_points: 3,
            first_sidelobe_max_error: 0.9,
            first_sidelobe_rmse: 0.5,
            first_sidelobe_meets_target: true,
            outliers: vec![],
            num_outliers: 0,
            frequency_band_analysis: vec![],
            angular_region_analysis: vec![],
            cross_validation: None,
            meets_accuracy_requirements: true,
        }
    }

    #[test]
    fn test_export_metadata_json_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("metadata.json");

        export_metadata_json(&test_metadata(), &path).expect("export metadata");

        let text = std::fs::read_to_string(&path).expect("read metadata");
        let parsed: ArtifactMetadata = serde_json::from_str(&text).expect("parse metadata");
        assert_eq!(parsed.measurement_source, "test_measurements.csv");
        assert_eq!(parsed.num_measurement_points, 10);
        assert!(parsed.parameters_tuned);
        assert_eq!(parsed.frequency_range, (2000.0, 8000.0));
    }

    #[test]
    fn test_export_validation_json_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.json");

        export_validation_json(&test_validation_report(), &path).expect("export report");

        let text = std::fs::read_to_string(&path).expect("read report");
        let parsed: ValidationReport = serde_json::from_str(&text).expect("parse report");
        assert_eq!(parsed.num_points, 10);
        assert!((parsed.corrected_rmse - 0.5).abs() < 1e-12);
        assert!(parsed.meets_accuracy_requirements);
    }

    /// A report whose cross-validation scored **nothing** must still round-trip.
    ///
    /// The test above carries `cross_validation: None`, so it never exercised the aggregate
    /// statistics at all. Roadmap **D22** made a fold refit failure non-fatal, which created a
    /// reachable state where no fold scores and there is no mean to report. Representing that
    /// as `f64::NAN` would have written JSON `null` — `serde_json` cannot encode non-finite
    /// floats — and a plain `f64` field cannot read `null` back, so the report this very
    /// function writes would not have parsed. `Option<f64>` is why it does.
    #[test]
    fn a_report_with_no_scored_cross_validation_folds_round_trips() {
        use crate::validator::{CrossValidationResults, FoldFailure};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report_no_folds.json");

        let mut report = test_validation_report();
        report.cross_validation = Some(CrossValidationResults {
            num_folds: 2,
            fold_rmse_values: vec![],
            failed_folds: vec![
                FoldFailure {
                    fold: 1,
                    training_points: 5,
                    reason: "fold 1/2 could not refit".to_string(),
                },
                FoldFailure {
                    fold: 2,
                    training_points: 5,
                    reason: "fold 2/2 could not refit".to_string(),
                },
            ],
            mean_rmse: None,
            std_rmse: None,
            min_rmse: None,
            max_rmse: None,
        });

        export_validation_json(&report, &path).expect("export report");
        let text = std::fs::read_to_string(&path).expect("read report");
        let parsed: ValidationReport = serde_json::from_str(&text)
            .expect("a report with an unscored cross-validation must parse back");

        let cv = parsed.cross_validation.expect("cross-validation present");
        assert!(
            cv.mean_rmse.is_none(),
            "no fold scored, so there is no mean"
        );
        assert_eq!(cv.failed_folds.len(), 2);
        assert!(!cv.is_complete());
    }

    /// The `sin θ → 0` resolution, whose clock lobe period is a deliberate `INFINITY`.
    fn boresight_angular_resolution() -> AngularResolution {
        AngularResolution {
            cone_knot_spacing_deg: 2.0,
            cone_lobe_period_deg: 1.154_047_472_000_335_5,
            clock_knot_spacing_deg: 40.0,
            // The case D26 kept: no clock structure to resolve, the BEST case.
            clock_lobe_period_deg: f64::INFINITY,
        }
    }

    /// A metadata sidecar carrying a non-finite figure must round-trip (roadmap D25).
    ///
    /// The pre-existing round-trip test above passes only because its fixture sets
    /// `angular_resolution: None`, which is why this gap survived D21's review.
    #[test]
    fn a_metadata_sidecar_with_a_non_finite_figure_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("metadata_infinite.json");

        let mut metadata = test_metadata();
        metadata.angular_resolution = Some(boresight_angular_resolution());

        export_metadata_json(&metadata, &path).expect("export metadata");
        let text = std::fs::read_to_string(&path).expect("read metadata");

        // Named, not dropped: the value must be recoverable by a reader outside Rust.
        assert!(
            text.contains("\"Infinity\""),
            "a non-finite figure must be written as a name, not null; got:\n{text}"
        );
        assert!(
            !text.contains("null"),
            "no field of this fixture is absent, so nothing should serialize as null; got:\n{text}"
        );

        let parsed: ArtifactMetadata = serde_json::from_str(&text)
            .expect("a sidecar carrying a non-finite figure must parse back");
        let resolution = parsed
            .angular_resolution
            .expect("angular_resolution survives the round trip");

        assert_eq!(
            resolution.clock_lobe_period_deg,
            f64::INFINITY,
            "INFINITY means 'no clock structure to resolve' (D26) and must survive verbatim"
        );
        assert_eq!(resolution.clock_knot_spacing_deg, 40.0);
        assert_eq!(resolution.cone_knot_spacing_deg, 2.0);
        assert!((resolution.cone_lobe_period_deg - 1.154_047_472_000_335_5).abs() < 1e-15);
    }

    /// Negative control for the test above (roadmap P13: a guard nothing has falsified is
    /// not known to have power).
    ///
    /// Serializing the **unwrapped** `AngularResolution` — i.e. what the sidecar did before
    /// D25 — must still exhibit the defect: `serde_json` writes the infinity as `null`, and
    /// reading it back into the plain-`f64` struct fails. If this ever starts passing,
    /// `serde_json` has changed and the helper's justification needs re-reading.
    #[test]
    fn serde_json_still_cannot_encode_a_non_finite_f64_directly() {
        let raw = serde_json::to_string(&boresight_angular_resolution())
            .expect("serializing itself succeeds — that is the trap");

        assert!(
            raw.contains("null"),
            "expected serde_json to write the infinity as null; got: {raw}"
        );

        let err = serde_json::from_str::<AngularResolution>(&raw)
            .expect_err("a null must not read back into a plain f64 field");
        assert!(
            err.to_string().contains("invalid type: null"),
            "expected the documented failure, got: {err}"
        );
    }

    /// Both spellings a hand-edited or non-Rust-written file might carry.
    #[test]
    fn the_non_finite_names_are_accepted_on_the_way_in() {
        for (text, expected) in [
            ("\"Infinity\"", f64::INFINITY),
            ("\"inf\"", f64::INFINITY),
            ("\"-Infinity\"", f64::NEG_INFINITY),
            ("\"-inf\"", f64::NEG_INFINITY),
            ("1.5", 1.5),
        ] {
            let json = format!(
                r#"{{"cone_knot_spacing_deg":1.0,"cone_lobe_period_deg":1.0,
                     "clock_knot_spacing_deg":1.0,"clock_lobe_period_deg":{text}}}"#
            );
            let wrapper = format!(
                r#"{{"created_at":"x","measurement_source":"x","parameters_tuned":false,
                     "num_measurement_points":0,"tool_version":"x","notes":null,
                     "frequency_range":[1.0,2.0],"angular_range":[0.0,1.0],
                     "angular_resolution":{json}}}"#
            );
            let parsed: ArtifactMetadata =
                serde_json::from_str(&wrapper).unwrap_or_else(|e| panic!("{text} must parse: {e}"));
            let got = parsed
                .angular_resolution
                .expect("present")
                .clock_lobe_period_deg;
            assert_eq!(got, expected, "{text} should read back as {expected}");
        }

        // A name that is not a number must still be refused, rather than silently
        // becoming NaN — the whole point is that the value is stated, not guessed.
        let bad = r#"{"created_at":"x","measurement_source":"x","parameters_tuned":false,
                      "num_measurement_points":0,"tool_version":"x","notes":null,
                      "frequency_range":[1.0,2.0],"angular_range":[0.0,1.0],
                      "angular_resolution":{"cone_knot_spacing_deg":1.0,
                        "cone_lobe_period_deg":1.0,"clock_knot_spacing_deg":1.0,
                        "clock_lobe_period_deg":"banana"}}"#;
        assert!(
            serde_json::from_str::<ArtifactMetadata>(bad).is_err(),
            "an unrecognised name must be an error, not a silent NaN"
        );
    }
}
