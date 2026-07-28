//! The warning vocabulary (roadmap unit **C8 stage 3**).
//!
//! Every response-level warning the service emits is an [`ApiWarning`]: a
//! machine-readable [`WarningCode`] plus the human-readable sentence that used to
//! be the *entire* warning. Before stage 3 the wire type was `Vec<String>`, which
//! made three things true and all three were problems:
//!
//! 1. **Clients had to pattern-match prose.** The only way to branch on "was this
//!    extrapolated?" was a substring test, and the service did exactly that to
//!    itself — `service::heatmap` counted extrapolated points with
//!    `w.contains("extrapolat")`, which silently depended on the spelling of two
//!    unrelated messages in two other modules.
//! 2. **Nothing enumerated the set.** A new `warnings.push(format!(...))` anywhere
//!    in `model/` or `service/` added an undocumented class that no test and no
//!    reviewer could notice.
//! 3. **Rewording was a breaking change.** Any improvement to a message risked
//!    breaking a consumer, so messages were frozen by accident rather than by
//!    decision.
//!
//! [`WarningCode`] is a **closed enum**, deliberately. Adding a producer requires
//! adding a variant, which requires updating [`WarningCode::ALL`], `openapi.yaml`
//! and `docs/api-documentation.md` — the drift test
//! `tests/warning_code_vocabulary.rs` fails otherwise. That is the property that
//! keeps the vocabulary honest after C7 freezes the contract: the compiler, not a
//! convention, is what stops an undocumented code reaching a client.
//!
//! # Stability contract
//!
//! **`code` is the contract; `message` is not.** Codes are frozen from C7 onward
//! and safe to branch on. Messages may be reworded, retranslated, or given more
//! numeric detail in any release — they exist to be shown to a human, and several
//! interpolate query-specific values (thresholds, angles, percentages).
//!
//! # Deduplication
//!
//! `/heatmap` and `/h3-heatmap` evaluate many points and aggregate the union of
//! their warnings into one response-level array. Aggregation dedupes on the whole
//! `ApiWarning` (code **and** message), which is why the per-antenna honesty
//! warnings in `service::evaluator` are documented as "constant per (antenna,
//! frequency)" and must not interpolate the query angle: a message that varies
//! per grid point would produce one array entry per point.
//!
//! This constrains every producer reachable from an aggregating endpoint, not just
//! the honesty warnings. A message there may interpolate values that are constant
//! across the grid (an antenna's accuracy estimate, a feed offset in units of `f`)
//! but not values that vary with the query. `model::correction_interpolator` names
//! the out-of-range *dimensions* rather than the angles for exactly this reason.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Machine-readable classification of a response warning.
///
/// Serializes as the snake_case string shown on each variant. See the module
/// documentation for the stability contract (codes are frozen, messages are not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// `extrapolated` — the correction surface was evaluated outside the range of
    /// its knot vectors in at least one dimension, so the returned correction is an
    /// extrapolation rather than an interpolation.
    /// Producer: `model::correction_interpolator`.
    Extrapolated,

    /// `out_of_coverage` — the query falls outside the azimuth/elevation region a
    /// partially calibrated antenna was measured over; the physics model is being
    /// extrapolated into it. Producer: `service::evaluator`.
    OutOfCoverage,

    /// `correction_not_applied` — the antenna has a correction surface but it was
    /// not applied to this query (the query fell outside the recorded coverage).
    /// The returned gain is raw physics. Producer: `service::evaluator`.
    CorrectionNotApplied,

    /// `uncalibrated` — the antenna has no measurement-derived calibration and is
    /// modelled from design specifications; the message carries the accuracy
    /// estimates. Producer: `service::evaluator`.
    Uncalibrated,

    /// `partially_calibrated` — the antenna's calibration covers only part of its
    /// operating envelope; the message carries the accuracy estimate.
    /// Producer: `service::evaluator`.
    PartiallyCalibrated,

    /// `off_axis_unvalidated` — the query is more than 3 first-null angles off
    /// boresight on an antenna served with uncorrected physics, so the returned
    /// level is idealised physical optics plus the statistical sidelobe floor,
    /// not a calibrated-grade sidelobe prediction (roadmap units P8, P11).
    /// Producer: `service::evaluator`.
    OffAxisUnvalidated,

    /// `rear_hemisphere_invalid` — the query is more than 90° off boresight, where
    /// the aperture-integration model has no physical validity (roadmap unit
    /// P10-tail). Fires for calibrated antennas too: a forward-hemisphere
    /// correction surface says nothing about back lobes.
    /// Producer: `service::evaluator`.
    RearHemisphereInvalid,

    /// `non_convergence` — the aperture integration exhausted its iteration budget
    /// without meeting the convergence criterion, so gain accuracy may be degraded.
    /// Producer: `model::pattern` (and re-emitted from the `/h3-heatmap` gain cache).
    NonConvergence,

    /// `ray_trace_degraded` — the feed offset exceeds 0.5·f, so gain came from the
    /// acknowledged ray-tracing stub rather than a full ray trace (roadmap unit P3;
    /// real ray tracing is gated as feature F2). Producer: `model::pattern`, with a
    /// service-layer re-emission for `/h3-heatmap` cache hits.
    RayTraceDegraded,

    /// `severe_feed_offset` — edge-case analysis found the feed displaced more than
    /// 0.5·f from the focus. Distinct from [`WarningCode::RayTraceDegraded`], which
    /// reports what the model *did* about it. Producer: `model::edge_cases`.
    SevereFeedOffset,

    /// `feed_offset_spillover_unmodeled` — the feed offset is in the 0.3·f–0.5·f
    /// band, where the exact coma phase still applies but spillover efficiency is
    /// not modelled. Producer: `model::edge_cases`.
    FeedOffsetSpilloverUnmodeled,

    /// `spillover_significant` — estimated feed spillover exceeds 10% of radiated
    /// power, enough to reduce aperture efficiency materially.
    /// Producer: `model::edge_cases`.
    SpilloverSignificant,

    /// `points_extrapolated` — grid-level summary from `/heatmap`: how many of the
    /// evaluated points carried [`WarningCode::Extrapolated`],
    /// [`WarningCode::CorrectionNotApplied`], or [`WarningCode::OutOfCoverage`] —
    /// the three ways a returned value can be an extrapolation. The first two are
    /// exactly the per-point `metadata.extrapolated` flag that `/api/v1/gain`
    /// returns. Producer: `service::heatmap`.
    PointsExtrapolated,

    /// `point_computation_failed` — at least one grid point (`/heatmap`) or cell
    /// (`/h3-heatmap`) could not be evaluated. The point is counted in
    /// `metadata.failed_points` and carries the failure sentinel rather than a gain.
    /// One code for both endpoints: the cause is identical and only the word for a
    /// grid element differs.
    /// Producers: `service::heatmap`, `service::h3_link_budget`.
    PointComputationFailed,
}

impl WarningCode {
    /// Every code, in the order documented in `docs/api-documentation.md` and
    /// `openapi.yaml`.
    ///
    /// Used by the vocabulary drift test and available to consumers that need to
    /// enumerate the set. Mirrors [`crate::api::schemas::error_codes::ALL`].
    pub const ALL: &'static [WarningCode] = &[
        WarningCode::Extrapolated,
        WarningCode::OutOfCoverage,
        WarningCode::CorrectionNotApplied,
        WarningCode::Uncalibrated,
        WarningCode::PartiallyCalibrated,
        WarningCode::OffAxisUnvalidated,
        WarningCode::RearHemisphereInvalid,
        WarningCode::NonConvergence,
        WarningCode::RayTraceDegraded,
        WarningCode::SevereFeedOffset,
        WarningCode::FeedOffsetSpilloverUnmodeled,
        WarningCode::SpilloverSignificant,
        WarningCode::PointsExtrapolated,
        WarningCode::PointComputationFailed,
    ];

    /// The wire representation, identical to what `serde` emits.
    ///
    /// Kept as a hand-written match rather than derived from the serde attribute so
    /// that adding a variant fails to compile here too, not just in [`Self::ALL`].
    pub fn as_str(self) -> &'static str {
        match self {
            WarningCode::Extrapolated => "extrapolated",
            WarningCode::OutOfCoverage => "out_of_coverage",
            WarningCode::CorrectionNotApplied => "correction_not_applied",
            WarningCode::Uncalibrated => "uncalibrated",
            WarningCode::PartiallyCalibrated => "partially_calibrated",
            WarningCode::OffAxisUnvalidated => "off_axis_unvalidated",
            WarningCode::RearHemisphereInvalid => "rear_hemisphere_invalid",
            WarningCode::NonConvergence => "non_convergence",
            WarningCode::RayTraceDegraded => "ray_trace_degraded",
            WarningCode::SevereFeedOffset => "severe_feed_offset",
            WarningCode::FeedOffsetSpilloverUnmodeled => "feed_offset_spillover_unmodeled",
            WarningCode::SpilloverSignificant => "spillover_significant",
            WarningCode::PointsExtrapolated => "points_extrapolated",
            WarningCode::PointComputationFailed => "point_computation_failed",
        }
    }

    /// Build an [`ApiWarning`] carrying this code and `message`.
    pub fn with(self, message: impl Into<String>) -> ApiWarning {
        ApiWarning {
            code: self,
            message: message.into(),
        }
    }
}

impl fmt::Display for WarningCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single response warning: a stable [`WarningCode`] plus a human-readable
/// message.
///
/// `Ord` is derived (code first, then message) so aggregating endpoints can sort
/// their deduplicated set into a stable order without a bespoke comparator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiWarning {
    /// Machine-readable classification. Stable; branch on this.
    pub code: WarningCode,

    /// Human-readable explanation. **Not** part of the stability contract — show
    /// it, do not parse it.
    pub message: String,
}

impl fmt::Display for ApiWarning {
    /// Renders as `[code] message`.
    ///
    /// For log lines and test failure output only — the wire format is the JSON
    /// object, never this string. The code is written first so that `grep`ping logs
    /// by code works without the message getting in the way.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl ApiWarning {
    /// Build a warning from a code and message.
    pub fn new(code: WarningCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// True when this warning carries `code`.
    ///
    /// The replacement for the substring tests that stage 3 removed.
    pub fn is(&self, code: WarningCode) -> bool {
        self.code == code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_serde_for_every_code() {
        for &code in WarningCode::ALL {
            let json = serde_json::to_string(&code).expect("code serializes");
            assert_eq!(
                json,
                format!("\"{}\"", code.as_str()),
                "as_str() and the serde representation disagree for {code:?}"
            );
        }
    }

    #[test]
    fn all_is_complete_and_free_of_duplicates() {
        // `ALL` is hand-maintained; a forgotten entry would silently shrink the
        // documented vocabulary. Round-tripping every entry through serde and
        // counting distinct strings is the cheapest available check.
        let mut seen = std::collections::HashSet::new();
        for &code in WarningCode::ALL {
            assert!(
                seen.insert(code.as_str()),
                "duplicate entry in WarningCode::ALL: {code:?}"
            );
        }
        assert_eq!(seen.len(), WarningCode::ALL.len());
    }

    #[test]
    fn round_trips_through_json() {
        let warning = WarningCode::OffAxisUnvalidated.with("beyond the validated region");
        let json = serde_json::to_string(&warning).expect("serializes");
        assert_eq!(
            json,
            r#"{"code":"off_axis_unvalidated","message":"beyond the validated region"}"#
        );

        let parsed: ApiWarning = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, warning);
        assert!(parsed.is(WarningCode::OffAxisUnvalidated));
    }

    #[test]
    fn an_unknown_code_is_rejected_rather_than_accepted_as_text() {
        // The closed enum is what makes the vocabulary a contract: a body carrying
        // a code outside the set must fail to deserialize, not round-trip.
        let err = serde_json::from_str::<ApiWarning>(r#"{"code":"invented","message":"x"}"#);
        assert!(err.is_err(), "an unknown warning code must not deserialize");
    }
}
