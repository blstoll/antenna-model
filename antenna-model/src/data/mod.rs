//! Service-side calibration data: the repository that loads the antennas named in
//! `antennas.yaml`. Artifact types live in `antenna_core::types`, the loader in
//! `antenna_core::data::loader`.

pub mod repository;

pub use repository::CalibrationRepository;
