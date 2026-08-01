//! Aperture Integration Engine
//!
//! This module implements numerical integration over the reflector aperture
//! to compute far-field antenna patterns using physical optics.
//!
//! # Mathematical Foundation
//!
//! The far-field electric field is computed via aperture integration:
//! ```text
//! E(θ,φ) = (jk·exp(-jkr))/(2λr) ∬_Aperture A(ρ,φ') · exp[jΨ(ρ,φ')] · ρ dρ dφ'
//! ```
//!
//! where:
//! - A(ρ,φ') is the aperture illumination amplitude (from feed pattern)
//! - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
//! - Integration limits: ρ ∈ [0, D/2], φ' ∈ [0, 2π]
//!
//! # Numerical Methods
//!
//! Uses composite Simpson's rule with adaptive refinement:
//! - 2D integration via nested 1D integration
//! - Adaptive grid refinement for accuracy
//! - Convergence monitoring
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Implementation plan Sprint 2, Task 2.4

use num_complex::Complex64;
use std::f64::consts::PI;
use std::time::{Duration, Instant};

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    // `bessel_jn` (the per-order call) is deliberately absent: since P10-perf every Jₘ on this
    // path comes from `bessel_jn_array`'s single sweep, and mixing the two would reintroduce
    // the branch mismatch documented on `radial_probe_field`.
    bessel::{bessel_j0, bessel_jn_array},
    geometry::AntennaConfiguration,
    illumination::illumination_amplitude,
    wavelength_from_frequency,
    wavenumber,
};
// `phase_total` and `ApertureCoordinates` are only used by the retained 2D reference
// integrand (`aperture_integrand`), which is now test-only: since P10 Task 2 the
// production asymmetric path uses `azimuthal_mode_field`, not the 2D quadrature.
// Gate the imports so production builds stay warning-clean under `-D warnings`.
#[cfg(test)]
use crate::model::{coordinates::ApertureCoordinates, phase::phase_total};

/// Floor for the adaptive φ' Fourier-coefficient sample count `n_phi` used to build
/// `g_m(ρ)`. A small coma (the served feeds) needs only a handful of modes, so this floor
/// keeps near-boresight / small-offset evaluations cheap.
const MODE_PHI_MIN: usize = 64;

/// Ceiling for the adaptive φ' sample count `n_phi`. The azimuthal bandwidth of the
/// aperture-plane phase `g(ρ,φ')` is physically bounded by `k·R` (the aperture's k-space
/// radius), so `n_phi` never needs to exceed `~2·k·R`; this cap bounds the pathological
/// heavily-steered-feed case (the ray-tracing regime, D-5). Empirically 512 resolves the
/// `g_0` DFT to convergence even for a feed steered a full aperture-radius off-axis
/// (`k·R ≈ 443`), where n_phi=64/128 alias badly — the root of the interim off-axis error.
/// Raised 512 → 2048 on 2026-07-31 when the steered-feed φ' cap was removed. 512 was chosen
/// when strongly-steered geometries were clamped to 64 anyway; with the clamp gone it became
/// the binding ceiling for ordinary beam-steering — a 5° steer on the 34 m dish at X-band asks
/// for `n_phi ≈ 536`, and rounding-to-power-of-two would have asked for 1024. `B` is itself
/// bounded by `k·R` (a purely azimuthal phase gradient cannot exceed `k`), so this covers every
/// geometry with `k·R ≲ 1020`; past that the sizing clamps and `azimuthally_resolved` says so.
const MODE_PHI_MAX: usize = 2048;

/// Azimuthal-mode truncation ceiling `M_max`. The runtime count is sized adaptively from
/// the coma strength AND the observation angle by [`mode_count_for`] (only modes with
/// `m ≲ k·R·sinθ` survive the `Jₘ(kρsinθ)` kernel), then clamped here; the `M`-vs-`M+1`
/// self-check (D-6) flags any residual under-resolution. Kept strictly below
/// `MODE_PHI_MAX/2 − 1` so even the `M+1` probe mode stays above the φ'-Nyquist of the
/// largest `n_phi`.
const MODE_M_MAX: u32 = 254;

// The azimuthal DFT that builds g_m(ρ) needs > 2·M samples in φ' or the top modes alias
// (Nyquist). The self-check probes one extra mode (M+1); `mode_count_for` additionally
// clamps the runtime M to `n_phi/2 − 2`, but this guard pins the constant ceilings so a
// future bump cannot silently break the invariant even at the maximum n_phi.
const _: () = assert!(MODE_PHI_MAX > 2 * (MODE_M_MAX as usize + 1));

/// Absolute safety ceiling on the radial sample count handed to the Hankel / mode
/// integrator (P10 Task 3, D-4). The working density is derived from `(D/λ, θ)` by
/// [`radial_points_for`]; this only bounds pathological requests (e.g. a 300 m dish at
/// Q-band, θ=90°) so a single evaluation cannot allocate unbounded work. The runtime
/// convergence self-check recomputes at `2·N`, so the hard allocation limit is `2×` this.
/// Chosen comfortably above the largest enabled antenna's need — `gbt_100m` q-band at
/// θ=90° lands near `4·(D/λ) ≈ 5.7·10⁴` (`radial_points_for_gbt_qband_is_tens_of_thousands`),
/// whose `2·N` self-check leg (~1.1·10⁵) stays under `2×` this cap so it still converges.
const RADIAL_POINTS_SAFETY_MAX: usize = 65_537; // 2^16 + 1 (odd)

/// Generous default per-integration wall-clock budget (S3, cooperative compute bound).
///
/// The slowest known SINGLE served integration is `dsn_34m` Ka at θ=90° ≈ 3.3 s
/// (`docs/roadmap-2026-07.md`); 30 s leaves ~9× headroom so no existing test — including
/// the wide-angle `reference_validation` sweeps — can trip the budget. This is only a
/// fallback: the served path overrides it from `performance.integration_budget_ms` via
/// `IntegrationParams::time_budget`, so the knob is genuinely config-driven, not decorative.
pub const DEFAULT_INTEGRATION_BUDGET: Duration = Duration::from_secs(30);

/// Radial-sample stride between wall-clock budget checks in the two hot integrators.
///
/// `Instant::now()` is far too expensive to call per radial sample, and CLAUDE.md pitfall
/// #2 forbids touching sample density, so the deadline is polled only once every this many
/// radial samples. The check is a pure side-effect: when the deadline is NOT hit the
/// returned field is byte-identical to a build without the check.
const BUDGET_CHECK_STRIDE: usize = 1024;

/// Per-integration wall-clock deadline for the cooperative S3 budget.
///
/// Carries the absolute `deadline` instant (polled at radial chunk boundaries) plus the
/// original `budget` so an expiry error can report both the elapsed time and the configured
/// budget. Constructed once per `integrate_aperture` call from `IntegrationParams::time_budget`
/// and threaded into the two hot helpers. `None` disables the check entirely.
#[derive(Clone, Copy)]
struct IntegrationDeadline {
    deadline: Instant,
    budget: Duration,
}

impl IntegrationDeadline {
    /// If the wall-clock deadline has passed, build the typed over-budget error naming
    /// `operation`; otherwise `None`. Elapsed time is reconstructed as
    /// `budget + (now − deadline)` = time since the integration started.
    #[inline]
    fn check(&self, operation: &str) -> ComputationResult<()> {
        if Instant::now() > self.deadline {
            let elapsed = self.budget + self.deadline.elapsed();
            return Err(ComputationError::TimeBudgetExceeded {
                operation: operation.to_string(),
                elapsed_ms: elapsed.as_secs_f64() * 1000.0,
                budget_ms: self.budget.as_millis() as u64,
            });
        }
        Ok(())
    }
}

/// Effort cap (in radial cycles) on the coma term of [`radial_points_for`], applied ONLY past
/// [`crate::model::edge_cases::SEVERE_OFFSET_THRESHOLD`] — i.e. where the feed offset has
/// already taken the evaluation outside physical-optics scope, the caller has
/// `SevereFeedOffset`/`RayTraceDegraded`, and this integral is only the ray-tracing stub's
/// normalization anchor. Converging the radial axis of a number the model has disclaimed is
/// effort with no consumer.
///
/// The predecessor constant (`MODE_RADIAL_CYCLE_CAP`, same value) applied from `δ/f > 0.05`,
/// which caught ordinary beam-steering and there made the answer *both* less accurate and more
/// expensive — see the call site. Unlike it, this one is not silent: P12's radial N-vs-2N check
/// reports `converged = false` whenever the clamp costs accuracy.
const BEYOND_SCOPE_COMA_CYCLE_CAP: f64 = 8.0;

/// Azimuthal modes used by the cheap radial-convergence **pre-gate** (P12, D-A).
///
/// Chosen by measurement, not by magnitude: at both failing coma geometries the two largest
/// contributors to the *radial quadrature error* are `m = 0` and `m = 1`, and they are **not**
/// the largest modes by `|gₘ|` (`gs_3.7m` ranks 5,7,2,3,4 by magnitude but 0,1,5,7,3 by error;
/// `dsn_34m` ranks 4,3,6,1,5 by magnitude but 0,1,3,4,5 by error). A pre-gate that picked its
/// probes by mode magnitude — the form originally proposed for D-A option (ii) — would watch
/// the wrong modes. See `docs/findings-2026-07-31-p12-mode-path-radial-budget.md` §4a.
const RADIAL_PROBE_MODES: [u32; 2] = [0, 1];

/// Safety factor applied to the radial pre-gate's error estimate before comparing it against
/// the tolerance (P12, D-A).
///
/// **The pre-gate's estimate is not a bound.** Measured against the honest full N-vs-2N
/// estimate on five geometries, the `{0,1}` probe is *conservative* where it fires (1.17×,
/// 1.43×, 2.18× the full estimate) but *anti-conservative* where it passes — it underestimates
/// by **3.5×** at `dsn_34m` Ka θ=5° and **26×** at θ=90°, because when the total is nearly
/// converged the probe's own movement is a poor proxy for the residual of the ~195 modes it
/// does not watch. Multiplying by 32 covers the measured worst case with margin, and errs
/// toward escalating to the honest check — the fail-safe direction.
///
/// This constant is the price of shipping the pre-gate on five data points. Retire it (or
/// re-derive it) once P12 task 4's θ × D/λ sweep bounds the probe-to-total ratio properly.
const RADIAL_PRE_GATE_SAFETY: f64 = 32.0;

/// Work estimate (`n_rho × n_phi × modes`) above which a full radial check leg is judged too
/// expensive to spend unconditionally, so the cheap [`RADIAL_PROBE_MODES`] pre-gate runs first.
///
/// Below it, the honest full N-vs-2N check runs directly — which is deliberate rather than
/// merely thrifty: **every measured radial failure is in the cheap regime** (`gs_3.7m` 0.42 ms,
/// `dsn_34m` X 0.45 ms, D12 UHF 0.17 ms), while the expensive geometries (`dsn_34m` Ka, 299 ms
/// and 3.7 s) are already converged to ±0.02 dB. Placing the under-validated pre-gate only
/// where nothing has been observed to fail keeps the heuristic out of the regime that matters.
///
/// Calibrated from the same measurements: `dsn_34m` Ka θ=5° is ~1.4·10⁸ work units at 299 ms,
/// i.e. ~2.2 ms per 10⁶, so 4·10⁶ is a full check leg of roughly 10 ms. The five measured
/// geometries straddle it by five orders of magnitude (2·10⁴…1.7·10⁹), so the exact value is
/// not delicate.
const FULL_RADIAL_CHECK_WORK_LIMIT: u64 = 4_000_000;

/// Maximum radial doublings in the mode path's refinement loop (P12, D-A).
///
/// Cost is linear in `n_rho` and the legs sum geometrically, so `d` doublings cost
/// `2^(d+1) − 1` baseline legs; 4 caps that at 31× while letting the density grow 16×. The
/// measured worst case needs 3 (D12 UHF: 19 → 145). Running out of refinements is NOT an
/// error — it returns the best estimate with `converged = false` and an honest
/// `error_estimate`, which is what P12 asks for on geometries that cannot be resolved cheaply.
const MAX_RADIAL_REFINEMENTS: usize = 4;

/// Adaptive azimuthal sizing for one `(geometry, θ)` — see [`mode_count_for`].
#[derive(Debug, Clone, Copy)]
struct ModeSizing {
    /// Highest azimuthal mode actually summed.
    m_max: u32,
    /// φ' sample count for the `gₘ(ρ)` Fourier coefficients.
    n_phi: usize,
    /// Whether `n_phi` Nyquist-covers the aperture function's azimuthal bandwidth. When
    /// false, high modes of `g(ρ,φ')` fold down into the low `gₘ` and the result can be
    /// arbitrarily wrong (measured: **+28.7 dB**) — a failure neither the radial N-vs-2N
    /// check nor the `M`-vs-`M+1` truncation check can detect, because both operate on the
    /// already-corrupted coefficients. Propagates into `IntegrationResult::converged`.
    azimuthally_resolved: bool,
}

/// One azimuthal-mode sweep at a fixed radial density.
///
/// Carries the three quantities the two independent self-checks need, all from a single φ'
/// sweep: the full mode sum, the top mode's contribution (azimuthal **truncation** check) and
/// the [`RADIAL_PROBE_MODES`] partial sum (**radial** pre-gate).
#[derive(Debug, Clone, Copy)]
struct ModeSweep {
    /// `I(θ,φ)` summed over all modes `0..=m_probe` (both `±m`).
    total: Complex64,
    /// The part of `total` contributed by the top mode `±m_probe`.
    top_mode: Complex64,
    /// The part of `total` contributed by [`RADIAL_PROBE_MODES`].
    radial_probe: Complex64,
}

/// Work units in one azimuthal-mode radial sweep, reported through
/// [`IntegrationResult::num_evaluations`].
///
/// Per radial sample the mode path does two separable pieces of work: `n_phi` evaluations of
/// the aperture-plane function `g(ρ,φ')`, and `modes` mode-level accumulations (one `Jₘ`
/// recurrence step plus the `±m` radial accumulation). Both are linear, so the sum is a
/// faithful cost proxy and the leg count is recoverable as `num_evaluations / (n_phi + modes)`.
///
/// Before P10-perf this reported `n_rho · n_phi` alone, which understated the real cost by up
/// to a factor of `M` — the φ' DFT was `O(n_phi · M)` per radial sample, so the mode dimension
/// was the dominant term and the reported figure omitted it entirely (a P10-review finding).
/// The FFT removed that `×M` term rather than hiding it; what remains genuinely is
/// `n_phi + modes`, and the transform's own `O(n_phi log n_phi)` is folded into the `n_phi`
/// term as a constant factor.
#[inline]
fn mode_sweep_work(n_rho: usize, n_phi: usize, modes: usize) -> usize {
    n_rho.saturating_mul(n_phi.saturating_add(modes))
}

/// Whether this geometry gets the cheap radial pre-gate instead of paying for a full check
/// leg outright. See [`FULL_RADIAL_CHECK_WORK_LIMIT`].
#[inline]
fn use_radial_pre_gate(n_rho: usize, n_phi: usize, m_probe: u32) -> bool {
    let work = (n_rho as u64)
        .saturating_mul(n_phi as u64)
        .saturating_mul(m_probe as u64 + 1);
    work > FULL_RADIAL_CHECK_WORK_LIMIT
}

/// Complex integration result
///
/// The aperture integration produces a complex-valued field in the far zone.
/// Both real and imaginary parts are needed for phase information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntegrationResult {
    /// Complex electric field value
    pub field: Complex64,

    /// Estimated integration error (magnitude).
    ///
    /// For the P10 Hankel / azimuthal-mode integrator this is the magnitude of the
    /// runtime convergence self-check difference (D-6): `|I(2N) − I(N)|` for the
    /// symmetric radial path, or `|I(M+1) − I(M)|` (the top-mode contribution) for the
    /// asymmetric mode path. It is always finite and non-negative; a value larger than
    /// `relative_tolerance · |field|` is what sets `converged = false`.
    pub error_estimate: f64,

    /// Number of function evaluations performed
    pub num_evaluations: usize,

    /// Whether the integration passed its runtime convergence self-check (D-6).
    ///
    /// `true`  – the coarse/fine estimates agree: `error_estimate ≤
    ///           relative_tolerance · |field|` (or below `absolute_tolerance`). The
    ///           returned `field` is the finer estimate (`I(2N)` / `I(M+1)`).
    /// `false` – the estimates disagree by more than tolerance (radial density hit the
    ///           safety cap below Nyquist, or the mode count was insufficient). The
    ///           returned `field` is still the best (finer) estimate and `error_estimate`
    ///           holds the honest coarse/fine difference — the value is NEVER silently
    ///           returned as converged. `compute_gain_standard` surfaces this as the
    ///           `INTEGRATION_NONCONVERGENCE_WARNING`.
    pub converged: bool,
}

/// Integration parameters for convergence control
#[derive(Debug, Clone)]
pub struct IntegrationParams {
    /// Minimum number of radial integration points
    pub min_rho_points: usize,

    /// Maximum number of radial integration points (for adaptive refinement)
    pub max_rho_points: usize,

    /// Minimum number of azimuthal integration points
    pub min_phi_points: usize,

    /// Maximum number of azimuthal integration points
    pub max_phi_points: usize,

    /// Relative error tolerance for adaptive refinement
    pub relative_tolerance: f64,

    /// Absolute error tolerance
    pub absolute_tolerance: f64,

    /// Maximum number of refinement iterations
    pub max_iterations: usize,

    /// Fold physical feed-spillover efficiency into the returned gain.
    ///
    /// Decided by the SERVICE layer (set only for antennas with no correction
    /// surface — the surface otherwise absorbs spillover empirically). The model
    /// itself never inspects calibration; it only reads this bool.
    pub apply_spillover: bool,

    /// Apply the Ruze scattered-power sidelobe floor (F7).
    ///
    /// Off by default everywhere in this module — enabling it is a SERVICE-layer
    /// decision. Applied in `pattern::compute_gain` as an incoherent power sum with
    /// the pattern in the forward hemisphere (`gain + floor`, linear) and as a
    /// floor-only value behind the dish (θ>90°, F7 redesign 2026-07-16). The served
    /// path sets this via `AntennaCalibration::physics_is_uncorrected()` (true iff
    /// there is no correction surface). See `pattern::sidelobe_floor_gain` for the
    /// physical model.
    pub apply_sidelobe_floor: bool,

    /// Optional per-integration wall-clock budget (S3, cooperative compute bound).
    ///
    /// When `Some(budget)`, `integrate_aperture` computes a deadline at entry and the two
    /// hot radial integrators abort with `ComputationError::TimeBudgetExceeded` (→ 504)
    /// if a SINGLE integration's radial loop runs past it, checked at chunk boundaries
    /// (every `BUDGET_CHECK_STRIDE` samples). `None` disables the check. Every preset sets
    /// [`DEFAULT_INTEGRATION_BUDGET`]; the served path overrides it from
    /// `performance.integration_budget_ms`. This caps ONE integral — not the whole request
    /// (that is S2's `RequestTimeout`) nor the rayon fan-out (S4).
    pub time_budget: Option<Duration>,
}

impl Default for IntegrationParams {
    fn default() -> Self {
        Self {
            min_rho_points: 32,       // Minimum for radial direction
            max_rho_points: 128,      // Maximum for adaptive refinement
            min_phi_points: 64,       // Azimuthal (full 2π circle)
            max_phi_points: 256,      // Maximum azimuthal points
            relative_tolerance: 1e-4, // 0.01% relative error
            absolute_tolerance: 1e-8, // Absolute error floor
            max_iterations: 5,        // Refinement iteration limit
            apply_spillover: false,
            apply_sidelobe_floor: false,
            time_budget: Some(DEFAULT_INTEGRATION_BUDGET),
        }
    }
}

impl IntegrationParams {
    /// Canonical parameters for the SERVED (production) path.
    ///
    /// This is the single constructor the service layer should use (see
    /// `service::evaluator` and `service::h3_link_budget`). Since the P10
    /// off-axis integrator landed, the number of radial samples is derived
    /// ADAPTIVELY from `(D/λ, θ)` by `radial_points_for` — roughly
    /// `N_ρ ≈ 4·(D/λ)·sinθ` — so the physical correctness of the off-axis
    /// pattern no longer depends on this preset's magnitude. In this new
    /// regime the `min_rho_points`/`max_rho_points` fields are just:
    ///   * `min_rho_points` — a DENSITY FLOOR (cheap near-boresight cases), and
    ///   * `max_rho_points` — a safety knob / fallback size for the
    ///     `#[cfg(test)]`-only fixed-density 2D Simpson path.
    ///
    /// They no longer gate the served pattern's correctness.
    ///
    /// `min_rho_points` was **16 until P12 (2026-07-31) raised it to 32**, matching
    /// `default()`. Deliberately *not* framed as a correctness fix, because it is not one:
    /// measured against the three geometries P12 filed, the floor of 16 was **not binding at
    /// any of them** (the budget asked for 42 / 28 / 18 points), so it was never the source of
    /// the silent radial error — the missing self-check was, and that is fixed in
    /// [`integrate_aperture`]. The floor moved for a different and real reason: `calibrate`
    /// tunes under `default()` while the service evaluates under `adaptive()`, and D17 left
    /// that preset divergence open; matching the two closes it. It was deliberately NOT raised
    /// to 64, which would reopen the same divergence inverted — the service would then be more
    /// converged than the tuner that produced the artifact it is serving.
    ///
    /// INERT on the served path: `min_phi_points`, `max_phi_points`, and `max_iterations`
    /// are NOT read by either production integrator. The served φ' sample count comes from
    /// `mode_count_for` (not `min/max_phi_points`), the radial density from
    /// `radial_points_for`, and the refinement loop's bound is
    /// [`MAX_RADIAL_REFINEMENTS`], not `max_iterations`. These three fields survive only for
    /// the `#[cfg(test)]`-only 2D reference
    /// (`integrate_2d_adaptive` / `integrate_2d_simpson_public_shim`) and for struct
    /// compatibility with the other presets — tuning them here does nothing to a served
    /// evaluation.
    pub fn adaptive() -> Self {
        Self {
            // P12 / D-B (2026-07-31): 16 → 32, matching `default()`. See the docstring —
            // this closes D17's calibrate-vs-service preset divergence; it is NOT the fix
            // for the silent radial error (that is the self-check in `integrate_aperture`).
            min_rho_points: 32,
            max_rho_points: 64,
            min_phi_points: 32,
            max_phi_points: 128,
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-7,
            max_iterations: 3,
            apply_spillover: false,
            apply_sidelobe_floor: false,
            time_budget: Some(DEFAULT_INTEGRATION_BUDGET),
        }
    }

    /// Create fast integration parameters (lower accuracy, faster).
    ///
    /// NOTE: since the P10 adaptive off-axis integrator landed, this preset no
    /// longer gates production correctness — the served radial density is
    /// derived adaptively from `(D/λ, θ)` regardless of these values (see
    /// [`IntegrationParams::adaptive`]). Retained for the many tests that
    /// construct it directly; prefer `adaptive()` for the served path.
    pub fn fast() -> Self {
        Self {
            min_rho_points: 16,
            max_rho_points: 64,
            min_phi_points: 32,
            max_phi_points: 128,
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-7,
            max_iterations: 3,
            apply_spillover: false,
            apply_sidelobe_floor: false,
            time_budget: Some(DEFAULT_INTEGRATION_BUDGET),
        }
    }

    /// Create high-accuracy integration parameters (slower, more accurate).
    ///
    /// As with [`IntegrationParams::fast`], since the P10 adaptive integrator
    /// landed this preset no longer gates production correctness (the served
    /// radial density is adaptive — see [`IntegrationParams::adaptive`]). Kept
    /// for tests that need a high-density floor.
    pub fn high_accuracy() -> Self {
        Self {
            min_rho_points: 64,
            max_rho_points: 256,
            min_phi_points: 128,
            max_phi_points: 512,
            relative_tolerance: 1e-6,
            absolute_tolerance: 1e-10,
            max_iterations: 8,
            apply_spillover: false,
            apply_sidelobe_floor: false,
            time_budget: Some(DEFAULT_INTEGRATION_BUDGET),
        }
    }

    /// Set the two gates that key off the P11 "physics is uncorrected" predicate.
    ///
    /// `physics_is_uncorrected` is [`crate::data::types::AntennaCalibration::physics_is_uncorrected`]
    /// — true iff the artifact carries **no** correction surface. Both
    /// [`Self::apply_spillover`] and [`Self::apply_sidelobe_floor`] are gated on exactly
    /// that predicate when deriving the params for a **served gain**, and this setter is how
    /// that is done (roadmap P11).
    ///
    /// **Every producer of a gain number for a given artifact must call this with the same
    /// argument** — the service when it serves the artifact, and `calibrate` when it fits
    /// the artifact's parameters. They are different crates evaluating the same physics for
    /// the same antenna, and if they disagree the calibrator optimizes one model while the
    /// service serves another: the tuner's own residuals stop describing the served value.
    /// That is roadmap **D17**, which measured a constant −0.326 dB (and −0.953 dB on a
    /// broader feed) of served error from precisely this disagreement. Routing both sides
    /// through one function is what stops it recurring — there is no second place to
    /// forget.
    ///
    /// Note the argument is a property of the **artifact**, not of the query: it is decided
    /// once per antenna, so no discontinuity is introduced between covered and
    /// out-of-coverage queries.
    ///
    /// # The one place that deliberately does not use this
    ///
    /// `service::evaluator`'s **ideal-reference** computation (the `loss_db` denominator)
    /// sets `apply_spillover` alone, from `result.spillover_loss_db.is_some()` — the
    /// spillover the actual evaluation *applied*, not the predicate. That is correct and
    /// must stay: the model layer restricts spillover to `StandardPhysicalOptics`, so a
    /// large-offset feed can have the flag on and yet fold in no spillover. Deriving the
    /// reference's flag from the predicate there would apply spillover to the ideal
    /// reference that the actual never got, leaving a one-sided bias in `loss_db`. Its
    /// sibling `apply_sidelobe_floor` is carried unchanged from the clone and is inert on
    /// that path anyway (the ideal reflector has `surface_rms = 0.0`, so the floor is
    /// identically zero).
    ///
    /// So the rule is not "these flags are never set individually" — it is that **every
    /// producer of a gain for a given artifact derives them from the same predicate through
    /// this setter**. The reference is not a served gain for the artifact; it is a
    /// deliberately matched counterfactual. Do not "unify" that call site into this one.
    #[must_use]
    pub fn with_uncorrected_physics_gates(mut self, physics_is_uncorrected: bool) -> Self {
        self.apply_spillover = physics_is_uncorrected;
        self.apply_sidelobe_floor = physics_is_uncorrected;
        self
    }

    /// Create adaptive integration parameters with doubled sampling density
    ///
    /// Used near pattern nulls where rapid phase changes require finer sampling
    /// to maintain numerical accuracy.
    pub fn with_adaptive_refinement(&self) -> Self {
        Self {
            min_rho_points: self.min_rho_points * 2,
            max_rho_points: self.max_rho_points * 2,
            min_phi_points: self.min_phi_points * 2,
            max_phi_points: self.max_phi_points * 2,
            relative_tolerance: self.relative_tolerance / 2.0, // Tighter tolerance
            ..self.clone()
        }
    }
}

/// Integrate aperture field to compute far-field pattern
///
/// Performs 2D numerical integration over the reflector aperture using
/// composite Simpson's rule with adaptive refinement.
///
/// # Arguments
/// - `theta`: Polar angle in far field (radians, from boresight)
/// - `phi`: Azimuthal angle in far field (radians)
/// - `config`: Antenna configuration (geometry, feed, mesh)
/// - `frequency_hz`: Operating frequency in Hz
/// - `params`: Integration parameters (convergence tolerances, grid sizes)
///
/// # Returns
/// `IntegrationResult` containing complex field value, error estimate, and evaluation count
///
/// # Errors
/// Returns `ComputationError` if:
/// - Integration fails to converge within max iterations
/// - Invalid antenna configuration
///
/// # Examples
/// ```
/// use antenna_model::model::integration::{integrate_aperture, IntegrationParams};
/// use antenna_model::model::geometry::{AntennaConfiguration, ReflectorGeometry, FeedParameters};
///
/// // Example integration at boresight (θ=0)
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let reflector = ReflectorGeometry::builder()
/// #     .diameter(1.0)
/// #     .focal_length(0.5)
/// #     .surface_rms(0.001)
/// #     .build()?;
/// # let feed = FeedParameters::builder()
/// #     .at_focus(0.5)
/// #     .q_factor(8.0)
/// #     .build()?;
/// # let config = AntennaConfiguration::builder()
/// #     .id("test")
/// #     .name("Test")
/// #     .reflector(reflector)
/// #     .feed(feed)
/// #     .build()?;
/// let result = integrate_aperture(
///     0.0,               // theta (boresight)
///     0.0,               // phi
///     &config,
///     8.4e9,             // 8.4 GHz
///     &IntegrationParams::default(),
/// )?;
///
/// println!("Field magnitude: {}", result.field.norm());
/// println!("Error estimate: {}", result.error_estimate);
/// # Ok(())
/// # }
/// ```
pub fn integrate_aperture(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<IntegrationResult> {
    // Validate inputs
    if !theta.is_finite() || !phi.is_finite() || !frequency_hz.is_finite() {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: "Angles and frequency must be finite".to_string(),
        });
    }

    if frequency_hz <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: format!("Frequency must be positive, got {}", frequency_hz),
        });
    }

    // Calculate wavelength and wavenumber
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);

    // S3 cooperative wall-clock budget: a single over-budget integration aborts with a
    // typed error (→ 504) instead of burning a core unbounded. The deadline is per-call
    // (each of the two integrations behind one gain — off-axis + boresight anchor — gets a
    // fresh one), polled inside the radial loops at chunk boundaries. `None` disables it.
    let deadline = params.time_budget.map(|budget| IntegrationDeadline {
        deadline: Instant::now() + budget,
        budget,
    });

    // P10 Task 1: azimuthally symmetric apertures (no lateral feed offset) reduce EXACTLY
    // to a 1D radial Hankel (J₀) transform. Unlike the retired 2D quadrature, this does
    // NOT alias off-axis for electrically large dishes (the P0 bug). The asymmetric / coma
    // case uses the Jₘ azimuthal-mode expansion below.
    let is_symmetric = config.feed.position.radial_displacement() == 0.0
        // Azimuthally-symmetric illumination only: a non-unity asymmetry_factor makes
        // illumination_amplitude φ'-dependent (elliptical beam), which breaks the J₀
        // collapse (it assumes A has no φ' dependence). Such feeds take the mode path.
        && config.feed.asymmetry_factor == 1.0;
    if is_symmetric {
        // Adaptive radial density from (D/λ, θ) at ~2× Nyquist (Task 3, D-6), with a
        // runtime N-vs-2N self-check: recompute at 2N and compare. Agreement within
        // tolerance ⇒ converged; disagreement (density hit the safety cap below Nyquist)
        // ⇒ converged=false with an honest error estimate — never silently returned.
        let n1 = radial_points_for(config, theta, wavelength, params);
        let n2 = radial_check_points(n1);
        let f1 = hankel_radial_field(config, theta, phi, k, n1, deadline)?;
        let f2 = hankel_radial_field(config, theta, phi, k, n2, deadline)?;
        let (field, error_estimate, converged) = self_check(f1, f2, params, HANKEL_SELF_CHECK_RTOL);
        return Ok(IntegrationResult {
            field,
            error_estimate,
            num_evaluations: n1 + n2,
            converged,
        });
    }

    // P10 Task 2/3: asymmetric aperture — a lateral feed offset (coma) and/or an
    // azimuthally dependent illumination (`asymmetry_factor != 1.0`). Route through the
    // azimuthal-mode (Jₘ) expansion — the general, non-aliasing closed form (the symmetric
    // Hankel path above is its m=0-only special case).
    //
    // Adaptive sizing (Task 3, D-6):
    //   `n_rho`          — from (D/λ, θ) at ~2× Nyquist, shared with the symmetric path.
    //   `n_phi`          — φ' samples for the `g_m(ρ)` Fourier coefficients (adaptive).
    //   `m_max`          — mode truncation from the coma strength `k·δ·(R/f)`.
    // Runtime M-vs-(M+1) self-check: `azimuthal_mode_field_inner` returns both the full
    // sum (modes 0..=M+1) and the contribution of the top probe mode (M+1). If that top
    // mode contributes more than the relative tolerance, the truncation is insufficient
    // ⇒ converged=false with an honest error estimate.
    let ModeSizing {
        m_max,
        n_phi,
        azimuthally_resolved,
    } = mode_count_for(config, wavelength, theta);
    // Probe one extra mode (M+1) so the self-check can measure its contribution in a
    // single φ' sweep. `mode_count_for` kept m_max ≤ n_phi/2 − 2, so the probe is alias-free.
    let m_probe = m_max + 1;
    let mut n_rho = radial_points_for(config, theta, wavelength, params);
    let mut sweep =
        azimuthal_mode_field_inner(config, theta, phi, k, n_rho, n_phi, m_probe, deadline)?;
    let mut evaluations = mode_sweep_work(n_rho, n_phi, m_probe as usize + 1);

    // ---- P12: the RADIAL axis, which before this unit was never verified at all ----
    //
    // The mode-truncation check below and this one answer different questions, and the
    // integrator is only honest when BOTH are answered. Radial convergence is established
    // exactly as the symmetric branch establishes it — compare N against 2N and return the
    // FINE leg — with two additions the symmetric branch does not need:
    //
    //   * a cheap pre-gate on geometries where a full 2N leg is expensive (`dsn_34m` Ka is
    //     ~3.7 s, so an unconditional check would make it ~11 s), and
    //   * refinement, because no single density is right everywhere: at the same budget the
    //     2N leg lands at −0.045 dB on `gs_3.7m` but −0.349 dB on D12's UHF fixture.
    let radial_rtol = params.relative_tolerance.max(HANKEL_SELF_CHECK_RTOL);
    let mut radial_error = 0.0_f64;
    let mut radially_converged = false;

    if use_radial_pre_gate(n_rho, n_phi, m_probe) {
        // Cheap leg: repeat ONLY the low modes at 2N and watch how far they move. Compared
        // against `|total|` — the scale the answer's accuracy is measured on — not against
        // the probe's own magnitude, which is not the quantity at risk.
        let n_fine = radial_check_points(n_rho);
        let probe_fine =
            radial_probe_field(config, theta, phi, k, n_fine, n_phi, m_probe, deadline)?;
        evaluations += mode_sweep_work(n_fine, n_phi, RADIAL_PROBE_MODES.len());
        let probe_diff = (probe_fine - sweep.radial_probe).norm();
        let scale = sweep.total.norm().max(params.absolute_tolerance);
        if probe_diff * RADIAL_PRE_GATE_SAFETY <= radial_rtol * scale {
            radial_error = probe_diff * RADIAL_PRE_GATE_SAFETY;
            radially_converged = true;
        }
        // Otherwise fall through to the honest loop. The pre-gate is allowed to be wrong in
        // the direction of "spend more"; it is never allowed to certify on its own once it
        // has said the answer is moving.
    }

    if !radially_converged {
        for _ in 0..MAX_RADIAL_REFINEMENTS {
            if n_rho >= RADIAL_POINTS_SAFETY_MAX {
                break;
            }
            let n_fine = radial_check_points(n_rho);
            let fine = azimuthal_mode_field_inner(
                config, theta, phi, k, n_fine, n_phi, m_probe, deadline,
            )?;
            evaluations += mode_sweep_work(n_fine, n_phi, m_probe as usize + 1);
            radial_error = (fine.total - sweep.total).norm();
            // Return the FINE leg: with Simpson's O(h⁴) the returned estimate's own error is
            // ≈ diff/15, which is the entire reason the symmetric branch is accurate at the
            // same budget the mode path was under-delivering on.
            n_rho = n_fine;
            sweep = fine;
            radially_converged = radial_error
                <= radial_rtol * sweep.total.norm().max(params.absolute_tolerance)
                || radial_error < params.absolute_tolerance;
            if radially_converged {
                break;
            }
        }
    }

    // ---- The AZIMUTHAL axis: mode truncation, unchanged in kind since P10 ----
    // I(M) = I(M+1) − (top-mode contribution); the self-check compares I(M) vs I(M+1).
    let f_m = sweep.total - sweep.top_mode;
    let (field, mode_error, mode_converged) =
        self_check(f_m, sweep.total, params, MODE_SELF_CHECK_RTOL);

    // The two estimates bound errors on DIFFERENT axes of the same returned field, so they
    // are summed rather than either overwriting the other (P12 required this be decided
    // explicitly). Summing is the conservative combination: it never understates, and both
    // are already absolute field-magnitude differences in the same units.
    //
    // `azimuthally_resolved` is the THIRD axis and carries no magnitude — φ' aliasing has no
    // cheap estimator, only a yes/no from the sampling theorem — so it gates `converged`
    // without contributing to `error_estimate`. A false here means the number may be wrong by
    // tens of dB, which is exactly why it must not be silent.
    Ok(IntegrationResult {
        field,
        error_estimate: mode_error + radial_error,
        num_evaluations: evaluations,
        converged: mode_converged && radially_converged && azimuthally_resolved,
    })
}

/// Radial sample count for the self-check's fine (2N) leg: ~double `n1`, kept odd, and
/// bounded by the absolute allocation ceiling. Staying above `n1` is what makes the
/// N-vs-2N comparison meaningful; when `n1` was already clamped to the safety cap below
/// Nyquist, this finer leg exposes the disagreement so it is flagged, not hidden.
#[inline]
fn radial_check_points(n1: usize) -> usize {
    let n2 = (2 * n1)
        .saturating_sub(1)
        .min(2 * RADIAL_POINTS_SAFETY_MAX + 1);
    if n2.is_multiple_of(2) {
        n2 + 1
    } else {
        n2
    }
}

/// Relative-tolerance FLOOR for the Hankel / mode convergence self-check (D-6).
///
/// The self-check compares the field at `N` and `2N` (radial) or `M` and `M+1` (modes).
/// At the adaptive ~2× Nyquist radial density (4 samples/cycle) a *converged* Simpson
/// integral still shows an `N`-vs-`2N` field difference of a few tenths of a percent —
/// far above the `1e-4`..`1e-3` `relative_tolerance` the retired 2D adaptive loop used,
/// which would spuriously flag physically-accurate results (e.g. gbt_100m q-band at
/// θ=90°: 0.6 % ≈ 0.05 dB, well inside the < 0.1 dB accuracy budget).
///
/// This floor sets the gate to the accuracy budget instead: a 2 % `N`-vs-`2N` field
/// difference bounds the *returned* (finer, `2N`) estimate's own error to ≈ diff/15
/// (Richardson, Simpson is O(h⁴)) ⇒ < 0.15 % ⇒ < 0.013 dB. It stays far below the O(1)
/// (~100 %) mismatch that genuine under-resolution (density capped below Nyquist, or too
/// few modes) produces, so real non-convergence is still caught. The effective tolerance
/// is `max(params.relative_tolerance, this)` — a caller may loosen further but never
/// tighten below the physically-meaningful floor.
const HANKEL_SELF_CHECK_RTOL: f64 = 2.0e-2;

/// Relative-tolerance FLOOR for the azimuthal-mode TRUNCATION self-check (D-6), used ONLY
/// on the Jₘ mode path — deliberately tighter than [`HANKEL_SELF_CHECK_RTOL`].
///
/// The 2 % radial floor is justified by Richardson extrapolation: for a *converged* O(h⁴)
/// Simpson integral the returned `2N` estimate's own error is ≈ `diff/15`, so a 2 %
/// `N`-vs-`2N` field difference bounds the returned error to < 0.013 dB. That `diff/15`
/// benefit does NOT exist for a mode-TRUNCATION tail. There the self-check `diff` is just
/// the single `M+1` mode's contribution `|I(M+1) − I(M)|`, whereas the returned field's
/// actual error is the ENTIRE unmeasured tail `|Σ_{m≥M+2}|`. For a slowly-decaying
/// azimuthal spectrum that tail can be comparable to `diff`, so a 2 % `M+1` diff could hide
/// up to ≈ 0.17 dB of truncation error — above the documented < 0.1 dB budget.
///
/// Tail model (conservative): assume the modes beyond `M+1` form a geometric tail with
/// ratio ≤ 0.5, so `Σ_{m≥M+2} ≤ term(M+1) = diff` — i.e. the returned error is at most one
/// more `diff`. Gating `diff ≤ 0.5 %` then keeps the returned amplitude error ≤ 0.5 %
/// ≈ 0.043 dB, comfortably inside the < 0.1 dB budget (and still under budget even for a
/// somewhat slower tail: ratio ≈ 0.7 gives a tail ≈ 2.3·diff ≈ 1.16 % ≈ 0.1 dB, the edge).
/// The `+6` mode margin in [`mode_count_for`] pushes the `M+1` probe well into the
/// negligible tail for every real (physically-offset / asymmetric-illumination) case, so
/// they stay `converged=true` despite the tighter gate. Effective tolerance is
/// `max(params.relative_tolerance, this)` — same loosen-not-tighten floor semantics.
const MODE_SELF_CHECK_RTOL: f64 = 5.0e-3;

/// Runtime convergence verdict (D-6): compare a coarse and a fine field estimate and
/// decide whether the integrator converged. Returns `(field, error_estimate, converged)`
/// where `field` is ALWAYS the finer estimate (`fine`) and `error_estimate` is the
/// finite, non-negative coarse/fine magnitude difference. `converged` is true iff that
/// difference is within the effective relative tolerance times `|fine|`, or below
/// `absolute_tolerance`.
///
/// `rtol_floor` is the physically-justified relative-tolerance floor for THIS check: the
/// radial N-vs-2N path passes [`HANKEL_SELF_CHECK_RTOL`] (Richardson `diff/15` benefit),
/// while the mode M-vs-(M+1) truncation path passes the tighter [`MODE_SELF_CHECK_RTOL`]
/// (no Richardson benefit for a truncation tail — see that constant's docstring). The
/// effective tolerance is `max(params.relative_tolerance, rtol_floor)`, so a caller may
/// loosen but never tighten below the floor.
#[inline]
fn self_check(
    coarse: Complex64,
    fine: Complex64,
    params: &IntegrationParams,
    rtol_floor: f64,
) -> (Complex64, f64, bool) {
    let diff = (fine - coarse).norm();
    let magnitude = fine.norm();
    let rtol = params.relative_tolerance.max(rtol_floor);
    let converged =
        diff <= rtol * magnitude.max(params.absolute_tolerance) || diff < params.absolute_tolerance;
    (fine, diff, converged)
}

/// Perform 2D integration using composite Simpson's rule
///
/// Integrates over rectangular domain [rho_min, rho_max] × [phi_min, phi_max]
/// using nested 1D Simpson's rule.
///
/// Returns (integrated_value, num_evaluations)
///
/// Retained as a test-only near-in reference (the small-dish regime where the 2D
/// quadrature is trustworthy) and as the trusted oracle for the azimuthal-mode
/// integrator. Since P10 Task 2 it is no longer on any production code path — the
/// production off-axis integral goes through `hankel_radial_field` (symmetric) or
/// `azimuthal_mode_field` (asymmetric), which do not alias.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn integrate_2d_simpson(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    wavelength: f64,
    rho_min: f64,
    rho_max: f64,
    phi_min: f64,
    phi_max: f64,
    n_rho: usize,
    n_phi: usize,
) -> (Complex64, usize) {
    // Ensure odd number of points for Simpson's rule
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = (rho_max - rho_min) / (n_rho - 1) as f64;
    let h_phi = (phi_max - phi_min) / (n_phi - 1) as f64;

    let mut sum = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;

    // Outer integral over φ' using Simpson's rule
    for j in 0..n_phi {
        let phi_prime = phi_min + j as f64 * h_phi;
        let phi_weight = simpson_weight(j, n_phi);

        // Inner integral over ρ using Simpson's rule
        let mut inner_sum = Complex64::new(0.0, 0.0);

        for i in 0..n_rho {
            let rho = rho_min + i as f64 * h_rho;
            let rho_weight = simpson_weight(i, n_rho);

            // Evaluate integrand
            let integrand_value =
                aperture_integrand(rho, phi_prime, theta, phi, config, k, wavelength);

            num_evaluations += 1;

            // Accumulate with weights and Jacobian (ρ for polar coordinates)
            inner_sum += integrand_value * rho * rho_weight;
        }

        // Accumulate outer integral
        sum += inner_sum * phi_weight;
    }

    // Apply Simpson's rule scaling factors
    let integral = sum * h_rho * h_phi / 9.0; // 1/9 = (1/3) * (1/3) for 2D Simpson's

    (integral, num_evaluations)
}

/// Retained 2D adaptive Simpson refinement loop — the interim carrier of the
/// non-convergence sentinel (`converged=false`, `error_estimate=INFINITY`).
///
/// This is the exact loop `integrate_aperture` used before P10 Task 2. Now that the
/// production asymmetric path uses `azimuthal_mode_field`, this loop is off every
/// production code path; it is kept test-only so the two non-convergence tests
/// (`test_non_convergence_is_reported` here and `test_non_convergence_warning_propagated`
/// in `pattern.rs`) can pin the 2D mechanism directly until Task 3 reworks the runtime
/// convergence self-check into the Hankel/mode paths.
#[cfg(test)]
pub(crate) fn integrate_2d_adaptive(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> IntegrationResult {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);
    let rho_max = config.reflector.diameter / 2.0;
    let phi_min = 0.0;
    let phi_max = 2.0 * PI;

    let mut n_rho = params.min_rho_points;
    let mut n_phi = params.min_phi_points;
    let mut previous_result = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;
    let mut last_difference = f64::INFINITY;

    for iteration in 0..params.max_iterations {
        let (result, evals) = integrate_2d_simpson(
            theta, phi, config, k, wavelength, 0.0, rho_max, phi_min, phi_max, n_rho, n_phi,
        );
        num_evaluations += evals;

        if iteration > 0 {
            let difference = (result - previous_result).norm();
            let magnitude = result.norm();
            last_difference = difference;
            let relative_error = if magnitude > params.absolute_tolerance {
                difference / magnitude
            } else {
                difference
            };
            if relative_error < params.relative_tolerance || difference < params.absolute_tolerance
            {
                return IntegrationResult {
                    field: result,
                    error_estimate: difference,
                    num_evaluations,
                    converged: true,
                };
            }
        }

        previous_result = result;
        n_rho = (n_rho * 3 / 2).min(params.max_rho_points);
        n_phi = (n_phi * 3 / 2).min(params.max_phi_points);
        if n_rho >= params.max_rho_points && n_phi >= params.max_phi_points {
            break;
        }
    }

    IntegrationResult {
        field: previous_result,
        error_estimate: last_difference,
        num_evaluations,
        converged: false,
    }
}

/// Single fixed-density 2D Simpson evaluation at `params.max_rho_points ×
/// params.max_phi_points` — the converged near-in reference used by the azimuthal-mode
/// cross-validation test on small dishes (where the 2D quadrature is trustworthy).
#[cfg(test)]
fn integrate_2d_simpson_public_shim(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> Complex64 {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);
    let rho_max = config.reflector.diameter / 2.0;
    let (field, _) = integrate_2d_simpson(
        theta,
        phi,
        config,
        k,
        wavelength,
        0.0,
        rho_max,
        0.0,
        2.0 * PI,
        params.max_rho_points,
        params.max_phi_points,
    );
    field
}

/// Adaptive radial sample count for the Hankel / mode integrator at ~2× Nyquist
/// (P10 Task 3, D-6).
///
/// The `Jₘ(kρ·sinθ)` kernel oscillates at radial rate `≈ (D/λ)·sinθ` cycles across
/// `[0, R]`, so its Nyquist count is `N ≈ 2·(D/λ)·sinθ`. We take ~2× that
/// (`N ≈ 4·(D/λ)·sinθ`) for the kernel, sum in the other phase terms below, then floor at
/// `params.min_rho_points`, cap at [`RADIAL_POINTS_SAFETY_MAX`], and force odd for
/// Simpson's rule.
///
/// The count sums the radial oscillation of EVERY integrand phase term, not just the
/// θ-dependent kernel — critically the θ-INDEPENDENT aperture-plane phase (lateral coma,
/// axial defocus), which oscillates radially even at θ=0. Missing it silently aliases a
/// steered/offset feed at boresight (the P0 signature — off-axis gain far too high). Each
/// term's cycle count across `[0, R]`:
/// - `Jₘ` kernel (θ-dependent):         `(D/λ)·|sinθ|`
/// - dish-depth chirp (θ-dependent):    `(R²/(4fλ))·(1−cosθ)`, from the parabola's axial
///   sag `k·ρ²/(4f)·(1−cosθ)`. SUBDOMINANT in the forward hemisphere (why every P10 test
///   passes without it) but DOMINANT in the rear hemisphere: as θ→180° the `sinθ` kernel
///   budget collapses toward `min_rho_points` while this chirp peaks at `R²/(2fλ)` cycles.
///   Uncapped — it is a genuine radial phase term the self-check must be able to resolve.
/// - lateral coma (θ-independent):      `(δ/λ)·(R/f)`, capped at the physical maximum
///   radial spatial frequency `R/λ = D/(2λ)` (a purely-radial aperture phase gradient can
///   never exceed `k`, so the linear-steer estimate is clamped there for large `δ/f`)
/// - axial defocus (θ-independent):     `(|axial|/λ)·(R/f)²`, capped the same way
///
/// At θ=0 with a centered feed all terms vanish and the count drops to
/// `params.min_rho_points` — the cheap near-boresight case (the P10 throughput fix). It
/// deliberately does NOT read `params.max_rho_points`: that preset knob sizes the
/// retained test-only 2D reference, whereas the production density is derived here from
/// the physics (D-4). Forced odd for Simpson; capped at [`RADIAL_POINTS_SAFETY_MAX`].
fn radial_points_for(
    config: &AntennaConfiguration,
    theta: f64,
    wavelength: f64,
    params: &IntegrationParams,
) -> usize {
    let d_lambda = config.reflector.diameter / wavelength;
    let r = config.reflector.diameter / 2.0;
    let f = config.reflector.focal_length;
    let r_over_f = r / f;
    // Physical ceiling on any single aperture-plane term's radial cycles: a radial phase
    // gradient cannot exceed k, i.e. R/λ = D/(2λ) cycles across [0, R].
    let radial_cycle_ceiling = 0.5 * d_lambda;
    let delta = config.feed.position.radial_displacement();
    let axial = (config.feed.position.z - f + config.feed.axial_defocus).abs();
    // Outside physical-optics scope — see `BEYOND_SCOPE_COMA_CYCLE_CAP`. Same predicate
    // `mode_count_for` uses for the φ' ceiling, so the two axes agree on where scope ends.
    let beyond_po_scope = delta / f > crate::model::edge_cases::SEVERE_OFFSET_THRESHOLD;

    // Coma radial content, bounded by the physical `D/(2λ)` ceiling — and, past the model's
    // own PO scope boundary, by an effort cap.
    //
    // A former `MODE_RADIAL_CYCLE_CAP` clamped this to 8 cycles for `δ/f > 0.05`. That
    // threshold was wrong for the same reason its φ' sibling's was: it caught ordinary
    // beam-steering. On a 34 m dish with δ/f = 0.0875 (a routine ~5° steer) at θ=0 the true
    // content is 41.9 cycles, and capping made the answer *both* worse and more expensive —
    // the budget asked for 8, so `n_rho` started at 33 and four doublings (997 radial units)
    // still ended 0.34 dB short with an honest `converged = false`, where starting at the
    // physics' 169 converges on the first check for 506 units. Starting a refinement loop
    // below the physics saves nothing: every wasted leg is discarded.
    //
    // Past `SEVERE_OFFSET_THRESHOLD` the calculus inverts. There the feed is outside PO
    // scope, the caller already has `SevereFeedOffset`/`RayTraceDegraded`, and this integral
    // is only the ray-tracing stub's normalization anchor — so converging its radial axis is
    // effort spent on a number the model has disclaimed. The cap returns, keyed to that
    // boundary rather than 0.05, and it is not silent: the N-vs-2N check P12 added reports
    // `converged = false` when the clamp costs accuracy.
    let coma_effort_cap = if beyond_po_scope {
        BEYOND_SCOPE_COMA_CYCLE_CAP.min(radial_cycle_ceiling)
    } else {
        radial_cycle_ceiling
    };
    let kernel_cycles = d_lambda * theta.sin().abs();
    let coma_cycles = ((delta / wavelength) * r_over_f).min(coma_effort_cap);
    let defocus_cycles = ((axial / wavelength) * r_over_f * r_over_f).min(radial_cycle_ceiling);
    // Dish-depth chirp k·ρ²/(4f)·(1−cosθ): (R²/(4fλ))·(1−cosθ) cycles across [0,R].
    // Subdominant forward, DOMINANT behind the dish (θ→180°: kernel_cycles→0 while this peaks).
    let chirp_cycles = r * r / (4.0 * f * wavelength) * (1.0 - theta.cos());
    let cycles = kernel_cycles + coma_cycles + defocus_cycles + chirp_cycles;

    // ~2× Nyquist: 4 samples per cycle.
    let target = 4.0 * cycles;
    // Guard against a non-finite target (e.g. wavelength underflow) — fall back to floor.
    let target = if target.is_finite() {
        target.ceil() as usize
    } else {
        params.min_rho_points
    };
    let n = target
        .max(params.min_rho_points)
        .min(RADIAL_POINTS_SAFETY_MAX);
    if n.is_multiple_of(2) {
        n + 1
    } else {
        n
    }
}

/// Adaptive azimuthal sizing for the coma / asymmetric path (P10 Task 3).
///
/// Three quantities are derived here:
///
/// - **`n_phi`** (φ' DFT sample count) must resolve the azimuthal spectrum of the INPUT
///   aperture-plane function `g(ρ,φ')`, whose maximum significant mode `B` is the wider of
///   two drivers (physically capped at `k·R`, the aperture's k-space radius — a
///   purely-azimuthal phase gradient cannot exceed `k`): the coma spread
///   `spread = k·δ·(R/f)` from a lateral feed offset, OR an illumination floor of `6` when
///   `asymmetry_factor != 1.0` (an elliptical feed modulates the effective q-factor by
///   `cos(2φ')`, so `g` carries m=±2 plus weaker ±4, ±6 harmonics even for a CENTERED feed,
///   δ=0). Under-sizing `n_phi` aliases high input modes into `g_0`. Only the pure-symmetric,
///   pure-axial-defocus case (`asymmetry_factor==1.0` AND no lateral coma) has no azimuthal
///   content and takes the cheap `(1, MODE_PHI_MIN)` fast path.
///
/// - **`m_max`** (modes actually summed) need only include modes that survive the
///   `Jₘ(kρ·sinθ)` kernel. `Jₘ(x)` does NOT switch off at `m = x` — it has an Airy-type
///   turning point there and decays over a width `~x^(1/3)` — so with `x = k·R·|sinθ|`,
///   `m_max = min(1.5·B + 6, x + 4·x^(1/3) + 6)`, clamped to [`MODE_M_MAX`] and to
///   `n_phi/2 − 2` (so the `M+1` self-check probe stays alias-free). The `x^(1/3)` term
///   replaced a flat `+6` on 2026-07-31: the flat margin sat far inside the transition
///   region for large `x` and cost +0.49 dB at θ=3° on a strongly-comaed feed. At θ=0 only
///   `m=0` survives (`Jₘ(0)=0, m>0`), so `m_max` collapses to the margin and the sum is
///   cheap even when `n_phi` is large.
///
/// - **`azimuthally_resolved`** — whether the φ' grid actually Nyquist-covers `B`. This is a
///   THIRD failure axis, invisible to both existing self-checks, and it was silently wrong
///   until 2026-07-31.
///
/// # Why `azimuthally_resolved` exists (and why the two other checks cannot substitute)
///
/// A former constant `MODE_PHI_STEERED_MAX` clamped `n_phi` to 64 whenever
/// `δ/f > MODE_STEERING_RATIO`, documented as safe because "the `M`-vs-`M+1` self-check
/// honestly reports `converged=false`". **It does not, and cannot.** That check compares
/// `I(M)` against `I(M+1)`, i.e. it measures mode *truncation* — but φ' under-sampling
/// corrupts every `gₘ` including the ones being compared, so the difference stays small while
/// both terms are wrong. At θ=0 it is worse than uninformative: `Jₘ(0) = 0` for `m > 0`, so the
/// increment is identically zero and the check is trivially satisfied.
///
/// Measured on a 3 m dish at 8.4 GHz with `δ/f = 0.4` (true bandwidth `k·δ·(R/f) ≈ 106`), against
/// the 2D Simpson quadrature — a trustworthy oracle at `D/λ = 84`: at `n_phi = 64` the mode path
/// converges radially to **+28.67 dB above the oracle** and stays there at ANY radial density,
/// reporting `converged = true`. At `n_phi ≥ 256` it reproduces the oracle to −0.017 dB. The
/// cap was a P10-class silent error hiding behind a check that structurally could not see it.
///
/// Re-measured on the `coma_aberration_test` geometry — 34 m, `δ/f = 0.0875`, i.e. a **routine
/// ~5° beam steer**, nowhere near the 0.5f ray-tracing regime — the same clamp was wrong by
/// **+77 dB at θ=0 and +82 dB at θ=1°**. This was not a graceful degradation for exotic inputs.
///
/// Now `n_phi` is sized from the physics — enough to Nyquist-cover `B` — bounded only by
/// [`MODE_PHI_MAX`], and when that bound binds the caller is
/// told so instead of being handed a plausible wrong number. Cost is bounded by S3's wall-clock
/// budget, which is what that budget is for.
///
/// Validated: `spread ≈ 8.8` needs `M ≈ 16`; `dsn_34m` X-band `spread ≈ 33` needs
/// `M ≈ 46`; a feed steered a full aperture radius off-axis needs `n_phi ≈ 512`.
fn mode_count_for(config: &AntennaConfiguration, wavelength: f64, theta: f64) -> ModeSizing {
    let k = wavenumber(wavelength);
    let r = config.reflector.diameter / 2.0;
    let delta = config.feed.position.radial_displacement();
    let r_over_f = r / config.reflector.focal_length;
    let k_r = k * r; // physical azimuthal-bandwidth ceiling
    let spread = k * delta * r_over_f;

    // Coma-driven azimuthal bandwidth of g(ρ,φ') from a lateral feed offset, physically
    // capped at k·R. Zero for a centered feed (δ=0) or a non-finite spread.
    let coma_bandwidth = if spread.is_finite() && spread > 0.0 {
        spread.min(k_r)
    } else {
        0.0
    };

    // Illumination-driven azimuthal bandwidth. When `asymmetry_factor != 1.0`,
    // `illumination_amplitude` modulates the effective q-factor by `cos(2φ')`, so the
    // aperture function g(ρ,φ') carries a genuine m=±2 fundamental PLUS weaker m=±4, ±6
    // harmonics from the nonlinear `cos_q_pattern(ψ, q(φ'))` dependence — even for a
    // CENTERED feed (δ=0, coma_bandwidth=0). A bandwidth floor of 6 ensures those
    // harmonics are resolved; without it `m_max=1` under-resolves the m=2 content AND the
    // M-vs-(M+1) self-check would see the large m=2 jump and spuriously flag non-convergence.
    let asym_bandwidth = if config.feed.asymmetry_factor != 1.0 {
        6.0
    } else {
        0.0
    };

    // Combine the two drivers (take the wider). Pure-symmetric, pure-axial-defocus feeds
    // (asymmetry_factor==1.0 AND no lateral coma) genuinely have no azimuthal content, so
    // they keep the cheap (1, MODE_PHI_MIN) fast path.
    let bandwidth = coma_bandwidth.max(asym_bandwidth);
    if bandwidth <= 0.0 {
        return ModeSizing {
            m_max: 1,
            n_phi: MODE_PHI_MIN,
            azimuthally_resolved: true,
        };
    }

    // Modes that survive the Jₘ(kρ·sinθ) kernel at this θ, BEFORE any sampling ceiling. This
    // is what the geometry needs; the ceilings below are what we can afford.
    //
    // `Jₘ(x)` does not switch off at `m = x`; it has an Airy-type turning point there and
    // decays over a transition region of width `~x^(1/3)`. A flat `+6` margin (what this used
    // until 2026-07-31) is far inside that region for large `x`, so it truncates live spectrum
    // whenever `gₘ` is still strong at the kernel cutoff — i.e. exactly on strongly-comaed
    // feeds. Measured on the 34 m / δ = 1.19 m geometry against an `m = 382` reference:
    // `+6` cost **+0.103 dB at θ=1°** and **+0.490 dB at θ=3°**, both fully recovered by ~16
    // more modes. `4·x^(1/3)` covers the measured cases (15 and 22 modes respectively) and
    // scales the way the transition region does, rather than being a constant fitted to two
    // points. The `M`-vs-`M+1` self-check — which flagged this correctly, at 14% top-mode
    // contribution, while the value-level anchors could not see it because they truncated
    // their references identically — remains the backstop.
    let kernel_arg = k_r * theta.sin().abs();
    let m_theta = kernel_arg.ceil() + 4.0 * kernel_arg.cbrt() + 6.0;
    let m_spectrum = (1.5 * bandwidth).ceil() + 6.0;
    let m_needed = m_spectrum.min(m_theta).max(1.0);

    // `n_phi` must Nyquist-cover the INPUT bandwidth `B`, or high modes of `g(ρ,φ')` fold down
    // into the low `gₘ` — including `g₀`, which at θ=0 IS the answer.
    //
    // Rounded up to the next **5-smooth** length (P10-perf, 2026-08-01), because the φ'
    // transform is now an FFT ([`crate::model::fft`]) rather than a direct DFT. Still NOT
    // rounded to a power of two, and for the reason that predates the FFT: the padding is
    // aperture-plane evaluations, which are the integrator's floor cost, and B ≈ 263 asking
    // for 536 would be given 1024 — 91 % extra — where the nearest fast length is 540, i.e.
    // 0.7 %. Kept even so the ±m pairs stay symmetric on the grid.
    let n_phi_needed = crate::model::fft::next_fast_len(
        ((2.0 * bandwidth).ceil() as usize + 8).next_multiple_of(2),
    );

    // Effort ceiling, keyed to the model's OWN scope boundary. Past
    // `SEVERE_OFFSET_THRESHOLD` (0.5f) the feed is outside physical-optics scope: the caller
    // is already told so (`SevereFeedOffset` + `RayTraceDegraded`), gain routes to the
    // ray-tracing stub, and this aperture integral survives only as that stub's normalization
    // anchor. Resolving its azimuthal spectrum exactly there is expensive and buys nothing —
    // a request steering the feed to 3.06f asks for `n_phi ≈ 628` to converge a number the
    // model has already disclaimed.
    //
    // This is the same *shape* as the deleted `MODE_PHI_STEERED_MAX`, and deliberately so, but
    // it differs on the two things that made that constant a defect: it triggers at the
    // documented 0.5f scope boundary rather than an arbitrary 0.05 (which caught ordinary
    // beam-steering — a 5° steer is δ/f = 0.0875 — and was wrong there by up to +82 dB), and
    // it is NOT silent: `azimuthally_resolved` below goes false, so `converged` does too.
    let beyond_po_scope =
        delta / config.reflector.focal_length > crate::model::edge_cases::SEVERE_OFFSET_THRESHOLD;
    let n_phi_ceiling = if beyond_po_scope {
        MODE_PHI_MIN
    } else {
        MODE_PHI_MAX
    };
    let n_phi = n_phi_needed.clamp(MODE_PHI_MIN, n_phi_ceiling);

    // The mode sum is separately clamped by the φ' Nyquist (`n_phi/2 − 2`, keeping the `M+1`
    // probe alias-free) and by MODE_M_MAX. Truncation here is NOT silent: it is exactly what
    // the `M`-vs-`M+1` self-check measures, so it needs no flag of its own.
    let m_cap = (n_phi / 2).saturating_sub(2) as f64;
    let m_max = m_needed.min(m_cap).min(MODE_M_MAX as f64).max(1.0) as u32;

    // Resolved iff the φ' grid covers the input bandwidth. The test is `n_phi ≥ 2·B` — a
    // straight Nyquist line on `B` — and it is deliberately the *conservative* reading:
    // `gₘ(ρ) = jᵐ Jₘ(k·δ·ρ/f)` and `Jₘ(a)` decays super-exponentially for `m > a`, while `a`
    // reaches `B` only at the rim and is further damped by the illumination taper, so real
    // content dies out somewhat below `B`. Measured on the 34 m / δ=1.19 m coma geometry
    // (`B = 263`, Nyquist line 527): `n_phi = 512` already reproduces `n_phi = 4096` to
    // **0.000 dB** at every angle sampled, i.e. this flag would call a converged answer
    // unresolved. Erring that way is deliberate — a spurious `converged = false` costs a
    // warning, a spurious `true` costs up to +82 dB (the measured error of the old capped
    // `n_phi = 64` at this same geometry).
    let azimuthally_resolved = (n_phi as f64) >= 2.0 * bandwidth;

    ModeSizing {
        m_max,
        n_phi,
        azimuthally_resolved,
    }
}

/// Symmetric-aperture (no lateral feed offset) Hankel radial field.
///
/// For an azimuthally symmetric aperture the closed-form φ' integral (Jacobi–Anger)
/// collapses the 2D aperture integral to the 1D radial transform
/// ```text
/// I(θ) = 2π ∫₀^R exp(j·k·ρ²/(4f)·(1−cosθ)) · A(ρ) · exp(j·Ψ_ρonly) · J₀(kρ sinθ) · ρ dρ
/// ```
/// where `Ψ_ρonly` is the ρ-only (azimuthally symmetric) phase: axial-defocus (feed
/// z-offset + deliberate `axial_defocus`, folded in via the exact geometric
/// `phase_feed_displacement` with zero lateral offset) plus mesh phase. Evaluated by
/// composite Simpson's rule over ρ with `n_rho` (forced odd) points.
///
/// At θ=0: `sinθ=0 ⇒ J₀(0)=1` and the chirp vanishes, so the integral reduces to
/// `2π ∫ A(ρ)·exp(j·Ψ_ρonly)·ρ dρ` — identical to the 2D path on-axis.
fn hankel_radial_field(
    config: &AntennaConfiguration,
    theta: f64,
    _phi: f64,
    k: f64,
    n_rho: usize,
    deadline: Option<IntegrationDeadline>,
) -> ComputationResult<Complex64> {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);
    // Axial defocus (feed z-offset + deliberate axial_defocus) adds a ρ-only quadratic
    // phase that is azimuthally symmetric — fold it into the phase. Lateral offset is
    // excluded here by the caller (symmetric path only).
    let axial = config.feed.position.z - f + config.feed.axial_defocus;

    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let sin_theta = theta.sin();
    let one_minus_cos = 1.0 - theta.cos();

    let mut sum = Complex64::new(0.0, 0.0);
    for i in 0..n {
        // S3 cooperative budget: poll the wall-clock deadline at chunk boundaries only
        // (never per-sample — Instant::now() is too costly, and sample density is fixed).
        if i % BUDGET_CHECK_STRIDE == 0 {
            if let Some(dl) = deadline {
                dl.check("hankel_radial_field")?;
            }
        }
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        let amp = illumination_amplitude(rho, 0.0, &config.feed, f);
        // Dish-depth chirp (ρ-only, θ-dependent — the parabola's equiphase term).
        // NOTE: must stay in sync with phase_path's term1 in phase.rs — it is
        // duplicated from there because phase_path returns term1−term2 fused and
        // only term1 (this ρ²/(4f)·(1−cosθ) chirp) is wanted here.
        let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
        // Axial defocus: exact geometric ρ-only phase (φ'-independent when lateral=0).
        let defocus = if axial != 0.0 {
            crate::model::phase::phase_feed_displacement(rho, 0.0, 0.0, 0.0, axial, f, k)
        } else {
            0.0
        };
        // Mesh phase (ρ-only, via the surface incidence angle θ_inc ≈ ρ/(2f)).
        let mesh = if mesh_spacing > 0.0 {
            let theta_inc = rho / (2.0 * f);
            crate::model::phase::phase_mesh(mesh_spacing, theta_inc, k)
        } else {
            0.0
        };
        let j0 = bessel_j0(k * rho * sin_theta);
        let phase = chirp + defocus + mesh;
        sum += Complex64::new(0.0, phase).exp() * amp * j0 * rho * w;
    }
    Ok(sum * (h / 3.0) * 2.0 * PI)
}

/// Config-derived, ρ/φ'-independent constants for the aperture-plane function `g(ρ,φ')`.
///
/// Computed ONCE per [`azimuthal_mode_field_inner`] call and shared across every
/// `(ρ, φ')` evaluation (`n_rho · n_phi_coeff` of them). Hoisting these out of the hot
/// path removes the per-evaluation `radial_displacement()` (a `hypot`), `atan2`, and
/// axial-offset arithmetic — hundreds of thousands of transcendental calls per gain on
/// the large offset-feed dishes.
struct AperturePlaneConst<'a> {
    feed: &'a crate::model::geometry::FeedParameters,
    /// Focal length (m).
    f: f64,
    /// Lateral feed offset magnitude `δ` (m); coma driver.
    delta: f64,
    /// `cos α` / `sin α` for the azimuth of the lateral offset, `α = atan2(y, x)` (rad).
    /// Stored as the pair rather than the angle: `phase_feed_displacement` only ever wants
    /// the two, and it runs `n_ρ · n_φ` times per sweep (roadmap P10-perf).
    cos_alpha: f64,
    sin_alpha: f64,
    /// Axial phase-center offset from focus (m); defocus driver.
    axial: f64,
    /// Mesh wire spacing (m); `0.0` if no mesh.
    mesh_spacing: f64,
}

impl<'a> AperturePlaneConst<'a> {
    fn new(config: &'a AntennaConfiguration) -> Self {
        let f = config.reflector.focal_length;
        let alpha = config.feed.position.y.atan2(config.feed.position.x);
        Self {
            feed: &config.feed,
            f,
            delta: config.feed.position.radial_displacement(),
            cos_alpha: alpha.cos(),
            sin_alpha: alpha.sin(),
            axial: config.feed.position.z - f + config.feed.axial_defocus,
            mesh_spacing: config.mesh.as_ref().map_or(0.0, |m| m.spacing),
        }
    }

    /// The ρ-only part of the aperture-plane phase: the mesh term, which depends on ρ through
    /// the surface incidence angle `θ_inc ≈ ρ/(2f)` and not on φ' at all.
    ///
    /// Hoisted out of the φ' loop (roadmap P10-perf): it costs an `atan` and a `sin`, and
    /// computing it inside the loop repeated that work `n_φ − 1` times per radial sample for
    /// an identical result. Guarded on `spacing > 0.0` for consistency with `phase_total` and
    /// `hankel_radial_field` (a zero-spacing mesh would divide by zero in `phase_mesh`).
    #[inline]
    fn rho_only_phase(&self, rho: f64, k: f64) -> f64 {
        if self.mesh_spacing > 0.0 {
            let theta_inc = rho / (2.0 * self.f);
            crate::model::phase::phase_mesh(self.mesh_spacing, theta_inc, k)
        } else {
            0.0
        }
    }
}

/// Per-φ'-grid-index trigonometric table, built once per sweep and reused at every radial
/// sample (roadmap P10-perf).
///
/// `cos φ'`, `sin φ'` and `cos 2φ'` are needed by [`aperture_plane_g`] at each of the
/// `n_ρ · n_φ` evaluation points, but depend only on the grid index — the φ' grid is fixed for
/// the whole sweep. Tabling them turns `3 · n_ρ · n_φ` transcendental calls into `3 · n_φ`.
struct PhiGrid {
    cos_phi: Vec<f64>,
    sin_phi: Vec<f64>,
    cos_2phi: Vec<f64>,
}

impl PhiGrid {
    fn new(n_phi: usize) -> Self {
        let dphi = 2.0 * PI / n_phi as f64;
        let mut cos_phi = Vec::with_capacity(n_phi);
        let mut sin_phi = Vec::with_capacity(n_phi);
        let mut cos_2phi = Vec::with_capacity(n_phi);
        for j in 0..n_phi {
            let phi = j as f64 * dphi;
            cos_phi.push(phi.cos());
            sin_phi.push(phi.sin());
            cos_2phi.push((2.0 * phi).cos());
        }
        Self {
            cos_phi,
            sin_phi,
            cos_2phi,
        }
    }
}

/// θ-independent aperture-plane function
/// ```text
/// g(ρ,φ') = A(ρ,φ') · exp( j·[ Ψ_feed_displacement(ρ,φ') + Ψ_mesh(ρ) ] )
/// ```
/// i.e. the full aperture integrand phase MINUS the parabolic dish-depth chirp
/// `k·ρ²/(4f)·(1−cosθ)` and MINUS the Fourier kernel `−k·ρ·sinθ·cos(φ−φ')` (both added,
/// respectively folded, in the radial loop of [`azimuthal_mode_field_inner`]). Neither
/// the observation angle θ nor φ enters here — this is what makes the φ'-Fourier
/// coefficients `g_m(ρ)` reusable across all θ.
///
/// The guards mirror `aperture_integrand`/`phase_total` exactly (lateral coma + axial
/// defocus via the exact geometric `phase_feed_displacement`; mesh phase when a mesh with
/// positive spacing is present) so the mode integrator and the 2D reference agree wherever
/// both are valid. The config-derived constants arrive precomputed in [`AperturePlaneConst`],
/// the φ'-only trigonometry in [`PhiGrid`], and the ρ-only mesh phase as `rho_phase` — all
/// three hoists exist because this function is the integrator's floor cost, evaluated
/// `n_ρ · n_φ` times per sweep (roadmap P10-perf).
#[inline]
fn aperture_plane_g(
    c: &AperturePlaneConst,
    grid: &PhiGrid,
    j: usize,
    rho: f64,
    rho_phase: f64,
    k: f64,
) -> Complex64 {
    let cos_phi = grid.cos_phi[j];
    let sin_phi = grid.sin_phi[j];
    let amp = crate::model::illumination::illumination_amplitude_precomputed(
        rho,
        cos_phi,
        sin_phi,
        grid.cos_2phi[j],
        c.feed,
        c.f,
    );

    let mut phase = rho_phase;
    if c.delta > 0.0 || c.axial != 0.0 {
        phase += crate::model::phase::phase_feed_displacement_precomputed(
            rho,
            cos_phi,
            sin_phi,
            c.delta,
            c.cos_alpha,
            c.sin_alpha,
            c.axial,
            c.f,
            k,
        );
    }
    // `exp(j·phase)` for a purely imaginary argument. `Complex64::exp` would compute
    // `exp(0.0) · (cos, sin)` — the same two transcendentals plus an `exp` whose result is
    // exactly 1.0, so this is bit-identical and one call cheaper.
    Complex64::new(phase.cos(), phase.sin()) * amp
}

/// `(−j)^m` for integer `m` (which may be negative): `(−j)^m = exp(−j·m·π/2)`.
#[inline]
fn pow_neg_j(m: i32) -> Complex64 {
    Complex64::new(0.0, -(m as f64) * std::f64::consts::FRAC_PI_2).exp()
}

/// Azimuthal-mode-expansion aperture field for an asymmetric (coma / azimuthally
/// dependent) aperture:
/// ```text
/// I(θ,φ) = 2π · Σ_{m=−M}^{M} (−j)^m e^{jmφ} · R_m(θ)
/// R_m(θ)  = ∫₀^R exp(j·k·ρ²/(4f)·(1−cosθ)) · g_m(ρ) · J_m(kρ sinθ) · ρ dρ
/// g_m(ρ)  = (1/2π) ∫₀^{2π} g(ρ,φ') e^{−jmφ'} dφ'          (θ-independent)
/// ```
/// The negative modes reuse `g_{-m}(ρ)` (the `e^{+jmφ'}` coefficient) and the identity
/// `J_{-m}(a) = (−1)^m J_m(a)`, with `(−j)^{-m} = e^{+jmπ/2}`. For a real, `+x`-offset
/// feed the sum is real-symmetric (`g_{-m} = conj(g_m)`); the code does NOT assume that
/// — the served Ka-band feeds are offset along `+y`.
///
/// Radial quadrature is composite Simpson over ρ (`n_rho` forced odd); each `g_m(ρ)` is
/// a uniform-grid DFT over φ' with `n_phi_coeff` samples (trapezoid == rectangle on a
/// periodic grid), computed once per ρ and shared across modes. `J_m` is evaluated with
/// the in-house `bessel_jn`, accurate at every argument magnitude reached here.
///
/// The symmetric aperture is exactly the `M = 0` special case (only `g_0` survives,
/// `J_0`), reproducing [`hankel_radial_field`].
///
/// Thin wrapper returning only the full mode sum; see [`azimuthal_mode_field_inner`] for
/// the variant that also returns the top-mode contribution for the convergence self-check.
#[cfg(test)]
fn azimuthal_mode_field(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_max: u32,
) -> Complex64 {
    // No deadline in the test oracle path (None never errors); unwrap is test-only.
    azimuthal_mode_field_inner(config, theta, phi, k, n_rho, n_phi_coeff, m_max, None)
        .expect("azimuthal_mode_field_inner with no time budget cannot error")
        .total
}

/// Azimuthal-mode field, returning a [`ModeSweep`] — all three observables the two runtime
/// self-checks need, from a SINGLE φ' sweep:
/// - `total` = `I(θ,φ)` summed over all modes `0..=m_max` (both `±m`).
/// - `top_mode` = the part of `total` contributed by the top mode `±m_max`, so the caller has
///   both `I(M+1)` (`total`, calling with `m_max = M+1`) and `I(M) = total − top_mode` without
///   a second integration — the M-vs-(M+1) azimuthal truncation check (D-6).
/// - `radial_probe` = the part of `total` contributed by [`RADIAL_PROBE_MODES`], the `N`-density
///   half of P12's cheap radial pre-gate, likewise free here.
///
/// See [`azimuthal_mode_field`].
#[allow(clippy::too_many_arguments)]
fn azimuthal_mode_field_inner(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_max: u32,
    deadline: Option<IntegrationDeadline>,
) -> ComputationResult<ModeSweep> {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    // Config-derived constants for g(ρ,φ'), computed once (hoisted out of the hot loop).
    let apc = AperturePlaneConst::new(config);
    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let dphi = 2.0 * PI / n_phi_coeff as f64;
    let sin_theta = theta.sin();
    let one_minus_cos = 1.0 - theta.cos();
    let mmax = m_max as usize;

    // P10-perf: the φ' coefficients come from an FFT, not a direct DFT. `gₘ(ρ)` for every
    // wanted `m` is one length-`n_phi_coeff` transform of the `g(ρ,φ'_j)` samples — O(n log n)
    // instead of the O(n_phi·M) inner loop this replaced, which was the integrator's dominant
    // term on steered and wide-spectrum geometries (n_phi=536, M=254 ⇒ ~137 000 complex
    // multiply-accumulates *per radial sample*).
    //
    // Index mapping, and it is the one thing here that must not be gotten backwards:
    //   G[k] = Σ_j g_j e^{−2πi jk/n}        (the forward transform)
    //   gm_pos[m] = Σ_j g_j e^{−jmφ'_j}/n  = G[m mod n] / n
    //   gm_neg[m] = Σ_j g_j e^{+jmφ'_j}/n  = G[(n − m mod n) mod n] / n
    // The `mod n` is not defensive padding: the direct DFT this replaces is exactly periodic
    // in `m` with period `n`, so an `m ≥ n` (reachable only from tests, which drive `m_max`
    // directly) aliases identically under both implementations.
    let plan = crate::model::fft::FftPlan::new(n_phi_coeff);
    let phi_grid = PhiGrid::new(n_phi_coeff);
    let mut g_samples = vec![Complex64::new(0.0, 0.0); n_phi_coeff];
    let mut fft_scratch = vec![Complex64::new(0.0, 0.0); n_phi_coeff];

    // Radial accumulators for R_{+m} and R_{-m} (m = 0..=m_max); Simpson scale applied
    // once at the end. r_neg[0] is unused (m=0 has no distinct negative counterpart).
    let mut r_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
    let mut r_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];

    // Per-ρ Fourier-coefficient buffers, reused each radial step to avoid reallocation.
    let mut gm_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
    let mut gm_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];

    // P10-perf: every order `J_0(a) … J_{m_max}(a)` from ONE recurrence sweep per radial
    // sample, instead of a fresh recurrence per order (which made the Bessel work O(M²)).
    let mut jm = vec![0.0_f64; mmax + 1];

    for i in 0..n {
        // S3 cooperative budget: poll the wall-clock deadline at chunk boundaries only
        // (never per-sample — Instant::now() is too costly, and sample density is fixed).
        // This is the hot loop for offset-feed Ka (the ~3.3 s worst case), so it is exactly
        // where a runaway single integration must be able to stop itself.
        if i % BUDGET_CHECK_STRIDE == 0 {
            if let Some(dl) = deadline {
                dl.check("azimuthal_mode_field")?;
            }
        }
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        // Dish-depth chirp (ρ-only, θ-dependent — the parabola's equiphase term).
        // NOTE: must stay in sync with phase_path's term1 in phase.rs — it is
        // duplicated from there because phase_path returns term1−term2 fused and
        // only term1 (this ρ²/(4f)·(1−cosθ) chirp) is wanted here.
        let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
        let chirp_factor = Complex64::new(0.0, chirp).exp();
        let a = k * rho * sin_theta;

        // g_m(ρ): one φ' sweep to sample g, one FFT to get every mode at once.
        let rho_phase = apc.rho_only_phase(rho, k);
        for (jj, slot) in g_samples.iter_mut().enumerate() {
            *slot = aperture_plane_g(&apc, &phi_grid, jj, rho, rho_phase, k);
        }
        plan.forward(&mut g_samples, &mut fft_scratch);
        // `dphi/(2π) = 1/n_phi_coeff` — the same normalization the direct DFT applied.
        let norm = dphi / (2.0 * PI);
        for m in 0..=mmax {
            let mm = m % n_phi_coeff;
            gm_pos[m] = g_samples[mm] * norm;
            gm_neg[m] = g_samples[(n_phi_coeff - mm) % n_phi_coeff] * norm;
        }

        // Radial integrand contribution for each mode. J_m(a) for every m in one sweep;
        // J_{-m} = (−1)^m J_m.
        bessel_jn_array(m_max, a, &mut jm);
        for (m, (rp, rn)) in r_pos.iter_mut().zip(r_neg.iter_mut()).enumerate() {
            let base = chirp_factor * jm[m] * rho * w;
            *rp += base * gm_pos[m];
            if m > 0 {
                let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                *rn += base * gm_neg[m] * sign;
            }
        }
    }

    let scale = h / 3.0;
    // I(θ,φ) = 2π Σ_{m=−M}^{M} (−j)^m e^{jmφ} R_m(θ). Track the top mode's (±m_max)
    // contribution separately so the caller can form I(M) = total − top for the D-6
    // self-check without a second sweep, and the low-mode partial sum separately so the
    // P12 radial pre-gate has its N-density observable without a second sweep either.
    let mut acc = r_pos[0] * scale; // m = 0: (−j)^0 = 1, e^0 = 1
    let mut top = Complex64::new(0.0, 0.0);
    let mut probe = if RADIAL_PROBE_MODES.contains(&0) {
        acc
    } else {
        Complex64::new(0.0, 0.0)
    };
    for m in 1..=mmax {
        let mf = m as f64;
        let epos = Complex64::new(0.0, mf * phi).exp();
        let eneg = Complex64::new(0.0, -mf * phi).exp();
        let contrib = pow_neg_j(m as i32) * epos * r_pos[m] * scale
            + pow_neg_j(-(m as i32)) * eneg * r_neg[m] * scale;
        acc += contrib;
        if RADIAL_PROBE_MODES.contains(&(m as u32)) {
            probe += contrib;
        }
        if m == mmax {
            top = contrib;
        }
    }
    Ok(ModeSweep {
        total: acc * 2.0 * PI,
        top_mode: top * 2.0 * PI,
        radial_probe: probe * 2.0 * PI,
    })
}

/// The `Σ_{m ∈ RADIAL_PROBE_MODES}` partial sum on its own, at an arbitrary radial density.
///
/// This is the **cheap leg** of P12's radial pre-gate: it repeats only the low modes at `2N`
/// so their movement can be compared against the same modes' movement inside the `N` sweep
/// ([`ModeSweep::radial_probe`]).
///
/// It is NOT proportionally cheap. The φ' sweep must still evaluate [`aperture_plane_g`] at
/// every `(ρ, φ')` point regardless of how few modes are wanted, so this costs a floor of
/// ~18% of a full sweep at `m_max ≈ 195` (`dsn_34m` Ka) but ~52–62% at `m_max ≈ 12–20`. That
/// is precisely why [`use_radial_pre_gate`] only reaches for it on geometries where a full
/// check leg is expensive.
///
/// `m_probe` is the full sweep's top mode. It is **not** used to sum more modes — only
/// [`RADIAL_PROBE_MODES`] are accumulated — but it must be passed so the `Jₘ` ladder is built
/// with the same branch decision [`azimuthal_mode_field_inner`] makes. [`bessel_jn_array`]
/// selects upward or downward recurrence from the *highest* wanted order, and those two
/// directions differ by ~2.8e-9 near the origin (the rational approximations' error at `x=0`
/// enters the upward seeds). Sizing the ladder differently here would make the pre-gate
/// difference two subtly different functions, which is exactly what
/// `radial_probe_field_matches_the_full_sweeps_probe_accumulation` exists to forbid.
#[allow(clippy::too_many_arguments)]
fn radial_probe_field(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_probe: u32,
    deadline: Option<IntegrationDeadline>,
) -> ComputationResult<Complex64> {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    let apc = AperturePlaneConst::new(config);
    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let dphi = 2.0 * PI / n_phi_coeff as f64;
    let sin_theta = theta.sin();
    let one_minus_cos = 1.0 - theta.cos();

    const P: usize = RADIAL_PROBE_MODES.len();
    let mut r_pos = [Complex64::new(0.0, 0.0); P];
    let mut r_neg = [Complex64::new(0.0, 0.0); P];

    // φ' twiddles e^{−jmφ'_j} for the handful of probe modes, hoisted out of the radial loop
    // (P10-perf). Only `P` modes are wanted, so a full FFT would be wasted work here — but
    // recomputing the exponential per `(ρ, φ', mode)`, which this used to do, was `n_rho·n_phi·P`
    // transcendental calls on the leg whose entire purpose is to be the cheap one.
    // Flat layout: twiddle[idx * n_phi_coeff + j].
    let mut twiddle = vec![Complex64::new(0.0, 0.0); P * n_phi_coeff];
    for (idx, chunk) in twiddle.chunks_mut(n_phi_coeff).enumerate() {
        let m = RADIAL_PROBE_MODES[idx] as f64;
        for (jj, t) in chunk.iter_mut().enumerate() {
            *t = Complex64::new(0.0, -m * jj as f64 * dphi).exp();
        }
    }
    let mut jm_ladder = vec![0.0_f64; m_probe as usize + 1];
    let phi_grid = PhiGrid::new(n_phi_coeff);

    for i in 0..n {
        if i % BUDGET_CHECK_STRIDE == 0 {
            if let Some(dl) = deadline {
                dl.check("radial_probe_field")?;
            }
        }
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
        let chirp_factor = Complex64::new(0.0, chirp).exp();
        let a = k * rho * sin_theta;

        let mut gm_pos = [Complex64::new(0.0, 0.0); P];
        let mut gm_neg = [Complex64::new(0.0, 0.0); P];
        let rho_phase = apc.rho_only_phase(rho, k);
        for jj in 0..n_phi_coeff {
            let g = aperture_plane_g(&apc, &phi_grid, jj, rho, rho_phase, k);
            for idx in 0..P {
                let t = twiddle[idx * n_phi_coeff + jj]; // e^{−jmφ'_j}
                gm_pos[idx] += g * t;
                gm_neg[idx] += g * t.conj();
            }
        }
        let norm = dphi / (2.0 * PI);
        for idx in 0..P {
            gm_pos[idx] *= norm;
            gm_neg[idx] *= norm;
        }

        // Same ladder, same branch decision as the full sweep — see the `m_probe` note above.
        bessel_jn_array(m_probe, a, &mut jm_ladder);
        for (idx, &m) in RADIAL_PROBE_MODES.iter().enumerate() {
            let base = chirp_factor * jm_ladder[m as usize] * rho * w;
            r_pos[idx] += base * gm_pos[idx];
            if m > 0 {
                let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                r_neg[idx] += base * gm_neg[idx] * sign;
            }
        }
    }

    let scale = h / 3.0;
    let mut acc = Complex64::new(0.0, 0.0);
    for (idx, &m) in RADIAL_PROBE_MODES.iter().enumerate() {
        if m == 0 {
            acc += r_pos[idx] * scale;
        } else {
            let mf = m as f64;
            let epos = Complex64::new(0.0, mf * phi).exp();
            let eneg = Complex64::new(0.0, -mf * phi).exp();
            acc += pow_neg_j(m as i32) * epos * r_pos[idx] * scale
                + pow_neg_j(-(m as i32)) * eneg * r_neg[idx] * scale;
        }
    }
    Ok(acc * 2.0 * PI)
}

/// Simpson's rule weight for index i in array of n points
///
/// Returns:
/// - 1 for first and last points
/// - 4 for odd interior indices
/// - 2 for even interior indices
#[inline]
fn simpson_weight(i: usize, n: usize) -> f64 {
    if i == 0 || i == n - 1 {
        1.0
    } else if i % 2 == 1 {
        4.0
    } else {
        2.0
    }
}

/// Aperture integrand function
///
/// Computes the integrand at a single aperture point (ρ, φ') for observation
/// direction (θ, φ).
///
/// # Formula
/// ```text
/// Integrand = A(ρ,φ') · exp[j·Ψ(ρ,φ')]
/// ```
///
/// where:
/// - A(ρ,φ') is the illumination amplitude from the feed
/// - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
///
/// # Arguments
/// - `rho`: Radial coordinate in aperture (meters)
/// - `phi_prime`: Azimuthal coordinate in aperture (radians)
/// - `theta`: Observation polar angle (radians)
/// - `phi`: Observation azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `k`: Wavenumber (rad/m)
/// - `wavelength`: Wavelength (meters)
///
/// # Returns
/// Complex integrand value
///
/// Test-only since P10 Task 2: it is the single-point integrand of the retained 2D
/// reference (`integrate_2d_simpson`), which no longer runs in production.
#[cfg(test)]
#[inline]
fn aperture_integrand(
    rho: f64,
    phi_prime: f64,
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    _wavelength: f64,
) -> Complex64 {
    // Calculate illumination amplitude
    let amplitude =
        illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);

    // Create aperture coordinates
    let aperture = ApertureCoordinates { rho, phi_prime };

    // Calculate feed displacement from position.
    // Lateral (xy-plane) displacement drives coma; axial (z) offset drives defocus.
    let feed_displacement = config.feed.position.radial_displacement();
    let feed_displacement_angle = config.feed.position.y.atan2(config.feed.position.x);
    // Axial offset of the feed's PHASE CENTER from the focal point: physical
    // z-offset plus any DELIBERATE defocus (positive = away from the vertex,
    // matching phase_feed_displacement's delta_z). The feed's own
    // phase_center_offset is assumed compensated by per-band feed positioning
    // (auto-refocus, roadmap P7 decided 2026-07-10) and does not contribute.
    let feed_axial_offset =
        config.feed.position.z - config.reflector.focal_length + config.feed.axial_defocus;

    // Calculate angle of incidence (simplified - assumes small angles)
    // For parabolic reflector, theta_incident ≈ ρ/(2f)
    let theta_incident = rho / (2.0 * config.reflector.focal_length);

    // Get mesh spacing (0.0 if no mesh)
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);

    // Surface error at this point (ρ, φ')
    //
    // FUTURE ENHANCEMENT: Spatially-varying surface error model
    // Currently uses ideal surface (surface_error = 0.0) for all aperture points.
    // The Ruze efficiency factor in pattern.rs handles the statistical effect
    // of surface RMS on overall gain, which is sufficient for most applications.
    //
    // For higher fidelity modeling of specific antennas with measured surface maps:
    // - Option 1: Zernike polynomial expansion of measured surface
    // - Option 2: Interpolate from measured surface map (x, y, z points)
    // - Option 3: Use correction surface from calibration (already implemented)
    //
    // Rationale for current approach:
    // - Calibration correction surface (B-spline) captures measured deviations
    // - Ruze statistical model is accurate for random surface errors
    // - Explicit surface modeling adds complexity with marginal accuracy gain
    let surface_error = 0.0;

    // Calculate total phase
    let total_phase = phase_total(
        aperture,
        theta,
        phi,
        config.reflector.focal_length,
        feed_displacement,
        feed_displacement_angle,
        feed_axial_offset,
        surface_error,
        theta_incident,
        mesh_spacing,
        k,
    );

    // Combine: A(ρ,φ') · exp(j·Ψ)
    let phase_factor = Complex64::new(0.0, total_phase).exp();

    amplitude * phase_factor
}

/// ∬ |A(ρ,φ')|² ρ dρ dφ' over the aperture — denominator of the aperture-directivity
/// formula. Uses the same illumination model and Simpson scheme as the field integral.
///
/// The directivity of an aperture is
/// ```text
/// D(θ,φ) = (4π/λ²) · |∬ A e^{jΨ} ρ dρ dφ'|² / ∬ |A|² ρ dρ dφ'
/// ```
/// This function computes the (real, phase-free) denominator. The numerator is the
/// raw aperture integral from [`integrate_aperture`] (i.e. `IntegrationResult::field`),
/// NOT the normalized [`compute_far_field`] value.
pub fn integrate_amplitude_squared(
    config: &AntennaConfiguration,
    n_rho: usize,
    n_phi: usize,
) -> f64 {
    let rho_max = config.reflector.diameter / 2.0;

    // Ensure odd number of points for Simpson's rule.
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = rho_max / (n_rho - 1) as f64;
    let h_phi = 2.0 * PI / (n_phi - 1) as f64;

    let mut sum = 0.0;
    for j in 0..n_phi {
        let phi_prime = j as f64 * h_phi;
        let wj = simpson_weight(j, n_phi);
        let mut inner = 0.0;
        for i in 0..n_rho {
            let rho = i as f64 * h_rho;
            let a =
                illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);
            inner += a * a * rho * simpson_weight(i, n_rho);
        }
        sum += inner * wj;
    }

    sum * h_rho * h_phi / 9.0
}

/// Compute far-field normalization factor
///
/// The complete far-field formula includes a normalization factor:
/// ```text
/// E(θ,φ) = (jk·exp(-jkr))/(2λr) × [aperture integral]
/// ```
///
/// This function computes the normalization factor, excluding the r-dependent
/// terms which are typically omitted in pattern calculations (relative patterns).
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Complex normalization factor (jk)/(2λ)
pub fn far_field_normalization(wavelength: f64) -> Complex64 {
    let k = wavenumber(wavelength);

    // (jk) / (2λ) = (j * 2π/λ) / (2λ) = jπ/λ²
    Complex64::new(0.0, 1.0) * k / (2.0 * wavelength)
}

/// Compute normalized far-field electric field
///
/// Combines aperture integration with normalization factor to produce
/// the complete far-field electric field (excluding r-dependent terms).
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `params`: Integration parameters
///
/// # Returns
/// Complex electric field value (normalized, excluding 1/r factor)
pub fn compute_far_field(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<Complex64> {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let integration_result = integrate_aperture(theta, phi, config, frequency_hz, params)?;

    let normalization = far_field_normalization(wavelength);

    Ok(normalization * integration_result.field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{FeedParameters, FeedPosition, MeshParameters, ReflectorGeometry};

    /// Create a simple test antenna configuration
    fn test_antenna() -> AntennaConfiguration {
        use crate::model::geometry::MeshPattern;

        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(); // 1m diameter, f/D=0.5, ideal surface
        let feed_pos = FeedPosition::at_focus(0.5);
        let feed = FeedParameters::new(feed_pos, 8.0, 0.0, 1.0).unwrap(); // q=8, no offset, symmetric
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap(); // 5mm spacing, 0.5mm wire

        AntennaConfiguration::new(
            "test_antenna".to_string(),
            "Test Antenna".to_string(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    /// Large synthetic dish (10 m, f/D=0.5, feed at focus, broad q≈2 feed, no mesh):
    /// D/λ ≈ 280 at 8.4 GHz, so the 2D quadrature ALIASES off-axis (returns a
    /// spuriously high, roughly flat value) while the exact 1D Hankel transform must
    /// fall monotonically well below boresight.
    fn large_test_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(10.0, 5.0, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(5.0), 2.0, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("large".into(), "Large".into(), reflector, feed, None).unwrap()
    }

    /// Small dish (`D/λ ≈ 104` at X-band) with a lateral feed offset (coma). The 2D
    /// quadrature is trustworthy at this size, so it is the near-in ground truth against
    /// which the azimuthal-mode expansion is validated. `lateral = 0.05 m` matches the
    /// served `gs_3.7m` x-band feed; `q = 2` gives a broad illumination (heavy edge
    /// content, a stress test for the mode truncation).
    fn offset_feed_test_antenna(diameter: f64, focal: f64, lateral: f64) -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(diameter, focal, 0.0).unwrap();
        let mut pos = FeedPosition::at_focus(focal);
        pos.x = lateral; // lateral offset in +x → breaks azimuthal symmetry (coma)
        let feed = FeedParameters::new(pos, 2.0, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("off".into(), "Off".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn azimuthal_modes_match_2d_small_dish_with_offset() {
        // 3.7 m dish, X-band, lateral feed offset 0.05 m (served gs_3.7m x-band feed).
        // The 2D quadrature is ground truth near-in here (D/λ ~ 104, small), so the mode
        // expansion must match it AND reproduce coma asymmetry (|E(φ=0)| ≠ |E(φ=π)|).
        let config = offset_feed_test_antenna(3.7, 1.85, 0.05);
        let f = 8.4e9;
        let mut hi = IntegrationParams::high_accuracy();
        hi.min_rho_points = 512;
        hi.max_rho_points = 512;
        hi.min_phi_points = 1024;
        hi.max_phi_points = 1024;
        hi.max_iterations = 1;
        let k = wavenumber(wavelength_from_frequency(f));

        for deg in [0.0_f64, 1.0, 5.0, 20.0] {
            let th = deg.to_radians();
            let ref_field = integrate_2d_simpson_public_shim(th, 0.0, &config, f, &hi);
            let mode_field = azimuthal_mode_field(&config, th, 0.0, k, 4097, 128, 48);
            let d_db = 20.0 * (mode_field.norm() / ref_field.norm()).log10();
            assert!(d_db.abs() < 0.1, "θ={deg}°: mode vs 2D Δ={d_db:.4} dB");
        }

        // Coma asymmetry: off-axis in the +x plane (φ=0) vs the −x plane (φ=π) must differ.
        let th = 3.0_f64.to_radians();
        let plus = azimuthal_mode_field(&config, th, 0.0, k, 4097, 128, 48).norm();
        let minus = azimuthal_mode_field(&config, th, PI, k, 4097, 128, 48).norm();
        assert!(
            (plus - minus).abs() / plus.max(minus) > 1e-3,
            "coma asymmetry absent: |E(φ=0)|={plus}, |E(φ=π)|={minus}"
        );
    }

    /// S3: an already-expired wall-clock budget must abort a single integration with the
    /// typed `TimeBudgetExceeded` error rather than run to completion. A 1 ns budget is
    /// expired at the first radial chunk boundary, so this is deterministic and instant.
    /// Uses the offset-feed (coma → azimuthal-mode) path — the expensive worst case S3
    /// exists to bound — at a wide angle.
    #[test]
    fn time_budget_exceeded_aborts_single_integration() {
        let config = offset_feed_test_antenna(3.7, 1.85, 0.05);
        let f = 8.4e9;
        let mut params = IntegrationParams::adaptive();
        params.time_budget = Some(Duration::from_nanos(1));
        let theta = 45.0_f64.to_radians();

        let result = integrate_aperture(theta, 0.0, &config, f, &params);
        match result {
            Err(ComputationError::TimeBudgetExceeded {
                operation,
                budget_ms,
                ..
            }) => {
                assert_eq!(budget_ms, 0, "a 1 ns budget rounds to 0 ms");
                assert!(
                    operation.contains("azimuthal_mode") || operation.contains("hankel"),
                    "operation should name the aborting integrator, got {operation}"
                );
            }
            other => panic!("expected TimeBudgetExceeded, got {other:?}"),
        }
    }

    /// S3: the generous default budget must never trip on a normal evaluation, and the
    /// returned field must be byte-identical to a `time_budget: None` build (the check is a
    /// pure side-effect when the deadline is not hit).
    #[test]
    fn time_budget_default_is_transparent() {
        let config = offset_feed_test_antenna(3.7, 1.85, 0.05);
        let f = 8.4e9;
        let theta = 5.0_f64.to_radians();

        let with_default =
            integrate_aperture(theta, 0.0, &config, f, &IntegrationParams::adaptive())
                .expect("default budget must not trip");

        let mut no_budget = IntegrationParams::adaptive();
        no_budget.time_budget = None;
        let without = integrate_aperture(theta, 0.0, &config, f, &no_budget)
            .expect("no-budget path must succeed");

        assert_eq!(
            with_default.field, without.field,
            "budget check must not perturb the result when the deadline is not hit"
        );
    }

    /// The azimuthal-mode integrator must reproduce the symmetric Hankel path when the
    /// aperture is symmetric (m=0-only special case) — a consistency self-check that the ±m
    /// assembly and normalisation are correct.
    ///
    /// # Why the tolerance is 1e-8 and not machine precision
    ///
    /// The two paths obtain `J₀` from different routines, and have since P10-perf: the Hankel
    /// path calls [`bessel_j0`] directly (it needs one order), while the mode path takes `J₀`
    /// from `bessel_jn_array`'s ladder. Those disagree by **2.83e-9** near the origin, because
    /// `bessel_j0`'s Numerical Recipes rational approximation evaluates to `1 + 2.83e-9` at
    /// `x = 0` where the ladder's normalized recurrence gives exactly 1.
    ///
    /// The disagreement is therefore the *rational approximation's* error, not the mode
    /// assembly's, and it is bounded by it: 2.83e-9 in field amplitude is 2.5e-8 dB. The
    /// property this test exists to protect — that the ±m assembly, the `(−j)^m` factors and
    /// the `2π` normalisation reconstruct the symmetric case — is unaffected by a scale error
    /// six orders below the tolerance, and would still fail loudly at 1e-8 if any of them were
    /// wrong (they are wrong by O(1) factors when they are wrong at all).
    #[test]
    fn azimuthal_modes_reduce_to_hankel_when_symmetric() {
        let config = large_test_antenna(); // symmetric: feed at focus, asymmetry_factor=1
        let f = 8.4e9;
        let k = wavenumber(wavelength_from_frequency(f));
        for deg in [0.0_f64, 1.0, 5.0, 20.0, 90.0] {
            let th = deg.to_radians();
            let hankel = hankel_radial_field(&config, th, 0.0, k, 4097, None).unwrap();
            let modes = azimuthal_mode_field(&config, th, 0.0, k, 4097, 64, 4);
            let rel = (hankel - modes).norm() / hankel.norm().max(1e-30);
            assert!(rel < 1e-8, "θ={deg}°: Hankel vs modes rel diff {rel:.2e}");
        }
    }

    #[test]
    fn hankel_symmetric_is_physical_off_axis() {
        // Large dish (D/λ ~ 280): 2D fast() aliases to a high, flat value off-axis;
        // the Hankel form must fall monotonically and stay well below boresight.
        let config = large_test_antenna(); // 10 m dish, feed at focus (symmetric)
        let f = 8.4e9;
        let g = |deg: f64| {
            let th = deg.to_radians();
            let r = integrate_aperture(th, 0.0, &config, f, &IntegrationParams::default()).unwrap();
            r.field.norm_sqr()
        };
        let g0 = g(0.0);
        // Off-axis power must be far below boresight and must DECREASE with angle
        // (the aliasing signature is a roughly flat high value — this rejects it).
        assert!(g(5.0) < g0 * 1e-2, "5deg not far below boresight");
        assert!(g(20.0) < g(5.0), "pattern must fall from 5deg to 20deg");
        assert!(g(90.0) < g(20.0), "pattern must fall from 20deg to 90deg");
    }

    /// A gbt_100m-like dish (`D=100 m`, `f=60 m`, symmetric feed) at Q-band. Adaptive
    /// density: `radial_points_for` at θ=90° must be O(10⁴) (≈ `4·D/λ`), NOT the O(10⁸)
    /// a fixed 2D grid at true Nyquist would imply, and the gain eval there must pass its
    /// N-vs-2N self-check (converged=true).
    fn gbt_like_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(100.0, 60.0, 0.000_275).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(60.0), 3.15, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("gbt".into(), "GBT".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn radial_points_for_gbt_qband_is_tens_of_thousands() {
        let config = gbt_like_antenna();
        let f_hz = 43.0e9; // Q-band
        let wl = wavelength_from_frequency(f_hz);
        let p = IntegrationParams::default();
        let n = radial_points_for(&config, PI / 2.0, wl, &p);
        println!("radial_points_for(gbt_100m q-band {f_hz:.0} Hz, θ=90°) = {n}");
        assert!(
            (10_000..200_000).contains(&n),
            "expected O(10^4) radial points, got {n}"
        );
        // A full gain eval at θ=90° must pass the N-vs-2N self-check.
        let r = integrate_aperture(PI / 2.0, 0.0, &config, f_hz, &p).unwrap();
        println!(
            "gbt q-band θ=90°: |field|={:.4e} converged={} err={:.3e} evals={}",
            r.field.norm(),
            r.converged,
            r.error_estimate,
            r.num_evaluations
        );
        assert!(
            r.converged,
            "gbt q-band θ=90° must converge (err={:.3e})",
            r.error_estimate
        );
    }

    #[test]
    fn radial_density_scales_with_dlambda_sintheta() {
        let small = test_antenna(); // 1 m
        let large = large_test_antenna(); // 10 m
        let wl = wavelength_from_frequency(8.4e9);
        let p = IntegrationParams::default();
        // θ=0 → floor (chirp & J_m kernel vanish; no oversampling), forced odd.
        let n0 = radial_points_for(&small, 0.0, wl, &p);
        assert_eq!(n0, p.min_rho_points | 1, "θ=0 must drop to the odd floor");
        // θ=90° → count ∝ D/λ, so the 10× larger dish needs ~10× the points.
        let ns = radial_points_for(&small, PI / 2.0, wl, &p);
        let nl = radial_points_for(&large, PI / 2.0, wl, &p);
        assert!(
            nl > ns * 5,
            "θ=90° density must scale with D/λ: large={nl} small={ns}"
        );
        assert!(ns % 2 == 1 && nl % 2 == 1, "counts must be odd for Simpson");
    }

    /// A dsn_34m-like dish with the served X-band lateral feed offset (`δ = 0.15 m`,
    /// `k·δ·(R/f) ≈ 33 rad`). The adaptive mode count must resolve the wide coma spectrum:
    /// the M-vs-(M+1) self-check must report converged=true at every angle, and the
    /// pattern must be physical off-axis (far below boresight, no rise with θ — the
    /// aliasing signature).
    fn dsn34m_like_xband() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.000_25).unwrap();
        let mut pos = FeedPosition::at_focus(13.6);
        pos.x = 0.15; // served x_band lateral offset
        let feed = FeedParameters::new(pos, 1.14, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("dsn".into(), "DSN".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn dsn34m_offset_feed_mode_count_converges() {
        let config = dsn34m_like_xband();
        let f_hz = 8.4e9;
        let wl = wavelength_from_frequency(f_hz);
        for deg in [0.0_f64, 1.0, 5.0, 20.0, 90.0] {
            let ModeSizing {
                m_max: m, n_phi, ..
            } = mode_count_for(&config, wl, deg.to_radians());
            println!("dsn_34m x-band θ={deg:>4}°: adaptive M={m} n_phi={n_phi}");
        }

        let p = IntegrationParams::default();
        let g = |deg: f64| integrate_aperture(deg.to_radians(), 0.0, &config, f_hz, &p).unwrap();
        let r0 = g(0.0);
        let g0 = r0.field.norm_sqr();
        let mut prev = f64::INFINITY;
        for deg in [1.0_f64, 5.0, 20.0, 90.0] {
            let r = g(deg);
            let power = r.field.norm_sqr();
            println!(
                "dsn_34m x-band θ={deg:>4}°: rel_power={:.3e} converged={} err={:.3e}",
                power / g0,
                r.converged,
                r.error_estimate
            );
            assert!(
                r.converged,
                "dsn_34m θ={deg}° mode count must converge (M vs M+1)"
            );
            // Physical: every off-axis angle far below boresight and not rising with θ.
            assert!(
                power < g0 * 1e-2,
                "dsn_34m θ={deg}° not far below boresight"
            );
            assert!(
                power <= prev * 1.5,
                "dsn_34m θ={deg}° pattern rose with θ (aliasing signature)"
            );
            prev = power;
        }
    }

    #[test]
    fn unconverged_is_flagged_not_silently_returned() {
        // A dish whose radial Nyquist rate (2·D/λ) EXCEEDS 2× the safety cap at θ=90°:
        // the adaptive count clamps to RADIAL_POINTS_SAFETY_MAX (below Nyquist → aliased),
        // while the self-check's 2N leg samples above Nyquist. They disagree → the result
        // MUST be flagged non-converged, never silently returned. D/λ = 100000 (750 m dish
        // at 40 GHz) ⇒ Nyquist = 2·10⁵ ≫ cap (6.5·10⁴), 2N leg = 1.31·10⁵ still < Nyquist
        // is NOT enough — so use a size where 2N clears Nyquist too. Here 2N ≈ 1.31·10⁵ and
        // Nyquist = 2·10⁵: the coarse leg is badly aliased and the fine leg less so, giving
        // a large, honest disagreement.
        let reflector = ReflectorGeometry::new(750.0, 375.0, 0.0).unwrap(); // f/D = 0.5
        let feed = FeedParameters::new(FeedPosition::at_focus(375.0), 2.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("huge".into(), "Huge".into(), reflector, feed, None).unwrap();
        let f_hz = 40.0e9; // λ = 0.0075 m → D/λ = 100000
        let r = integrate_aperture(PI / 2.0, 0.0, &config, f_hz, &IntegrationParams::default())
            .unwrap();
        println!(
            "huge dish θ=90°: converged={} err={:.3e} evals={}",
            r.converged, r.error_estimate, r.num_evaluations
        );
        assert!(
            !r.converged,
            "must flag non-convergence when density is capped below Nyquist"
        );
        // Even when flagged, the error estimate stays a finite, non-negative number.
        assert!(r.error_estimate.is_finite() && r.error_estimate >= 0.0);
    }

    #[test]
    fn asymmetric_amplitude_feed_bypasses_symmetric_hankel_path() {
        // A centered feed (no lateral offset) with a non-unity asymmetry_factor has an
        // azimuthally-dependent (elliptical) illumination, so it must NOT take the J₀
        // Hankel path — that path hardcodes phi_prime=0 and ignores observation φ.
        // Proof: the retained 2D path yields an observation-φ-dependent field, whereas
        // the Hankel path would return the identical value for every φ.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.5).unwrap();
        let config =
            AntennaConfiguration::new("asym".into(), "Asym".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams::fast();

        let theta = 0.05; // ~2.9° off-axis, where the elliptical beam is resolvable
        let g_e = integrate_aperture(theta, 0.0, &config, 8.4e9, &params)
            .unwrap()
            .field
            .norm();
        let g_h = integrate_aperture(theta, PI / 2.0, &config, 8.4e9, &params)
            .unwrap()
            .field
            .norm();

        // Non-trivial φ dependence proves the 2D (non-Hankel) path was taken.
        assert!(
            (g_e - g_h).abs() > 1e-6 * g_e.max(g_h),
            "asymmetric centered feed must retain φ dependence (2D path): E-plane={g_e}, H-plane={g_h}"
        );
    }

    #[test]
    fn asymmetric_illumination_centered_feed_converges_and_matches_2d() {
        // Review FIX 1: a CENTERED feed (δ=0) with a non-unity asymmetry_factor has NO
        // lateral coma (spread=0), but `illumination_amplitude` modulates the effective
        // q-factor by cos(2φ'), so the aperture function g(ρ,φ') carries genuine m=±2
        // (plus weaker ±4, ±6) azimuthal content. `mode_count_for` must NOT early-return
        // (1, MODE_PHI_MIN): with m_max=1 the mode sum under-resolves that content AND the
        // M-vs-(M+1) self-check sees the large m=2 jump → spurious converged=false.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(); // small dish → fast
        let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.5).unwrap();
        let config =
            AntennaConfiguration::new("asym".into(), "Asym".into(), reflector, feed, None).unwrap();
        let f_hz = 8.4e9;
        let wl = wavelength_from_frequency(f_hz);
        let params = IntegrationParams::default();

        let theta = 5.0_f64.to_radians();
        let r_off = integrate_aperture(theta, 0.0, &config, f_hz, &params).unwrap();
        let r_on = integrate_aperture(0.0, 0.0, &config, f_hz, &params).unwrap();

        // Not spuriously flagged, and physically plausible (finite, positive, below boresight).
        assert!(
            r_off.converged,
            "asymmetric-illumination centered feed must converge (err={:.3e})",
            r_off.error_estimate
        );
        let (off, on) = (r_off.field.norm(), r_on.field.norm());
        assert!(
            off.is_finite() && off > 0.0,
            "off-axis field must be finite/positive"
        );
        assert!(
            off < on,
            "off-axis must be below boresight: off={off} on={on}"
        );

        // The mode-path result must match the trusted 2D quadrature reference to <0.1 dB —
        // proof the m=±2, ±4, ±6 content is actually RESOLVED, not merely un-warned.
        let k = wavenumber(wl);
        let mut hi = IntegrationParams::high_accuracy();
        hi.max_rho_points = 513;
        hi.max_phi_points = 1025;
        let ref_field = integrate_2d_simpson_public_shim(theta, 0.0, &config, f_hz, &hi);
        let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, wl, theta);
        assert!(
            m_max >= 6,
            "asym illumination must resolve at least ~m=6, got M={m_max}"
        );
        let mode_field = azimuthal_mode_field(&config, theta, 0.0, k, 4097, n_phi, m_max);
        let d_db = 20.0 * (mode_field.norm() / ref_field.norm()).log10();
        assert!(
            d_db.abs() < 0.1,
            "mode vs 2D Δ={d_db:.4} dB (M={m_max}, n_phi={n_phi})"
        );
    }

    #[test]
    fn test_simpson_weight() {
        // Test Simpson's rule weights
        let n = 5; // 5 points

        assert_eq!(simpson_weight(0, n), 1.0); // First point
        assert_eq!(simpson_weight(1, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(2, n), 2.0); // Even interior
        assert_eq!(simpson_weight(3, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(4, n), 1.0); // Last point
    }

    #[test]
    fn test_integration_params_default() {
        let params = IntegrationParams::default();

        assert!(params.min_rho_points > 0);
        assert!(params.max_rho_points >= params.min_rho_points);
        assert!(params.relative_tolerance > 0.0);
        assert!(params.max_iterations > 0);
    }

    #[test]
    fn test_integration_params_fast() {
        let params = IntegrationParams::fast();
        let default_params = IntegrationParams::default();

        // Fast should use fewer points
        assert!(params.min_rho_points <= default_params.min_rho_points);
        assert!(params.max_rho_points <= default_params.max_rho_points);
    }

    /// The two P11-gated flags move together, from every preset, in both directions.
    ///
    /// They are one decision — "is this artifact's physics uncorrected" — wearing two names,
    /// and the whole point of the setter is that no caller can set one without the other.
    /// The preset sweep is deliberate: `calibrate` builds from `default()` and the service
    /// from `adaptive()`, so a setter that only worked off one of them would reopen roadmap
    /// D17's calibrate/service split from the other side.
    #[test]
    fn uncorrected_physics_gates_move_together_from_every_preset() {
        for base in [
            IntegrationParams::default(),
            IntegrationParams::adaptive(),
            IntegrationParams::fast(),
            IntegrationParams::high_accuracy(),
        ] {
            for uncorrected in [true, false] {
                let params = base.clone().with_uncorrected_physics_gates(uncorrected);
                assert_eq!(
                    params.apply_spillover, uncorrected,
                    "apply_spillover must track the predicate"
                );
                assert_eq!(
                    params.apply_sidelobe_floor, uncorrected,
                    "apply_sidelobe_floor must track the SAME predicate as apply_spillover \
                     (roadmap P11) — they are one decision, not two knobs"
                );
            }
        }
    }

    /// The setter touches the gates and nothing else — it must not quietly reshape the
    /// integration a caller chose, or `calibrate` and the service would agree about
    /// spillover while disagreeing about sampling.
    #[test]
    fn uncorrected_physics_gates_leave_the_rest_of_the_preset_alone() {
        let base = IntegrationParams::default();
        let gated = base.clone().with_uncorrected_physics_gates(true);

        assert_eq!(gated.min_rho_points, base.min_rho_points);
        assert_eq!(gated.max_rho_points, base.max_rho_points);
        assert_eq!(gated.min_phi_points, base.min_phi_points);
        assert_eq!(gated.max_phi_points, base.max_phi_points);
        assert_eq!(gated.relative_tolerance, base.relative_tolerance);
        assert_eq!(gated.absolute_tolerance, base.absolute_tolerance);
        assert_eq!(gated.max_iterations, base.max_iterations);
        assert_eq!(gated.time_budget, base.time_budget);
    }

    #[test]
    fn test_integration_params_high_accuracy() {
        let params = IntegrationParams::high_accuracy();
        let default_params = IntegrationParams::default();

        // High accuracy should use more points and tighter tolerance
        assert!(params.max_rho_points >= default_params.max_rho_points);
        assert!(params.relative_tolerance <= default_params.relative_tolerance);
    }

    #[test]
    fn test_aperture_integrand_on_axis() {
        let config = test_antenna();
        let wavelength = 0.0357; // ~8.4 GHz
        let k = wavenumber(wavelength);

        // On-axis (θ=0, φ=0), center of aperture (ρ=0)
        let integrand = aperture_integrand(0.0, 0.0, 0.0, 0.0, &config, k, wavelength);

        // At center, amplitude should be near maximum, phase should be well-defined
        assert!(integrand.norm() > 0.0);
        assert!(integrand.norm() <= 1.0);
    }

    #[test]
    fn test_aperture_integrand_symmetry() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // For symmetric feed and ideal surface, pattern should have azimuthal symmetry
        let rho = 0.2;
        let theta = 0.1;

        let integrand_0 = aperture_integrand(rho, 0.0, theta, 0.0, &config, k, wavelength);
        let integrand_90 =
            aperture_integrand(rho, PI / 2.0, theta, PI / 2.0, &config, k, wavelength);

        // Magnitudes should be equal due to symmetry
        assert!((integrand_0.norm() - integrand_90.norm()).abs() < 1e-6);
    }

    #[test]
    fn test_integrate_aperture_on_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast(); // Use fast for quicker tests

        let result = integrate_aperture(
            0.0, // theta (on-axis)
            0.0, // phi
            &config, 8.4e9, // 8.4 GHz
            &params,
        )
        .unwrap();

        // On-axis field should be non-zero
        assert!(result.field.norm() > 0.0);

        // Should have performed evaluations
        assert!(result.num_evaluations > 0);

        // On-axis integration with fast params must converge (smooth, no phase oscillation).
        assert!(result.converged, "on-axis fast integration must converge");
        // A converged result must have a finite, non-negative error estimate.
        assert!(
            result.error_estimate.is_finite(),
            "converged error_estimate must be finite"
        );
        assert!(result.error_estimate >= 0.0);
    }

    #[test]
    fn test_integrate_aperture_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // Small off-axis angle
        let result = integrate_aperture(
            0.05, // theta (small angle ~2.9°)
            0.0,  // phi
            &config, 8.4e9, &params,
        )
        .unwrap();

        // Off-axis field should be non-zero but smaller than on-axis
        assert!(result.field.norm() > 0.0);
    }

    #[test]
    fn test_integrate_aperture_convergence() {
        let config = test_antenna();

        // Test that higher accuracy params give better results
        let fast_params = IntegrationParams::fast();
        let accurate_params = IntegrationParams::high_accuracy();

        let fast_result = integrate_aperture(0.0, 0.0, &config, 8.4e9, &fast_params).unwrap();
        let accurate_result =
            integrate_aperture(0.0, 0.0, &config, 8.4e9, &accurate_params).unwrap();

        // Both must converge so the error-estimate comparison below is meaningful.
        assert!(
            fast_result.converged,
            "fast on-axis integration must converge"
        );
        assert!(
            accurate_result.converged,
            "accurate on-axis integration must converge"
        );

        // High accuracy should have lower error estimate
        assert!(accurate_result.error_estimate <= fast_result.error_estimate * 2.0);

        // Results should be similar
        let difference = (fast_result.field - accurate_result.field).norm();
        let magnitude = accurate_result.field.norm();
        assert!(difference / magnitude < 0.1); // Within 10%
    }

    #[test]
    fn test_integrate_aperture_invalid_inputs() {
        let config = test_antenna();
        let params = IntegrationParams::default();

        // Invalid frequency
        let result = integrate_aperture(0.0, 0.0, &config, -1.0, &params);
        assert!(result.is_err());

        // Invalid angle (NaN)
        let result = integrate_aperture(f64::NAN, 0.0, &config, 8.4e9, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_integrate_amplitude_squared_positive_finite() {
        let config = test_antenna();

        // The denominator of the directivity formula must be a positive, finite
        // real number for a physically-illuminated aperture.
        let amp_sq = integrate_amplitude_squared(&config, 33, 65);
        assert!(amp_sq.is_finite());
        assert!(amp_sq > 0.0);

        // Sanity upper bound: |A| <= 1 everywhere, so the integral is at most the
        // area integral ∬ ρ dρ dφ' = π(D/2)² = π·0.25 ≈ 0.785 for the 1m test dish.
        let area = PI * (config.reflector.diameter / 2.0).powi(2);
        assert!(amp_sq <= area + 1e-9);
    }

    #[test]
    fn test_far_field_normalization() {
        let wavelength = 0.0357; // ~8.4 GHz
        let norm = far_field_normalization(wavelength);

        // Should be purely imaginary (j factor)
        assert!(norm.re.abs() < 1e-10);
        assert!(norm.im != 0.0);

        // Magnitude should be k/(2λ) = π/λ²
        let expected_magnitude = PI / (wavelength * wavelength);
        assert!((norm.norm() - expected_magnitude).abs() < 1e-6);
    }

    #[test]
    fn test_compute_far_field() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let field = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Far field should be non-zero
        assert!(field.norm() > 0.0);

        // Should be complex-valued
        // (May have both real and imaginary parts depending on phase)
    }

    #[test]
    fn test_pattern_decreases_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // On-axis field
        let field_on_axis = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Off-axis field (5 degrees)
        let field_off_axis =
            compute_far_field(5.0_f64.to_radians(), 0.0, &config, 8.4e9, &params).unwrap();

        // Pattern should decrease off-axis
        assert!(field_off_axis.norm() < field_on_axis.norm());
    }

    #[test]
    fn test_non_convergence_is_reported() {
        // The 2D adaptive refinement loop is what carries the non-convergence sentinel
        // (converged=false, error_estimate=INFINITY). Since P10 Task 2 NO production
        // aperture goes through that loop — symmetric feeds take the exact 1D Hankel path
        // and asymmetric/coma feeds take the Jₘ mode expansion, both of which return
        // converged=true in the interim (the real runtime self-check is Task 3). This
        // test therefore pins the retained 2D mechanism DIRECTLY via `integrate_2d_adaptive`
        // (the exact loop `integrate_aperture` used pre-Task-2), pending Task 3 folding a
        // convergence self-check into the Hankel/mode paths.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(0.01, 0.0, 0.5), 8.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("off".into(), "Off".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams {
            max_iterations: 1, // cannot converge: convergence check needs iteration > 0
            relative_tolerance: 1e-15,
            ..IntegrationParams::fast()
        };
        let result = integrate_2d_adaptive(0.3, 0.0, &config, 8.4e9, &params);
        assert!(!result.converged);
        // With max_iterations == 1 the loop runs a single iteration and the convergence
        // check (iteration > 0) is never reached, so no inter-iteration difference is
        // ever computed.  last_difference remains at its INFINITY sentinel value.
        assert_eq!(result.error_estimate, f64::INFINITY);
    }

    #[test]
    fn test_integration_2d_simpson_basic() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // Simple integration test
        let (result, evals) = integrate_2d_simpson(
            0.0, // theta
            0.0, // phi
            &config,
            k,
            wavelength,
            0.0,      // rho_min
            0.5,      // rho_max (half diameter)
            0.0,      // phi_min
            2.0 * PI, // phi_max
            17,       // n_rho (odd)
            33,       // n_phi (odd)
        );

        // Should produce non-zero result
        assert!(result.norm() > 0.0);

        // Should have performed expected number of evaluations
        assert_eq!(evals, 17 * 33);
    }

    /// Auto-refocus (roadmap P7): phase_center_offset is a recorded feed property
    /// the model compensates — it must NOT change gain. Deliberate defocus goes
    /// through the explicit axial_defocus field instead.
    #[test]
    fn test_phase_center_offset_alone_produces_no_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let feed_pco = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.05, 1.0).unwrap();

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_pco = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_pco), 8.4e9, &params)
            .unwrap()
            .gain;

        assert!(
            (g_focused - g_pco).abs() < 1e-9,
            "phase_center_offset is auto-refocused and must not change gain: \
             focused={g_focused:.6}, pco={g_pco:.6}"
        );
    }

    /// The defocus math stays live through the explicit field: a 5 cm deliberate
    /// axial defocus must cost >1 dB at 8.4 GHz (same physics the old
    /// test_phase_center_offset_produces_defocus_loss pinned).
    #[test]
    fn test_axial_defocus_produces_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let mut feed_defocused =
            FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        feed_defocused.axial_defocus = 0.05;

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_defocused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_defocused), 8.4e9, &params)
                .unwrap()
                .gain;

        assert!(
            g_focused - g_defocused > 1.0,
            "5 cm axial_defocus must cost >1 dB defocus at 8.4 GHz: \
             focused={g_focused:.2}, defocused={g_defocused:.2}"
        );
    }
}

/// P12 task 1 — diagnostic instrument for the azimuthal-mode path's radial quadrature.
///
/// Roadmap unit **P12** ("The azimuthal-mode path never checks radial convergence") task 1
/// requires establishing *why* `radial_points_for`'s ~2× Nyquist budget is insufficient on
/// the mode path **before** any constant is changed, because a wrong budget formula is a
/// finding in its own right. These are `#[ignore]`d measurement harnesses, not assertions
/// about desired behavior — they print, they do not gate. The regression anchors that DO
/// gate come later, in P12 task 4.
///
/// Run with:
/// ```text
/// cargo test --release -p antenna-model --lib p12_ -- --ignored --nocapture
/// ```
///
/// This module lives inside `integration.rs` because the quantities under investigation
/// (`radial_points_for`, `azimuthal_mode_field_inner`, `aperture_plane_g`,
/// `AperturePlaneConst`) are private to it.
#[cfg(test)]
mod p12_radial_diagnostic {
    use super::*;
    // The per-order call, which the production path no longer uses. Kept for the diagnostics
    // below that evaluate a single named mode: there the ladder form buys nothing, and an
    // independently-computed `Jₘ` is the more useful reference.
    use crate::model::bessel::bessel_jn;
    use crate::model::geometry::{
        FeedParameters, FeedPosition, MeshParameters, MeshPattern, ReflectorGeometry,
    };

    /// The **exact** served `gs_3.7m_uncalibrated` / `x_band_feed` geometry, transcribed from
    /// `calibration_data/antennas.yaml:100-134`: D = 3.7 m, f = 1.85 m (f/D = 0.5),
    /// surface RMS 1.5 mm, feed q = 2.04 at a 0.05 m lateral (+x) offset with the phase
    /// centre at the focus, 5 mm / 0.5 mm mesh.
    ///
    /// Deliberately NOT `tests::offset_feed_test_antenna`, which is close but not the served
    /// configuration (q = 2.0, no mesh, ideal surface). P12's measured rows are against the
    /// real entry, so the diagnostic must be too.
    pub(super) fn gs_3_7m_x_band() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(3.7, 1.85, 0.0015).unwrap();
        let mut pos = FeedPosition::at_focus(1.85);
        pos.x = 0.05;
        let feed = FeedParameters::new(pos, 2.04, 0.0, 1.0).unwrap();
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap();
        AntennaConfiguration::new(
            "gs_3.7m_uncalibrated".into(),
            "Ground Station 3.7m - Uncalibrated".into(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    /// The **exact** served `dsn_34m_uncalibrated` / `x_band` geometry, for the second of
    /// P12's three measured rows (θ = 0.10°, where the `min_rho_points` floor DOES bind).
    pub(super) fn dsn_34m_x_band() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.00025).unwrap();
        let mut pos = FeedPosition::at_focus(13.6);
        pos.x = 0.15;
        // `phase_center_offset_m: 0.015` is a recorded feed property only — P7 auto-refocus
        // keeps it out of the aperture phase, so it is deliberately not an axial defocus here.
        let feed = FeedParameters::new(pos, 1.14, 0.015, 1.0).unwrap();
        AntennaConfiguration::new(
            "dsn_34m_uncalibrated".into(),
            "DSN 34m - Uncalibrated".into(),
            reflector,
            feed,
            None,
        )
        .unwrap()
    }

    /// D12's `UHF_Array_Element` calibration fixture (`calibrate/tests/support/mod.rs:98-132`,
    /// transcribed from `calibrate/antenna_classes.yaml:96-113`): D = 8 m, f/D = 0.45,
    /// surface RMS 2.0 mm, q = 5.0 feed **at the focus**, 10 mm / 1.0 mm mesh.
    ///
    /// This is P12's third measured row, and it is the instructive one: the feed has **no
    /// lateral offset at all**. It reaches the mode path purely through
    /// `asymmetry_factor = 1.1`, so its coma cycle budget is exactly zero — which makes it a
    /// clean check that the defect is not about coma.
    pub(super) fn d12_uhf_fixture() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(8.0, 8.0 * 0.45, 0.002).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(8.0 * 0.45), 5.0, 0.0, 1.1).unwrap();
        let mesh = MeshParameters::new(0.010, 0.001, MeshPattern::Square).unwrap();
        AntennaConfiguration::new(
            "UHF_Array_Element".into(),
            "UHF phased array element".into(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    /// `dsn_34m_uncalibrated` / `ka_band` (`antennas.yaml:221-226`): the same 34 m dish with the
    /// Ka feed offset 0.15 m along **+y**. This is P10-perf's pathological latency case — the
    /// widest azimuthal spectrum on any served antenna (~195 modes at 32 GHz) — and therefore
    /// the geometry that decides what a per-call radial check can afford.
    pub(super) fn dsn_34m_ka_band() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.00025).unwrap();
        let mut pos = FeedPosition::at_focus(13.6);
        pos.y = 0.15;
        let feed = FeedParameters::new(pos, 1.14, 0.008, 1.0).unwrap();
        AntennaConfiguration::new(
            "dsn_34m_uncalibrated".into(),
            "DSN 34m - Uncalibrated".into(),
            reflector,
            feed,
            None,
        )
        .unwrap()
    }

    /// Wall-clock of `f`, in milliseconds, as the **minimum** over `reps` runs (min rather than
    /// mean: we want the cost of the work, not of whatever else the machine was doing).
    pub(super) fn time_ms<F: FnMut() -> Complex64>(reps: usize, mut f: F) -> f64 {
        let mut best = f64::INFINITY;
        for _ in 0..reps {
            let t = Instant::now();
            let v = f();
            std::hint::black_box(v);
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        best
    }

    /// `radial_points_for`'s cycle budget, broken into the four terms it sums, so the
    /// measured radial content can be attributed term by term. Mirrors `:884-893` exactly.
    struct CycleBudget {
        kernel: f64,
        coma: f64,
        defocus: f64,
        chirp: f64,
        total: f64,
        n_rho: usize,
    }

    fn cycle_budget(
        config: &AntennaConfiguration,
        theta: f64,
        wavelength: f64,
        params: &IntegrationParams,
    ) -> CycleBudget {
        let d_lambda = config.reflector.diameter / wavelength;
        let r = config.reflector.diameter / 2.0;
        let f = config.reflector.focal_length;
        let r_over_f = r / f;
        let radial_cycle_ceiling = 0.5 * d_lambda;
        let delta = config.feed.position.radial_displacement();
        let axial = (config.feed.position.z - f + config.feed.axial_defocus).abs();
        let kernel = d_lambda * theta.sin().abs();
        let coma_effort_cap = if delta / f > crate::model::edge_cases::SEVERE_OFFSET_THRESHOLD {
            BEYOND_SCOPE_COMA_CYCLE_CAP.min(radial_cycle_ceiling)
        } else {
            radial_cycle_ceiling
        };
        let coma = ((delta / wavelength) * r_over_f).min(coma_effort_cap);
        let defocus = ((axial / wavelength) * r_over_f * r_over_f).min(radial_cycle_ceiling);
        let chirp = r * r / (4.0 * f * wavelength) * (1.0 - theta.cos());
        CycleBudget {
            kernel,
            coma,
            defocus,
            chirp,
            total: kernel + coma + defocus + chirp,
            n_rho: radial_points_for(config, theta, wavelength, params),
        }
    }

    /// Per-mode radial contributions to `I(θ,φ)`, i.e. the `m`-th term of
    /// `I = 2π Σ_m (−j)^m e^{jmφ} R_m(θ)` with the `±m` pair combined, for `m = 0..=m_max`.
    ///
    /// This replicates `azimuthal_mode_field_inner`'s math line for line rather than calling
    /// it, because that function deliberately exposes only the total and the top mode. The
    /// replication is **self-validated** by `per_mode_decomposition_reproduces_the_integrator`
    /// below: the sum of what this returns must equal `azimuthal_mode_field_inner`'s total to
    /// round-off. If that test fails, every number this module prints is suspect.
    fn per_mode_contributions(
        config: &AntennaConfiguration,
        theta: f64,
        phi: f64,
        k: f64,
        n_rho: usize,
        n_phi_coeff: usize,
        m_max: u32,
    ) -> Vec<Complex64> {
        let f = config.reflector.focal_length;
        let r_max = config.reflector.diameter / 2.0;
        let apc = AperturePlaneConst::new(config);
        let n = if n_rho.is_multiple_of(2) {
            n_rho + 1
        } else {
            n_rho
        };
        let h = r_max / (n - 1) as f64;
        let dphi = 2.0 * PI / n_phi_coeff as f64;
        let sin_theta = theta.sin();
        let one_minus_cos = 1.0 - theta.cos();
        let mmax = m_max as usize;

        let mut r_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
        let mut r_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];
        let plan = crate::model::fft::FftPlan::new(n_phi_coeff);
        let phi_grid = PhiGrid::new(n_phi_coeff);
        let mut g_samples = vec![Complex64::new(0.0, 0.0); n_phi_coeff];
        let mut fft_scratch = vec![Complex64::new(0.0, 0.0); n_phi_coeff];
        let mut jm = vec![0.0_f64; mmax + 1];

        for i in 0..n {
            let rho = i as f64 * h;
            let w = simpson_weight(i, n);
            let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
            let chirp_factor = Complex64::new(0.0, chirp).exp();
            let a = k * rho * sin_theta;

            // Mirrors `azimuthal_mode_field_inner` line for line, INCLUDING its P10-perf
            // kernels: the FFT for `gₘ` and the single-sweep `Jₘ` ladder. Reverting either to
            // its pre-P10-perf form here would not make this a stricter check — it would make
            // it a check of something else, and
            // `per_mode_decomposition_reproduces_the_integrator` would fail at ~1e-6 for a
            // reason having nothing to do with the decomposition.
            let rho_phase = apc.rho_only_phase(rho, k);
            for (jj, s) in g_samples.iter_mut().enumerate() {
                *s = aperture_plane_g(&apc, &phi_grid, jj, rho, rho_phase, k);
            }
            plan.forward(&mut g_samples, &mut fft_scratch);
            let norm = dphi / (2.0 * PI);
            let mut gm_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
            let mut gm_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];
            for m in 0..=mmax {
                let mm = m % n_phi_coeff;
                gm_pos[m] = g_samples[mm] * norm;
                gm_neg[m] = g_samples[(n_phi_coeff - mm) % n_phi_coeff] * norm;
            }

            bessel_jn_array(m_max, a, &mut jm);
            for (m, (rp, rn)) in r_pos.iter_mut().zip(r_neg.iter_mut()).enumerate() {
                let base = chirp_factor * jm[m] * rho * w;
                *rp += base * gm_pos[m];
                if m > 0 {
                    let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                    *rn += base * gm_neg[m] * sign;
                }
            }
        }

        let scale = h / 3.0;
        let mut out = vec![Complex64::new(0.0, 0.0); mmax + 1];
        out[0] = r_pos[0] * scale * 2.0 * PI;
        for m in 1..=mmax {
            let mf = m as f64;
            let epos = Complex64::new(0.0, mf * phi).exp();
            let eneg = Complex64::new(0.0, -mf * phi).exp();
            out[m] = (pow_neg_j(m as i32) * epos * r_pos[m] * scale
                + pow_neg_j(-(m as i32)) * eneg * r_neg[m] * scale)
                * 2.0
                * PI;
        }
        out
    }

    /// The mode-`m` radial integrand `F_m(ρ) = exp(j·k·ρ²/(4f)·(1−cosθ)) · g_m(ρ) · J_m(kρ sinθ) · ρ`
    /// — i.e. everything inside `∫₀^R … dρ`, sampled on a uniform grid of `n` points over
    /// `[0, R]`. This is the function the Simpson rule actually has to resolve, and the
    /// function whose radial bandwidth `radial_points_for` is trying to predict.
    fn radial_integrand_samples(
        config: &AntennaConfiguration,
        theta: f64,
        k: f64,
        m: u32,
        n_phi_coeff: usize,
        n: usize,
    ) -> Vec<Complex64> {
        let f = config.reflector.focal_length;
        let r_max = config.reflector.diameter / 2.0;
        let apc = AperturePlaneConst::new(config);
        let phi_grid = PhiGrid::new(n_phi_coeff);
        let dphi = 2.0 * PI / n_phi_coeff as f64;
        let sin_theta = theta.sin();
        let one_minus_cos = 1.0 - theta.cos();
        let h = r_max / (n - 1) as f64;

        (0..n)
            .map(|i| {
                let rho = i as f64 * h;
                let rho_phase = apc.rho_only_phase(rho, k);
                let mut gm = Complex64::new(0.0, 0.0);
                for jj in 0..n_phi_coeff {
                    let t = Complex64::new(0.0, -(m as f64) * jj as f64 * dphi).exp();
                    gm += aperture_plane_g(&apc, &phi_grid, jj, rho, rho_phase, k) * t;
                }
                gm *= dphi / (2.0 * PI);
                let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
                let chirp_factor = Complex64::new(0.0, chirp).exp();
                chirp_factor * gm * bessel_jn(m, k * rho * sin_theta) * rho
            })
            .collect()
    }

    /// Hann-windowed magnitude spectrum of a complex sequence, evaluated at integer
    /// frequencies `0..=f_max` **cycles across the whole `[0, R]` span** (both signs of
    /// frequency folded together, since a complex signal's ± content is independent).
    ///
    /// The Hann window is what makes the answer trustworthy: `F_m(ρ)` is not periodic on
    /// `[0, R]` (the illumination taper leaves ≈ −11 dB of amplitude at the rim), so a bare
    /// DFT would smear endpoint discontinuity across the whole band and manufacture
    /// "bandwidth" that is an artifact of the transform, not a property of the integrand.
    fn windowed_spectrum(samples: &[Complex64], f_max: usize) -> Vec<f64> {
        let n = samples.len();
        let windowed: Vec<Complex64> = samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / (n - 1) as f64).cos());
                s * w
            })
            .collect();
        (0..=f_max)
            .map(|freq| {
                let mut pos = Complex64::new(0.0, 0.0);
                let mut neg = Complex64::new(0.0, 0.0);
                for (i, s) in windowed.iter().enumerate() {
                    let ang = 2.0 * PI * freq as f64 * i as f64 / n as f64;
                    pos += s * Complex64::new(0.0, -ang).exp();
                    neg += s * Complex64::new(0.0, ang).exp();
                }
                (pos.norm_sqr() + if freq == 0 { 0.0 } else { neg.norm_sqr() }).sqrt()
            })
            .collect()
    }

    /// Lowest frequency (in cycles across `[0, R]`) below which `fraction` of the spectrum's
    /// total energy lies.
    fn bandwidth_containing(spectrum: &[f64], fraction: f64) -> usize {
        let total: f64 = spectrum.iter().map(|s| s * s).sum();
        let mut acc = 0.0;
        for (i, s) in spectrum.iter().enumerate() {
            acc += s * s;
            if acc >= fraction * total {
                return i;
            }
        }
        spectrum.len() - 1
    }

    /// `Σ_{m∈modes} (−j)^m e^{jmφ} R_m(θ)·2π` at an explicit radial density — the mode sum
    /// restricted to a chosen subset, which is what D-A option (ii) would recompute at 2N.
    ///
    /// The cost structure this exposes is the whole point of pricing D-A: the φ' sweep
    /// evaluates `aperture_plane_g` at `n_rho × n_phi` points **regardless of how few modes are
    /// requested**, while only the inner accumulation scales with the subset size. A subset
    /// check is therefore NOT proportional to `|subset| / m_max` — it is floored by the
    /// g-evaluation, and how binding that floor is depends entirely on the geometry.
    fn mode_subset_field(
        config: &AntennaConfiguration,
        theta: f64,
        phi: f64,
        k: f64,
        n_rho: usize,
        n_phi_coeff: usize,
        modes: &[u32],
    ) -> Complex64 {
        let f = config.reflector.focal_length;
        let r_max = config.reflector.diameter / 2.0;
        let apc = AperturePlaneConst::new(config);
        let n = if n_rho.is_multiple_of(2) {
            n_rho + 1
        } else {
            n_rho
        };
        let h = r_max / (n - 1) as f64;
        let dphi = 2.0 * PI / n_phi_coeff as f64;
        let sin_theta = theta.sin();
        let one_minus_cos = 1.0 - theta.cos();

        let mut r_pos = vec![Complex64::new(0.0, 0.0); modes.len()];
        let mut r_neg = vec![Complex64::new(0.0, 0.0); modes.len()];
        let mut gm_pos = vec![Complex64::new(0.0, 0.0); modes.len()];
        let mut gm_neg = vec![Complex64::new(0.0, 0.0); modes.len()];
        let phi_grid = PhiGrid::new(n_phi_coeff);

        for i in 0..n {
            let rho = i as f64 * h;
            let w = simpson_weight(i, n);
            let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
            let chirp_factor = Complex64::new(0.0, chirp).exp();
            let a = k * rho * sin_theta;

            for g in gm_pos.iter_mut() {
                *g = Complex64::new(0.0, 0.0);
            }
            for g in gm_neg.iter_mut() {
                *g = Complex64::new(0.0, 0.0);
            }
            let rho_phase = apc.rho_only_phase(rho, k);
            for jj in 0..n_phi_coeff {
                let g = aperture_plane_g(&apc, &phi_grid, jj, rho, rho_phase, k);
                for (idx, &m) in modes.iter().enumerate() {
                    let t = Complex64::new(0.0, -(m as f64) * jj as f64 * dphi).exp();
                    gm_pos[idx] += g * t;
                    gm_neg[idx] += g * t.conj();
                }
            }
            let norm = dphi / (2.0 * PI);
            for idx in 0..modes.len() {
                gm_pos[idx] *= norm;
                gm_neg[idx] *= norm;
            }

            for (idx, &m) in modes.iter().enumerate() {
                let jm = bessel_jn(m, a);
                let base = chirp_factor * jm * rho * w;
                r_pos[idx] += base * gm_pos[idx];
                if m > 0 {
                    let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                    r_neg[idx] += base * gm_neg[idx] * sign;
                }
            }
        }

        let scale = h / 3.0;
        let mut acc = Complex64::new(0.0, 0.0);
        for (idx, &m) in modes.iter().enumerate() {
            if m == 0 {
                acc += r_pos[idx] * scale;
            } else {
                let mf = m as f64;
                let epos = Complex64::new(0.0, mf * phi).exp();
                let eneg = Complex64::new(0.0, -mf * phi).exp();
                acc += pow_neg_j(m as i32) * epos * r_pos[idx] * scale
                    + pow_neg_j(-(m as i32)) * eneg * r_neg[idx] * scale;
            }
        }
        acc * 2.0 * PI
    }

    /// Mode field at an explicitly chosen radial density, bypassing `radial_points_for`.
    fn mode_field_at(
        config: &AntennaConfiguration,
        theta: f64,
        phi: f64,
        k: f64,
        n_rho: usize,
        n_phi: usize,
        m_max: u32,
    ) -> Complex64 {
        azimuthal_mode_field_inner(config, theta, phi, k, n_rho, n_phi, m_max, None)
            .expect("no deadline")
            .total
    }

    /// **Load-bearing consistency gate for P12's radial pre-gate.**
    ///
    /// The pre-gate compares `ModeSweep::radial_probe` (accumulated inside the full sweep) at
    /// `N` against [`radial_probe_field`] (a separate, standalone implementation) at `2N`. Those
    /// are two independently written code paths, and the comparison is only meaningful if they
    /// compute the *same* quantity. If they ever drift, the pre-gate would be differencing two
    /// different functions and could certify an arbitrarily wrong answer as converged — a
    /// silent-wrong-number failure of exactly the kind P12 exists to remove.
    ///
    /// Not `#[ignore]`d: this must fail loudly in CI.
    #[test]
    fn radial_probe_field_matches_the_full_sweeps_probe_accumulation() {
        let k_of = |f: f64| wavenumber(wavelength_from_frequency(f));
        let cases: Vec<(&str, AntennaConfiguration, f64, f64, f64)> = vec![
            ("gs_3.7m", gs_3_7m_x_band(), 8.4e9, 5.0_f64, 0.0_f64),
            ("dsn_34m X", dsn_34m_x_band(), 8.45e9, 0.10, 0.0),
            // Asymmetric-illumination door into the mode path (feed at focus, δ = 0).
            ("D12 UHF", d12_uhf_fixture(), 600.0e6, 16.0, 90.0),
            // φ ≠ 0 matters: the probe carries the m=1 term's e^{±jφ} factors.
            ("gs_3.7m φ=37°", gs_3_7m_x_band(), 8.4e9, 3.0, 37.0),
        ];
        for (name, config, freq, theta_deg, phi_deg) in cases {
            let lambda = wavelength_from_frequency(freq);
            let k = k_of(freq);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let n_rho = 129;

            let sweep =
                azimuthal_mode_field_inner(&config, theta, phi, k, n_rho, n_phi, m_max + 1, None)
                    .expect("sweep");
            let standalone =
                radial_probe_field(&config, theta, phi, k, n_rho, n_phi, m_max + 1, None)
                    .expect("probe");

            let rel = (sweep.radial_probe - standalone).norm()
                / sweep.radial_probe.norm().max(f64::MIN_POSITIVE);
            assert!(
                rel < 1e-12,
                "{name}: radial_probe_field disagrees with the full sweep's probe accumulation \
                 (rel={rel:.3e}); the P12 pre-gate would be differencing two different functions"
            );
        }
    }

    /// **φ'-axis sufficiency gate.** `mode_count_for` now sizes `n_phi` from the aperture
    /// function's azimuthal bandwidth and asserts `azimuthally_resolved`; this checks that the
    /// claim is true, by comparing the served sizing against a 4× denser φ' grid on every
    /// served geometry plus the steered case that exposed the old cap.
    ///
    /// Not `#[ignore]`d. It is the only automatic guard on this axis: the radial N-vs-2N check
    /// and the `M`-vs-`M+1` truncation check both operate on `gₘ` coefficients that φ'
    /// aliasing has already corrupted, so neither can see it — which is exactly how a +82 dB
    /// error survived with `converged = true`.
    ///
    /// It also guards the 2026-07-31 change that stopped rounding `n_phi` up to a power of two:
    /// that halved `n_phi` on `dsn_34m` X-band (128 → 76) and Ka (512 → 260), and "still ≥ 2B"
    /// is an argument, not a measurement.
    #[test]
    fn served_n_phi_sizing_is_sufficient_on_every_asymmetric_geometry() {
        let cases: Vec<(&str, AntennaConfiguration, f64, Vec<f64>)> = vec![
            (
                "gs_3.7m X",
                gs_3_7m_x_band(),
                8.4e9,
                vec![0.0, 1.0, 5.0, 20.0],
            ),
            (
                "dsn_34m X",
                dsn_34m_x_band(),
                8.45e9,
                vec![0.0, 0.1, 1.0, 5.0],
            ),
            ("dsn_34m Ka", dsn_34m_ka_band(), 32.0e9, vec![0.0, 0.5, 2.0]),
            ("D12 UHF", d12_uhf_fixture(), 600.0e6, vec![0.0, 5.0, 16.0]),
            (
                // δ/f = 0.0875 — a routine ~5° beam steer, and the geometry the former
                // MODE_PHI_STEERED_MAX clamp was wrong by up to +82 dB on.
                "steered coma 34 m",
                {
                    let reflector = ReflectorGeometry::new(34.0, 13.6, 0.0).unwrap();
                    let feed =
                        FeedParameters::new(FeedPosition::new(1.19, 0.0, 13.6), 10.0, 0.0, 1.0)
                            .unwrap();
                    AntennaConfiguration::new("c".into(), "c".into(), reflector, feed, None)
                        .unwrap()
                },
                8450.0e6,
                vec![0.0, 1.0, 3.0],
            ),
        ];

        for (name, config, freq, angles) in cases {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            for theta_deg in angles {
                let theta = theta_deg.to_radians();
                let sizing = mode_count_for(&config, lambda, theta);
                // Radial density held FIXED on both sides so only the φ' axis varies. It
                // deliberately does NOT need to be converged: this compares two computations
                // that share it, so radial error cancels in the ratio. Keeping it modest is
                // what makes this affordable in the DEBUG build `scripts/check.sh` uses.
                let n_rho = 257;
                let served =
                    azimuthal_mode_field(&config, theta, 0.0, k, n_rho, sizing.n_phi, sizing.m_max);
                // 2× is enough to expose aliasing: φ' under-sampling fails
                // catastrophically (tens of dB), never marginally.
                let denser = azimuthal_mode_field(
                    &config,
                    theta,
                    0.0,
                    k,
                    n_rho,
                    sizing.n_phi * 2,
                    sizing.m_max,
                );
                let d_db = 20.0 * (served.norm() / denser.norm()).log10();
                assert!(
                    sizing.azimuthally_resolved,
                    "{name} θ={theta_deg}°: sizing reports azimuthally_resolved=false \
                     (n_phi={}) — a served geometry must be resolvable",
                    sizing.n_phi
                );
                assert!(
                    d_db.abs() < 0.01,
                    "{name} θ={theta_deg}°: served n_phi={} differs from 2× denser by \
                     {d_db:+.4} dB — the φ' sizing is insufficient and nothing else would \
                     catch it",
                    sizing.n_phi
                );
            }
        }
    }

    /// Gate on the replica: the per-mode decomposition must reconstruct the integrator's own
    /// total. Not `#[ignore]`d — if this drifts, the diagnostic is lying and should fail loudly.
    #[test]
    fn per_mode_decomposition_reproduces_the_integrator() {
        let config = gs_3_7m_x_band();
        let freq = 8.4e9;
        let k = wavenumber(wavelength_from_frequency(freq));
        let theta = 5.0_f64.to_radians();
        let ModeSizing { m_max, n_phi, .. } =
            mode_count_for(&config, wavelength_from_frequency(freq), theta);
        let n_rho = 129;

        let modes = per_mode_contributions(&config, theta, 0.0, k, n_rho, n_phi, m_max);
        let summed: Complex64 = modes.iter().sum();
        let direct = mode_field_at(&config, theta, 0.0, k, n_rho, n_phi, m_max);

        let rel = (summed - direct).norm() / direct.norm();
        assert!(
            rel < 1e-12,
            "per-mode replica must reproduce azimuthal_mode_field_inner: rel={rel:.3e} \
             (replica={summed:?}, direct={direct:?})"
        );
    }

    /// **P12 task 1, part A** — where does the 0.82 dB live, mode by mode, and how does the
    /// error fall as radial density rises?
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_per_mode_radial_convergence_ladder() {
        for (name, config, freq, theta_deg, phi_deg) in [
            (
                "gs_3.7m/x_band_feed",
                gs_3_7m_x_band(),
                8.4e9,
                5.0_f64,
                0.0_f64,
            ),
            ("dsn_34m/x_band", dsn_34m_x_band(), 8.45e9, 0.10, 0.0),
            ("D12 UHF fixture", d12_uhf_fixture(), 600.0e6, 16.0, 0.0),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let adaptive = IntegrationParams::adaptive();
            let b = cycle_budget(&config, theta, lambda, &adaptive);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);

            println!("\n================ {name}  θ={theta_deg}° φ={phi_deg}° ================");
            println!(
                "D/λ={:.1}  budget cycles: kernel={:.3} coma={:.3} defocus={:.3} chirp={:.3} \
                 → total={:.3}",
                config.reflector.diameter / lambda,
                b.kernel,
                b.coma,
                b.defocus,
                b.chirp,
                b.total
            );
            println!(
                "served n_rho={} (adaptive floor={}, 4·cycles={:.1})  m_max={m_max}  n_phi={n_phi}",
                b.n_rho,
                adaptive.min_rho_points,
                4.0 * b.total
            );

            // Converged reference: far past any plausible budget, same n_phi and m_max so
            // every delta below is PURELY radial.
            let reference = mode_field_at(&config, theta, phi, k, 8193, n_phi, m_max);
            let ref_modes = per_mode_contributions(&config, theta, phi, k, 8193, n_phi, m_max);

            println!("\n  n_rho | pts/cycle |  total field Δ  | per-mode field Δ (dB), m=0,1,2,…");
            for n_rho in [b.n_rho, 65, 85, 129, 257, 513, 1025, 2049] {
                let total = mode_field_at(&config, theta, phi, k, n_rho, n_phi, m_max);
                let d_total = 20.0 * (total.norm() / reference.norm()).log10();
                let modes = per_mode_contributions(&config, theta, phi, k, n_rho, n_phi, m_max);
                let per_mode: Vec<String> = modes
                    .iter()
                    .zip(ref_modes.iter())
                    .take(6)
                    .map(|(a, r)| {
                        if r.norm() == 0.0 {
                            "  --  ".to_string()
                        } else {
                            format!("{:+6.3}", 20.0 * (a.norm() / r.norm()).log10())
                        }
                    })
                    .collect();
                println!(
                    "  {n_rho:5} |   {:6.2}  |  {d_total:+8.4} dB  | {}",
                    n_rho as f64 / b.total,
                    per_mode.join(" ")
                );
            }

            // Per-mode weight, so a large per-mode dB error on a negligible mode is not
            // mistaken for the driver of the total.
            let weights: Vec<String> = ref_modes
                .iter()
                .take(6)
                .map(|r| format!("{:6.3}", r.norm() / reference.norm()))
                .collect();
            println!("  |R_m| / |I| (converged), m=0..5:  {}", weights.join(" "));
        }
    }

    /// **P12 task 1, part B** — the decisive experiment. Is radial content MISSING from
    /// `radial_points_for`'s cycle count, or is the count right and the 4-samples-per-cycle
    /// constant simply too coarse for the accuracy the mode path is expected to deliver?
    ///
    /// Measures the true radial bandwidth of each mode's integrand and compares it with the
    /// budget's predicted cycle count.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_measured_radial_bandwidth_vs_budget() {
        let config = gs_3_7m_x_band();
        let freq = 8.4e9;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let theta = 5.0_f64.to_radians();
        let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
        let b = cycle_budget(&config, theta, lambda, &IntegrationParams::adaptive());

        const N_DENSE: usize = 4096;
        const F_MAX: usize = 200;

        println!("\n=========== gs_3.7m/x_band_feed θ=5°: measured radial bandwidth ===========");
        println!(
            "budget predicts {:.3} cycles across [0,R] (kernel {:.3} + coma {:.3} + \
             defocus {:.3} + chirp {:.3}); served n_rho={} ⇒ {:.2} samples/cycle",
            b.total,
            b.kernel,
            b.coma,
            b.defocus,
            b.chirp,
            b.n_rho,
            b.n_rho as f64 / b.total
        );
        println!("\n   m | energy share | 99% BW | 99.9% BW | 99.99% BW  (cycles across [0,R])");
        for m in 0..=m_max.min(5) {
            let samples = radial_integrand_samples(&config, theta, k, m, n_phi, N_DENSE);
            let energy: f64 = samples.iter().map(|s| s.norm_sqr()).sum::<f64>().sqrt();
            let spec = windowed_spectrum(&samples, F_MAX);
            println!(
                "  {m:2} |   {energy:10.3e} |  {:5} |   {:5}  |   {:5}",
                bandwidth_containing(&spec, 0.99),
                bandwidth_containing(&spec, 0.999),
                bandwidth_containing(&spec, 0.9999),
            );
        }
    }

    /// **P12 task 1, part C** — the delivered-density asymmetry between the two branches.
    ///
    /// Both branches size themselves with the SAME `radial_points_for` budget, but the
    /// symmetric branch **returns the fine (2N) leg** of its N-vs-2N self-check
    /// (`:519-523`, `self_check` returns `fine`), while the mode branch returns the budget
    /// density N directly (`:545-551`). This measures how much of the served error that
    /// single structural difference accounts for, and asks what the symmetric branch's
    /// self-check would have reported had it been run on these numbers.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_delivered_density_asymmetry_between_branches() {
        for (name, config, freq, theta_deg) in [
            ("gs_3.7m/x_band_feed", gs_3_7m_x_band(), 8.4e9, 5.0_f64),
            ("dsn_34m/x_band", dsn_34m_x_band(), 8.45e9, 0.10),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let b = cycle_budget(&config, theta, lambda, &params);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);

            let n1 = b.n_rho;
            let n2 = radial_check_points(n1);
            let f1 = mode_field_at(&config, theta, 0.0, k, n1, n_phi, m_max);
            let f2 = mode_field_at(&config, theta, 0.0, k, n2, n_phi, m_max);
            let reference = mode_field_at(&config, theta, 0.0, k, 8193, n_phi, m_max);

            let (_, err, converged_hankel) = self_check(f1, f2, &params, HANKEL_SELF_CHECK_RTOL);
            let (_, _, converged_mode) = self_check(f1, f2, &params, MODE_SELF_CHECK_RTOL);

            println!("\n================ {name}  θ={theta_deg}° ================");
            println!(
                "  served (mode path returns N)      n={n1:5}: {:+8.4} dB vs converged",
                20.0 * (f1.norm() / reference.norm()).log10()
            );
            println!(
                "  what a 2N leg would return        n={n2:5}: {:+8.4} dB vs converged",
                20.0 * (f2.norm() / reference.norm()).log10()
            );
            println!(
                "  hypothetical radial N-vs-2N check: |Δ|/|fine| = {:.4} ⇒ \
                 converged={converged_hankel} at the 2% radial floor, \
                 converged={converged_mode} at the 0.5% mode floor",
                err / f2.norm()
            );
        }
    }

    /// **P12 task 1, part D — the control.** Parts A–C show the mode path's N leg is ~0.8–1.2 dB
    /// short while its 2N leg is within 0.05 dB. Two hypotheses survive:
    ///
    /// 1. **Mode-specific**: the mode sum cancels (per-mode `|R_m|` far exceeds `|I|`), so a
    ///    per-mode quadrature error that is small *relative to that mode* is large relative to
    ///    the total. The symmetric branch, having only `m = 0`, has no such amplification.
    /// 2. **Universal**: 4 samples/cycle is simply not enough for the accuracy this integrator
    ///    claims, on EITHER branch — and the symmetric branch only looks correct because it
    ///    returns the fine (2N) leg, i.e. it silently runs at 8 samples/cycle.
    ///
    /// These imply different fixes, so they must be told apart. The control is the same dish at
    /// the same θ with the feed moved to the focus (δ = 0), which routes to the symmetric branch
    /// and reduces the mode sum to a single term. If the symmetric N leg is ALSO ~0.8 dB short,
    /// hypothesis 2 holds and cancellation is not the driver.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_symmetric_control_at_the_same_budget() {
        let mut config = gs_3_7m_x_band();
        config.feed.position.x = 0.0; // feed at focus ⇒ symmetric branch, single mode
        let freq = 8.4e9;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let params = IntegrationParams::adaptive();

        println!("\n===== CONTROL: gs_3.7m with the feed AT FOCUS (symmetric branch) =====");
        println!(
            "  θ    | n_rho | pts/cycle | N leg vs converged | 2N leg vs converged | N-vs-2N check"
        );
        for theta_deg in [0.5_f64, 2.0, 5.0, 20.0] {
            let theta = theta_deg.to_radians();
            let b = cycle_budget(&config, theta, lambda, &params);
            let n1 = b.n_rho;
            let n2 = radial_check_points(n1);
            let f1 = hankel_radial_field(&config, theta, 0.0, k, n1, None).unwrap();
            let f2 = hankel_radial_field(&config, theta, 0.0, k, n2, None).unwrap();
            let reference = hankel_radial_field(&config, theta, 0.0, k, 16385, None).unwrap();
            let (_, err, converged) = self_check(f1, f2, &params, HANKEL_SELF_CHECK_RTOL);
            println!(
                "  {theta_deg:4}° | {n1:5} |   {:6.2}  |     {:+8.4} dB    |     {:+8.4} dB     | \
                 |Δ|/|fine|={:.4} converged={converged}",
                n1 as f64 / b.total,
                20.0 * (f1.norm() / reference.norm()).log10(),
                20.0 * (f2.norm() / reference.norm()).log10(),
                err / f2.norm(),
            );
        }
    }

    /// **P12 task 1, part E** — the cancellation-amplification measurement.
    ///
    /// Quantifies hypothesis 1 of part D directly. For each mode it reports the COMPLEX
    /// quadrature error `|R_m(N) − R_m(∞)|` normalized by the TOTAL field `|I|`, not by
    /// `|R_m|`. Magnitude-ratio dB (part A) understates this badly: a mode whose magnitude is
    /// perfect but whose phase is off by 0.02 rad contributes `0.02·|R_m|` of error, which
    /// matters enormously when `|R_m|` is 25× `|I|`.
    ///
    /// Also reports the cancellation ratio `Σ|R_m| / |I|` — the factor by which a per-mode
    /// relative error is amplified into a total relative error in the worst case. That factor
    /// is exactly what `radial_points_for` does not look at.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_mode_cancellation_amplification() {
        for (name, config, freq, theta_deg) in [
            ("gs_3.7m/x_band_feed", gs_3_7m_x_band(), 8.4e9, 5.0_f64),
            ("dsn_34m/x_band", dsn_34m_x_band(), 8.45e9, 0.10),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let b = cycle_budget(&config, theta, lambda, &params);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);

            let ref_modes = per_mode_contributions(&config, theta, 0.0, k, 8193, n_phi, m_max);
            let served = per_mode_contributions(&config, theta, 0.0, k, b.n_rho, n_phi, m_max);
            let total: Complex64 = ref_modes.iter().sum();
            let sum_abs: f64 = ref_modes.iter().map(|r| r.norm()).sum();

            println!(
                "\n================ {name}  θ={theta_deg}°  n_rho={} ================",
                b.n_rho
            );
            println!(
                "  |I| = {:.6e};  Σ|R_m| = {:.6e};  cancellation ratio Σ|R_m|/|I| = {:.1}×",
                total.norm(),
                sum_abs,
                sum_abs / total.norm()
            );
            println!(
                "\n    m | |R_m|/|I| | per-mode err |R_m(N)−R_m(∞)| as % of |R_m| | … as % of |I|"
            );
            let mut err_sum = 0.0;
            for m in 0..=m_max as usize {
                let e = (served[m] - ref_modes[m]).norm();
                err_sum += e;
                if m <= 7 {
                    println!(
                        "   {m:2} |  {:7.2}  |                       {:8.4} %          |  {:8.3} %",
                        ref_modes[m].norm() / total.norm(),
                        100.0 * e / ref_modes[m].norm().max(f64::MIN_POSITIVE),
                        100.0 * e / total.norm(),
                    );
                }
            }
            let served_total: Complex64 = served.iter().sum();
            println!(
                "  ---\n  Σ per-mode error = {:.3} % of |I|   (worst case if they aligned)\n  \
                 actual total error = {:.3} % of |I|  ⇒ {:+.4} dB",
                100.0 * err_sum / total.norm(),
                100.0 * (served_total - total).norm() / total.norm(),
                20.0 * (served_total.norm() / total.norm()).log10(),
            );

            // Which modes a "dominant-mode subset" check (P12 decision D-A option ii) would
            // select, vs which modes actually carry the error. If these rankings disagree, a
            // subset check can miss the very error it exists to catch.
            let mut by_magnitude: Vec<(usize, f64)> = ref_modes
                .iter()
                .enumerate()
                .map(|(m, r)| (m, r.norm()))
                .collect();
            by_magnitude.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut by_error: Vec<(usize, f64)> = ref_modes
                .iter()
                .enumerate()
                .map(|(m, r)| (m, (served[m] - r).norm()))
                .collect();
            by_error.sort_by(|a, b| b.1.total_cmp(&a.1));
            println!(
                "  D-A(ii) ranking check — top-5 modes by |R_m|: {:?}\n  \
                 \x20                      top-5 modes by ERROR: {:?}",
                by_magnitude.iter().take(5).map(|x| x.0).collect::<Vec<_>>(),
                by_error.iter().take(5).map(|x| x.0).collect::<Vec<_>>(),
            );
        }
    }

    /// **P12 task 1, part F — the D-B probe.** P12's decision D-B asks whether `adaptive()`'s
    /// `min_rho_points: 16` is simply wrong against `default()`'s 32, and its recommendation
    /// rests on the premise that the floor binds and "costs >1 dB there (rows 2 and 3)".
    ///
    /// This measures the premise directly: for each of P12's three measured rows it reports the
    /// `n_rho` the budget asks for, whether the floor is actually binding at 16 / 32 / 64, and
    /// what each floor delivers.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_floor_probe_for_decision_d_b() {
        for (name, config, freq, theta_deg, phi_deg) in [
            (
                "gs_3.7m/x_band_feed  ",
                gs_3_7m_x_band(),
                8.4e9,
                5.0_f64,
                0.0_f64,
            ),
            ("dsn_34m/x_band       ", dsn_34m_x_band(), 8.45e9, 0.10, 0.0),
            // P12's third row does not record φ, and this fixture's pattern is
            // φ-dependent (asymmetry_factor = 1.1), so measure both principal planes.
            (
                "D12 UHF fixture φ=0  ",
                d12_uhf_fixture(),
                600.0e6,
                16.0,
                0.0,
            ),
            (
                "D12 UHF fixture φ=90 ",
                d12_uhf_fixture(),
                600.0e6,
                16.0,
                90.0,
            ),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let reference = mode_field_at(&config, theta, phi, k, 8193, n_phi, m_max);

            let budget_only = cycle_budget(&config, theta, lambda, &IntegrationParams::adaptive());
            println!(
                "\n{name} θ={theta_deg}°: budget asks for {:.1} cycles ⇒ 4·cycles = {:.0}",
                budget_only.total,
                (4.0 * budget_only.total).ceil()
            );
            for floor in [16_usize, 32, 64] {
                let mut params = IntegrationParams::adaptive();
                params.min_rho_points = floor;
                let n = radial_points_for(&config, theta, lambda, &params);
                let field = mode_field_at(&config, theta, phi, k, n, n_phi, m_max);
                // The floor binds only when it EXCEEDS what the budget asked for. (`n` can
                // still exceed `4·cycles` by one from the odd-count adjustment, which is not
                // the floor binding.)
                let binding = floor > (4.0 * budget_only.total).ceil() as usize;
                println!(
                    "   floor {floor:3} ⇒ n_rho={n:4} (floor binding: {:5}) ⇒ {:+8.4} dB vs converged",
                    binding,
                    20.0 * (field.norm() / reference.norm()).log10(),
                );
            }
        }
    }

    /// The five geometries D-A is priced against: P12's three measured rows plus the two
    /// `dsn_34m` Ka points that set the cost ceiling (P10-perf's pathological θ=90° case and a
    /// realistic wide angle). Returns `(label, config, frequency_hz, θ°, φ°, timing reps)`.
    fn d_a_pricing_geometries() -> Vec<(&'static str, AntennaConfiguration, f64, f64, f64, usize)> {
        vec![
            ("gs_3.7m X   θ=5°   ", gs_3_7m_x_band(), 8.4e9, 5.0, 0.0, 20),
            (
                "dsn_34m X   θ=0.1° ",
                dsn_34m_x_band(),
                8.45e9,
                0.10,
                0.0,
                20,
            ),
            (
                "D12 UHF     θ=16°  ",
                d12_uhf_fixture(),
                600.0e6,
                16.0,
                0.0,
                20,
            ),
            (
                "dsn_34m Ka  θ=5°   ",
                dsn_34m_ka_band(),
                32.0e9,
                5.0,
                0.0,
                3,
            ),
            (
                "dsn_34m Ka  θ=90°  ",
                dsn_34m_ka_band(),
                32.0e9,
                90.0,
                0.0,
                1,
            ),
        ]
    }

    /// **P12 decision D-A — cost.** Wall-clock price of each candidate radial-check design,
    /// measured rather than counted, because the mode path's cost does NOT decompose the way an
    /// operation count suggests: the φ' sweep evaluates `aperture_plane_g` at `n_rho × n_phi`
    /// points independently of the mode count, so a "check only 2 of 195 modes" design is
    /// floored by the g-evaluation, not proportional to 2/195.
    ///
    /// Candidates priced (N = `radial_points_for`, 2N = `radial_check_points(N)`):
    /// - **baseline** — today: compute at N over all modes, return N, check nothing.
    /// - **(A) fine-leg only** — compute at 2N, return it, no comparison. Not on P12's option
    ///   list; task 1 found it captures ~95% of the accuracy gap on its own.
    /// - **(i) full N-vs-2N** — both legs over all modes, return 2N, compare. The honest option.
    /// - **(ii-a) subset check, return N** — full N, plus a subset recompute at 2N.
    /// - **(ii-b) subset check, return 2N** — full 2N (the accurate answer), plus a cheap subset
    ///   at N purely to detect non-convergence. The sensible form of (ii) given task 1.
    /// - **(iii) 3N, no runtime check** — validated budget with margin, checked in tests only.
    #[test]
    #[ignore = "diagnostic: prints timings, asserts nothing"]
    fn p12_price_decision_d_a_options() {
        println!(
            "\n#### D-A cost. N = radial_points_for; 2N = radial_check_points(N). \
             Subset = modes {{0,1}}.\n"
        );
        for (name, config, freq, theta_deg, phi_deg, reps) in d_a_pricing_geometries() {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let n1 = radial_points_for(&config, theta, lambda, &params);
            let n2 = radial_check_points(n1);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let subset = [0_u32, 1];

            let t_full_n = time_ms(reps, || {
                mode_field_at(&config, theta, phi, k, n1, n_phi, m_max)
            });
            let t_full_2n = time_ms(reps, || {
                mode_field_at(&config, theta, phi, k, n2, n_phi, m_max)
            });
            let t_sub_n = time_ms(reps, || {
                mode_subset_field(&config, theta, phi, k, n1, n_phi, &subset)
            });
            let t_sub_2n = time_ms(reps, || {
                mode_subset_field(&config, theta, phi, k, n2, n_phi, &subset)
            });
            let t_full_3n = time_ms(reps, || {
                mode_field_at(&config, theta, phi, k, 3 * n1 / 2 * 2 + 1, n_phi, m_max)
            });

            println!("=== {name} n_rho={n1} n_phi={n_phi} m_max={m_max} (reps={reps}) ===");
            println!(
                "  g-evaluation floor: a 2-mode sweep at N costs {:.2} ms = {:.1}% of the \
                 full N-mode sweep ({:.2} ms)",
                t_sub_n,
                100.0 * t_sub_n / t_full_n,
                t_full_n
            );
            let rows: [(&str, f64); 6] = [
                ("baseline (return N, no check)", t_full_n),
                ("(A)    fine leg only, return 2N", t_full_2n),
                ("(i)    full N-vs-2N, return 2N", t_full_n + t_full_2n),
                ("(ii-a) subset@2N check, return N", t_full_n + t_sub_2n),
                ("(ii-b) subset@N check, return 2N", t_full_2n + t_sub_n),
                ("(iii)  3N, checked in tests only", t_full_3n),
            ];
            for (label, ms) in rows {
                println!("    {label:34}  {ms:10.2} ms   {:5.2}×", ms / t_full_n);
            }
            println!();
        }
    }

    /// **P12 decision D-A — efficacy.** A cost number for a check that does not fire is
    /// worthless, so this grades the cheap candidates against the honest one, exactly as P12
    /// requires ("never ship (ii) without (i) to grade it").
    ///
    /// Each candidate produces an error *estimate*; the question is whether that estimate is
    /// large enough to trip a tolerance when the true error is large. All estimates are
    /// normalized by the returned field, which is how `self_check` gates.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_grade_decision_d_a_options() {
        println!(
            "\n#### D-A efficacy. `true err` = returned vs converged (n=8193). \
             Estimates are |Δ|/|returned|, gated at the 2% radial floor.\n"
        );
        for (name, config, freq, theta_deg, phi_deg, _) in d_a_pricing_geometries() {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let n1 = radial_points_for(&config, theta, lambda, &params);
            let n2 = radial_check_points(n1);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let subset = [0_u32, 1];

            // The Ka rows start at n_rho ≫ 8193, so a fixed reference would be COARSER than
            // the thing it is grading. Scale it: at least 4× the served density.
            let n_ref = (4 * n1 + 1).max(8193);
            let reference = mode_field_at(&config, theta, phi, k, n_ref, n_phi, m_max);
            let f_n = mode_field_at(&config, theta, phi, k, n1, n_phi, m_max);
            let f_2n = mode_field_at(&config, theta, phi, k, n2, n_phi, m_max);
            let s_n = mode_subset_field(&config, theta, phi, k, n1, n_phi, &subset);
            let s_2n = mode_subset_field(&config, theta, phi, k, n2, n_phi, &subset);

            let db = |f: Complex64| 20.0 * (f.norm() / reference.norm()).log10();
            // (i) compares full legs; (ii) compares only the subset's movement, but must
            // normalize by the RETURNED field — the subset's own magnitude is not the scale
            // the answer's accuracy is measured against.
            let est_full = (f_2n - f_n).norm() / f_2n.norm();
            let est_sub_a = (s_2n - s_n).norm() / f_n.norm();
            let est_sub_b = (s_2n - s_n).norm() / f_2n.norm();

            println!("=== {name} n_rho={n1}→{n2} m_max={m_max} (ref n={n_ref}) ===");
            println!(
                "  returned N  : true err {:+8.4} dB      | (ii-a) estimate {:7.4} ⇒ fires={}",
                db(f_n),
                est_sub_a,
                est_sub_a > HANKEL_SELF_CHECK_RTOL
            );
            println!(
                "  returned 2N : true err {:+8.4} dB      | (i)    estimate {:7.4} ⇒ fires={}",
                db(f_2n),
                est_full,
                est_full > HANKEL_SELF_CHECK_RTOL
            );
            println!(
                "                                          | (ii-b) estimate {:7.4} ⇒ fires={}",
                est_sub_b,
                est_sub_b > HANKEL_SELF_CHECK_RTOL
            );
            println!();
        }
    }

    /// Is `m_theta = k·R·sinθ + 6` a sufficient truncation, or does the `+6` margin cut into
    /// live spectrum when the coma is strong?
    ///
    /// This has to be measured against a reference that varies `m_max` ALONE — the earlier
    /// anchors derive `m_max` internally from θ, so both sides truncate identically and the
    /// axis is invisible to them. That blind spot is why the `M`-vs-`M+1` self-check, which
    /// does measure it, disagreed with them.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_mode_truncation_margin_on_a_strongly_comaed_feed() {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(1.19, 0.0, 13.6), 10.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("coma".into(), "coma".into(), reflector, feed, None).unwrap();
        let freq = 8450.0e6;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let k_r = k * 17.0;
        let bandwidth = k * 1.19 * (17.0 / 13.6);
        let n_phi = 768;
        let n_rho = 1025;

        println!(
            "\n===== mode-truncation margin (B = {bandwidth:.0}, n_phi={n_phi}, n_rho={n_rho}) ====="
        );
        for theta_deg in [1.0_f64, 3.0, 5.0, 10.0] {
            let theta = theta_deg.to_radians();
            let m_theta = (k_r * theta.sin().abs()).ceil() + 6.0;
            let m_spectrum = (1.5 * bandwidth).ceil() + 6.0;
            let served_m = m_theta.min(m_spectrum).min(MODE_M_MAX as f64) as u32;
            let full_m = m_spectrum.min((n_phi / 2 - 2) as f64) as u32;
            let reference = azimuthal_mode_field(&config, theta, 0.0, k, n_rho, n_phi, full_m);
            print!(
                "  θ={theta_deg:>5}° m_theta={m_theta:>5.0} served_m={served_m:>4} \
                 full_m={full_m:>4} |"
            );
            for m in [served_m, served_m + 16, served_m + 48, served_m + 96] {
                let m = m.min(full_m);
                let f = azimuthal_mode_field(&config, theta, 0.0, k, n_rho, n_phi, m);
                print!(
                    " m={m}:{:+7.3}",
                    20.0 * (f.norm() / reference.norm()).log10()
                );
            }
            println!("  (dB vs m={full_m})");
        }
    }

    /// Which of the three convergence axes is unhappy on the steered coma geometry, and why.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_which_axis_is_unconverged_on_the_steered_coma_geometry() {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(1.19, 0.0, 13.6), 10.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("coma".into(), "coma".into(), reflector, feed, None).unwrap();
        let freq = 8450.0e6;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let params = IntegrationParams::fast();

        println!("\n===== steered coma: which axis? =====");
        for theta_deg in [0.0_f64, 1.0, 3.0, 5.0, 10.0] {
            let theta = theta_deg.to_radians();
            let sizing = mode_count_for(&config, lambda, theta);
            let b = cycle_budget(&config, theta, lambda, &params);
            let m_probe = sizing.m_max + 1;

            // Walk the radial refinement by hand to see where it stops.
            let mut n = b.n_rho;
            let mut coarse =
                azimuthal_mode_field_inner(&config, theta, 0.0, k, n, sizing.n_phi, m_probe, None)
                    .unwrap();
            let mut legs = 1;
            let mut last_rel = f64::NAN;
            let mut radial_ok = false;
            for _ in 0..MAX_RADIAL_REFINEMENTS {
                let nf = radial_check_points(n);
                let fine = azimuthal_mode_field_inner(
                    &config,
                    theta,
                    0.0,
                    k,
                    nf,
                    sizing.n_phi,
                    m_probe,
                    None,
                )
                .unwrap();
                legs += 1;
                last_rel = (fine.total - coarse.total).norm() / fine.total.norm();
                n = nf;
                coarse = fine;
                if last_rel <= HANKEL_SELF_CHECK_RTOL {
                    radial_ok = true;
                    break;
                }
            }
            let mode_rel = coarse.top_mode.norm() / coarse.total.norm();

            println!(
                "  θ={theta_deg:>5}°  n_phi={:>5} m_max={:>4} azim_ok={:<5} | radial: n0={:>6} \
                 →{n:>6} legs={legs} rel={last_rel:.5} ok={radial_ok} | mode rel={mode_rel:.5} \
                 ok={}",
                sizing.n_phi,
                sizing.m_max,
                sizing.azimuthally_resolved,
                b.n_rho,
                mode_rel <= MODE_SELF_CHECK_RTOL
            );
        }
    }

    /// **φ'-cap fix: is `MODE_PHI_MAX = 512` the right ceiling?**
    ///
    /// Removing `MODE_PHI_STEERED_MAX` made `coma_aberration_test`'s geometry (34 m, f = 13.6,
    /// δ = 1.19 ⇒ δ/f = 0.0875, 8.45 GHz) ~65× more expensive: `n_phi` 64 → 512 and `m_max`
    /// 30 → 254. Its azimuthal bandwidth is `B = k·δ·(R/f) ≈ 263`, so Nyquist wants
    /// `n_phi > 527` — just past the ceiling, which is why it is flagged unresolved.
    ///
    /// Before accepting either the cost or the flag, this measures what the answer actually
    /// does as `n_phi` grows, with `m_max` allowed to grow with it (otherwise the two axes are
    /// confounded). It answers three things at once: whether 512 is nearly right, whether the
    /// `azimuthally_resolved` line is drawn in a defensible place, and what the old capped
    /// `n_phi = 64` was really costing.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_phi_ceiling_sufficiency_for_the_coma_test_geometry() {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(1.19, 0.0, 13.6), 10.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("coma".into(), "coma".into(), reflector, feed, None).unwrap();
        let freq = 8450.0e6;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let r = 17.0;
        let bandwidth = k * 1.19 * (r / 13.6);
        println!(
            "\n===== φ' ceiling sufficiency: B = k·δ·(R/f) = {bandwidth:.1} ⇒ Nyquist wants \
             n_phi > {:.0} (MODE_PHI_MAX = {MODE_PHI_MAX}) =====",
            2.0 * bandwidth
        );

        for theta_deg in [0.0_f64, 1.0, 3.0, 5.0, 10.0] {
            let theta = theta_deg.to_radians();
            let m_theta = (k * r * theta.sin().abs()).ceil() + 6.0;
            let m_spectrum = (1.5 * bandwidth).ceil() + 6.0;
            let m_needed = m_spectrum.min(m_theta).max(1.0);
            // Dense radially so only the azimuthal axis varies.
            let n_rho = 2049;
            let reference = {
                let n_phi = 4096;
                let m = m_needed.min((n_phi / 2 - 2) as f64) as u32;
                azimuthal_mode_field(&config, theta, 0.0, k, n_rho, n_phi, m)
            };
            print!("  θ={theta_deg:>5}° (m_needed={m_needed:>5.0}):");
            for n_phi in [64_usize, 128, 256, 512, 1024, 2048] {
                let m = m_needed.min((n_phi / 2 - 2) as f64) as u32;
                let f = azimuthal_mode_field(&config, theta, 0.0, k, n_rho, n_phi, m);
                print!(
                    "  {n_phi}:{:+8.3}",
                    20.0 * (f.norm() / reference.norm()).log10()
                );
            }
            println!("   (dB vs n_phi=4096)");
        }
    }

    /// **P12 post-fix acceptance** — what `integrate_aperture` now returns on the served path
    /// for every geometry in play: error against a converged reference, the `converged` verdict,
    /// and the work actually spent.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_post_fix_served_behaviour() {
        let p2_reflector = ReflectorGeometry::new(3.0, 1.5, 0.0005).unwrap();
        let p2_feed = FeedParameters::new(FeedPosition::new(0.6, 0.0, 1.5), 8.0, 0.0, 1.0).unwrap();
        let p2 =
            AntennaConfiguration::new("p2mod".into(), "P2".into(), p2_reflector, p2_feed, None)
                .unwrap();

        let cases: Vec<(&str, AntennaConfiguration, f64, f64, f64)> = vec![
            ("gs_3.7m X   θ=5°  ", gs_3_7m_x_band(), 8.4e9, 5.0, 0.0),
            ("dsn_34m X   θ=0.1°", dsn_34m_x_band(), 8.45e9, 0.10, 0.0),
            ("D12 UHF     θ=16° ", d12_uhf_fixture(), 600.0e6, 16.0, 0.0),
            (
                "D12 UHF     θ=16° φ=90",
                d12_uhf_fixture(),
                600.0e6,
                16.0,
                90.0,
            ),
            ("dsn_34m Ka  θ=5°  ", dsn_34m_ka_band(), 32.0e9, 5.0, 0.0),
            ("p2 steered  θ=0°  ", p2, 8.4e9, 0.0, 0.0),
        ];

        println!("\n#### Post-fix served behaviour (IntegrationParams::adaptive())\n");
        for (name, config, freq, theta_deg, phi_deg) in cases {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let n0 = radial_points_for(&config, theta, lambda, &params);
            // Reference holds n_phi and m_max fixed, so this isolates the RADIAL axis only.
            let reference = mode_field_at(&config, theta, phi, k, 32769, n_phi, m_max + 1);

            let r = integrate_aperture(theta, phi, &config, freq, &params).unwrap();
            println!(
                "  {name}: n_rho started {n0:5}, n_phi={n_phi:3}, m_max={m_max:3}\n\
                 {:22}radial error now {:+8.4} dB   converged={}   evals={}",
                "",
                20.0 * (r.field.norm() / reference.norm()).log10(),
                r.converged,
                r.num_evaluations
            );
        }
    }

    /// **P12 pre-gate yield** — how often does the `{0,1}` pre-gate actually *certify*, given
    /// `RADIAL_PRE_GATE_SAFETY`? A pre-gate that always declines costs an extra leg and buys
    /// nothing, so this is the measurement that decides whether it earns its complexity.
    ///
    /// `legs` is inferred from `num_evaluations` against the per-leg work model
    /// [`mode_sweep_work`]: a full sweep costs `n_rho · (n_phi + m_probe + 1)` and a probe leg
    /// `n_rho · (n_phi + RADIAL_PROBE_MODES.len())`.
    ///
    /// **Post-P10-perf note for unit P13.** The pre-gate's whole justification is that a full
    /// check leg is much dearer than a probe leg. The FFT narrowed that gap sharply: the mode
    /// work the probe skips used to be an `O(n_phi · M)` DFT and is now `O(M)`, so a probe leg
    /// costs `n_phi + 2` against a full leg's `n_phi + M + 1` — at `dsn_34m` Ka that is 272 vs
    /// 405, i.e. the probe now saves only ~33 % of a leg where it once saved ~80 %. P13 should
    /// re-ask whether `RADIAL_PRE_GATE_SAFETY` is worth validating or the pre-gate is worth
    /// deleting outright, with these numbers rather than the pre-FFT ones.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_pre_gate_yield_across_geometries() {
        println!("\n#### Pre-gate yield (safety factor = {RADIAL_PRE_GATE_SAFETY})\n");
        println!("  geometry                       work      pre-gate?   N     legs   evals");
        for (name, config, freq, theta_deg) in [
            ("gs_3.7m X    θ=5°  ", gs_3_7m_x_band(), 8.4e9, 5.0_f64),
            ("dsn_34m X    θ=0.1°", dsn_34m_x_band(), 8.45e9, 0.10),
            ("dsn_34m X    θ=5°  ", dsn_34m_x_band(), 8.45e9, 5.0),
            ("dsn_34m X    θ=10° ", dsn_34m_x_band(), 8.45e9, 10.0),
            ("dsn_34m X    θ=45° ", dsn_34m_x_band(), 8.45e9, 45.0),
            ("D12 UHF      θ=16° ", d12_uhf_fixture(), 600.0e6, 16.0),
            ("dsn_34m Ka   θ=1°  ", dsn_34m_ka_band(), 32.0e9, 1.0),
            ("dsn_34m Ka   θ=5°  ", dsn_34m_ka_band(), 32.0e9, 5.0),
            ("dsn_34m Ka   θ=45° ", dsn_34m_ka_band(), 32.0e9, 45.0),
            ("dsn_34m Ka   θ=90° ", dsn_34m_ka_band(), 32.0e9, 90.0),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let theta = theta_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let n0 = radial_points_for(&config, theta, lambda, &params);
            let gated = use_radial_pre_gate(n0, n_phi, m_max + 1);
            let work = (n0 as u64) * (n_phi as u64) * (m_max as u64 + 2);

            let r = integrate_aperture(theta, 0.0, &config, freq, &params).unwrap();
            // Per-leg work under `mode_sweep_work`: the opening full sweep, then either a
            // probe leg (pre-gate) or successive full sweeps at 2N−1, 4N−3, …
            let full = |n: usize| mode_sweep_work(n, n_phi, m_max as usize + 2);
            let probe = |n: usize| mode_sweep_work(n, n_phi, RADIAL_PROBE_MODES.len());
            let n1 = radial_check_points(n0);
            let n2 = radial_check_points(n1);
            let two = full(n0) + if gated { probe(n1) } else { full(n1) };
            let three = two + full(if gated { n1 } else { n2 });
            let legs = if r.num_evaluations <= two {
                2
            } else if r.num_evaluations <= three {
                3
            } else {
                4
            };
            println!(
                "  {name}  {work:>10}   {:>7}   {n0:>5}   {legs:>4}   {}",
                if gated { "yes" } else { "no" },
                r.num_evaluations
            );
        }
    }

    /// **P12 implementation check** — the `p2_moderate_offset` pin moved 16.05 → 13.72 dBi at
    /// **boresight**, a 2.3 dB change, so it has to be arbitrated by something that is not the
    /// integrator being changed. This dish is 3 m at 8.4 GHz (`D/λ = 84`), squarely in the
    /// regime where the retired 2D Simpson quadrature is a trustworthy oracle, so it can settle
    /// which value is right — and separate the radial axis from the azimuthal one, since this
    /// geometry (`δ/f = 0.4`) tripped BOTH of the former steering performance caps —
    /// `MODE_RADIAL_CYCLE_CAP` (radial cycles clamped to 8) and `MODE_PHI_STEERED_MAX`
    /// (`n_phi` clamped to 64) — until both were removed on 2026-07-31. Retained after that
    /// fix as the standing witness: the `n_phi = 64` column is what the cap used to serve, and
    /// the `n_phi ≥ 256` columns are what the geometry actually needs.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_arbitrate_p2_moderate_offset_boresight() {
        let reflector = ReflectorGeometry::new(3.0, 1.5, 0.0005).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(0.6, 0.0, 1.5), 8.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("p2mod".into(), "P2".into(), reflector, feed, None).unwrap();
        let freq = 8.4e9;
        let lambda = wavelength_from_frequency(freq);
        let k = wavenumber(lambda);
        let theta = 0.0;

        let delta = 0.6;
        let r_over_f = 1.5 / 1.5;
        println!(
            "\n===== p2_moderate_offset boresight arbitration (D/λ = {:.0}) =====",
            3.0 / lambda
        );
        println!(
            "  TRUE coma radial content = (δ/λ)(R/f) = {:.2} cycles; the former \
             MODE_RADIAL_CYCLE_CAP clamped the budget's coma term to 8.0",
            (delta / lambda) * r_over_f
        );
        let sizing = mode_count_for(&config, lambda, theta);
        println!(
            "  TRUE azimuthal bandwidth = k·δ·(R/f) = {:.1} modes; the former \
             MODE_PHI_STEERED_MAX clamped n_phi to 64 (silently). Now sized to n_phi={} \
             m_max={} azimuthally_resolved={}",
            k * delta * r_over_f,
            sizing.n_phi,
            sizing.m_max,
            sizing.azimuthally_resolved
        );

        // Independent oracle: the 2D quadrature, which has neither cap.
        let mut hi = IntegrationParams::high_accuracy();
        hi.min_rho_points = 2049;
        hi.max_rho_points = 2049;
        hi.min_phi_points = 1024;
        hi.max_phi_points = 1024;
        hi.max_iterations = 1;
        let oracle = integrate_2d_simpson_public_shim(theta, 0.0, &config, freq, &hi);
        println!(
            "\n  2D Simpson oracle (n_rho=2049, n_phi=1024): |I| = {:.6e}",
            oracle.norm()
        );

        println!("\n  mode path, sweeping BOTH axes (Δ vs oracle, dB):");
        println!("   n_rho \\ n_phi |      64 (served cap) |        256 |        512");
        for n_rho in [33_usize, 65, 129, 257, 513, 1025, 2049] {
            let cell = |n_phi: usize| {
                let f = mode_field_at(&config, theta, 0.0, k, n_rho, n_phi, 8);
                20.0 * (f.norm() / oracle.norm()).log10()
            };
            println!(
                "   {n_rho:12} | {:+18.4} | {:+10.4} | {:+10.4}",
                cell(64),
                cell(256),
                cell(512)
            );
        }
    }

    /// **P12 decision D-A — the option the measurements point at, which is not on the list.**
    ///
    /// None of P12's options iterate: each computes a fixed density (or two) and either accepts
    /// or flags the result. But the fixed-multiplier options do not deliver a uniform accuracy —
    /// task 1 measured the 2N leg at −0.0445 dB (`gs_3.7m`), −0.0553 dB (`dsn_34m`) and
    /// −0.3494 dB (D12 UHF). No single multiplier is right for all three.
    ///
    /// This prices **refine-until-converged**: double the radial density until the N-vs-2N
    /// estimate is within the 2% floor, bounded by a doubling cap (in production, by S3's
    /// existing wall-clock budget). Cost is exactly linear in `n_rho`, so `d` doublings cost
    /// `2^(d+1) − 1` baselines and the multiple can be read off without running the expensive
    /// geometries to completion.
    #[test]
    #[ignore = "diagnostic: prints measurements, asserts nothing"]
    fn p12_price_refine_until_converged() {
        const MAX_DOUBLINGS: usize = 5;
        println!(
            "\n#### D-A, emergent option: refine until the N-vs-2N estimate ≤ 2%.\n\
             #### Cost is linear in n_rho, so d doublings = (2^(d+1) − 1)× the baseline leg.\n"
        );
        for (name, config, freq, theta_deg, phi_deg, _) in
            d_a_pricing_geometries().into_iter().take(4)
        {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let phi = phi_deg.to_radians();
            let params = IntegrationParams::adaptive();
            let n_start = radial_points_for(&config, theta, lambda, &params);
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let reference = mode_field_at(&config, theta, phi, k, 32769, n_phi, m_max);

            println!("=== {name} n_rho starts at {n_start}, m_max={m_max} ===");
            let mut n = n_start;
            let mut coarse = mode_field_at(&config, theta, phi, k, n, n_phi, m_max);
            let mut legs = 1;
            for d in 1..=MAX_DOUBLINGS {
                let n_fine = radial_check_points(n);
                let fine = mode_field_at(&config, theta, phi, k, n_fine, n_phi, m_max);
                legs += 1;
                let est = (fine - coarse).norm() / fine.norm();
                let true_err = 20.0 * (fine.norm() / reference.norm()).log10();
                // Cost in baseline-leg units: legs at N, 2N, 4N… sum geometrically.
                let cost: f64 = (0..d + 1).map(|j| (1_u64 << j) as f64).sum::<f64>();
                println!(
                    "  doubling {d}: n_rho={n_fine:6}  estimate={est:7.4}  \
                     true err={true_err:+8.4} dB  cost≈{cost:5.0}×  ({legs} legs)"
                );
                if est <= HANKEL_SELF_CHECK_RTOL {
                    println!("    ⇒ converged at doubling {d}");
                    break;
                }
                n = n_fine;
                coarse = fine;
            }
            println!();
        }
    }
}

/// **P10-perf cost measurements.** Diagnostics only — nothing here asserts, because wall
/// clock is not a property a CI machine can be held to. The numbers they print are what the
/// roadmap unit is scored on; the *structural* cost guards that DO assert live in
/// `reference_validation.rs` (leg counts) and in [`tests`] (work-per-leg).
#[cfg(test)]
mod p10_perf_diagnostic {
    use super::p12_radial_diagnostic::{
        d12_uhf_fixture, dsn_34m_ka_band, dsn_34m_x_band, gs_3_7m_x_band, time_ms,
    };
    use super::*;
    use crate::model::geometry::{FeedParameters, FeedPosition, ReflectorGeometry};

    /// The `coma_aberration_test` / `test_feed_steering_large_offset` geometry: the 34 m dish
    /// with the feed steered ~5° off boresight (δ = 1.19 m, δ/f = 0.0875). Well inside the 0.5f
    /// PO scope boundary, so the model is expected to get it RIGHT — and since P12 removed the
    /// φ' cap that was hiding its cost inside a wrong answer, it is this unit's headline case:
    /// ~22 s of CPU, enough to exhaust S3's 30 s wall-clock budget and serve a 504.
    pub(super) fn steered_34m() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.00025).unwrap();
        let mut pos = FeedPosition::at_focus(13.6);
        pos.x = 1.19;
        let feed = FeedParameters::new(pos, 1.14, 0.0, 1.0).unwrap();
        AntennaConfiguration::new(
            "steered".into(),
            "Steered 34m".into(),
            reflector,
            feed,
            None,
        )
        .unwrap()
    }

    /// End-to-end served cost of one `integrate_aperture` call across the geometries this unit
    /// exists to speed up, with the sizing that drives each one.
    ///
    /// Run with:
    /// `cargo test --release -p antenna-model --lib p10_perf_served_integration_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "diagnostic: prints wall-clock measurements, asserts nothing"]
    fn p10_perf_served_integration_cost() {
        let params = IntegrationParams::adaptive();
        println!("\n#### P10-perf: served `integrate_aperture` cost (release build)\n");
        println!("  geometry                    n_phi  m_max   n_rho0    evals      ms  conv");
        for (name, config, freq, theta_deg) in [
            ("gs_3.7m X    θ=5°  ", gs_3_7m_x_band(), 8.4e9, 5.0_f64),
            ("dsn_34m X    θ=0.1°", dsn_34m_x_band(), 8.45e9, 0.10),
            ("dsn_34m X    θ=5°  ", dsn_34m_x_band(), 8.45e9, 5.0),
            ("D12 UHF      θ=16° ", d12_uhf_fixture(), 600.0e6, 16.0),
            ("steered 34m  θ=0°  ", steered_34m(), 8.45e9, 0.0),
            ("steered 34m  θ=2°  ", steered_34m(), 8.45e9, 2.0),
            ("steered 34m  θ=5°  ", steered_34m(), 8.45e9, 5.0),
            ("dsn_34m Ka   θ=1°  ", dsn_34m_ka_band(), 32.0e9, 1.0),
            ("dsn_34m Ka   θ=5°  ", dsn_34m_ka_band(), 32.0e9, 5.0),
            ("dsn_34m Ka   θ=45° ", dsn_34m_ka_band(), 32.0e9, 45.0),
            ("dsn_34m Ka   θ=90° ", dsn_34m_ka_band(), 32.0e9, 90.0),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let theta = theta_deg.to_radians();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let n0 = radial_points_for(&config, theta, lambda, &params);
            let mut result = None;
            let ms = time_ms(2, || {
                let r = integrate_aperture(theta, 0.0, &config, freq, &params).unwrap();
                result = Some(r);
                r.field
            });
            let r = result.unwrap();
            println!(
                "  {name}  {n_phi:5}  {m_max:5}  {n0:7}  {:8}  {ms:7.1}  {}",
                r.num_evaluations, r.converged
            );
        }
    }

    /// Cost of the two hot kernels in isolation, at a fixed radial density, so the FFT (φ')
    /// and Bessel-recurrence (mode) speedups can be attributed separately from the refinement
    /// loop's leg count.
    #[test]
    #[ignore = "diagnostic: prints wall-clock measurements, asserts nothing"]
    fn p10_perf_single_sweep_cost() {
        const N_RHO: usize = 1025;
        println!("\n#### P10-perf: one `azimuthal_mode_field_inner` sweep at n_rho={N_RHO}\n");
        println!("  geometry                    n_phi  m_max       ms   µs/ρ-sample");
        for (name, config, freq, theta_deg) in [
            ("gs_3.7m X    θ=5°  ", gs_3_7m_x_band(), 8.4e9, 5.0_f64),
            ("D12 UHF      θ=16° ", d12_uhf_fixture(), 600.0e6, 16.0),
            ("steered 34m  θ=2°  ", steered_34m(), 8.45e9, 2.0),
            ("dsn_34m Ka   θ=5°  ", dsn_34m_ka_band(), 32.0e9, 5.0),
            ("dsn_34m Ka   θ=90° ", dsn_34m_ka_band(), 32.0e9, 90.0),
        ] {
            let lambda = wavelength_from_frequency(freq);
            let k = wavenumber(lambda);
            let theta = theta_deg.to_radians();
            let ModeSizing { m_max, n_phi, .. } = mode_count_for(&config, lambda, theta);
            let ms = time_ms(3, || {
                azimuthal_mode_field_inner(&config, theta, 0.0, k, N_RHO, n_phi, m_max + 1, None)
                    .unwrap()
                    .total
            });
            println!(
                "  {name}  {n_phi:5}  {m_max:5}  {ms:8.2}  {:9.3}",
                ms * 1e3 / N_RHO as f64
            );
        }
    }
}
