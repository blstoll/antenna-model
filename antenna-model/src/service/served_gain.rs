//! Prepared served directional gain (issue #61, parent #59).
//!
//! One place that knows how to turn *one antenna-frame direction* into the gain this
//! service serves. Before this module the `/gain` evaluator and the `/h3-heatmap` cell
//! evaluator each rebuilt that law from parts — calibration-artifact projection, physical
//! feed positioning, beam squint, integration policy, correction gating, warning assembly —
//! and the same correctness fixes had to be made twice (roadmap D23, P11, C10 each landed
//! in two places).
//!
//! # The seam
//!
//! Preparation happens **once** per antenna/feed/request-geometry; evaluation happens once
//! per direction. Everything that does not depend on the observation direction is hoisted
//! into [`PreparedServedGain`]:
//!
//! ```text
//! prepare(calibration, feed steering, frequencies, budget)
//!   ├─ project artifact → reflector / feed / mesh model values (mm → m)
//!   ├─ combine request feed steering with the design feed offset
//!   ├─ adaptive integration policy + P11 uncorrected-physics gates + per-integration budget
//!   └─ operating vs pointing frequency
//!
//! evaluate_direct / evaluate_cached(pre-squint direction, reference policy)
//!   ├─ 1. beam squint            ← BEFORE the cache key, physics, coverage, and correction
//!   ├─ 2. physical optics        (aperture integration, or the cached result of one)
//!   ├─ 3. correction surface     ← ONLY after physics
//!   ├─ 4. correction disposition ← sole authority for applied / extrapolated
//!   ├─ 5. warnings               (fixed order, see `assemble`)
//!   └─ 6. ideal reference gain   ← spillover matched to what the ACTUAL evaluation did
//! ```
//!
//! The two evaluation operations differ in step 2 alone — everything from step 3 on is
//! one shared implementation — so direct, cold-cache and hot-cache evaluation of the same
//! direction return the same [`ServedGain`] (issue #62). The cache holds physics only:
//! never a corrected gain, a correction disposition, a coverage outcome, or a warning
//! collection.
//!
//! # What callers cannot do
//!
//! There is no public way to obtain physics-only gain, and no public correction
//! operation. [`ServedGain::gain_db`] is always final served gain, and
//! [`CorrectionDisposition`] is the only source of "was correction applied" and "is this
//! extrapolated" — callers cannot recreate an inconsistent boolean formula for either.
//!
//! That holds for code that goes *through* this module, which today is `/gain` (and, by
//! delegation, batch and rectangular heatmap). `/h3-heatmap` still open-codes its own
//! correction sequencing around the physics cache and only borrows this module's coverage
//! and warning helpers; issue #63 moves it onto [`PreparedServedGain::evaluate_cached`]
//! and deletes that copy.
//!
//! # Ownership boundary
//!
//! Repository access, API DTO construction, response timing, and HTTP error mapping stay
//! **outside**: the endpoint adapts its request into a [`PreSquintDirection`] plus a
//! [`FeedSteering`], and converts [`ServedGain`] into whatever its response type is.
//!
//! [`PreparedServedGain`] is immutable and `Sync` (pinned by a compile-time assertion in
//! this module's tests) so one instance can be shared across parallel grid workers.

use std::time::Duration;

use antenna_core::data::types::{AntennaCalibration, CalibrationCoverage, CalibrationStatus};
use antenna_core::error::{AntennaModelError, Result};
use antenna_core::model::{
    analyze_edge_cases, compute_gain_db, evaluate_correction, squint_corrected_direction,
    AntennaConfiguration, FeedParameters as ModelFeedParams, FeedPosition, IntegrationParams,
    MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
};
use antenna_core::warnings::{ApiWarning, WarningCode};

use crate::service::cache::{CachedGain, GainCache, GainCacheKey};

/// A direction in the antenna frame, **before** beam-squint correction.
///
/// The two angles are deliberately not called "azimuth" and "elevation": one of them does
/// not mean what those names suggest, and conflating them has cost this codebase real
/// bugs (see `docs/domain-contract.md`).
///
/// * **E-clock** — the azimuthal *clock* angle about the reflector boresight axis. This is
///   the API field `emitter_azimuth_deg`.
/// * **E-cone** — the *polar* angle measured **from the reflector boresight**: 0° is on
///   boresight, 90° is broadside to the dish, and values above 90° are in the rear
///   hemisphere. This is the API field `emitter_elevation_deg`, which is **not** elevation
///   above the horizon.
///
/// Callers hand the module pre-squint directions only; the module applies squint itself,
/// because squint has to precede physics, coverage gating, and correction interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PreSquintDirection {
    /// Clock angle about the boresight axis, degrees.
    pub(crate) e_clock_deg: f64,
    /// Polar angle from reflector boresight, degrees (0° = boresight, >90° = rear).
    pub(crate) e_cone_deg: f64,
}

impl PreSquintDirection {
    pub(crate) fn new(e_clock_deg: f64, e_cone_deg: f64) -> Self {
        Self {
            e_clock_deg,
            e_cone_deg,
        }
    }
}

/// A direction after beam-squint correction, carrying the squint that produced it.
///
/// This is the direction everything downstream uses: the physics integration, the
/// calibration-coverage test, the correction-surface interpolation, and the angles
/// reported back to the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SquintCorrectedDirection {
    /// Clock angle about the boresight axis, degrees. See [`PreSquintDirection`].
    pub(crate) e_clock_deg: f64,
    /// Polar angle from reflector boresight, degrees. See [`PreSquintDirection`].
    pub(crate) e_cone_deg: f64,
    /// Magnitude of the applied squint, degrees; `0.0` when no correction applied.
    pub(crate) squint_magnitude_deg: f64,
}

/// Operating frequency versus pointing frequency.
///
/// They are distinct on purpose: the **operating** frequency is what the emitter actually
/// radiates at — it drives the physics integration, the correction-surface lookup, and
/// cache identity. The **pointing** frequency is what the feed was aimed for; the offset
/// between the two is the entire cause of beam squint. Defaulting pointing to operating
/// (a request that omits `pointing_frequency_mhz`) happens here, once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ServedFrequencies {
    operating_mhz: f64,
    pointing_mhz: f64,
}

impl ServedFrequencies {
    /// Build from a request's operating frequency and its optional pointing override.
    pub(crate) fn new(operating_mhz: f64, pointing_override_mhz: Option<f64>) -> Self {
        Self {
            operating_mhz,
            pointing_mhz: pointing_override_mhz.unwrap_or(operating_mhz),
        }
    }
}

/// The request-derived part of the physical feed position, in the antenna frame (metres).
///
/// This is the displacement `compute_feed_position_from_pointing` derives from the
/// request's `feed_pointing_location` — where the feed is *aimed*. Preparation adds the
/// artifact's design feed offset to it; the two are combined in exactly one place so
/// multi-feed and steered-feed behaviour cannot drift between endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FeedSteering {
    pub(crate) x_m: f64,
    pub(crate) y_m: f64,
    pub(crate) z_m: f64,
}

impl FeedSteering {
    pub(crate) fn new(x_m: f64, y_m: f64, z_m: f64) -> Self {
        Self { x_m, y_m, z_m }
    }
}

/// Physical feed displacement from the **focal point**, in the antenna frame (metres).
///
/// Not to be confused with the vertex-relative physical feed *position* used for physics
/// and for cache identity: the two differ by the focal length, which is precisely why both
/// exist. For an on-axis, unsteered feed this is `(0, 0, 0)`; for a steered feed `x`/`y`
/// are the lateral displacement and `z` the (small, second-order) defocus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PhysicalFeedOffset {
    pub(crate) x_m: f64,
    pub(crate) y_m: f64,
    pub(crate) z_m: f64,
}

/// Whether the caller wants the ideal-reference gain and the loss derived from it.
///
/// A named choice rather than a bare `bool` because the reference is *not* free: it costs
/// a second aperture integration, with its own time budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceGainRequest {
    /// Compute the ideal-reference gain and the loss relative to it.
    Include,
    /// Skip the reference integration; `reference_gain_db` and `loss_db` stay `None`.
    Omit,
}

/// What happened to the correction surface for one direction.
///
/// This type is the **sole authority** for both "was correction applied" and "is this
/// result extrapolated". Those two questions used to be answered by open-coded boolean
/// expressions (`surface.is_some() && !applied`, `extrapolated || out_of_coverage`) that
/// every caller was free to re-derive — and to re-derive differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CorrectionDisposition {
    /// This artifact carries no correction surface at all. Served gain is raw physics.
    Unavailable,
    /// The surface was applied. `extrapolated` is the B-spline's own knot-range verdict:
    /// the query was inside calibrated coverage but outside the fitted knot span.
    Applied { extrapolated: bool },
    /// A surface exists but the query is outside calibrated coverage, so it was not
    /// applied. Served gain is physics-model extrapolation.
    OutsideCoverage,
}

impl CorrectionDisposition {
    /// Whether the correction surface contributed to the served gain.
    pub(crate) fn applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    /// Whether the served value is an extrapolation — either the B-spline extrapolated
    /// inside coverage, or coverage was left entirely and physics extrapolated.
    ///
    /// An antenna with no correction surface is not "extrapolated": there is no fitted
    /// surface to leave.
    pub(crate) fn extrapolated(&self) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Applied { extrapolated } => *extrapolated,
            Self::OutsideCoverage => true,
        }
    }
}

/// The complete result of serving one direction.
///
/// Named fields, not a tuple: gain, angles, warnings, correction state and provenance are
/// all the same shape as each other and were previously threaded through multi-value
/// returns where a transposition would have compiled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ServedGain {
    /// Final served gain (dBi) — physics plus correction, when correction applied.
    pub(crate) gain_db: f64,
    /// The direction actually evaluated, after beam squint.
    pub(crate) direction: SquintCorrectedDirection,
    /// Authority for applied / extrapolated state.
    pub(crate) correction: CorrectionDisposition,
    /// Physical feed displacement from the focal point (metres).
    pub(crate) physical_feed_offset: PhysicalFeedOffset,
    /// Spillover loss actually folded into the physics term, if any. This is
    /// *provenance*, not policy: the model layer restricts spillover to the standard
    /// physical-optics mode, so a large feed offset can leave the P11 gate on and still
    /// apply none.
    pub(crate) spillover_loss_db: Option<f64>,
    /// Boresight gain of an ideal version of this antenna, when requested.
    pub(crate) reference_gain_db: Option<f64>,
    /// `reference_gain_db - gain_db`, when a reference was requested.
    pub(crate) loss_db: Option<f64>,
    /// Every warning this direction earned, in a fixed order.
    pub(crate) warnings: Vec<ApiWarning>,
}

impl ServedGain {
    /// Beam squint as it is reported to clients: `None` below the 0.001° reporting
    /// threshold, so an unsquinted request does not carry a field full of float noise.
    pub(crate) fn reported_beam_squint_deg(&self) -> Option<f64> {
        if self.direction.squint_magnitude_deg > 0.001 {
            Some(self.direction.squint_magnitude_deg)
        } else {
            None
        }
    }
}

/// Everything about serving a direction that does not depend on the direction.
///
/// Immutable by construction: there are no `&mut self` methods and no interior
/// mutability. In particular the ideal-reference gain is **not** memoised — a `OnceCell`
/// or `Mutex` here would buy one integration per grid at the price of making the shared
/// value's `Sync` story a runtime property instead of a structural one.
#[derive(Debug, Clone)]
pub(crate) struct PreparedServedGain {
    /// The projected artifact. Held whole because coverage, the correction surface, the
    /// calibration temperature, the status advisories and the antenna id are all read
    /// from it per direction.
    calibration: AntennaCalibration,
    /// Physical-optics model built from the artifact plus the request's feed geometry.
    antenna_config: AntennaConfiguration,
    /// Adaptive policy, P11 uncorrected-physics gates, and the per-integration budget.
    integration_params: IntegrationParams,
    frequencies: ServedFrequencies,
    /// Vertex-relative physical feed position — what physics and cache identity use.
    physical_feed_position: FeedPosition,
    /// Focal-point-relative physical feed offset — what the response reports.
    physical_feed_offset: PhysicalFeedOffset,
    focal_length_m: f64,
    diameter_m: f64,
}

impl PreparedServedGain {
    /// Project a calibration artifact and one request's feed geometry into a value that
    /// can serve any number of directions.
    ///
    /// `steering` is the request-derived feed displacement; the artifact's design feed
    /// offset is added to it here. `frequencies` carries the operating/pointing
    /// distinction. `time_budget` bounds **each** aperture integration this value
    /// performs — it is deliberately per-integration, not a deadline for the whole
    /// request or grid (roadmap S3).
    pub(crate) fn prepare(
        calibration: AntennaCalibration,
        steering: FeedSteering,
        frequencies: ServedFrequencies,
        time_budget: Duration,
    ) -> Result<Self> {
        let focal_length_m = calibration.physical_config.reflector.focal_length_m;
        let diameter_m = calibration.physical_config.reflector.diameter_m;

        let reflector = ModelReflector::builder()
            .diameter(diameter_m)
            .focal_length(focal_length_m)
            .surface_rms(calibration.physical_config.reflector.surface_rms_mm / 1000.0) // mm to m
            .build()
            .map_err(|e| AntennaModelError::Generic(format!("Failed to build reflector: {}", e)))?;

        // Combine steering-induced position with the design feed offset. The design
        // position is the physical offset of THIS feed from the optical axis (multi-feed
        // antennas put their feeds at different physical locations).
        let design_pos = &calibration.physical_config.feed.position;
        let feed_x = steering.x_m + design_pos.0;
        let feed_y = steering.y_m + design_pos.1;
        let feed_z = steering.z_m + design_pos.2;
        let physical_feed_position = FeedPosition::new(feed_x, feed_y, feed_z);

        // Vertex-relative position minus the focal length is the displacement from the
        // focal point — the quantity the response reports. Keeping both is the point.
        let physical_feed_offset = PhysicalFeedOffset {
            x_m: feed_x,
            y_m: feed_y,
            z_m: feed_z - focal_length_m,
        };

        let feed = ModelFeedParams::builder()
            .position(physical_feed_position)
            .q_factor(calibration.physical_config.feed.q_factor)
            .phase_center_offset(calibration.physical_config.feed.phase_center_offset_m)
            .axial_defocus(calibration.physical_config.feed.axial_defocus_m)
            // Roadmap D23: every field this builder can take must come from the artifact,
            // never from the builder's own default. Omitting this one substituted a
            // symmetric feed for an asymmetric one — worth up to 1.20 dB, and it also
            // silently moved the evaluation from the azimuthal-mode integrator branch to
            // the symmetric one.
            .asymmetry_factor(calibration.physical_config.feed.asymmetry_factor)
            .build()
            .map_err(|e| AntennaModelError::Generic(format!("Failed to build feed: {}", e)))?;

        let mut config_builder = AntennaConfiguration::builder()
            .id(&calibration.antenna_id)
            .name(&calibration.metadata.antenna_name)
            .reflector(reflector)
            .feed(feed);

        if let Some(ref mesh_data) = calibration.physical_config.mesh {
            let mesh = ModelMeshParams::builder()
                .spacing(mesh_data.mesh_spacing_mm / 1000.0) // mm to m
                .wire_diameter(mesh_data.wire_diameter_mm / 1000.0) // mm to m
                .build()
                .map_err(|e| AntennaModelError::Generic(format!("Failed to build mesh: {}", e)))?;
            config_builder = config_builder.mesh(mesh);
        }

        let antenna_config = config_builder.build().map_err(|e| {
            AntennaModelError::Generic(format!("Failed to build antenna configuration: {}", e))
        })?;

        // Canonical served-path integration params. Radial density is derived adaptively
        // from (D/λ, θ), so this satisfies the <100ms target near boresight while remaining
        // numerically correct off-axis (P10).
        //
        // The two uncorrected-physics gates (P11) come from one shared setter, which
        // `calibrate` calls with the same predicate for the artifact it writes — see
        // `IntegrationParams::with_uncorrected_physics_gates` and roadmap D17. Setting
        // either flag by hand here would reopen the calibrate/service split that unit
        // closed.
        //
        // What the gates mean:
        //   * spillover — a double-counting gate: physical spillover is folded in only when
        //     NO correction surface exists (the surface otherwise absorbs it empirically).
        //     Note the model layer further restricts spillover to StandardPhysicalOptics
        //     mode, so a large feed offset may leave the flag on yet apply no spillover.
        //     `reference_gain` below tracks the ACTUAL result's spillover state (not this
        //     flag) so base spillover cancels in loss_db without a one-sided bias.
        //   * sidelobe floor — F7 (redesign 2026-07-16): incoherent power sum forward,
        //     floor-only behind the dish (see model::pattern::compute_gain). Calibrated
        //     antennas keep it off for the same double-counting reason.
        //
        // Both are whole-antenna gates — never per query — so no discontinuity is
        // introduced between covered and out-of-coverage queries on a calibrated antenna.
        let mut integration_params = IntegrationParams::adaptive()
            .with_uncorrected_physics_gates(calibration.physics_is_uncorrected());
        // S3: bound each aperture integration to the configured wall-clock budget. Carried
        // in IntegrationParams so `integrate_aperture`'s signature stays stable; every
        // integration behind this value — off-axis, boresight anchor, and the ideal
        // reference — each gets a fresh deadline of this duration.
        integration_params.time_budget = Some(time_budget);

        Ok(Self {
            calibration,
            antenna_config,
            integration_params,
            frequencies,
            physical_feed_position,
            physical_feed_offset,
            focal_length_m,
            diameter_m,
        })
    }

    /// The artifact's calibration status, for endpoints that report it.
    ///
    /// Deliberately narrow: the endpoint needs this to build its status DTO, and DTO
    /// construction stays outside this module. It is *not* an escape hatch to the
    /// artifact — in particular `correction_applied` on that DTO must come from
    /// [`ServedGain::correction`], which is the authority.
    pub(crate) fn calibration_status(&self) -> Option<&CalibrationStatus> {
        self.calibration.calibration_status.as_ref()
    }

    /// Serve one direction by running the physics integration directly.
    ///
    /// This is the whole gain law for a single direction: squint, physics, correction,
    /// disposition, warnings, reference. There is no variant that returns the physics
    /// term alone.
    pub(crate) fn evaluate_direct(
        &self,
        direction: PreSquintDirection,
        reference: ReferenceGainRequest,
    ) -> Result<ServedGain> {
        // Squint FIRST: the corrected direction is what physics integrates, what the
        // coverage test asks about, and what the correction surface is interpolated at.
        let corrected = self.squint(direction);
        let physics = self.physics_direct(&corrected)?;
        self.assemble(corrected, physics, reference)
    }

    /// Serve one direction, taking the physics term from `cache` when it is already there
    /// and running the integration only on a miss (issue #62).
    ///
    /// **Observationally identical to [`evaluate_direct`](Self::evaluate_direct)**, and
    /// that identity is the point: direct, cold-cache and hot-cache evaluation of the same
    /// direction return the same [`ServedGain`], field for field and warning for warning.
    /// The two share [`assemble`](Self::assemble) verbatim — they differ only in how the
    /// [`PhysicsOutcome`] was obtained — so correction gating, disposition, the reference,
    /// and warning order cannot drift between them.
    ///
    /// What the cache holds is **physics only**. The correction surface is interpolated
    /// after every lookup, so a warm cache never serves a stale correction; coverage,
    /// calibration advisories, and the direction-derived warnings are likewise re-evaluated
    /// per call. The one thing that cannot be re-derived — what the integration itself
    /// learned — rides in the [`CachedGain`] payload: convergence and the spillover it
    /// actually applied.
    ///
    /// Cache identity is `(antenna_id, feed_id)` from the **artifact**, keyed on the
    /// squint-corrected direction, the operating frequency, and the vertex-relative
    /// physical feed position. The artifact's own composite identifier is used rather than
    /// a caller-supplied one because the repository stores each calibration under exactly
    /// that pair — taking it from the value being evaluated makes a namespace/artifact
    /// mismatch unrepresentable.
    ///
    /// That namespace assumes what the repository guarantees: one artifact per
    /// `(antenna_id, feed_id)` at a time, with [`GainCache::invalidate`] called when it is
    /// replaced. Two *different* artifacts for one identifier would share entries while
    /// disagreeing about the P11 gates — which change the physics but are not part of the
    /// key — so cached physics is only interchangeable for as long as that holds.
    // No production caller yet: `/h3-heatmap` migrates onto this in issue #63, which
    // removes this allow. Until then only this module's tests exercise it.
    #[allow(dead_code)]
    pub(crate) fn evaluate_cached(
        &self,
        direction: PreSquintDirection,
        reference: ReferenceGainRequest,
        cache: &GainCache,
    ) -> Result<ServedGain> {
        // Squint FIRST — before the cache key, for the same reason it precedes physics:
        // the key must name the direction that was actually evaluated. Keying on the
        // requested direction would serve one angle's gain for another.
        let corrected = self.squint(direction);
        let physics = self.physics_cached(&corrected, cache)?;
        self.assemble(corrected, physics, reference)
    }

    /// Apply beam squint. Depends on the actual feed displacement, which is why it can
    /// only happen after preparation has positioned the feed.
    fn squint(&self, direction: PreSquintDirection) -> SquintCorrectedDirection {
        let (e_clock_deg, e_cone_deg, squint_magnitude_deg) = squint_corrected_direction(
            direction.e_clock_deg,
            direction.e_cone_deg,
            self.frequencies.operating_mhz,
            self.frequencies.pointing_mhz,
            self.physical_feed_position.x,
            self.physical_feed_position.y,
            self.focal_length_m,
        );
        tracing::debug!(
            corrected_az = %e_clock_deg,
            corrected_el = %e_cone_deg,
            "Emitter direction after beam squint correction"
        );
        SquintCorrectedDirection {
            e_clock_deg,
            e_cone_deg,
            squint_magnitude_deg,
        }
    }

    /// Run the aperture integration for one squint-corrected direction.
    ///
    /// The physics model's polar angle `theta` is measured from boresight, which is
    /// exactly the E-cone convention, so the conversion is a plain degrees→radians.
    fn physics_direct(&self, corrected: &SquintCorrectedDirection) -> Result<PhysicsOutcome> {
        let theta_rad = corrected.e_cone_deg.to_radians();
        let phi_rad = corrected.e_clock_deg.to_radians();

        tracing::debug!(
            theta_rad = %theta_rad,
            phi_rad = %phi_rad,
            feed_x = %self.physical_feed_position.x,
            feed_y = %self.physical_feed_position.y,
            feed_z = %self.physical_feed_position.z,
            "Physics model inputs"
        );

        let result = compute_gain_db(
            theta_rad,
            phi_rad,
            &self.antenna_config,
            self.frequencies.operating_mhz * 1e6,
            &self.integration_params,
        )?; // ComputationError automatically converts via #[from]

        Ok(PhysicsOutcome {
            gain_db: result.gain,
            spillover_loss_db: result.spillover_loss_db,
            warnings: result.warnings,
        })
    }

    /// Obtain the physics term for one squint-corrected direction from the cache,
    /// integrating only on a miss.
    ///
    /// The miss path stores the integration's value and provenance and then **discards its
    /// warnings**, so that both the miss and every later hit build their warnings the same
    /// way — through [`reconstruct_physics_warnings`](Self::reconstruct_physics_warnings).
    /// Returning the model's own list on a miss would make cold-cache evaluation agree with
    /// direct evaluation for free while leaving hot-cache evaluation the only path anything
    /// tested; here a divergence in the reconstruction fails the direct-vs-cold comparison
    /// too, which is the comparison a real physics change moves.
    fn physics_cached(
        &self,
        corrected: &SquintCorrectedDirection,
        cache: &GainCache,
    ) -> Result<PhysicsOutcome> {
        let key = GainCacheKey::new(
            corrected.e_clock_deg,
            corrected.e_cone_deg,
            self.frequencies.operating_mhz,
            self.physical_feed_position.x,
            self.physical_feed_position.y,
            self.physical_feed_position.z,
        );

        let cached = cache.get_or_compute(
            &self.calibration.antenna_id,
            &self.calibration.feed_id,
            key,
            || {
                let physics = self.physics_direct(corrected)?;
                Ok(CachedGain::new(
                    physics.gain_db,
                    !physics
                        .warnings
                        .iter()
                        .any(|w| w.is(WarningCode::NonConvergence)),
                    physics.spillover_loss_db,
                ))
            },
        )?;

        Ok(PhysicsOutcome {
            gain_db: cached.value,
            spillover_loss_db: cached.spillover_loss_db,
            warnings: self.reconstruct_physics_warnings(corrected, cached.converged),
        })
    }

    /// Rebuild the warnings `compute_gain_db` would have emitted for this direction.
    ///
    /// Every warning the model pushes from inside the integration is a deterministic
    /// function of the antenna configuration, the direction, and the integration policy —
    /// all of which the prepared value still holds — with exactly one exception:
    /// convergence, which only the integration learns and which therefore rides in the
    /// cached payload.
    ///
    /// The reconstruction mirrors `model::pattern::compute_gain`'s own structure and order:
    ///
    /// 1. `analyze_edge_cases` — the severe / moderate feed-offset band and the
    ///    significant-spillover advisory. Called here rather than copied; it ignores
    ///    `(theta, phi)`, so it is the configuration's verdict.
    /// 2. The ray-tracing stub warning, iff the mode dispatch is actually reached and
    ///    selects it. Mode selection goes through [`ray_trace_stub_warning`], this module's
    ///    single mirror of it — expressing the same threshold a second time here is exactly
    ///    how the two would come to disagree. What is added on top is the *reachability*
    ///    question that helper cannot answer: the F7 floor-only rear path
    ///    (`apply_sidelobe_floor` on, past 90°) returns *before* the dispatch, so a severe
    ///    feed offset earns the severe-offset advisory there but **not** the degradation
    ///    warning — the model never ran the stub, and claiming otherwise would be a
    ///    fabricated diagnostic.
    /// 3. Non-convergence, from the cached flag.
    ///
    /// The floor-only gate is the one model-internal condition this module restates, and
    /// the direct-vs-cold-vs-hot equality tests are what hold it to the model's behaviour:
    /// a gate that drifts fails them at the first affected geometry.
    fn reconstruct_physics_warnings(
        &self,
        corrected: &SquintCorrectedDirection,
        converged: bool,
    ) -> Vec<ApiWarning> {
        let theta_rad = corrected.e_cone_deg.to_radians();
        let phi_rad = corrected.e_clock_deg.to_radians();

        let analysis = analyze_edge_cases(&self.antenna_config, theta_rad, phi_rad);
        let mut warnings = analysis.warnings;

        let floor_only_rear = self.integration_params.apply_sidelobe_floor
            && theta_rad.abs() > std::f64::consts::FRAC_PI_2;
        if !floor_only_rear {
            warnings.extend(ray_trace_stub_warning(&self.antenna_config));
        }

        if !converged {
            warnings.push(antenna_core::model::pattern::nonconvergence_warning());
        }

        warnings
    }

    /// The shared tail of every evaluation: correction, disposition, reference, warnings.
    ///
    /// Direct and cache-backed evaluation (issue #62) differ only in how [`PhysicsOutcome`]
    /// was obtained; from here on there is one implementation, so the two cannot drift.
    fn assemble(
        &self,
        corrected: SquintCorrectedDirection,
        physics: PhysicsOutcome,
        reference: ReferenceGainRequest,
    ) -> Result<ServedGain> {
        let mut warnings = physics.warnings;

        // Correction surface, gated on the FULL coverage question (direction and
        // frequency). Interpolated at the squint-corrected direction.
        let (correction_db, disposition) = match &self.calibration.correction_surface {
            None => (0.0, CorrectionDisposition::Unavailable),
            Some(surface) => {
                if is_in_coverage(
                    &self.calibration.calibration_coverage,
                    corrected.e_clock_deg,
                    corrected.e_cone_deg,
                    self.frequencies.operating_mhz,
                ) {
                    let result = evaluate_correction(
                        surface,
                        corrected.e_clock_deg,
                        corrected.e_cone_deg,
                        self.frequencies.operating_mhz,
                        self.calibration.validity_ranges.temperature_const,
                    )?;
                    warnings.extend(result.warnings);
                    (
                        result.correction_db,
                        CorrectionDisposition::Applied {
                            extrapolated: result.extrapolated,
                        },
                    )
                } else {
                    (0.0, CorrectionDisposition::OutsideCoverage)
                }
            }
        };

        let gain_db = physics.gain_db + correction_db;

        let (reference_gain_db, loss_db) = match reference {
            ReferenceGainRequest::Omit => (None, None),
            ReferenceGainRequest::Include => {
                let reference_gain_db = self.reference_gain(physics.spillover_loss_db.is_some())?;
                // Loss is reference minus actual gain (final gain, including the
                // correction surface when it was applied).
                (Some(reference_gain_db), Some(reference_gain_db - gain_db))
            }
        };

        // Fixed assembly order. `/gain` pins this order in its response, so it is part of
        // the served contract, not an implementation detail:
        //   physics/edge-case → correction interpolation → calibration status/coverage
        //   → off-axis validity → rear-hemisphere validity
        warnings.extend(generate_calibration_warnings(
            &self.calibration,
            corrected.e_clock_deg,
            corrected.e_cone_deg,
            disposition.applied(),
        ));

        // Off-axis honesty warning (P8): E-cone IS the off-boresight angle.
        warnings.extend(off_axis_unvalidated_warning(
            &self.calibration,
            corrected.e_cone_deg,
            self.frequencies.operating_mhz,
        ));

        // Rear-hemisphere hard-invalidity warning (P10-tail): fires for E-cone > 90° on ANY
        // antenna, calibrated or not — a forward-hemisphere correction surface says
        // nothing about back lobes.
        warnings.extend(rear_hemisphere_warning(
            &self.calibration,
            corrected.e_cone_deg,
            self.frequencies.operating_mhz,
        ));

        Ok(ServedGain {
            gain_db,
            direction: corrected,
            correction: disposition,
            physical_feed_offset: self.physical_feed_offset,
            spillover_loss_db: physics.spillover_loss_db,
            reference_gain_db,
            loss_db,
            warnings,
        })
    }

    /// Boresight gain of an IDEAL version of this antenna — feed at the focal point,
    /// perfect surface — evaluated through the SAME `compute_gain_db` pipeline as the
    /// actual gain. Because both numbers come from the identical aperture-directivity
    /// formula, `loss_db` has no built-in offset: it is purely the pointing/aberration
    /// loss (≈0 dB at boresight with a focused feed).
    ///
    /// This stays behind the prepared boundary because it depends on
    /// `actual_applied_spillover` — what the actual evaluation *did*, not what the
    /// artifact's calibration status or a stored computation mode would predict.
    fn reference_gain(&self, actual_applied_spillover: bool) -> Result<f64> {
        let ideal_reflector = ModelReflector::new(self.diameter_m, self.focal_length_m, 0.0)
            .map_err(|e| AntennaModelError::Generic(format!("ideal reflector: {e}")))?;
        let ideal_feed = ModelFeedParams::new(
            FeedPosition::at_focus(self.focal_length_m),
            self.calibration.physical_config.feed.q_factor,
            self.calibration.physical_config.feed.phase_center_offset_m,
            1.0,
        )
        .map_err(|e| AntennaModelError::Generic(format!("ideal feed: {e}")))?;
        let ideal_config = AntennaConfiguration::new(
            format!("{}_ideal", self.calibration.antenna_id),
            "ideal".into(),
            ideal_reflector,
            ideal_feed,
            self.antenna_config.mesh.clone(),
        )
        .map_err(|e| AntennaModelError::Generic(format!("ideal config: {e}")))?;

        // Match the reference's spillover to the ACTUAL path: if the actual evaluation was
        // in a mode where spillover was folded in (StandardPhysicalOptics), apply it to the
        // ideal reference too so the base spillover cancels in loss_db; if the actual did
        // NOT get spillover (large offset / non-standard mode, or calibrated), the
        // reference must not either, keeping loss_db free of a one-sided spillover bias.
        //
        // This is the ONE place that sets a P11-gated flag without going through
        // `with_uncorrected_physics_gates`, and that is deliberate — the setter's docs name
        // this exception. Routing this line through the setter would derive the flag from
        // the *predicate* rather than from what the actual evaluation applied,
        // reintroducing exactly the one-sided bias the paragraph above exists to prevent.
        let mut reference_params = self.integration_params.clone();
        reference_params.apply_spillover = actual_applied_spillover;
        // `apply_sidelobe_floor` is carried unchanged from the clone. For the ideal
        // REFERENCE it is inert under the F7 power sum for two independent reasons: the
        // ideal reflector has surface_rms = 0.0, so `sidelobe_floor_gain` is identically
        // zero (adding zero changes nothing — exactly, not approximately), and the
        // reference is evaluated at boresight (theta=0), which is forward-hemisphere.
        let reference = compute_gain_db(
            0.0,
            0.0,
            &ideal_config,
            self.frequencies.operating_mhz * 1e6,
            &reference_params,
        )?;
        Ok(reference.gain)
    }
}

/// The physics term for one direction, plus the provenance that cannot be re-derived
/// without redoing the integration.
///
/// Private on purpose: this is the "physics-only gain" that callers must not be able to
/// get hold of and correct themselves.
#[derive(Debug, Clone)]
struct PhysicsOutcome {
    gain_db: f64,
    spillover_loss_db: Option<f64>,
    warnings: Vec<ApiWarning>,
}

/// Check if the query is within the calibrated coverage region.
///
/// This is the served correction-application gate. It owns exactly one decision
/// the calibration artifact cannot make for itself — what an *absent* coverage
/// record means — and delegates the range test to
/// [`CalibrationCoverage::contains_direction_at_frequency`], which is the sole
/// authority for it (issue #60). The service previously carried its own copy of
/// that expression; the two agreed only by inspection, and the pole limitation
/// documented on the core predicate had to be fixed in two places.
///
/// When no coverage restriction is recorded (`None`) the correction surface is
/// treated as valid everywhere it has data — the query is considered in-coverage.
/// Actual correction application is still gated separately on
/// `correction_surface.is_some()`, so returning `true` here for `None` is safe.
///
/// This is the **full** coverage question: azimuth, E-cone, and frequency. The
/// partial-calibration advisory asks the narrower spatial-only question
/// ([`CalibrationCoverage::contains_direction`]) and the two must stay distinct —
/// a query on the measured grid at an uncalibrated frequency gets no correction
/// but is not outside the calibrated *region*.
///
/// **Visibility is interim.** This is `pub(crate)` only because `service::h3_link_budget`
/// still calls it directly while it open-codes its own correction sequencing around the
/// physics cache. Issue #63 moves that endpoint onto [`PreparedServedGain`], at which
/// point the module's own `assemble` is the only caller and this becomes private.
pub(crate) fn is_in_coverage(
    coverage: &Option<CalibrationCoverage>,
    azimuth_deg: f64,
    elevation_deg: f64,
    frequency_mhz: f64,
) -> bool {
    match coverage {
        Some(cov) => cov.contains_direction_at_frequency(azimuth_deg, elevation_deg, frequency_mhz),
        // No coverage restriction recorded (fully calibrated artifact):
        // the correction surface applies everywhere it has data.
        None => true,
    }
}

/// Generate warnings based on calibration status and query parameters.
///
/// Returns a vector of warning messages to be included in the response.
fn generate_calibration_warnings(
    calibration: &antenna_core::data::types::AntennaCalibration,
    azimuth_deg: f64,
    elevation_deg: f64,
    correction_applied: bool,
) -> Vec<ApiWarning> {
    let mut warnings = Vec::new();

    // Get calibration status (default to FullyCalibrated if not specified for backward compatibility)
    let status = calibration.calibration_status.as_ref();

    match status {
        Some(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db,
            loss_accuracy_estimate_db,
        }) => {
            warnings.push(WarningCode::Uncalibrated.with(format!(
                "Antenna '{}' is uncalibrated (using design specifications). \
                 Absolute gain accuracy: ±{:.1} dB, Loss accuracy: ±{:.1} dB",
                calibration.antenna_id, accuracy_estimate_db, loss_accuracy_estimate_db
            )));
        }
        Some(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db,
            coverage,
        }) => {
            warnings.push(WarningCode::PartiallyCalibrated.with(format!(
                "Antenna '{}' is partially calibrated. Accuracy estimate: ±{:.1} dB",
                calibration.antenna_id, accuracy_estimate_db
            )));

            // Whether the query left the measured *region*. Deliberately spatial
            // only: this advisory reports direction, not band, so an in-grid query
            // at an uncalibrated frequency gets `correction_not_applied` below
            // without also claiming to be outside the calibrated region.
            if !coverage.contains_direction(azimuth_deg, elevation_deg) {
                warnings.push(WarningCode::OutOfCoverage.with(
                    "Query is outside calibrated region - using physics model extrapolation",
                ));
            }
        }
        Some(CalibrationStatus::FullyCalibrated { .. }) | None => {
            // No calibration warnings for fully calibrated antennas
        }
    }

    // Warn if correction surface exists but wasn't applied
    if !correction_applied && calibration.correction_surface.is_some() {
        warnings.push(
            WarningCode::CorrectionNotApplied
                .with("Correction surface not applied (out of coverage)"),
        );
    }

    warnings
}

/// First-null angle coefficient for tapered circular-aperture illumination:
/// θ_null ≈ 1.6·λ/D radians (uniform illumination would be 1.22·λ/D; the
/// taper widens the main lobe). See docs/domain-contract.md, "Off-axis
/// pattern / sidelobe fidelity".
const FIRST_NULL_COEFFICIENT: f64 = 1.6;

/// The off-axis honesty warning fires beyond this many first-null angles off
/// boresight. Inside ~3 first nulls the main beam and first sidelobe are the
/// region the model is validated for (<1 dB).
///
/// Beyond it, the served value is now **numerically correct**: roadmap unit P10
/// (LANDED 2026-07-15) replaced the aliasing fixed-density quadrature with the
/// Hankel / azimuthal-mode integrator, which computes the physical-optics pattern
/// to convergence at all angles (no more 20–35 dB-too-high aliasing, no gain that
/// rises with angle). The remaining caveat is therefore **physical, not
/// numerical**: the served value is *idealised* physical optics — it omits
/// blockage, feed/strut scatter, and aperture-edge diffraction — so far-off-axis
/// sidelobe *levels* are optimistic and not calibrated-grade (the pattern shape is
/// validated; the absolute levels are not). Per the F7 redesign (landed
/// 2026-07-16) the served path on uncorrected-physics antennas combines this
/// idealised PO term with the F7 statistical sidelobe floor as an incoherent
/// power sum, so far off-axis the returned value tracks a best-estimate MEDIAN
/// wide-angle level rather than the raw PO number alone. The warning below
/// states this physical caveat.
const OFF_AXIS_FIRST_NULL_MULTIPLE: f64 = 3.0;

/// Off-axis honesty warning for uncorrected-physics antennas (roadmap units P8, P11).
///
/// Returns a warning when a query on an antenna whose served gain is RAW
/// (uncorrected) physics falls beyond the validated main-beam/near-in region
/// (3× the first-null angle ≈ 1.6·λ/D — a beamwidth-relative threshold, not a
/// fixed angle).
///
/// The gate is [`AntennaCalibration::physics_is_uncorrected`] — the SAME
/// predicate that gates the spillover fold-in (roadmap P11). Any antenna that
/// carries a correction surface stays silent (regardless of calibration
/// status): out-of-coverage queries there already receive the extrapolation
/// warning, so stacking a second warning was explicitly ruled out (P8 design
/// constraint 1), and that constraint is preserved exactly by keying on surface
/// presence. Conversely a `PartiallyCalibrated` antenna produced with NO
/// frequency correction (see `calibrate/boresight_calibration.rs`) has no
/// surface to extrapolate and DOES warn — closing the pre-P11 honesty gap where
/// such an antenna had its physics modified (spillover) yet served no off-axis
/// honesty warning.
///
/// The message is intentionally constant per (antenna, frequency) — it must
/// not embed the query angle, so that heatmap/H3 warning aggregation
/// deduplicates it to a single entry across grid points. Aggregation dedupes on
/// `(code, message)`, so this remains load-bearing after C8 stage 3 typed the
/// warning: a per-angle message would yield one array entry per grid point even
/// though every entry carried the same code.
///
/// Carries [`WarningCode::OffAxisUnvalidated`] (typed by C8 stage 3,
/// 2026-07-27).
pub(crate) fn off_axis_unvalidated_warning(
    calibration: &antenna_core::data::types::AntennaCalibration,
    off_boresight_deg: f64,
    frequency_mhz: f64,
) -> Option<ApiWarning> {
    if !calibration.physics_is_uncorrected() {
        return None;
    }

    let diameter_m = calibration.physical_config.reflector.diameter_m;
    if diameter_m <= 0.0 || frequency_mhz <= 0.0 {
        return None;
    }

    let wavelength_m = antenna_core::model::wavelength_from_frequency(frequency_mhz * 1e6);
    let threshold_deg = (OFF_AXIS_FIRST_NULL_MULTIPLE * FIRST_NULL_COEFFICIENT * wavelength_m
        / diameter_m)
        .to_degrees();

    if off_boresight_deg.abs() <= threshold_deg {
        return None;
    }

    Some(WarningCode::OffAxisUnvalidated.with(format!(
        "Antenna '{}' is uncalibrated and this query is more than {:.2}° off boresight \
         (3× the first-null angle ≈ 1.6·λ/D at {:.0} MHz) — beyond the validated main-beam \
         region. The off-axis gain returned here is numerically converged (the P10 Hankel / \
         azimuthal-mode integrator computes the physical-optics pattern correctly at all \
         angles), but the physical-optics term is IDEALISED: it omits blockage, feed/strut \
         scatter, and aperture-edge diffraction. The value includes the statistical Ruze \
         sidelobe floor as an incoherent power sum — a best-estimate MEDIAN wide-angle \
         level tracking measured earth-station statistics (NTIA 84-164), not a precise \
         per-antenna prediction. For sidelobe, interference, off-axis-EIRP, or \
         adjacent-satellite analysis, use calibration data or a regulatory envelope such \
         as the ITU-R S.580 mask.",
        calibration.antenna_id, threshold_deg, frequency_mhz
    )))
}

/// Rear-hemisphere hard-invalidity warning (roadmap unit P10-tail, maintainer
/// decision 2026-07-15).
///
/// Fires iff the observation direction has ANY backward component, i.e.
/// `|off_boresight_deg| > 90.0`. The aperture-integration formulation is a
/// forward-radiating model: the moment θ crosses 90° the returned value is a
/// numerical extrapolation of an idealised UNSHADOWED aperture field with no
/// physical validity behind the reflector — the far-field conversion carries the
/// Huygens obliquity factor (F7, 2026-07-16), but there is still no rim
/// diffraction, dish shadowing, feed spillover, or mesh leakage modeled (those,
/// not the aperture field, set real rear levels). The value is numerically
/// converged in the forward hemisphere but categorically meaningless here, so
/// per the maintainer decision it is still returned (grid totality on
/// `/heatmap` and `/h3-heatmap` must be preserved) but carries this harder
/// warning. What value is actually served behind the dish depends on
/// calibration status: uncorrected-physics antennas serve the statistical
/// sidelobe floor only (rear integration skipped, F7 redesign 2026-07-16);
/// corrected antennas still serve the raw physical-optics extrapolation.
///
/// Unlike [`off_axis_unvalidated_warning`], this is **NOT** gated on calibration
/// status: a correction surface fitted from forward-hemisphere measurements says
/// nothing about back lobes, so it fires for calibrated antennas too and takes
/// no calibration/status argument for the gate. The warning fires for every
/// antenna (a forward-hemisphere correction surface says nothing about back
/// lobes); only the WORDING branches on `physics_is_uncorrected()` —
/// uncorrected-physics antennas serve the statistical floor (F7 redesign
/// 2026-07-16), corrected antennas still serve the raw PO extrapolation.
///
/// The message is intentionally constant per (antenna, frequency) — it embeds no
/// query angle — so heatmap/H3 warning aggregation deduplicates it to a single
/// entry across grid points (the P8 convention).
///
/// Carries [`WarningCode::RearHemisphereInvalid`] (typed by C8 stage 3,
/// 2026-07-27). Both wording branches share the one code: the distinction they
/// draw — statistical floor vs raw PO extrapolation — is *what was served*, which
/// a client reads from `calibration_status`, not a different reason to distrust
/// the number.
pub(crate) fn rear_hemisphere_warning(
    calibration: &antenna_core::data::types::AntennaCalibration,
    off_boresight_deg: f64,
    frequency_mhz: f64,
) -> Option<ApiWarning> {
    // Gate at exactly θ=90°: the aperture formulation is meaningless the moment
    // the observation direction has a backward component.
    if off_boresight_deg.abs() <= 90.0 {
        return None;
    }

    if calibration.physics_is_uncorrected() {
        // F7 redesign (2026-07-16): on uncorrected-physics antennas the rear value IS
        // the statistical floor (PO excluded behind the dish).
        Some(WarningCode::RearHemisphereInvalid.with(format!(
            "Antenna '{}' query at {:.0} MHz is in the REAR HEMISPHERE (more than 90° off \
             boresight). The returned value is the statistical sidelobe floor ONLY — a \
             best-estimate median wide-angle level (NTIA 84-164) scaled by this antenna's \
             surface quality; the aperture-integration model has no physical validity \
             behind the reflector, so its term is excluded there. Real rear-hemisphere \
             levels are set by feed spillover past the rim, aperture-edge diffraction, and \
             mesh leakage — none of which are modeled individually. Use measured data or a \
             regulatory envelope (e.g. an ITU-R rear-lobe mask) for any rear-hemisphere \
             analysis.",
            calibration.antenna_id, frequency_mhz
        )))
    } else {
        Some(WarningCode::RearHemisphereInvalid.with(format!(
            "Antenna '{}' query at {:.0} MHz is in the REAR HEMISPHERE (more than 90° off \
             boresight). The aperture-integration model has NO physical validity behind the \
             reflector: the returned value is a numerical extrapolation of an idealised, \
             unshadowed aperture field, not a prediction. Real rear-hemisphere levels are set \
             by feed spillover past the rim, aperture-edge diffraction, and mesh leakage — none \
             of which are modeled here. Use measured data or a regulatory envelope (e.g. an \
             ITU-R rear-lobe mask) for any rear-hemisphere analysis.",
            calibration.antenna_id, frequency_mhz
        )))
    }
}

/// Ray-tracing stub degraded-accuracy warning (roadmap unit P3, maintainer
/// decision 2026-07-16: document + flag).
///
/// Returns [`antenna_core::model::pattern::RAY_TRACING_STUB_WARNING`] iff the antenna's
/// feed offset exceeds the severe threshold (> 0.5·f), i.e. the regime that the
/// model routes to the acknowledged ray-tracing stub (`ray_trace.rs`). The gate
/// mirrors the model's own `analyze_edge_cases` mode selection exactly: same
/// `displacement_from_focus / focal_length` ratio, same
/// [`antenna_core::model::edge_cases::SEVERE_OFFSET_THRESHOLD`].
///
/// **Why this exists at the service layer.** For single gain / batch / rectangular
/// heatmap the model pushes this warning itself (those paths call `compute_gain_db`
/// directly per query/point). The `/h3-heatmap` path instead caches PHYSICS-ONLY
/// gain and only runs `compute_gain_db` on a cache MISS, so the model-pushed
/// warning is lost on cache hits. This helper is re-emitted OUTSIDE the cache
/// closure in `compute_cell_gain`, exactly like [`off_axis_unvalidated_warning`]
/// and [`rear_hemisphere_warning`], so the honesty warning survives cache hits.
/// On a cache miss the model also emits the identical string; the H3 warning-set
/// aggregation deduplicates the pair to a single entry.
///
/// It is also this module's single expression of the model's mode selection, used by
/// [`PreparedServedGain::reconstruct_physics_warnings`] to decide whether a cache hit
/// earned the warning. That caller adds one condition this helper deliberately does not
/// know about — whether the mode dispatch was reached at all, which the F7 floor-only rear
/// path skips — because the threshold and the reachability question drift independently
/// and only the threshold belongs to `analyze_edge_cases`.
///
/// Deliberately **not** gated on calibration status: the ray-tracing stub is a
/// numerical/geometric limitation independent of whether a correction surface
/// exists. The message is constant per antenna config, so heatmap/H3 aggregation
/// deduplicates it to a single entry.
pub(crate) fn ray_trace_stub_warning(
    config: &antenna_core::model::geometry::AntennaConfiguration,
) -> Option<ApiWarning> {
    let focal_length = config.reflector.focal_length;
    if focal_length <= 0.0 {
        return None;
    }
    let offset_ratio = config.feed.position.displacement_from_focus(focal_length) / focal_length;
    if offset_ratio > antenna_core::model::edge_cases::SEVERE_OFFSET_THRESHOLD {
        Some(antenna_core::model::pattern::ray_trace_stub_warning())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{create_test_calibration, dummy_correction_surface};

    /// The prepared value is shared across parallel grid workers (issue #63 onward), so
    /// `Sync` is a structural requirement, not an accident of today's fields. Adding a
    /// `Cell`, `RefCell`, or non-atomic memoisation cache to memoise reference gain would
    /// break this line at compile time — which is the point.
    #[allow(dead_code)]
    fn assert_prepared_is_sync<T: Sync>() {}
    #[test]
    fn prepared_served_gain_is_sync() {
        assert_prepared_is_sync::<PreparedServedGain>();
    }

    // ---------------------------------------------------------------------------------
    // The prepared served-gain law (issue #61).
    //
    // These exercise the module's own seam. End-to-end `/gain` behaviour — status codes,
    // error precedence, response fields — stays pinned in `service::evaluator::tests`.
    // ---------------------------------------------------------------------------------

    /// 8400 MHz, the X-band frequency every other service fixture uses.
    const TEST_FREQ_MHZ: f64 = 8400.0;

    fn uncalibrated_status() -> CalibrationStatus {
        CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        }
    }

    /// The served warning codes, in order. Order matters to several of these tests (it is
    /// part of the `/gain` contract), so this keeps the sequence rather than a set.
    fn warning_codes(served: &ServedGain) -> Vec<WarningCode> {
        served.warnings.iter().map(|w| w.code).collect()
    }

    /// Prepare with no feed steering, no pointing-frequency offset, and a budget generous
    /// enough that nothing times out.
    fn prepare_unsteered(calibration: AntennaCalibration) -> PreparedServedGain {
        PreparedServedGain::prepare(
            calibration,
            // `create_test_calibration` puts the design feed at (0, 0, 0), so the steering
            // displacement is the whole physical feed position: park it at the focus.
            FeedSteering::new(0.0, 0.0, 5.0),
            ServedFrequencies::new(TEST_FREQ_MHZ, None),
            Duration::from_secs(300),
        )
        .expect("prepare must succeed for the canonical test artifact")
    }

    /// A correction surface that evaluates to exactly `correction_db` anywhere in its
    /// span. Shaped the way `calibrate::fit_frequency_correction` writes a flat-axis
    /// frequency correction — `order + 1` identical layers per axis over the full
    /// queryable span — because a B-spline is only a partition of unity when its
    /// coefficient count actually matches its knot vectors. (`dummy_correction_surface`
    /// is deliberately not reused here: it exists to make a surface *present*, and its
    /// all-zero coefficients hide the shape mismatch.)
    fn constant_correction_surface(
        correction_db: f64,
    ) -> antenna_core::data::types::BSplineModel4D {
        let order = 3usize;
        let layers = order + 1;
        let n_freq = 4;
        antenna_core::data::types::BSplineModel4D {
            coefficients: vec![correction_db; layers * layers * n_freq * layers],
            shape: [layers, layers, n_freq, layers],
            knots_azimuth: vec![0.0, 0.0, 0.0, 180.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 180.0, 180.0, 180.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 8300.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 500.0, 1000.0, 1000.0, 1000.0],
            spline_order: order as u8,
        }
    }

    #[test]
    fn correction_disposition_is_the_authority_for_applied_and_extrapolated() {
        // No surface: nothing was applied, and there is no fitted surface to extrapolate
        // from — "uncorrected" is not "extrapolated".
        assert!(!CorrectionDisposition::Unavailable.applied());
        assert!(!CorrectionDisposition::Unavailable.extrapolated());

        // Applied: the B-spline's own knot-range verdict is the answer.
        assert!(CorrectionDisposition::Applied {
            extrapolated: false
        }
        .applied());
        assert!(!CorrectionDisposition::Applied {
            extrapolated: false
        }
        .extrapolated());
        assert!(CorrectionDisposition::Applied { extrapolated: true }.applied());
        assert!(CorrectionDisposition::Applied { extrapolated: true }.extrapolated());

        // A surface exists but coverage was left: not applied, and the served physics is
        // an extrapolation beyond the calibrated region.
        assert!(!CorrectionDisposition::OutsideCoverage.applied());
        assert!(CorrectionDisposition::OutsideCoverage.extrapolated());
    }

    /// Preparation combines the request's steering displacement with THIS feed's design
    /// offset. Multi-feed artifacts put their feeds at different physical locations, and
    /// dropping the design term was worth up to 1.20 dB (roadmap D23).
    #[test]
    fn preparation_adds_the_design_feed_offset_to_the_request_steering() {
        let mut calibration = create_test_calibration(uncalibrated_status());
        calibration.physical_config.feed.position = (0.10, -0.20, 0.30);

        let served = PreparedServedGain::prepare(
            calibration,
            FeedSteering::new(1.0, 2.0, 5.0),
            ServedFrequencies::new(TEST_FREQ_MHZ, None),
            Duration::from_secs(300),
        )
        .unwrap()
        .evaluate_direct(
            PreSquintDirection::new(0.0, 0.0),
            ReferenceGainRequest::Omit,
        )
        .unwrap();

        // Reported offset is focal-point-relative, and focal_length_m = 5.0, so the
        // vertex-relative position this asserts is (1.10, 1.80, 5.30) — steering plus
        // design offset on every axis.
        let offset = served.physical_feed_offset;
        assert!((offset.x_m - 1.10).abs() < 1e-12, "x: {}", offset.x_m);
        assert!((offset.y_m - 1.80).abs() < 1e-12, "y: {}", offset.y_m);
        assert!((offset.z_m - 0.30).abs() < 1e-12, "z: {}", offset.z_m);
    }

    /// The vertex-relative physical feed POSITION (what physics and cache identity use)
    /// and the focal-point-relative OFFSET (what the response reports) differ by the focal
    /// length. Both exist precisely because they are not the same number; conflating them
    /// would report a 5 m defocus for a perfectly focused feed.
    #[test]
    fn reported_feed_offset_is_focal_point_relative_not_vertex_relative() {
        // focal_length_m = 5.0 on the canonical fixture, and the steering below parks the
        // feed at z = 5.0 — the focus. A vertex-relative report would say "5 m of defocus".
        let served = prepare_unsteered(create_test_calibration(uncalibrated_status()))
            .evaluate_direct(
                PreSquintDirection::new(0.0, 0.0),
                ReferenceGainRequest::Omit,
            )
            .unwrap();
        assert!(
            served.physical_feed_offset.x_m.abs() < 1e-12
                && served.physical_feed_offset.y_m.abs() < 1e-12
                && served.physical_feed_offset.z_m.abs() < 1e-12,
            "a feed at the focus reports a zero offset, got {:?}",
            served.physical_feed_offset
        );
    }

    /// Correction is added to the physics term, never folded into it. Both artifacts carry
    /// a surface, so `physics_is_uncorrected()` — and with it the P11 spillover and
    /// sidelobe-floor gates — is identical for the two; the ONLY difference is the
    /// surface's constant value, so the gain difference must be exactly that constant.
    #[test]
    fn correction_is_applied_after_physics_and_only_shifts_the_result() {
        let direction = PreSquintDirection::new(30.0, 2.0);

        let mut zero = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        zero.correction_surface = Some(constant_correction_surface(0.0));

        let mut shifted = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        shifted.correction_surface = Some(constant_correction_surface(2.5));

        let zero_served = prepare_unsteered(zero)
            .evaluate_direct(direction, ReferenceGainRequest::Omit)
            .unwrap();
        let shifted_served = prepare_unsteered(shifted)
            .evaluate_direct(direction, ReferenceGainRequest::Omit)
            .unwrap();

        assert_eq!(
            zero_served.correction,
            CorrectionDisposition::Applied {
                extrapolated: false
            }
        );
        assert_eq!(
            shifted_served.correction,
            CorrectionDisposition::Applied {
                extrapolated: false
            }
        );
        let delta = shifted_served.gain_db - zero_served.gain_db;
        assert!(
            (delta - 2.5).abs() < 1e-9,
            "a +2.5 dB constant surface must move served gain by exactly 2.5 dB, moved {delta}"
        );
    }

    /// Beam squint runs BEFORE calibration-coverage gating and correction interpolation.
    ///
    /// The test builds coverage that contains the PRE-squint direction and excludes the
    /// squint-corrected one; if the module gated on the pre-squint angles it would report
    /// a correction as applied. Phase one learns where squint actually lands so the
    /// coverage window is derived from the code under test rather than from a hand-computed
    /// squint the physics could quietly change.
    #[test]
    fn beam_squint_precedes_coverage_gating_and_correction() {
        let pre_squint = PreSquintDirection::new(0.0, 3.0);
        // A laterally displaced feed plus a pointing/operating frequency offset is what
        // produces squint at all.
        let steering = FeedSteering::new(0.4, 0.0, 5.0);
        let frequencies = ServedFrequencies::new(TEST_FREQ_MHZ, Some(TEST_FREQ_MHZ * 0.85));

        let mut unrestricted = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        unrestricted.correction_surface = Some(constant_correction_surface(0.0));
        let probe = PreparedServedGain::prepare(
            unrestricted,
            steering,
            frequencies,
            Duration::from_secs(300),
        )
        .unwrap()
        .evaluate_direct(pre_squint, ReferenceGainRequest::Omit)
        .unwrap();

        let squinted_cone = probe.direction.e_cone_deg;
        assert!(
            probe.direction.squint_magnitude_deg > 0.01,
            "fixture must actually squint, got {}",
            probe.direction.squint_magnitude_deg
        );
        assert!(
            (squinted_cone - pre_squint.e_cone_deg).abs() > 0.01,
            "squint must move the E-cone angle: {} vs {}",
            squinted_cone,
            pre_squint.e_cone_deg
        );

        // Coverage spanning the pre-squint E-cone but stopping short of the squinted one.
        let (lo, hi) = if squinted_cone > pre_squint.e_cone_deg {
            (0.0, (squinted_cone + pre_squint.e_cone_deg) / 2.0)
        } else {
            ((squinted_cone + pre_squint.e_cone_deg) / 2.0, 90.0)
        };
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(lo, hi)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(1000)
            .has_correction_surface(true)
            .build()
            .unwrap();
        assert!(
            coverage.contains_direction(pre_squint.e_clock_deg, pre_squint.e_cone_deg),
            "precondition: coverage must contain the PRE-squint direction"
        );
        assert!(
            !coverage.contains_direction(probe.direction.e_clock_deg, squinted_cone),
            "precondition: coverage must exclude the SQUINTED direction"
        );

        let mut restricted = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        restricted.correction_surface = Some(constant_correction_surface(0.0));
        restricted.calibration_coverage = Some(coverage);

        let served = PreparedServedGain::prepare(
            restricted,
            steering,
            frequencies,
            Duration::from_secs(300),
        )
        .unwrap()
        .evaluate_direct(pre_squint, ReferenceGainRequest::Omit)
        .unwrap();

        assert_eq!(
            served.correction,
            CorrectionDisposition::OutsideCoverage,
            "coverage must be tested at the squint-corrected direction, not the pre-squint one"
        );
        assert!(served.correction.extrapolated());
        assert!(!served.correction.applied());
    }

    /// Warning assembly order is part of the `/gain` contract, so it is pinned here rather
    /// than left to whatever order the branches happen to run in. A rear-hemisphere query
    /// on an uncalibrated antenna earns the whole uncorrected-physics set at once.
    #[test]
    fn warning_assembly_order_is_fixed() {
        let served = prepare_unsteered(create_test_calibration(uncalibrated_status()))
            .evaluate_direct(
                PreSquintDirection::new(0.0, 120.0),
                ReferenceGainRequest::Omit,
            )
            .unwrap();

        let codes = warning_codes(&served);
        let calibration_at = codes
            .iter()
            .position(|c| *c == WarningCode::Uncalibrated)
            .expect("uncalibrated advisory");
        let off_axis_at = codes
            .iter()
            .position(|c| *c == WarningCode::OffAxisUnvalidated)
            .expect("off-axis advisory");
        let rear_at = codes
            .iter()
            .position(|c| *c == WarningCode::RearHemisphereInvalid)
            .expect("rear-hemisphere advisory");

        assert!(
            calibration_at < off_axis_at && off_axis_at < rear_at,
            "fixed order is calibration status → off-axis → rear hemisphere, got {codes:?}"
        );
    }

    /// The reference is optional because it costs a second aperture integration. When it
    /// is requested, `loss_db` is derived from it and the FINAL served gain.
    #[test]
    fn reference_gain_is_optional_and_loss_is_derived_from_the_served_gain() {
        let direction = PreSquintDirection::new(0.0, 0.0);

        let omitted = prepare_unsteered(create_test_calibration(uncalibrated_status()))
            .evaluate_direct(direction, ReferenceGainRequest::Omit)
            .unwrap();
        assert_eq!(omitted.reference_gain_db, None);
        assert_eq!(omitted.loss_db, None);

        let included = prepare_unsteered(create_test_calibration(uncalibrated_status()))
            .evaluate_direct(direction, ReferenceGainRequest::Include)
            .unwrap();
        let reference = included.reference_gain_db.expect("reference requested");
        let loss = included.loss_db.expect("loss accompanies the reference");
        assert!(
            (loss - (reference - included.gain_db)).abs() < 1e-12,
            "loss must be reference - served gain"
        );
        assert!(
            (included.gain_db - omitted.gain_db).abs() < 1e-12,
            "asking for a reference must not change the served gain"
        );
    }

    /// Beam squint below the reporting threshold is `None` on the wire, so an unsquinted
    /// request does not carry a field full of float noise. The magnitude itself is always
    /// present on the result.
    #[test]
    fn unsquinted_evaluation_reports_no_beam_squint() {
        let served = prepare_unsteered(create_test_calibration(uncalibrated_status()))
            .evaluate_direct(
                PreSquintDirection::new(15.0, 4.0),
                ReferenceGainRequest::Omit,
            )
            .unwrap();

        assert_eq!(served.direction.squint_magnitude_deg, 0.0);
        assert_eq!(served.reported_beam_squint_deg(), None);
        // No squint means the evaluated direction IS the requested one.
        assert_eq!(served.direction.e_clock_deg, 15.0);
        assert_eq!(served.direction.e_cone_deg, 4.0);
    }

    /// Operating and pointing frequency are distinct: omitting the pointing frequency
    /// means "pointed for the frequency you are operating at", which is what makes an
    /// ordinary request unsquinted.
    #[test]
    fn pointing_frequency_defaults_to_the_operating_frequency() {
        let defaulted = ServedFrequencies::new(TEST_FREQ_MHZ, None);
        let explicit = ServedFrequencies::new(TEST_FREQ_MHZ, Some(TEST_FREQ_MHZ));
        assert_eq!(defaulted, explicit);

        let offset = ServedFrequencies::new(TEST_FREQ_MHZ, Some(7000.0));
        assert_ne!(defaulted, offset);
    }

    #[test]
    fn test_is_in_coverage_fully_covered() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(1000)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    /// A boresight artifact's coverage must accept a boresight query whatever its
    /// azimuth reads, because azimuth is degenerate at the pole (`atan2` on float
    /// noise). This is the gate that silently skipped the boresight frequency
    /// correction while the encoding was `azimuth_range = (0, 0)`.
    #[test]
    fn boresight_cone_coverage_accepts_a_pole_query_at_any_azimuth() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, antenna_core::data::types::BORESIGHT_COVERAGE_CONE_DEG)
                .frequency_range(3700.0, 6425.0)
                .num_measurements(6)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        // 63.43° is the azimuth measured for a query aimed exactly at the boresight
        // point on a realistic ECEF geometry; 0.0 and 359.9 are equally valid there.
        for az in [0.0, 63.43, 180.0, 359.9] {
            assert!(
                is_in_coverage(&coverage, az, 0.0, 4000.0),
                "boresight coverage rejected a boresight query at azimuth {az}"
            );
        }

        // Outside the cone is genuinely off-axis, whatever the azimuth.
        assert!(!is_in_coverage(&coverage, 63.43, 5.0, 4000.0));
    }

    /// The encoding this replaced, kept as an explicit record of the defect: a
    /// zero-width azimuth range rejects the point it is meant to cover.
    #[test]
    fn legacy_degenerate_boresight_coverage_rejects_its_own_point() {
        let legacy = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 0.0)
                .elevation_range(0.0, 0.0)
                .frequency_range(3700.0, 6425.0)
                .num_measurements(6)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(
            !is_in_coverage(&legacy, 63.43, 0.0, 4000.0),
            "if this now passes, the azimuth clause has been made pole-aware — good, \
             but update CalibrationCoverage::contains_direction_at_frequency's doc \
             comment and the roadmap item it points at"
        );
    }

    #[test]
    fn test_is_in_coverage_outside_azimuth() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 90.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_outside_elevation() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 30.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_outside_frequency() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(9000.0, 10000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_none_means_unrestricted() {
        // No coverage restriction recorded (fully calibrated artifact) → always in coverage.
        assert!(is_in_coverage(&None, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_generate_warnings_uncalibrated() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, WarningCode::Uncalibrated);
        // The accuracy figures are interpolated into the message, so they stay
        // message assertions — the code says *which* warning, not what it carries.
        assert!(warnings[0].message.contains("±3.0 dB"));
        assert!(warnings[0].message.contains("±2.0 dB"));
    }

    #[test]
    fn test_generate_warnings_partially_calibrated_in_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, true);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, WarningCode::PartiallyCalibrated);
        assert!(warnings[0].message.contains("±1.5 dB"));
    }

    #[test]
    fn test_generate_warnings_partially_calibrated_out_of_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 90.0)
            .elevation_range(0.0, 30.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });

        // Add a dummy correction surface to trigger the "not applied" warning
        calibration.correction_surface = Some(antenna_core::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0],
            knots_elevation: vec![0.0, 90.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(
            warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
            vec![
                WarningCode::PartiallyCalibrated,
                WarningCode::OutOfCoverage,
                WarningCode::CorrectionNotApplied,
            ]
        );
    }

    #[test]
    fn test_generate_warnings_fully_calibrated() {
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, true);

        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_generate_warnings_correction_not_applied() {
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });

        // Add a dummy correction surface
        calibration.correction_surface = Some(antenna_core::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0],
            knots_elevation: vec![0.0, 90.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, WarningCode::CorrectionNotApplied);
    }

    /// The two coverage questions are deliberately different, and centralizing
    /// them on `CalibrationCoverage` (issue #60) must not merge them.
    ///
    /// A query on the calibrated grid but at an uncalibrated FREQUENCY fails full
    /// coverage, so no correction is applied and `correction_not_applied` fires.
    /// It is still inside the measured spatial region, so `out_of_coverage` — the
    /// partial-calibration advisory, which reports only that the *direction* left
    /// the measured region — must stay silent.
    #[test]
    fn frequency_outside_the_band_does_not_report_out_of_spatial_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 30.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });
        calibration.correction_surface = Some(dummy_correction_surface());

        // 12 GHz is far outside the calibrated band; (45°, 15°) is inside the grid.
        assert!(!is_in_coverage(
            &calibration.calibration_coverage,
            45.0,
            15.0,
            12_000.0
        ));

        let warnings = generate_calibration_warnings(&calibration, 45.0, 15.0, false);

        assert_eq!(
            warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
            vec![
                WarningCode::PartiallyCalibrated,
                WarningCode::CorrectionNotApplied,
            ],
            "frequency alone must not raise the spatial out-of-coverage advisory"
        );
    }

    /// The served predicate is the calibration-coverage authority plus the
    /// `None` (unrestricted) case — nothing else.
    ///
    /// This asserts *agreement*, not semantics: `CalibrationCoverage`'s own tests
    /// own what the bounds mean. What this guards is a service-local copy of the
    /// range test growing back, and such a copy is only visible **at the
    /// boundary** — a clearly-inside and a clearly-outside probe agree even with
    /// a copy that has `>` where the authority has `>=`. So the probes walk every
    /// bound, derived from the coverage's own ranges rather than restated, and a
    /// coarser in/out set would not be a cheaper version of this test.
    #[test]
    fn is_in_coverage_agrees_with_the_calibration_coverage_authority() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(10.0, 350.0)
            .elevation_range(5.0, 60.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(true)
            .build()
            .unwrap();

        /// One step outside a bound, in degrees and in MHz.
        const STEP: f64 = 0.1;

        let (az_lo, az_hi) = coverage.azimuth_range;
        let (el_lo, el_hi) = coverage.elevation_range;
        let (f_lo, f_hi) = coverage.frequency_range;
        let (az_mid, el_mid, f_mid) = (
            f64::midpoint(az_lo, az_hi),
            f64::midpoint(el_lo, el_hi),
            f64::midpoint(f_lo, f_hi),
        );

        let probes = [
            // Plainly inside, then both extreme corners of the closed box.
            (az_mid, el_mid, f_mid),
            (az_lo, el_lo, f_lo),
            (az_hi, el_hi, f_hi),
            // One step outside each bound, one axis at a time.
            (az_lo - STEP, el_mid, f_mid),
            (az_hi + STEP, el_mid, f_mid),
            (az_mid, el_lo - STEP, f_mid),
            (az_mid, el_hi + STEP, f_mid),
            (az_mid, el_mid, f_lo - STEP),
            (az_mid, el_mid, f_hi + STEP),
        ];

        for (az, el, freq) in probes {
            assert_eq!(
                is_in_coverage(&Some(coverage.clone()), az, el, freq),
                coverage.contains_direction_at_frequency(az, el, freq),
                "served coverage diverged from CalibrationCoverage at ({az}, {el}, {freq})"
            );
        }

        // The one decision the service owns rather than delegates: an absent
        // coverage record is unrestricted, so every probe above is in coverage.
        for (az, el, freq) in probes {
            assert!(
                is_in_coverage(&None, az, el, freq),
                "absent coverage must be unrestricted at ({az}, {el}, {freq})"
            );
        }
    }

    // ------------------------------------------------------------------
    // Off-axis honesty warning (roadmap unit P8)
    //
    // Test fixture geometry: 10 m dish. At 8400 MHz, λ ≈ 0.0357 m, so the
    // first-null angle ≈ 1.6·λ/D ≈ 0.327° and the warning threshold
    // (3× first null) ≈ 0.98°.
    // ------------------------------------------------------------------

    #[test]
    fn test_off_axis_warning_fires_beyond_threshold_for_uncalibrated() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        let warning = off_axis_unvalidated_warning(&calibration, 2.0, 8400.0);
        let warning = warning.expect("2.0° off boresight > ~0.98° threshold must warn");
        assert_eq!(warning.code, WarningCode::OffAxisUnvalidated);
        // The rest of this test pins the honest *wording*, which is the point of the
        // P8/F7 warning — the code alone would not catch a message that regressed to
        // a stale or dishonest claim.
        let msg = &warning.message;
        assert!(msg.contains("beyond the validated main-beam region"));
        assert!(msg.contains("ITU-R S.580"));
        // Post-P10 honesty (2026-07-15): the P10 integrator landed, so the
        // off-axis value is now numerically converged/correct. The remaining
        // caveat is PHYSICAL — idealised PO omits blockage/strut/edge diffraction.
        assert!(
            msg.contains("IDEALISED"),
            "message must describe idealised physical optics: {msg}"
        );
        // F7 redesign (2026-07-16): the served value now includes the statistical
        // Ruze sidelobe floor as an incoherent power sum — a best-estimate median,
        // not a precise per-antenna prediction. It no longer claims the floor is
        // "intentionally off".
        assert!(
            msg.contains("incoherent power sum"),
            "message must describe the power-sum floor: {msg}"
        );
        assert!(
            msg.contains("best-estimate MEDIAN"),
            "message must state the floor is a best-estimate median: {msg}"
        );
        assert!(
            !msg.contains("intentionally off"),
            "stale D-2 wording (floor intentionally off) must not return: {msg}"
        );
        // The stale D-3 interim wording (numerical invalidity / aliasing) must be gone.
        assert!(
            !msg.contains("NUMERICALLY INVALID"),
            "stale interim wording must not return: {msg}"
        );
    }

    #[test]
    fn test_off_axis_warning_silent_inside_main_beam() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        assert!(off_axis_unvalidated_warning(&calibration, 0.0, 8400.0).is_none());
        assert!(off_axis_unvalidated_warning(&calibration, 0.5, 8400.0).is_none());
    }

    #[test]
    fn test_off_axis_warning_uses_absolute_angle() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        assert!(off_axis_unvalidated_warning(&calibration, -2.0, 8400.0).is_some());
    }

    /// Antennas whose served gain is CORRECTED physics (a correction surface is
    /// present) must NOT get the off-axis warning, regardless of calibration
    /// status: out-of-coverage queries there already receive the extrapolation
    /// warning, and stacking a second warning was explicitly ruled out (P8
    /// design constraint 1). Roadmap P11 keys this on surface presence
    /// (`physics_is_uncorrected()`), so this test attaches a surface to each
    /// fixture to pin the true invariant "surface present ⇒ silent".
    #[test]
    fn test_off_axis_warning_silent_when_correction_surface_present() {
        let mut fully = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        fully.correction_surface = Some(dummy_correction_surface());
        assert!(off_axis_unvalidated_warning(&fully, 45.0, 8400.0).is_none());

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 90.0)
            .elevation_range(0.0, 30.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let mut partial = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        });
        partial.correction_surface = Some(dummy_correction_surface());
        assert!(off_axis_unvalidated_warning(&partial, 45.0, 8400.0).is_none());

        // Status None is treated as fully calibrated (backward compatibility);
        // with a surface present it must stay silent.
        let mut unspecified = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        unspecified.calibration_status = None;
        unspecified.correction_surface = Some(dummy_correction_surface());
        assert!(off_axis_unvalidated_warning(&unspecified, 45.0, 8400.0).is_none());
    }

    /// P11 mismatch case (roadmap P11, from
    /// `docs/findings-2026-07-13-off-axis-integration-aliasing.md` §7): a
    /// `PartiallyCalibrated` antenna produced with NO correction surface (the
    /// no-frequency-correction path in `calibrate/boresight_calibration.rs`) has
    /// UNCORRECTED physics. Both uncorrected-physics behaviors must engage: the
    /// spillover fold-in gate is ON, and the off-axis honesty warning fires
    /// beyond threshold. Pre-P11 this antenna had spillover applied yet served no
    /// off-axis warning — the silent honesty gap this unit closes.
    #[test]
    fn test_partially_calibrated_without_surface_is_uncorrected_physics() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(50)
            .has_correction_surface(false)
            .build()
            .unwrap();
        let calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        });

        // No correction surface ⇒ uncorrected physics.
        assert!(calibration.correction_surface.is_none());
        assert!(calibration.physics_is_uncorrected());

        // (a) Spillover gate is ON (the same predicate drives it — see `prepare`'s
        //     `with_uncorrected_physics_gates` call above).
        assert!(
            calibration.physics_is_uncorrected(),
            "spillover fold-in must be gated ON for surfaceless partial calibration"
        );

        // (b) Off-axis honesty warning fires beyond threshold (~0.98° at 8400 MHz).
        let warning = off_axis_unvalidated_warning(&calibration, 2.0, 8400.0);
        let warning = warning.expect(
            "surfaceless PartiallyCalibrated antenna must get the off-axis honesty warning",
        );
        assert_eq!(warning.code, WarningCode::OffAxisUnvalidated);
        assert!(warning
            .message
            .contains("beyond the validated main-beam region"));

        // And stays silent inside the main beam (gate is the same, threshold intact).
        assert!(off_axis_unvalidated_warning(&calibration, 0.0, 8400.0).is_none());
    }

    /// The threshold is beamwidth-relative (λ/D), not a fixed angle: the same
    /// off-boresight angle warns for an electrically large antenna (narrow
    /// beam) and stays silent for an electrically small one (wide beam).
    #[test]
    fn test_off_axis_threshold_scales_with_wavelength_over_diameter() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        // 0.6° at 2000 MHz: threshold ≈ 4.1° → silent.
        assert!(off_axis_unvalidated_warning(&calibration, 0.6, 2000.0).is_none());
        // 0.6° at 30000 MHz: threshold ≈ 0.27° → warns.
        assert!(off_axis_unvalidated_warning(&calibration, 0.6, 30000.0).is_some());
    }

    /// P10-tail: the rear-hemisphere warning fires for θ>90° and is gated at
    /// exactly 90° (frequency-independent — the gate is purely geometric).
    #[test]
    fn test_rear_hemisphere_warning_fires_beyond_90_degrees() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        // Silent up to and including exactly 90°.
        assert!(rear_hemisphere_warning(&calibration, 0.0, 8400.0).is_none());
        assert!(rear_hemisphere_warning(&calibration, 45.0, 8400.0).is_none());
        assert!(rear_hemisphere_warning(&calibration, 90.0, 8400.0).is_none());

        // Fires the moment there is any backward component (and uses |angle|).
        let warning =
            rear_hemisphere_warning(&calibration, 90.001, 8400.0).expect("just past 90° must warn");
        assert_eq!(warning.code, WarningCode::RearHemisphereInvalid);
        let msg = &warning.message;
        assert!(msg.contains("REAR HEMISPHERE"));
        assert!(msg.contains("no physical validity") || msg.contains("NO physical validity"));
        assert!(rear_hemisphere_warning(&calibration, 120.0, 8400.0).is_some());
        assert!(rear_hemisphere_warning(&calibration, 180.0, 8400.0).is_some());
        // Absolute angle: negative backward angles warn too.
        assert!(rear_hemisphere_warning(&calibration, -163.0, 8400.0).is_some());
    }

    /// P10-tail: UNLIKE the off-axis warning, the rear-hemisphere warning is NOT
    /// gated on calibration status — a correction surface fitted from
    /// forward-hemisphere measurements says nothing about back lobes, so it fires
    /// for a FullyCalibrated antenna too.
    #[test]
    fn test_rear_hemisphere_warning_fires_for_calibrated_antenna() {
        let mut fully = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        // Corrected physics ⇒ a correction surface is present (P11 predicate gate).
        fully.correction_surface = Some(dummy_correction_surface());
        // The off-axis warning stays silent for a calibrated antenna even far off-axis...
        assert!(off_axis_unvalidated_warning(&fully, 120.0, 8400.0).is_none());
        // ...but the rear-hemisphere warning fires regardless of calibration status.
        assert!(rear_hemisphere_warning(&fully, 120.0, 8400.0).is_some());

        // Status None (treated as fully calibrated) also gets the rear warning.
        let mut unspecified = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        unspecified.calibration_status = None;
        assert!(rear_hemisphere_warning(&unspecified, 120.0, 8400.0).is_some());
    }

    /// P10-tail: the message is constant per (antenna, frequency) — it embeds no
    /// query angle — so heatmap/H3 warning-set aggregation deduplicates it to a
    /// single entry across a grid of rear-hemisphere cells (the P8 convention).
    #[test]
    fn test_rear_hemisphere_warning_dedups_across_grid() {
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        let mut set = std::collections::HashSet::new();
        for angle in [95.0_f64, 110.0, 130.0, 163.0, 179.9] {
            if let Some(msg) = rear_hemisphere_warning(&calibration, angle, 8400.0) {
                set.insert(msg);
            }
        }
        assert_eq!(
            set.len(),
            1,
            "rear-hemisphere warning must dedup to one entry across a rear grid"
        );
    }

    /// F7 redesign (2026-07-16): on an uncorrected-physics antenna (no
    /// correction surface — `physics_is_uncorrected()` true) the rear-hemisphere
    /// value IS the statistical sidelobe floor, since the PO term is excluded
    /// behind the dish. The wording must say so, and must still carry the
    /// pinned `REAR HEMISPHERE` marker.
    #[test]
    fn test_rear_hemisphere_warning_uncorrected_physics_states_floor_only() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.physics_is_uncorrected());

        let msg = rear_hemisphere_warning(&calibration, 120.0, 8400.0)
            .expect("rear hemisphere must warn on uncorrected-physics antennas");
        assert_eq!(msg.code, WarningCode::RearHemisphereInvalid);
        assert!(msg.message.contains("REAR HEMISPHERE"));
        assert!(msg.message.contains("statistical sidelobe floor ONLY"));
    }

    /// F7 redesign (2026-07-16): an antenna WITH a correction surface
    /// (`physics_is_uncorrected()` false) keeps the pre-existing extrapolation
    /// wording — a forward-hemisphere fit says nothing about back lobes, so no
    /// statistical floor is served there.
    #[test]
    fn test_rear_hemisphere_warning_corrected_physics_keeps_extrapolation_wording() {
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.correction_surface = Some(dummy_correction_surface());
        assert!(!calibration.physics_is_uncorrected());

        let msg = rear_hemisphere_warning(&calibration, 120.0, 8400.0)
            .expect("rear hemisphere must warn regardless of calibration status");
        assert_eq!(msg.code, WarningCode::RearHemisphereInvalid);
        assert!(msg.message.contains("REAR HEMISPHERE"));
        assert!(msg.message.contains("numerical extrapolation"));
    }

    // ---------------------------------------------------------------------------------
    // Cache-backed evaluation (issue #62).
    //
    // The invariant these pin is a three-way identity: DIRECT, COLD-cache (a miss that
    // runs the integration) and HOT-cache (a hit that skips it) must return the same
    // `ServedGain`, field for field and warning for warning. Two of those three can agree
    // while the third drifts — pre-C10 the hot path silently dropped a non-convergence
    // warning that both other paths carried — so every scenario below asserts all three.
    // ---------------------------------------------------------------------------------

    /// Prepare with an explicit feed steering, so a test can put the feed off-axis.
    fn prepare_with_steering(
        calibration: AntennaCalibration,
        steering: FeedSteering,
    ) -> PreparedServedGain {
        PreparedServedGain::prepare(
            calibration,
            steering,
            ServedFrequencies::new(TEST_FREQ_MHZ, None),
            Duration::from_secs(300),
        )
        .expect("prepare must succeed for the canonical test artifact")
    }

    /// A feed displacement of 0.6·f — past
    /// [`antenna_core::model::edge_cases::SEVERE_OFFSET_THRESHOLD`] (0.5·f), so the model
    /// dispatches to the acknowledged ray-tracing stub. The fixture's focal length is 5 m,
    /// so `z = 5.0` parks the feed at the focus and `x = 3.0` displaces it laterally by
    /// 0.6·f.
    fn severe_offset_steering() -> FeedSteering {
        FeedSteering::new(3.0, 0.0, 5.0)
    }

    /// A constant correction surface whose ELEVATION knots stop at 2°, so a query further
    /// off boresight lands outside the fitted span and earns the interpolator's
    /// extrapolation warning while staying inside calibrated coverage. Everything else
    /// matches [`constant_correction_surface`].
    fn narrow_elevation_correction_surface(
        correction_db: f64,
    ) -> antenna_core::data::types::BSplineModel4D {
        let mut surface = constant_correction_surface(correction_db);
        surface.knots_elevation = vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0];
        surface
    }

    /// The cache key the prepared value builds for one squint-corrected direction.
    ///
    /// Deliberately rebuilt from the prepared value's own fields rather than exposed by
    /// the module: a test that can ask the code under test for its key could not detect
    /// the key being built from the wrong quantities.
    fn cache_key_for(
        prepared: &PreparedServedGain,
        direction: &SquintCorrectedDirection,
    ) -> GainCacheKey {
        GainCacheKey::new(
            direction.e_clock_deg,
            direction.e_cone_deg,
            prepared.frequencies.operating_mhz,
            prepared.physical_feed_position.x,
            prepared.physical_feed_position.y,
            prepared.physical_feed_position.z,
        )
    }

    /// Put a known physics payload into the cache under `prepared`'s namespace and key,
    /// so a following evaluation that hits it is provably not integrating.
    fn prime(
        cache: &GainCache,
        prepared: &PreparedServedGain,
        direction: &SquintCorrectedDirection,
        entry: CachedGain,
    ) {
        cache
            .get_or_compute(
                &prepared.calibration.antenna_id,
                &prepared.calibration.feed_id,
                cache_key_for(prepared, direction),
                || Ok(entry),
            )
            .expect("priming the cache must not fail");
    }

    /// Assert the three-way identity for one prepared value and one direction.
    fn assert_direct_cold_hot_agree(
        prepared: &PreparedServedGain,
        direction: PreSquintDirection,
        reference: ReferenceGainRequest,
    ) -> ServedGain {
        let direct = prepared
            .evaluate_direct(direction, reference)
            .expect("direct evaluation");

        let cache = GainCache::new(true, 128);
        let cold = prepared
            .evaluate_cached(direction, reference, &cache)
            .expect("cold-cache evaluation");
        let hot = prepared
            .evaluate_cached(direction, reference, &cache)
            .expect("hot-cache evaluation");

        assert_eq!(direct, cold, "cold-cache evaluation must equal direct");
        assert_eq!(cold, hot, "hot-cache evaluation must equal cold");
        direct
    }

    /// Boresight on an uncorrected-physics antenna: the case where spillover IS folded
    /// into the physics term, so this is what pins spillover provenance surviving the
    /// cache. The reference gain is requested too, because it is derived from that
    /// provenance and would drift with it.
    #[test]
    fn cached_evaluation_matches_direct_at_boresight_with_spillover_provenance() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 0.0),
            ReferenceGainRequest::Include,
        );
        assert!(
            served.spillover_loss_db.is_some(),
            "fixture must actually apply spillover, or this pins nothing"
        );
        assert!(served.reference_gain_db.is_some());
    }

    /// Off-axis on an uncorrected-physics antenna: calibration advisory plus the P8
    /// off-axis honesty warning, both reconstructed after a hit.
    #[test]
    fn cached_evaluation_matches_direct_off_axis_with_calibration_and_off_axis_warnings() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 5.0),
            ReferenceGainRequest::Omit,
        );
        let codes = warning_codes(&served);
        assert!(codes.contains(&WarningCode::Uncalibrated), "{codes:?}");
        assert!(
            codes.contains(&WarningCode::OffAxisUnvalidated),
            "{codes:?}"
        );
        // This geometry also happens to be a genuine non-convergence case, so the identity
        // asserted above covers the canonical warning on a real integration rather than on
        // a hand-planted flag. If the integrator ever converges here this assert fails
        // loudly, which is the signal to re-point the case at a geometry that does not —
        // not to drop it, because the cached-flag path would then go untested end to end.
        assert!(
            codes.contains(&WarningCode::NonConvergence),
            "fixture must exercise the canonical non-convergence warning: {codes:?}"
        );
    }

    /// Rear hemisphere on an uncorrected-physics antenna: the F7 floor-only path, which
    /// returns before the mode dispatch and therefore before any integration at all.
    #[test]
    fn cached_evaluation_matches_direct_in_the_rear_hemisphere() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 120.0),
            ReferenceGainRequest::Omit,
        );
        let codes = warning_codes(&served);
        assert!(
            codes.contains(&WarningCode::RearHemisphereInvalid),
            "{codes:?}"
        );
    }

    /// A partially-calibrated antenna queried outside its coverage: the correction is not
    /// applied, and the advisory pair (partial calibration, out of coverage) plus
    /// `correction_not_applied` must all survive a hit.
    #[test]
    fn cached_evaluation_matches_direct_outside_calibrated_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 10.0)
            .elevation_range(0.0, 1.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(1000)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        });
        calibration.correction_surface = Some(constant_correction_surface(2.0));

        let prepared = prepare_unsteered(calibration);
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 5.0),
            ReferenceGainRequest::Omit,
        );
        assert_eq!(served.correction, CorrectionDisposition::OutsideCoverage);
        let codes = warning_codes(&served);
        assert!(
            codes.contains(&WarningCode::PartiallyCalibrated),
            "{codes:?}"
        );
        assert!(codes.contains(&WarningCode::OutOfCoverage), "{codes:?}");
        assert!(
            codes.contains(&WarningCode::CorrectionNotApplied),
            "{codes:?}"
        );
    }

    /// A correction applied outside the fitted knot span: the interpolator's own
    /// extrapolation warning is produced AFTER the cache lookup on every path, so it must
    /// be identical on a hit.
    #[test]
    fn cached_evaluation_matches_direct_when_the_correction_extrapolates() {
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.correction_surface = Some(narrow_elevation_correction_surface(2.0));

        let prepared = prepare_unsteered(calibration);
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 5.0),
            ReferenceGainRequest::Omit,
        );
        assert_eq!(
            served.correction,
            CorrectionDisposition::Applied { extrapolated: true },
            "fixture must actually extrapolate, or this pins nothing"
        );
        let codes = warning_codes(&served);
        assert!(codes.contains(&WarningCode::Extrapolated), "{codes:?}");
    }

    /// A severe feed offset in the FORWARD hemisphere routes to the ray-tracing stub, so
    /// the canonical degraded-accuracy warning is part of the served result — on all three
    /// paths. The model pushes it from inside the integration, which only a MISS runs;
    /// the cache-backed path has to reconstruct it.
    #[test]
    fn severe_offset_forward_reports_ray_tracing_degradation_on_every_path() {
        let prepared = prepare_with_steering(
            create_test_calibration(uncalibrated_status()),
            severe_offset_steering(),
        );
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 5.0),
            ReferenceGainRequest::Omit,
        );
        let codes = warning_codes(&served);
        assert!(codes.contains(&WarningCode::SevereFeedOffset), "{codes:?}");
        assert!(codes.contains(&WarningCode::RayTraceDegraded), "{codes:?}");
    }

    /// The same severe offset in the REAR hemisphere on an uncorrected-physics antenna
    /// never reaches the stub: the F7 floor-only early return happens before the mode
    /// dispatch. Emitting the degradation warning here would be a fabrication — the
    /// reconstruction must reproduce the model's silence, while keeping the diagnostics
    /// that DO apply.
    #[test]
    fn uncorrected_severe_offset_rear_omits_ray_tracing_degradation_on_every_path() {
        let prepared = prepare_with_steering(
            create_test_calibration(uncalibrated_status()),
            severe_offset_steering(),
        );
        assert!(
            prepared.integration_params.apply_sidelobe_floor,
            "precondition: the F7 floor must be on, which is what skips the integration"
        );
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 120.0),
            ReferenceGainRequest::Omit,
        );
        let codes = warning_codes(&served);
        assert!(
            !codes.contains(&WarningCode::RayTraceDegraded),
            "the floor-only rear path never reaches the ray-tracing stub: {codes:?}"
        );
        assert!(codes.contains(&WarningCode::SevereFeedOffset), "{codes:?}");
        assert!(
            codes.contains(&WarningCode::RearHemisphereInvalid),
            "{codes:?}"
        );
    }

    /// A CORRECTED antenna keeps the F7 floor off, so a rear-hemisphere query at the same
    /// severe offset does reach the mode dispatch and the stub — and must keep the
    /// warning. This is the counterpart that stops the test above from being satisfied by
    /// a blanket "no stub warning behind the dish" rule.
    #[test]
    fn corrected_severe_offset_rear_retains_ray_tracing_degradation_on_every_path() {
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.correction_surface = Some(constant_correction_surface(0.0));

        let prepared = prepare_with_steering(calibration, severe_offset_steering());
        assert!(
            !prepared.integration_params.apply_sidelobe_floor,
            "precondition: a corrected antenna keeps the floor off, so the stub is reached"
        );
        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 120.0),
            ReferenceGainRequest::Omit,
        );
        let codes = warning_codes(&served);
        assert!(codes.contains(&WarningCode::RayTraceDegraded), "{codes:?}");
        assert!(
            codes.contains(&WarningCode::RearHemisphereInvalid),
            "{codes:?}"
        );
    }

    /// A hit returns the CACHED physics term, not a fresh integration. Priming a value
    /// no integration could produce is the only way to assert that without timing the
    /// call: if the served gain is the sentinel, the aperture integral did not run.
    #[test]
    fn cache_hit_performs_no_integration() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let direction = PreSquintDirection::new(0.0, 5.0);
        let corrected = prepared.squint(direction);

        let cache = GainCache::new(true, 128);
        const SENTINEL_DB: f64 = -12.5;
        prime(
            &cache,
            &prepared,
            &corrected,
            CachedGain::new(SENTINEL_DB, true, None),
        );

        let served = prepared
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
            .unwrap();
        assert_eq!(
            served.gain_db, SENTINEL_DB,
            "a hit must serve the cached physics term; this artifact has no correction \
             surface, so served gain IS that term"
        );
    }

    /// The cache holds physics only: the correction surface is applied after every
    /// lookup, so two artifacts differing ONLY in their surface constant serve gains that
    /// differ by exactly that constant off one shared cached physics value.
    ///
    /// Both artifacts carry a surface, so `physics_is_uncorrected()` — and every P11 gate
    /// with it — is identical; the physics term they would each compute is the same
    /// number, which is what makes sharing a cache entry between them legitimate here.
    #[test]
    fn cache_hit_applies_the_current_correction_surface() {
        let direction = PreSquintDirection::new(0.0, 5.0);
        let with_constant = |correction_db: f64| {
            let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            });
            calibration.correction_surface = Some(constant_correction_surface(correction_db));
            prepare_unsteered(calibration)
        };

        let cache = GainCache::new(true, 128);
        let first = with_constant(1.0)
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
            .unwrap();
        let second = with_constant(4.0)
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
            .unwrap();

        assert!(first.correction.applied() && second.correction.applied());
        assert!(
            (second.gain_db - first.gain_db - 3.0).abs() < 1e-9,
            "a hit must re-apply the CURRENT surface to the cached physics term: \
             {} vs {}",
            first.gain_db,
            second.gain_db
        );
    }

    /// Convergence is the one piece of provenance a hit cannot re-derive — only the
    /// integration knows it, and a hit is precisely the case that skips the integration
    /// (roadmap C10). A primed non-converged entry must therefore produce the canonical
    /// warning, inserted where the model would have put it, and change nothing else.
    #[test]
    fn nonconvergence_rides_the_cache_into_the_served_warnings() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        // Boresight, where this fixture's integration converges — so the warning under
        // test can only have come from the cached flag.
        let direction = PreSquintDirection::new(0.0, 0.0);
        let corrected = prepared.squint(direction);

        let direct = prepared
            .evaluate_direct(direction, ReferenceGainRequest::Omit)
            .unwrap();
        assert!(
            !direct
                .warnings
                .iter()
                .any(|w| w.is(WarningCode::NonConvergence)),
            "precondition: this geometry converges, so the flag under test is the cache's"
        );

        let cache = GainCache::new(true, 128);
        prime(
            &cache,
            &prepared,
            &corrected,
            CachedGain::new(direct.gain_db, false, direct.spillover_loss_db),
        );

        let hot = prepared
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
            .unwrap();

        let canonical = antenna_core::model::pattern::nonconvergence_warning();
        let mut without_flag = hot.warnings.clone();
        let at = without_flag
            .iter()
            .position(|w| *w == canonical)
            .expect("a non-converged cache entry must warn");
        without_flag.remove(at);
        assert_eq!(
            without_flag, direct.warnings,
            "the non-convergence warning is the ONLY difference a stale flag makes"
        );
        assert_eq!(
            hot.gain_db, direct.gain_db,
            "the flag must not move the served number"
        );
    }

    /// Cache identity is scoped by (antenna, feed): a value primed for one composite
    /// identifier must never be served for another. The two prepared values are otherwise
    /// identical, so only the namespace can keep them apart.
    #[test]
    fn cache_identity_is_scoped_by_antenna_and_feed() {
        let direction = PreSquintDirection::new(0.0, 5.0);
        let mine = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let corrected = mine.squint(direction);

        let mut other_feed_artifact = create_test_calibration(uncalibrated_status());
        other_feed_artifact.feed_id = "other_feed".to_string();
        let other_feed = prepare_unsteered(other_feed_artifact);

        let mut other_antenna_artifact = create_test_calibration(uncalibrated_status());
        other_antenna_artifact.antenna_id = "other_antenna".to_string();
        let other_antenna = prepare_unsteered(other_antenna_artifact);

        let cache = GainCache::new(true, 128);
        const SENTINEL_DB: f64 = -12.5;
        prime(
            &cache,
            &mine,
            &corrected,
            CachedGain::new(SENTINEL_DB, true, None),
        );

        assert_eq!(
            mine.evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "precondition: the priming key must be the one this value looks up"
        );
        for (label, neighbour) in [("feed", &other_feed), ("antenna", &other_antenna)] {
            let served = neighbour
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap();
            assert_ne!(
                served.gain_db, SENTINEL_DB,
                "a different {label} must not read this cache entry"
            );
        }
    }

    /// Beam squint precedes cache-key construction. The key is built from the
    /// SQUINT-CORRECTED direction, so a value primed at the pre-squint angles is a miss
    /// and one primed at the corrected angles is a hit. Keying on the requested direction
    /// instead would return the gain for an angle the service never evaluated.
    #[test]
    fn cache_key_is_built_from_the_squint_corrected_direction() {
        let direction = PreSquintDirection::new(0.0, 3.0);
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.correction_surface = Some(constant_correction_surface(0.0));
        let prepared = PreparedServedGain::prepare(
            calibration,
            // A laterally displaced feed plus a pointing/operating frequency offset is
            // what produces squint at all.
            FeedSteering::new(0.4, 0.0, 5.0),
            ServedFrequencies::new(TEST_FREQ_MHZ, Some(TEST_FREQ_MHZ * 0.85)),
            Duration::from_secs(300),
        )
        .unwrap();

        let corrected = prepared.squint(direction);
        assert!(
            (corrected.e_cone_deg - direction.e_cone_deg).abs() > 0.01,
            "fixture must actually squint, got {corrected:?}"
        );
        let pre_squint = SquintCorrectedDirection {
            e_clock_deg: direction.e_clock_deg,
            e_cone_deg: direction.e_cone_deg,
            squint_magnitude_deg: 0.0,
        };

        const SENTINEL_DB: f64 = -12.5;

        let wrong_key = GainCache::new(true, 128);
        prime(
            &wrong_key,
            &prepared,
            &pre_squint,
            CachedGain::new(SENTINEL_DB, true, None),
        );
        assert_ne!(
            prepared
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &wrong_key)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "an entry stored at the PRE-squint angles must not be served"
        );

        let right_key = GainCache::new(true, 128);
        prime(
            &right_key,
            &prepared,
            &corrected,
            CachedGain::new(SENTINEL_DB, true, None),
        );
        assert_eq!(
            prepared
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &right_key)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "an entry stored at the squint-corrected angles must be served"
        );
    }

    /// A physical feed position that quantizes to a different millimetre is a different
    /// cache entry: the vertex-relative feed position is part of cache identity because
    /// it changes the physics, and two steerings that share a key would serve each
    /// other's gain.
    #[test]
    fn cache_identity_quantizes_the_vertex_relative_feed_position() {
        let direction = PreSquintDirection::new(0.0, 5.0);
        let steered = |x_m: f64| {
            prepare_with_steering(
                create_test_calibration(uncalibrated_status()),
                FeedSteering::new(x_m, 0.0, 5.0),
            )
        };

        let baseline = steered(0.0);
        let corrected = baseline.squint(direction);
        let cache = GainCache::new(true, 128);
        const SENTINEL_DB: f64 = -12.5;
        prime(
            &cache,
            &baseline,
            &corrected,
            CachedGain::new(SENTINEL_DB, true, None),
        );

        // Within the 1 mm quantum: the same entry, by design.
        assert_eq!(
            steered(0.0002)
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "sub-millimetre steering differences share a cache entry"
        );
        // Beyond it: a different entry, so a real integration runs.
        assert_ne!(
            steered(0.05)
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "a 50 mm steering difference must not read the baseline's entry"
        );
    }

    /// The operating frequency is part of cache identity. It reaches the key directly (not
    /// only through squint), because it changes the physics: two frequencies that shared an
    /// entry would serve each other's gain on an antenna whose feed never moved.
    #[test]
    fn cache_identity_includes_the_operating_frequency() {
        let direction = PreSquintDirection::new(0.0, 5.0);
        let at = |operating_mhz: f64| {
            PreparedServedGain::prepare(
                create_test_calibration(uncalibrated_status()),
                FeedSteering::new(0.0, 0.0, 5.0),
                ServedFrequencies::new(operating_mhz, None),
                Duration::from_secs(300),
            )
            .unwrap()
        };

        let baseline = at(TEST_FREQ_MHZ);
        let corrected = baseline.squint(direction);
        let cache = GainCache::new(true, 128);
        const SENTINEL_DB: f64 = -12.5;
        prime(
            &cache,
            &baseline,
            &corrected,
            CachedGain::new(SENTINEL_DB, true, None),
        );

        assert_eq!(
            baseline
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "precondition: the priming key must be the one this value looks up"
        );
        assert_ne!(
            at(TEST_FREQ_MHZ - 100.0)
                .evaluate_cached(direction, ReferenceGainRequest::Omit, &cache)
                .unwrap()
                .gain_db,
            SENTINEL_DB,
            "a different operating frequency must not read this cache entry"
        );
    }

    /// The moderate feed-offset band (0.3·f–0.5·f) is where "provenance, not policy" is
    /// load-bearing: the P11 spillover gate is ON, and the model still applies none,
    /// because `estimate_spillover` is not trusted past 0.3·f. A hit that re-derived
    /// spillover from the gate instead of from the payload would report a loss the served
    /// number never took, and bias `loss_db` against the ideal reference.
    #[test]
    fn cached_evaluation_matches_direct_in_the_moderate_offset_band() {
        // Focal length 5 m, so a 2 m lateral displacement is 0.4·f: past the spillover
        // ceiling, short of the severe threshold that would switch on ray tracing.
        let prepared = prepare_with_steering(
            create_test_calibration(uncalibrated_status()),
            FeedSteering::new(2.0, 0.0, 5.0),
        );
        assert!(
            prepared.integration_params.apply_spillover,
            "precondition: the P11 gate must be ON, or the None below proves nothing"
        );

        let served = assert_direct_cold_hot_agree(
            &prepared,
            PreSquintDirection::new(0.0, 5.0),
            ReferenceGainRequest::Include,
        );
        assert_eq!(
            served.spillover_loss_db, None,
            "the model applies no spillover past 0.3·f even with the gate on"
        );
        let codes = warning_codes(&served);
        assert!(
            codes.contains(&WarningCode::FeedOffsetSpilloverUnmodeled),
            "the moderate-band configuration warning must survive a hit: {codes:?}"
        );
        assert!(
            !codes.contains(&WarningCode::RayTraceDegraded),
            "0.4·f is below the severe threshold: {codes:?}"
        );
    }

    /// With the cache disabled every evaluation integrates, and must still agree with
    /// direct evaluation exactly — the disabled path is not a different gain law.
    #[test]
    fn disabled_cache_still_matches_direct_evaluation() {
        let prepared = prepare_unsteered(create_test_calibration(uncalibrated_status()));
        let direction = PreSquintDirection::new(0.0, 5.0);
        let disabled = GainCache::new(false, 128);

        let direct = prepared
            .evaluate_direct(direction, ReferenceGainRequest::Omit)
            .unwrap();
        let first = prepared
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &disabled)
            .unwrap();
        let second = prepared
            .evaluate_cached(direction, ReferenceGainRequest::Omit, &disabled)
            .unwrap();

        assert_eq!(direct, first);
        assert_eq!(first, second);
    }
}
