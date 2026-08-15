//! Data management module for antenna calibration.
//!
//! The artifact types and ANTC loader live in `antenna-core` (roadmap D4) and
//! are re-exported here so existing `antenna_model::data::…` paths keep
//! resolving. The service-side repository stays local — it depends on the
//! service configuration system.

pub mod repository;

// Glob re-export, not a curated list (roadmap D27 finding 7). This subsumes both the
// `{loader, types}` module re-export and the nine-name convenience list that used to sit
// here — a hand-copy of core's own list, which meant adding a tenth name in core silently
// failed to reach `antenna_model::data::`.
//
// The tradeoff is deliberate: a glob makes this facade's public surface track
// `antenna_core::data`'s automatically. That is exactly what a compatibility facade wants
// and exactly what you would NOT want of a hand-curated public API — this module is the
// former (it exists so pre-D4 `antenna_model::data::…` paths keep resolving), so core is
// the single place where the decision to export a name gets made.
pub use antenna_core::data::*;

// Re-export repository for easy access
pub use repository::CalibrationRepository;
