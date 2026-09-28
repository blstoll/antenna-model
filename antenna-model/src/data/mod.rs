//! Service-side calibration data: the repository that loads the antennas named in
//! `antennas.yaml`. Artifact types live in `antenna_core::types`, reading and writing
//! them in `antenna_core::artifact`.

pub mod repository;

pub use repository::CalibrationRepository;
