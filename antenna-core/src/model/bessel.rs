//! In-house cylindrical Bessel functions Jₘ(x) for real argument.
//!
//! Pure Rust (no BLAS / no external crate — matches the repo's dependency rule).
//!
//! - `bessel_j0`/`bessel_j1`, **|x| < 8**: the convergent ascending power series. Exact
//!   mathematics with no fitted coefficients, accurate to ~2e-14 relative across the whole
//!   branch and *exactly* 1 at `J₀(0)`.
//! - `bessel_j0`/`bessel_j1`, **|x| >= 8**: the Numerical Recipes (Press et al.) asymptotic
//!   amplitude/phase rational approximations, accurate to **~3e-9 absolute** — a
//!   single-precision-grade fit, and the accuracy ceiling of this module for large arguments.
//!   See `j01_asymptotic_branch_absolute_accuracy_is_the_module_ceiling`, which pins that
//!   number so it is a known property rather than a surprise.
//! - `bessel_jn`/`bessel_jn_array`: a two-branch recurrence — upward for |x| > n, Miller's
//!   downward recurrence with renormalization otherwise.
//!   - **Downward branch: ~ε·(largest Jₘ in the sweep), ABSOLUTE.** A renormalized recurrence
//!     is accurate in absolute terms, so an order well below the turning-point peak is
//!     *relatively* less accurate by exactly that ratio — measured `J₂₂₀(200)`, 5.6e-16
//!     absolute but **5.1e-12 relative** against a peak of 0.0765. Size tolerances against the
//!     peak, never against the value being read: a relative bound is the one thing this branch
//!     cannot promise, and asking for it is asking a normalized recurrence for something no
//!     normalized recurrence can give.
//!   - **Upward branch:** inherits the `J₀`/`J₁` seed accuracy above — so ~1e-15 below
//!     |x| = 8 and ~3e-9 absolute at or above it.
//!
//! Validated in BOTH branches, and against an **independent** trapezoidal quadrature of
//! `Jₘ(x) = (1/2π)∫₀^{2π} cos(mτ − x sinτ) dτ` that shares no machinery with any of them —
//! see the module tests. A special-function routine that is wrong is *confidently* wrong, and
//! a routine graded only by its own recurrence identity can be uniformly mis-scaled and still
//! pass; the quadrature oracle is what rules that out.

/// Largest |x| handled by the ascending power series in [`bessel_j0`] / [`bessel_j1`].
///
/// The series converges for every x, but its terms peak at `(x/2)^(2k)/(k!)²` before decaying,
/// so the cancellation — and with it the round-off — grows with x: harmless to ~2e-14 relative
/// at |x| = 8, already ~5e-10 by |x| = 20, and useless past |x| ≈ 50. The value 8 is the
/// existing branch point, kept so the two halves of each function stay individually pinned.
///
/// **Raising this requires raising [`SERIES_MAX_TERMS`] with it** — the number of terms needed
/// to converge grows with |x| (k ≈ 23 at |x| = 8, **40 at |x| = 20**, 54 at |x| = 30), and the
/// loop bound is a silent truncation once it binds. `series_iteration_bound_covers_the_branch`
/// derives the requirement from this constant, so the two cannot drift apart unnoticed.
const SERIES_MAX_ARG: f64 = 8.0;

/// Hard iteration bound for the ascending series. The convergence break fires at k ≈ 23 for the
/// worst case (|x| just under [`SERIES_MAX_ARG`]), so this only bounds the loop — but it bounds
/// it *silently*: exhaustion and convergence are indistinguishable to the caller, which is why
/// the headroom between the two is asserted rather than assumed. See [`SERIES_MAX_ARG`].
const SERIES_MAX_TERMS: usize = 40;

/// Bessel function of the first kind, order 0.
pub fn bessel_j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < SERIES_MAX_ARG {
        // Ascending series J₀(x) = Σ_{k≥0} (−x²/4)^k / (k!)², term_k = term_{k−1}·(−x²/4)/k².
        //
        // This replaced the Numerical Recipes rational approximation (roadmap P14): that fit
        // carried ~3e-9 of absolute error across this whole branch and evaluated to
        // `1 + 2.83e-9` at x = 0, a bias every upward recurrence it seeds inherited. The
        // series is exact mathematics — nothing here is fitted — and is written in x² so the
        // evenness of J₀ is exact rather than approximate.
        let q = -0.25 * x * x;
        let mut term = 1.0_f64;
        let mut sum = 1.0_f64;
        for k in 1..=SERIES_MAX_TERMS {
            term *= q / (k * k) as f64;
            sum += term;
            // The terms peak at k ≈ x/2 and fall monotonically after; once one is below
            // f64::EPSILON it is beneath the round-off already committed (the peak term is
            // ≥ 1, so the accumulated error is ≥ ε), and the rest cannot recover it.
            if term.abs() <= f64::EPSILON {
                break;
            }
        }
        sum
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 0.785_398_164;
        let p1 = 1.0
            + y * (-0.109_862_862_7e-2
                + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let p2 = -0.156_249_999_5e-1
            + y * (0.143_048_876_5e-3
                + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 + y * (-0.934_935_152e-7))));
        (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p1 - z * xx.sin() * p2)
    }
}

/// Bessel function of the first kind, order 1.
pub fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < SERIES_MAX_ARG {
        // Ascending series J₁(x) = (x/2)·Σ_{k≥0} (−x²/4)^k / (k!(k+1)!), replacing the
        // Numerical Recipes rational approximation for the reasons given in [`bessel_j0`].
        // The leading (x/2) carries the sign, so oddness is exact and no sign correction is
        // needed on this branch.
        let q = -0.25 * x * x;
        let mut term = 1.0_f64;
        let mut sum = 1.0_f64;
        for k in 1..=SERIES_MAX_TERMS {
            term *= q / (k * (k + 1)) as f64;
            sum += term;
            if term.abs() <= f64::EPSILON {
                break;
            }
        }
        0.5 * x * sum
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356_194_491;
        let p1 = 1.0
            + y * (0.183_105e-2
                + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019_9e-6))));
        let p2 = 0.046_874_999_95
            + y * (-0.200_269_087_3e-3
                + y * (0.844_919_970_1e-5 + y * (-0.882_898_918_1e-6 + y * 0.105_787_412e-6)));
        let ans = (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p1 - z * xx.sin() * p2);
        // Asymptotic form uses |x|; J1 is odd, so restore the sign of x.
        if x < 0.0 {
            -ans
        } else {
            ans
        }
    }
}

/// Bessel function of the first kind, integer order `n >= 0`, real argument.
///
/// n=0,1 delegate to [`bessel_j0`]/[`bessel_j1`]. For n>=2 this follows the
/// standard two-branch Numerical Recipes `bessj` design, choosing the recurrence
/// direction that is numerically stable for the given argument:
///
/// - **|x| > n: UPWARD recurrence** — stable in this regime and O(n) rather than
///   O(x). Seeds `J0(|x|)`, `J1(|x|)` and steps up to order n.
/// - **|x| <= n: DOWNWARD Miller recurrence** with renormalization — the stable
///   direction when the order dominates. Here `|x| <= n` keeps the argument small,
///   so the recurrence seed reaches the decaying tail and no overflow can occur.
///
/// A fixed-offset downward-only scheme is wrong at large x: the turning-point
/// transition width grows like x^(1/3), so a constant seed offset fails to reach
/// the decaying tail and seed contamination survives (errors of tens of percent by
/// x~1e5). It also cost O(x) per call and could overflow `ax as usize`. The
/// two-branch form fixed cost and the overflow outright, and fixed accuracy for
/// `m ≪ x`; [`miller_start_offset`] (roadmap P14) fixes what was left — the
/// turning point itself, where downward is the only stable direction and the
/// offset used to be a constant.
pub fn bessel_jn(n: u32, x: f64) -> f64 {
    match n {
        0 => return bessel_j0(x),
        1 => return bessel_j1(x),
        _ => {}
    }
    if x == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let n = n as usize;

    // Non-finite argument: propagate NaN rather than panic (ax as usize would
    // otherwise be reached in the downward branch). Never occurs for finite x.
    if !ax.is_finite() {
        return f64::NAN;
    }

    let tox = 2.0 / ax;
    let ans = if ax > n as f64 {
        // UPWARD recurrence: J_{j+1}(x) = (2j/x) J_j(x) - J_{j-1}(x). Stable for x>n.
        let mut bjm = bessel_j0(ax); // J_{j-1}, starting j=1 -> J_0
        let mut bj = bessel_j1(ax); // J_j,   starting j=1 -> J_1
        for j in 1..n {
            let bjp = j as f64 * tox * bj - bjm; // J_{j+1}
            bjm = bj;
            bj = bjp;
        }
        bj
    } else {
        miller_downward(n, ax, miller_start_offset(ax))
    };

    // Jₙ(−x) = (−1)ⁿ Jₙ(x): correct the sign for negative x, odd n.
    if x < 0.0 && n % 2 == 1 {
        -ans
    } else {
        ans
    }
}

/// Lower bound on the Miller start offset — the flat value this function replaced.
///
/// Binds below `|x| ≈ (40/12)³ ≈ 37`, where the Airy width is narrower than 40 orders and a
/// flat offset was always sufficient: for `n` well above a small `x` the tail decays
/// *factorially*, not by the Airy law, so extra orders buy nothing. Keeping it as a floor
/// means no argument gets a shorter sweep — and so no worse an answer — than before P14.
const MILLER_ACC_MIN: usize = 40;

/// Width of the `Jₘ(x)` turning-point transition, in units of `x^(1/3)`, that the Miller
/// recurrence must start above for the seed to have decayed to `f64::EPSILON`.
///
/// **Derived, not fitted** (roadmap P14). Near the turning point the uniform Airy
/// approximation gives `J_{x+c·x^(1/3)}(x) ≈ (2/x)^(1/3)·Ai(2^(1/3)·c)`, so the ratio to the
/// peak `J_x(x) ≈ (2/x)^(1/3)·Ai(0)` is `Ai(2^(1/3)·c)/Ai(0)` — **independent of x**, which is
/// why one dimensionless constant covers every argument. With
/// `Ai(t) ≈ e^(−(2/3)t^(3/2)) / (2√π·t^(1/4))` and `Ai(0) = 0.35503`, requiring that ratio to
/// fall to `f64::EPSILON = 2.2e-16` gives `t ≈ 14.2`, i.e. `c = t/2^(1/3) ≈ 11.3`. Rounded up
/// to 12.
///
/// The surviving seed contamination is linear in that ratio, so this directly sets the
/// achievable accuracy. The decay law is independently confirmed by quadrature —
/// `|J_{x+c·x^(1/3)}(x)|` relative to the peak measures 3.4e-4, 5.1e-7, 2.4e-10 at c = 4, 6, 8,
/// and *the same three numbers* at x = 255, 1000 and 10⁴, which is the x-independence above
/// showing up in measurement (`turning_point_decay_law_is_x_independent`). Past c ≈ 9 the ratio
/// drops under the oracle's own ~1e-15 floor, so the constant itself is verified a different
/// way: `miller_start_offset_has_real_margin` re-runs the shipped recurrence at 3× this offset
/// and requires the answer not to move.
///
/// **Do not tune this against a measured error table.** It is derived from a decay
/// requirement; if a measurement disagrees, the derivation or the error model is what needs
/// revisiting. (That is the P13 lesson: a constant fitted to measurements is coupled to every
/// input of those measurements, including ones nobody thinks of as inputs.)
const MILLER_TURNING_WIDTH: f64 = 12.0;

/// Extra recurrence steps started above the highest wanted order in the downward (Miller)
/// branch, so the arbitrary seed has decayed into negligibility by the time the recurrence
/// reaches an order we keep.
///
/// Shared by [`bessel_jn`] and [`bessel_jn_array`] so the two cannot drift apart — they must
/// agree to be interchangeable, which `jn_array_matches_the_per_order_function` requires.
fn miller_start_offset(ax: f64) -> usize {
    let derived = MILLER_TURNING_WIDTH * ax.cbrt();
    // `ax <= n <= u32::MAX` on this branch, so `derived <= 12·1626 ≈ 19_512`: no overflow.
    (derived.ceil() as usize).max(MILLER_ACC_MIN)
}

/// `J_n(|x|)` by Miller's downward recurrence, started `acc` orders above `n`.
///
/// `J_{m-1}(x) = (2m/x)·J_m(x) − J_{m+1}(x)`, run downward from an arbitrary seed and rescaled
/// at the end by `1 = J₀ + 2·(J₂ + J₄ + …)`. Only called for `|x| <= n`, so `n` bounds the
/// order range and no overflow is possible.
///
/// `acc` is a parameter rather than a call to [`miller_start_offset`] for one reason: it lets
/// `miller_start_offset_has_real_margin` re-run the *same* code at a larger offset and check
/// that the answer does not move. A margin measured against a copy of the recurrence would
/// prove nothing about this one.
fn miller_downward(n: usize, ax: f64, acc: usize) -> f64 {
    let tox = 2.0 / ax;
    let big = 1.0e10_f64;
    let bigi = 1.0e-10_f64;

    let m_start = 2 * ((n + acc) / 2 + 1); // force even
    let mut jn = 0.0_f64; // captured J_n (in the unnormalized seed scale)
    let mut bjp = 0.0_f64; // J_{m+1}
    let mut bj = 1.0_f64; // J_m (arbitrary seed; renormalized at the end)
    let mut sum = 0.0_f64;
    let mut jsum = false; // toggles each step; true selects the even-order terms
    for m in (1..=m_start).rev() {
        let bjm = m as f64 * tox * bj - bjp; // J_{m-1}
        bjp = bj;
        bj = bjm;
        if bj.abs() > big {
            // Renormalize to avoid overflow.
            bj *= bigi;
            bjp *= bigi;
            jn *= bigi;
            sum *= bigi;
        }
        if jsum {
            sum += bj; // accumulates the even-order Bessel values
        }
        jsum = !jsum;
        if m == n {
            jn = bjp; // capture J_n as we pass it
        }
    }
    // Normalization: 1 = J0 + 2*(J2 + J4 + ...); `sum` here is J0 + 2ΣJ_even
    // after undoing the double count of J0 (the last-added term, bj).
    sum = 2.0 * sum - bj;
    jn / sum
}

/// Every order `J_0(x) … J_{m_max}(x)` in ONE recurrence sweep — `O(m_max)` total, not
/// `O(m_max)` per order (roadmap P10-perf).
///
/// The azimuthal-mode aperture integrator needs the whole ladder of orders at a single
/// argument `a = kρ·sinθ`, once per radial sample. Calling [`bessel_jn`] per order re-runs a
/// recurrence from scratch each time, making that `O(m_max²)` — ~32 000 recurrence steps per
/// radial sample at the served `m_max = 254`, several thousand radial samples deep. The
/// recurrences below already compute every intermediate order on the way to the top one; this
/// simply keeps them.
///
/// Fills `out[0..=m_max]`. Branch selection mirrors [`bessel_jn`] exactly, applied to the
/// **highest** wanted order so that every order in the array is in the stable regime:
///
/// - **`|x| > m_max`: upward recurrence.** Stable for all `m < |x|`, which the branch
///   condition guarantees for the whole array.
/// - **`|x| <= m_max`: downward Miller recurrence** with the standard `J_0 + 2ΣJ_even = 1`
///   normalization, capturing every order on the way down. Unlike the per-order call, the
///   *whole* array comes from a single seed, so low orders are computed from a start point far
///   above them — if anything more converged than the per-order path, never less.
///
/// # Panics
/// If `out.len() <= m_max`.
pub fn bessel_jn_array(m_max: u32, x: f64, out: &mut [f64]) {
    let top = m_max as usize;
    assert!(
        out.len() > top,
        "bessel_jn_array: output buffer holds {} values, need {}",
        out.len(),
        top + 1
    );
    let out = &mut out[..=top];

    let ax = x.abs();
    // Non-finite argument: propagate NaN rather than panic, matching `bessel_jn`.
    if !ax.is_finite() {
        out.fill(f64::NAN);
        return;
    }
    if ax == 0.0 {
        // J_0(0) = 1, J_m(0) = 0 for m > 0.
        out.fill(0.0);
        out[0] = 1.0;
        return;
    }

    if ax > top as f64 {
        // UPWARD: J_{j+1}(x) = (2j/x)·J_j(x) − J_{j-1}(x). Stable while the order stays below
        // the argument, which `ax > top` guarantees for every order written here.
        let tox = 2.0 / ax;
        out[0] = bessel_j0(ax);
        if top >= 1 {
            out[1] = bessel_j1(ax);
        }
        for j in 1..top {
            out[j + 1] = j as f64 * tox * out[j] - out[j - 1];
        }
    } else {
        // DOWNWARD (Miller) with renormalization. Identical recurrence and normalization sum
        // to `bessel_jn`'s downward branch; the only difference is that the orders passed on
        // the way down are kept instead of discarded.
        let tox = 2.0 / ax;
        let big = 1.0e10_f64;
        let bigi = 1.0e-10_f64;
        let m_start = 2 * ((top + miller_start_offset(ax)) / 2 + 1); // force even

        out.fill(0.0);
        let mut bjp = 0.0_f64; // J_{m+1}
        let mut bj = 1.0_f64; // J_m (arbitrary seed; renormalized at the end)
        let mut sum = 0.0_f64;
        let mut jsum = false; // toggles each step; true selects the even-order terms
        for m in (1..=m_start).rev() {
            let bjm = m as f64 * tox * bj - bjp; // J_{m-1}
            bjp = bj;
            bj = bjm;
            if bj.abs() > big {
                // Renormalize to avoid overflow. Unlike the per-order call, the orders already
                // captured are part of the running scale and must be rescaled with it.
                bj *= bigi;
                bjp *= bigi;
                sum *= bigi;
                for v in out.iter_mut() {
                    *v *= bigi;
                }
            }
            if jsum {
                sum += bj; // accumulates the even-order Bessel values
            }
            jsum = !jsum;
            if m <= top {
                out[m] = bjp; // bjp is J_m after the shift
            }
        }
        out[0] = bj;
        // 1 = J0 + 2·(J2 + J4 + …); `sum` here is J0 + 2ΣJ_even after undoing the double
        // count of J0 (the last-added term, bj).
        sum = 2.0 * sum - bj;
        for v in out.iter_mut() {
            *v /= sum;
        }
    }

    // Jₘ(−x) = (−1)ᵐ Jₘ(x): the recurrences above ran on |x|.
    if x < 0.0 {
        for v in out.iter_mut().skip(1).step_by(2) {
            *v = -*v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from Abramowitz & Stegun / standard tables.
    const TOL: f64 = 1e-6;

    /// **Independent oracle** for `Jₘ(x)`, sharing no machinery with anything in this module.
    ///
    /// `Jₘ(x) = (1/2π)∫₀^{2π} cos(mτ − x·sinτ) dτ` for integer m (Bessel's own integral). The
    /// integrand is smooth and periodic, so the trapezoidal rule converges *spectrally* — with
    /// `n ≫ 2(m + x)` samples it reaches machine precision, and it uses no recurrence, no
    /// rational fit and no normalization sum. That independence is the point: this module's
    /// other graders are recurrence identities, which any uniformly mis-scaled result satisfies
    /// exactly, and Miller's algorithm fails precisely by mis-scaling.
    ///
    /// **Absolute noise floor ≈ 1e-16.** The integrand is O(1) while the answer can be far
    /// smaller, so the quadrature cancels away most of its own magnitude; no summation scheme
    /// recovers digits below that. Tests must *skip* values under it, never loosen a tolerance
    /// to cover them. Self-convergence checked in `quadrature_oracle_converges_on_itself`.
    fn jm_by_quadrature(m: u32, x: f64) -> f64 {
        jm_by_quadrature_with(m, x, quadrature_n(m, x))
    }

    /// The sample count [`jm_by_quadrature`] actually uses, exposed so
    /// `quadrature_oracle_converges_on_itself` can grade *that* configuration against a
    /// refinement of it rather than against a number it hopes is different.
    fn quadrature_n(m: u32, x: f64) -> usize {
        (16 * (m as usize + x.abs() as usize + 8))
            .next_power_of_two()
            .max(1024)
    }

    /// [`jm_by_quadrature`] with an explicit sample count, so convergence can be demonstrated
    /// by varying it rather than asserted.
    fn jm_by_quadrature_with(m: u32, x: f64, n_min: usize) -> f64 {
        let n = n_min.next_power_of_two().max(1024);
        let step = 2.0 * std::f64::consts::PI / n as f64;
        // Neumaier compensated summation. Plain accumulation costs ~3e-15 here — the partial
        // sums reach ~n/2 while the answer is O(1), so the round-off is amplified by exactly
        // the cancellation the integral performs. That was measured, not assumed: it is what
        // made this oracle disagree with itself at m=0, x=1.
        let (mut sum, mut comp) = (0.0_f64, 0.0_f64);
        for i in 0..n {
            let tau = i as f64 * step;
            // `m·τ` reduced mod 2π *exactly*, in integer arithmetic, before it is formed as a
            // float: at m = 1000 the unreduced value reaches 6283, where the ulp alone is
            // 1.4e-12 of phase and the cosine inherits all of it. `(m·i) mod n` costs nothing
            // and removes that entire error term. What remains — `x·sin τ`, absolute error
            // ~x·ε — is irreducible, and is why this oracle's floor grows with x.
            let phase = ((m as usize * i) % n) as f64 * step;
            let term = (phase - x * tau.sin()).cos();
            let t = sum + term;
            comp += if sum.abs() >= term.abs() {
                (sum - t) + term
            } else {
                (term - t) + sum
            };
            sum = t;
        }
        (sum + comp) / n as f64
    }

    /// The oracle must be shown to have converged before anything is graded against it.
    ///
    /// Grades the **default** configuration — the `n` that [`jm_by_quadrature`] actually uses,
    /// which is what every other test here is graded against — by refining it 4×. An earlier
    /// version derived both counts from `base` and `base / 2`, which for small `m + x` both
    /// clamped to the `max(1024)` floor: the two calls were *identical*, the difference was
    /// exactly 0, and the test had no power over the entire `|x| < 8` range where P14 rewrote
    /// `J₀`/`J₁`. `assert_ne!` on the two counts is what keeps that from coming back.
    #[test]
    fn quadrature_oracle_converges_on_itself() {
        // Small arguments included deliberately: they sit on the sample-count floor, they are
        // where the new series branch is graded, and they are exactly what the vacuous version
        // of this test was blind to.
        for &(m, x) in &[
            (0u32, 0.0_f64),
            (0, 1.0),
            (0, 7.9),
            (1, 0.5),
            (1, 7.9),
            (5, 10.0),
            (255, 255.0),
            (1000, 1000.0),
        ] {
            let n = quadrature_n(m, x);
            let refined_n = 4 * n;
            assert_ne!(n, refined_n, "the refinement must actually refine");
            let default = jm_by_quadrature(m, x);
            let refined = jm_by_quadrature_with(m, x, refined_n);
            // `1e-16 + x·ε` — the constant is the discretization (spectrally converged, so
            // effectively zero) and the x-scaled term is the irreducible floor named in
            // `jm_by_quadrature_with`: forming `x·sin τ` commits an absolute phase error of
            // ~x·ε that no sample count removes. Measured across this grid: 0 at x ≤ 7.9
            // (bit-identical), 2.8e-17 at x = 0.5, 5.6e-17 at x = 255, 2.1e-17 at x = 1000.
            let floor = 1e-16 + x * f64::EPSILON;
            assert!(
                (default - refined).abs() < floor,
                "oracle not converged at m={m}, x={x}: {default:.17e} (n={n}) vs \
                 {refined:.17e} (n={refined_n}), floor {floor:.2e}"
            );
        }
    }

    #[test]
    fn j0_small_argument_branch() {
        assert!((bessel_j0(0.0) - 1.0).abs() < TOL);
        assert!((bessel_j0(1.0) - 0.765_197_686_5).abs() < TOL);
        assert!((bessel_j0(2.404_825_558) - 0.0).abs() < 1e-6); // first zero of J0
        assert!((bessel_j0(5.0) - (-0.177_596_771_3)).abs() < TOL);
    }

    #[test]
    fn j0_asymptotic_branch() {
        // |x| >= 8 exercises the asymptotic polynomial (the branch the spike got
        // right by luck while J0 small-arg was wrong — pin it explicitly).
        assert!((bessel_j0(10.0) - (-0.245_935_764_5)).abs() < TOL);
        assert!((bessel_j0(20.0) - 0.167_024_664_5).abs() < TOL);
    }

    #[test]
    fn j0_is_even() {
        assert!((bessel_j0(-3.3) - bessel_j0(3.3)).abs() < 1e-12);
    }

    /// [`SERIES_MAX_TERMS`] must have headroom over what [`SERIES_MAX_ARG`] actually needs.
    ///
    /// The two constants are coupled and nothing else checks it: the loop bound truncates
    /// *silently*, so a branch point raised without the term count raised with it would return
    /// a partial sum that looks like a converged one. At |x| = 20 — a value `SERIES_MAX_ARG`'s
    /// own doc quotes an accuracy for, and so an inviting place to move the branch — the break
    /// fires on iteration 40, which is `SERIES_MAX_TERMS` exactly: zero margin.
    ///
    /// Derived from the constants rather than hard-coded, so raising either one re-runs the
    /// check. Graded on `J₀`, whose `1/(k!)²` denominator makes it the binding case (`J₁`'s
    /// `1/(k!(k+1)!)` converges strictly sooner).
    #[test]
    fn series_iteration_bound_covers_the_branch() {
        // The worst case on the branch is |x| just under SERIES_MAX_ARG.
        let q = 0.25 * SERIES_MAX_ARG * SERIES_MAX_ARG;
        let mut term = 1.0_f64;
        let mut fired_at = None;
        for k in 1..=SERIES_MAX_TERMS {
            term *= q / (k * k) as f64;
            if term <= f64::EPSILON {
                fired_at = Some(k);
                break;
            }
        }
        let k = fired_at.unwrap_or_else(|| {
            panic!(
                "the ε-break never fires within SERIES_MAX_TERMS = {SERIES_MAX_TERMS} at \
                 |x| = {SERIES_MAX_ARG}: bessel_j0/j1 silently return a truncated series"
            )
        });
        // Headroom, not just sufficiency: landing on the last iteration is the failure this
        // guards against, one increment away.
        assert!(
            k + 8 <= SERIES_MAX_TERMS,
            "the ε-break fires at k={k} against a bound of {SERIES_MAX_TERMS} — too tight. \
             Raise SERIES_MAX_TERMS alongside SERIES_MAX_ARG = {SERIES_MAX_ARG}."
        );
    }

    /// **`J₀(0)` is exactly 1** — roadmap P14.
    ///
    /// The Numerical Recipes rational approximation this branch replaced evaluated to
    /// `1 + 2.83e-9` here, and every upward recurrence it seeds inherited that bias. Asserted
    /// with `==`, deliberately: the series' first term *is* 1 and every later term carries a
    /// factor of x², so exactness is a structural property of the algorithm, not a rounding
    /// coincidence that a tolerance would let quietly regress.
    #[test]
    fn j0_is_exactly_one_at_the_origin() {
        assert_eq!(bessel_j0(0.0), 1.0);
        assert_eq!(bessel_j0(-0.0), 1.0);
        assert_eq!(bessel_j1(0.0), 0.0);
    }

    /// The **series** branch (`|x| < 8`) of `J₀`/`J₁`, graded against the quadrature oracle.
    ///
    /// Worst measured relative error across a 0.01-spaced sweep: **1.9e-13** for `J₀`,
    /// 6.8e-14 for `J₁` — against ~5.8e-7 relative (~3e-9 absolute) from the rational
    /// approximation P14 replaced. Points near a zero of the function are graded absolutely,
    /// since a relative test there measures the zero's location rather than the routine.
    #[test]
    fn j01_series_branch_matches_the_quadrature_oracle() {
        let (mut worst0, mut worst1) = (0.0_f64, 0.0_f64);
        for i in 0..800 {
            let x = i as f64 * 0.01;
            let (r0, r1) = (jm_by_quadrature(0, x), jm_by_quadrature(1, x));
            let graded =
                |got: f64, reference: f64| (got - reference).abs() / reference.abs().max(1e-2);
            worst0 = worst0.max(graded(bessel_j0(x), r0));
            worst1 = worst1.max(graded(bessel_j1(x), r1));
        }
        assert!(
            worst0 < 1e-12 && worst1 < 1e-12,
            "series branch degraded: J₀ worst {worst0:.2e}, J₁ worst {worst1:.2e}"
        );
    }

    /// The **asymptotic** branch (`|x| >= 8`) is a single-precision-grade rational fit, and
    /// that is this module's accuracy ceiling for large arguments.
    ///
    /// Pinned so the split is a known property rather than a surprise: P14 made `|x| < 8`
    /// exact-mathematics-accurate (~1e-14) and deliberately left this branch alone. Replacing
    /// it is not a matter of adding series terms — the Hankel asymptotic expansion's smallest
    /// term at `x = 8` is itself ~2e-8, so it cannot beat what is here; it would take a
    /// genuine Chebyshev minimax fit, which is a different unit and buys ~2.6e-8 dB.
    ///
    /// Measured worst **absolute** error over `8 <= x < 200`: 8.6e-9 (`J₀`), 8.9e-9 (`J₁`).
    #[test]
    fn j01_asymptotic_branch_absolute_accuracy_is_the_module_ceiling() {
        let (mut worst0, mut worst1) = (0.0_f64, 0.0_f64);
        for i in 800..20_000 {
            let x = i as f64 * 0.01;
            worst0 = worst0.max((bessel_j0(x) - jm_by_quadrature(0, x)).abs());
            worst1 = worst1.max((bessel_j1(x) - jm_by_quadrature(1, x)).abs());
        }
        assert!(
            worst0 < 5e-8 && worst1 < 5e-8,
            "asymptotic branch worse than documented: J₀ {worst0:.2e}, J₁ {worst1:.2e}"
        );
        // It is genuinely worse than the series branch — if this ever fails because the
        // asymptotic branch got *better*, the module header and this test need updating
        // together, which is the point of asserting it.
        assert!(
            worst0 > 1e-11,
            "asymptotic branch is now {worst0:.2e}: better than documented, so update the docs"
        );
    }

    #[test]
    fn j1_both_branches() {
        assert!((bessel_j1(0.0) - 0.0).abs() < TOL);
        assert!((bessel_j1(1.0) - 0.440_050_585_7).abs() < TOL);
        assert!((bessel_j1(5.0) - (-0.327_579_137_9)).abs() < TOL); // small-arg
        assert!((bessel_j1(10.0) - 0.043_472_746_2).abs() < TOL); // asymptotic
    }

    #[test]
    fn j1_is_odd() {
        assert!((bessel_j1(-2.5) + bessel_j1(2.5)).abs() < 1e-12);
    }

    #[test]
    fn jn_matches_j0_j1() {
        for &x in &[0.5, 1.0, 5.0, 9.0, 15.0] {
            assert!((bessel_jn(0, x) - bessel_j0(x)).abs() < 1e-9, "n=0 x={x}");
            assert!((bessel_jn(1, x) - bessel_j1(x)).abs() < 1e-9, "n=1 x={x}");
        }
    }

    #[test]
    fn jn_negative_x_symmetry() {
        // Jn(-x) = (-1)^n Jn(x): exercises the n>=2 sign path across both branches
        // (x=2.5<=4 downward; x=7,12>4 upward for n=4).
        for &x in &[2.5, 7.0, 12.0] {
            assert!(
                (bessel_jn(4, -x) - bessel_jn(4, x)).abs() < 1e-12,
                "J4 even n: x={x}"
            ); // even n
            assert!(
                (bessel_jn(5, -x) + bessel_jn(5, x)).abs() < 1e-12,
                "J5 odd n: x={x}"
            ); // odd n
        }
    }

    #[test]
    fn jn_known_values() {
        // J2(5)=0.046565..., J3(10)=0.058379..., J5(10)=-0.234061...
        assert!((bessel_jn(2, 5.0) - 0.046_565_116_3).abs() < TOL);
        assert!((bessel_jn(3, 10.0) - 0.058_379_379_3).abs() < TOL);
        assert!((bessel_jn(5, 10.0) - (-0.234_061_528_2)).abs() < TOL);
    }

    #[test]
    fn jn_large_x_recurrence_identity() {
        // Primary large-x guard: the exact three-term recurrence
        //   (2n/x) Jn(x) = J_{n-1}(x) + J_{n+1}(x)
        // holds identically for every real x. It is independent of any table and
        // directly exercises the UPWARD-recurrence branch (x > n), which the old
        // downward-only scheme got badly wrong (errors of tens of percent by x~1e5).
        for &x in &[1.0e4_f64, 5.0e4, 1.0e5] {
            let n = 5u32;
            let lhs = (2.0 * n as f64 / x) * bessel_jn(n, x);
            let rhs = bessel_jn(n - 1, x) + bessel_jn(n + 1, x);
            let scale = rhs.abs().max(1e-12);
            assert!(
                (lhs - rhs).abs() < scale * 1e-9,
                "recurrence identity broken at x={x}: lhs={lhs}, rhs={rhs}"
            );
        }
    }

    #[test]
    fn jn_large_x_absolute_values() {
        // Absolute-value guard at large x. References computed independently via the
        // Hankel large-argument asymptotic expansion (validated against A&S table
        // values J0(10)/J2(10)/J5(10) to ~1e-8). NOTE: the coordinator's quoted
        // J2(1000)=4.6596e-4 was a wrong hand-estimate; the true value is
        // -2.477_722_95e-2 (near the envelope max sqrt(2/(pi·1000))≈2.52e-2).
        let cases = [
            (2u32, 1000.0_f64, -2.477_722_952_861e-2),
            (5, 1.0e4, 3.638_932_738_307e-3),
            (5, 1.0e5, 1.846_551_245_453e-3),
        ];
        for &(n, x, reference) in &cases {
            let v = bessel_jn(n, x);
            let rel = (v - reference).abs() / reference.abs();
            eprintln!("J{n}({x}) = {v:.12e}  ref = {reference:.12e}  rel_err = {rel:.2e}");
            assert!(
                rel < 1e-6,
                "J{n}({x}) = {v}, expected ≈ {reference} (rel_err {rel})"
            );
        }
    }

    #[test]
    fn jn_nonfinite_argument_is_nan_not_panic() {
        // "Panic-free for any finite f64" plus graceful handling of ±inf/NaN:
        // must not panic (the old `ax as usize` overflowed for |x| >~ 1.8e19).
        assert!(bessel_jn(3, f64::INFINITY).is_nan());
        assert!(bessel_jn(3, f64::NEG_INFINITY).is_nan());
        assert!(bessel_jn(3, f64::NAN).is_nan());
        // A very large finite argument beyond usize range must not panic and must
        // stay within the physical envelope sqrt(2/(pi x)).
        let big = 1.0e20_f64;
        let v = bessel_jn(3, big);
        assert!(v.is_finite() && v.abs() < 1.0, "got {v}");
    }

    /// **Turning-point coverage at high order** (roadmap P10-perf, filed by the P10 review).
    ///
    /// `Jₘ(x)` has an Airy-type turning point at `m ≈ x`, where neither recurrence direction is
    /// comfortable and where the two branches of [`bessel_jn`] meet. The served integrator sits
    /// squarely here: `m` runs to 254 while `a = kρ·sinθ` sweeps through that whole range as ρ
    /// goes from 0 to R, so every radial sweep crosses the turning point. Until this test the
    /// pinned orders stopped at `m = 5`.
    ///
    /// Graded by two table-free identities that hold for every real `x` and integer `m`, so
    /// neither can be satisfied by a matching pair of errors in the implementation and a
    /// hand-copied reference:
    ///   1. the three-term recurrence `(2m/x)·Jₘ = J_{m−1} + J_{m+1}`, and
    ///   2. the Debye envelope `|Jₘ(x)| ≲ sqrt(2/(π·sqrt(x²−m²)))` below the turning point,
    ///      plus `|Jₘ(x)| ≤ 1` everywhere and decay well above it.
    ///
    /// Both are needed: the recurrence identity is **scale-invariant**, so it is satisfied by a
    /// uniformly mis-normalized array — exactly the failure mode a Miller recurrence has. The
    /// magnitude bounds are what pin the scale. (Note the envelope is the *Debye* form, not the
    /// flat `sqrt(2/(πx))`: near the turning point the true values exceed the flat bound — e.g.
    /// `J₉(10) = 0.2919` against `sqrt(2/(10π)) = 0.2523` — so a flat bound would fail on
    /// correct values.)
    #[test]
    fn jn_high_order_near_the_turning_point() {
        use std::f64::consts::PI;
        // Bounded at 300 deliberately: `MODE_M_MAX` caps the served integrator at order 254.
        // The tolerance below stays at 1e-7 even though P14 made the downward branch
        // machine-precision, because this grid includes `m = x` exactly, where `J_{m−1}`
        // crosses to the seed-limited upward branch — see
        // `jn_upward_branch_inherits_the_j0_j1_seed_accuracy`. The all-downward closure is
        // pinned at 1e-14 by `jn_turning_point_closure_is_machine_precision_at_every_argument`.
        for &x in &[10.0_f64, 50.0, 120.0, 200.0, 255.0, 300.0] {
            // Straddle the turning point m ≈ x, and include the high orders the integrator
            // actually reaches.
            let orders: Vec<u32> = [0.5, 0.9, 0.99, 1.0, 1.01, 1.1, 1.5, 2.0]
                .iter()
                .map(|f| (x * f).round().max(1.0) as u32)
                .chain([200u32, 254])
                .collect();
            for m in orders {
                let jm = bessel_jn(m, x);
                let jm1 = bessel_jn(m - 1, x);
                let jp1 = bessel_jn(m + 1, x);
                let mf = m as f64;
                assert!(jm.is_finite(), "J{m}({x}) = {jm} is not finite");
                assert!(jm.abs() <= 1.0, "J{m}({x}) = {jm} breaks |Jₘ| ≤ 1");
                if mf < x {
                    let envelope = (2.0 / (PI * (x * x - mf * mf).sqrt())).sqrt() * 1.1;
                    assert!(
                        jm.abs() <= envelope,
                        "J{m}({x}) = {jm} exceeds the Debye envelope {envelope}"
                    );
                }
                if mf >= 1.5 * x {
                    // Well past the turning point Jₘ decays super-exponentially. A Miller
                    // recurrence that failed to normalize would land here at O(1).
                    assert!(
                        jm.abs() < 0.1,
                        "J{m}({x}) = {jm} is not decaying past the turning point"
                    );
                }
                let lhs = (2.0 * mf / x) * jm;
                let rhs = jm1 + jp1;
                // Scale by the largest term in the identity: near a zero of Jₘ the two sides
                // are both tiny and a relative test on their own magnitude is meaningless.
                //
                // 1e-7, not machine precision, and the difference is a MEASUREMENT. Before P14
                // it bounded the Miller seed: at `m = x = 255` the downward branch started a
                // flat 40 orders up, where `J₂₉₆(255)/J₂₅₅(255)` is still ~4e-5, so the seed
                // had not decayed and the identity closed to 2e-8. P14 fixed that (it is now
                // ~3e-16 — see the all-downward pin), but this grid deliberately keeps `m = x`,
                // where `J_{m−1}` crosses to the upward branch and inherits the ~3e-9 absolute
                // error of `bessel_j0`/`bessel_j1`'s asymptotic fit. So 1e-7 still buys real
                // headroom over a real ~6e-10 measurement — just over a different one.
                let scale = lhs.abs().max(jm1.abs()).max(jp1.abs()).max(1e-300);
                assert!(
                    (lhs - rhs).abs() <= scale * 1e-7,
                    "recurrence identity broken at m={m}, x={x}: {lhs} vs {rhs}"
                );
            }
        }
    }

    /// **The turning point is accurate at every argument** — roadmap **P14**.
    ///
    /// This test used to be named `..._degrades_far_above_the_served_order_ceiling` and pinned
    /// a defect: with a flat 40-order Miller start offset, the closure of
    /// `(2m/x)·Jₘ = J_{m−1} + J_{m+1}` at `m = x` degraded as x grew, because the turning-point
    /// transition width grows like `x^(1/3)` and a constant offset stops reaching the decayed
    /// tail. Measured then (left column) against now, after [`miller_start_offset`] made the
    /// offset scale with that width:
    ///
    /// | x | 255 | 400 | 700 | 1000 | 3000 | 10000 |
    /// |---|-----|-----|-----|------|------|-------|
    /// | before | 2e-8 | 2e-7 | 4e-6 | 2e-5 | 9e-4 | **9e-3** |
    /// | after | 4e-16 | 6e-16 | 6e-16 | 3e-16 | 2e-16 | 5e-16 |
    ///
    /// The x-dependence is gone, which is the property worth pinning: the old numbers grew
    /// with x without bound, so the routine's accuracy depended on how large an argument a
    /// caller happened to reach. The tolerance below is therefore **flat**, not a table — a
    /// per-x tolerance is exactly what a scale-dependent defect looks like.
    ///
    /// Evaluated at `m = x + 1` so all three orders sit on the downward branch. At `m = x`
    /// exactly, `J_{m−1}` crosses to the *upward* branch and the closure is limited to ~6e-10
    /// by the `J₀`/`J₁` seeds instead — a different, flat, documented ceiling
    /// (`jn_upward_branch_inherits_the_j0_j1_seed_accuracy`), and not what P14 was about.
    #[test]
    fn jn_turning_point_closure_is_machine_precision_at_every_argument() {
        for &x in &[255.0_f64, 400.0, 700.0, 1000.0, 3000.0, 10000.0] {
            let m = x as u32 + 1;
            let lhs = (2.0 * m as f64 / x) * bessel_jn(m, x);
            let rhs = bessel_jn(m - 1, x) + bessel_jn(m + 1, x);
            let rel = (lhs - rhs).abs() / lhs.abs().max(1e-300);
            assert!(
                rel <= 1e-14,
                "J_{m}({x}) turning-point closure is {rel:.3e}, expected machine precision"
            );
        }
    }

    /// **[`MILLER_TURNING_WIDTH`] must have real margin, measured rather than argued.**
    ///
    /// This is the test P13 said to write. `RADIAL_PRE_GATE_SAFETY` rotted silently because it
    /// guarded a runtime property and nothing asserted its *margin* — so a change with no
    /// physics content moved the quantity underneath it and the build could not notice. The
    /// constant here is exposed the same way, so its margin is asserted directly rather than
    /// inferred from the accuracy tests above passing.
    ///
    /// **Method: re-run the shipped recurrence with a larger offset and require the answer not
    /// to move.** That is the margin itself, with no theory in the loop — if the shipped offset
    /// were even marginally short, doubling it would shift the result. Deliberately *not*
    /// graded against the quadrature oracle: the seed order's true magnitude at the shipped
    /// offset is ~1e-17 of the peak, which is below the oracle's resolution, so the oracle can
    /// confirm the decay law but cannot confirm this constant.
    #[test]
    fn miller_start_offset_has_real_margin() {
        for &x in &[
            64.0_f64, 128.0, 255.0, 512.0, 1000.0, 2000.0, 4000.0, 10000.0, 20000.0,
        ] {
            for &f in &[1.0_f64, 1.005, 1.01, 1.03, 1.1, 1.3] {
                let n = (x * f).round() as usize;
                let shipped = miller_downward(n, x, miller_start_offset(x));
                // 3× the shipped offset: far past where any residual seed could survive.
                let generous = miller_downward(n, x, 3 * miller_start_offset(x));
                let moved = (shipped - generous).abs() / shipped.abs().max(1e-300);
                // The tolerance SCALES with sweep length, and that is not decoration. Two
                // sweeps of different lengths disagree at accumulated round-off, which grows
                // like √(steps)·ε — measured 1.8e-14 at x = 2e4, against 2.3e-14 from that
                // model. A flat threshold silently loses headroom as the grid extends: the
                // first version of this test used 1e-13, which the x = 2e4 row above sits only
                // 5.5× under. The factor of 10 is the margin; the √n is the shape.
                let tol = 10.0 * (n as f64).sqrt() * f64::EPSILON;
                assert!(
                    moved <= tol,
                    "x={x} n={n}: the shipped Miller offset ({}) is not converged — \
                     {shipped:.17e} moves by {moved:.2e} relative (tol {tol:.2e}) when tripled",
                    miller_start_offset(x)
                );
            }
        }

        // **Negative control.** A test that only ever passes proves nothing about its own
        // power. The pre-P14 flat offset must fail the very check the shipped one passes,
        // otherwise this test would keep passing if `miller_start_offset` regressed to a
        // constant. At x = 10⁴ the flat 40 is ~5× short of the derived width.
        let short = miller_downward(10_000, 10_000.0, MILLER_ACC_MIN);
        let converged = miller_downward(10_000, 10_000.0, 3 * miller_start_offset(10_000.0));
        let moved = (short - converged).abs() / converged.abs();
        assert!(
            moved > 1e-8,
            "the pre-P14 flat offset of {MILLER_ACC_MIN} now agrees to {moved:.2e} — this test \
             can no longer distinguish a sufficient offset from an insufficient one"
        );
    }

    /// The **decay law** [`MILLER_TURNING_WIDTH`] is derived from, checked by quadrature at the
    /// offsets where the oracle can still resolve it.
    ///
    /// The derivation's load-bearing claim is that `|J_{x+c·x^(1/3)}(x)| / |J_x(x)|` depends
    /// only on `c` and **not on x** — that is what lets one dimensionless constant cover every
    /// argument. Measured here at c = 4, 6, 8 across three decades of x: 3.4e-4, 5.1e-7,
    /// 2.4e-10, agreeing across x to well inside a factor of 2. The decay per 2 units of c
    /// *accelerates* — 660× then 2100× — which is the `t^(3/2)` in the Airy exponent showing
    /// up, and extrapolates to `f64::EPSILON` at c ≈ 11.3: the derived value, rounded up to 12.
    ///
    /// It stops at c = 8 because that is where the oracle stops: by c = 10 the ratio is ~3e-14
    /// of a peak of ~0.02, i.e. under 1e-15 absolute, and the spread across x is oracle noise
    /// rather than physics. The constant itself sits four units further out still, which is
    /// precisely why `miller_start_offset_has_real_margin` measures it a different way.
    #[test]
    fn turning_point_decay_law_is_x_independent() {
        let mut by_c: Vec<(f64, Vec<f64>)> = Vec::new();
        for &c in &[4.0_f64, 6.0, 8.0] {
            let ratios: Vec<f64> = [255.0_f64, 1000.0, 10000.0]
                .iter()
                .map(|&x| {
                    let peak = jm_by_quadrature(x as u32, x).abs();
                    let m = (x + c * x.cbrt()).round() as u32;
                    jm_by_quadrature(m, x).abs() / peak
                })
                .collect();
            let (lo, hi) = ratios
                .iter()
                .fold((f64::MAX, 0.0_f64), |(l, h), &r| (l.min(r), h.max(r)));
            assert!(
                hi / lo < 2.5,
                "c={c}: decay ratio varies with x by {:.2}× ({ratios:?}) — the x^(1/3) scaling \
                 the constant rests on does not hold",
                hi / lo
            );
            by_c.push((c, ratios));
        }
        // At least ~2.7 decades of decay per 2 units of c, and accelerating.
        let mut previous_drop = 0.0_f64;
        for w in by_c.windows(2) {
            let (lo, hi) = (&w[0], &w[1]);
            let drop = lo.1[0] / hi.1[0];
            assert!(
                drop > 500.0,
                "decay from c={} to c={} is only {drop:.1e}× — too slow for the derivation",
                lo.0,
                hi.0
            );
            assert!(
                drop > previous_drop,
                "decay from c={} to c={} ({drop:.1e}×) did not accelerate — the Airy exponent's \
                 t^(3/2) is what makes the extrapolation past c=8 safe",
                lo.0,
                hi.0
            );
            previous_drop = drop;
        }
        // The floor must bind exactly where the derivation says it does, and nowhere else.
        assert_eq!(miller_start_offset(1.0), MILLER_ACC_MIN);
        assert_eq!(miller_start_offset(36.0), MILLER_ACC_MIN, "(40/12)³ ≈ 37");
        assert!(miller_start_offset(64.0) > MILLER_ACC_MIN);
        assert_eq!(miller_start_offset(1000.0), 120);
    }

    /// [`bessel_jn`] against the independent quadrature oracle, on the **downward** branch —
    /// at the turning point, and past it, where P14's defect lived.
    ///
    /// The recurrence-identity tests above cannot see a uniform mis-scaling and the Miller
    /// normalization is exactly what mis-scales, so this is the grader that closes that hole.
    #[test]
    fn jn_downward_branch_matches_the_quadrature_oracle() {
        for &x in &[50.0_f64, 200.0, 255.0, 400.0, 700.0, 1000.0, 3000.0] {
            for &f in &[1.0_f64, 1.02, 1.1] {
                let m = (x * f).round() as u32;
                assert!(x <= m as f64, "must be the downward branch");
                let reference = jm_by_quadrature(m, x);
                // Only grade where the oracle itself is meaningful. Far past the turning point
                // Jₘ underflows toward zero and the oracle's ~1e-16 absolute floor is all that
                // is left; those orders contribute nothing to any caller's sum anyway.
                if reference.abs() < 1e-13 {
                    continue;
                }
                let got = bessel_jn(m, x);
                // Relative term plus an absolute floor, and the floor is not slack: a
                // renormalized downward sweep is accurate to ~ε·(largest Jₘ in the sweep) in
                // *absolute* terms, so an order well below the turning-point peak is
                // relatively less accurate by exactly that ratio. Measured worst here is
                // J₂₂₀(200) — 5.6e-16 absolute, 5.1e-12 relative, against a peak of 0.0765.
                // Chasing that relatively would mean asking a normalized recurrence for
                // something no normalized recurrence can give.
                let tol = 1e-12 * reference.abs() + 1e-14;
                assert!(
                    (got - reference).abs() < tol,
                    "J{m}({x}) = {got:.17e} vs oracle {reference:.17e} \
                     (abs {:.2e}, tol {tol:.2e})",
                    (got - reference).abs()
                );
            }
        }
    }

    /// [`bessel_jn`] against the oracle on the **upward** branch, where the accuracy ceiling is
    /// not the recurrence but the `J₀`/`J₁` seeds.
    ///
    /// Below `|x| = 8` those come from the exact power series and this is ~1e-15; above it they
    /// come from the Numerical Recipes asymptotic fit, whose ~3e-9 absolute error the upward
    /// recurrence carries to every order it produces. That is a *seed* limit, invisible to the
    /// recurrence identity (an upward recurrence satisfies it by construction, however wrong
    /// its seeds), which is why it needed an independent grader to surface at all.
    #[test]
    fn jn_upward_branch_inherits_the_j0_j1_seed_accuracy() {
        let mut worst = 0.0_f64;
        for &x in &[50.0_f64, 200.0, 255.0, 400.0, 1000.0, 3000.0, 10000.0] {
            for &f in &[0.5_f64, 0.9, 0.99] {
                let m = (x * f).floor() as u32;
                assert!(x > m as f64, "must be the upward branch");
                let reference = jm_by_quadrature(m, x);
                worst = worst.max((bessel_jn(m, x) - reference).abs());
            }
        }
        // Absolute, not relative: the seed error is an absolute offset that the recurrence
        // propagates, so it is unbounded in relative terms near a zero of Jₘ, and bounding it
        // there would be measuring the zero rather than the routine.
        assert!(
            worst < 1e-8,
            "upward-branch worst absolute error {worst:.2e} exceeds the seed-accuracy ceiling"
        );
        // And it is genuinely seed-limited rather than exact — the number this test exists to
        // record. If this ever fails, `bessel_j0`/`bessel_j1`'s asymptotic branch improved and
        // the module header's accuracy claims need updating with it.
        assert!(
            worst > 1e-12,
            "upward branch is now accurate to {worst:.2e}: better than documented"
        );
    }

    /// [`bessel_jn_array`] must agree with the per-order [`bessel_jn`] it replaces — across
    /// both branches, across the turning point, and for negative arguments.
    ///
    /// This is the load-bearing test for the P10-perf Bessel change: the array form is an
    /// optimization, so any disagreement is a served-value change that was not asked for.
    #[test]
    fn jn_array_matches_the_per_order_function() {
        let m_max = 254u32;
        let mut arr = vec![0.0; m_max as usize + 1];
        // Arguments straddling every regime: far below the top order (deep Miller), at it,
        // just above it, and far above (upward recurrence). `x = 0` is deliberately absent —
        // there the array returns the exact `J₀(0) = 1` while `bessel_j0`'s rational
        // approximation evaluates to 1.000_000_002_8 (a pre-existing 2.8e-9 error at the
        // origin, well inside that function's own pinned 1e-6 tolerance). Grading the array
        // against the closed form at that point is `jn_array_handles_degenerate_inputs`.
        for &x in &[
            1e-3_f64, 0.5, 5.0, 50.0, 253.0, 254.0, 255.0, 300.0, 1000.0, 11383.0, -50.0, -300.0,
        ] {
            bessel_jn_array(m_max, x, &mut arr);
            for m in 0..=m_max {
                let expected = bessel_jn(m, x);
                let got = arr[m as usize];
                // Absolute floor of 5e-9 rather than a pure relative test, for a reason worth
                // stating: where the array takes the Miller branch it is *more* accurate than
                // `bessel_j0`/`bessel_j1` near the origin. Those use the Numerical Recipes
                // rational approximations, which evaluate to `1 + 2.83e-9` at `x = 0` instead
                // of exactly 1 — e.g. `J₀(0.001)` is 0.999_999_752_831 from `bessel_j0` and
                // 0.999_999_750_000 from the array, the latter being right to 1e-16. That
                // offset is inside `bessel_j0`'s own pinned 1e-6 tolerance and is ~2.5e-8 dB
                // in gain terms, so it is accommodated here rather than chased: this test
                // grades the array as an *optimization* of the per-order function, and 5e-9 is
                // small enough that any algorithmic disagreement still fails it.
                //
                // Orders far past the turning point are astronomically small, where only
                // absolute agreement is meaningful anyway — those modes contribute nothing to
                // the mode sum.
                let tol = 5e-9 + 1e-9 * expected.abs();
                assert!(
                    (got - expected).abs() <= tol,
                    "J{m}({x}): array {got:.17e} vs per-order {expected:.17e}"
                );
            }
        }
    }

    /// The normalization identity `J₀(x) + 2·Σ_{k≥1} J_{2k}(x) = 1`, evaluated from the array.
    ///
    /// Independent of [`bessel_jn`] entirely — it grades the array against a closed-form
    /// property of the Bessel family, so a shared defect in both routines cannot hide here.
    /// The sum must be taken well past the turning point for the tail to be negligible, which
    /// is exactly the high-order regime the array is built to serve.
    #[test]
    fn jn_array_satisfies_the_normalization_identity() {
        for &x in &[0.5_f64, 5.0, 50.0, 120.0, 200.0] {
            let m_max = (x.ceil() as u32 + 80).max(64);
            let mut arr = vec![0.0; m_max as usize + 1];
            bessel_jn_array(m_max, x, &mut arr);
            let sum: f64 = arr[0] + 2.0 * arr.iter().skip(2).step_by(2).sum::<f64>();
            assert!(
                (sum - 1.0).abs() < 1e-10,
                "x={x}: J₀ + 2ΣJ_even = {sum}, expected 1"
            );
        }
    }

    #[test]
    fn jn_array_handles_degenerate_inputs() {
        let mut arr = vec![7.0; 5];
        bessel_jn_array(0, 3.0, &mut arr);
        assert!((arr[0] - bessel_j0(3.0)).abs() < 1e-15);

        bessel_jn_array(4, f64::NAN, &mut arr);
        assert!(
            arr.iter().all(|v| v.is_nan()),
            "NaN must propagate: {arr:?}"
        );

        bessel_jn_array(4, f64::INFINITY, &mut arr);
        assert!(arr.iter().all(|v| v.is_nan()));

        bessel_jn_array(4, 0.0, &mut arr);
        assert_eq!(arr[..5], [1.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "output buffer holds")]
    fn jn_array_rejects_a_short_buffer() {
        let mut arr = [0.0; 3];
        bessel_jn_array(3, 1.0, &mut arr);
    }

    #[test]
    fn jn_high_order_small_x_underflows_to_zero() {
        // High-order small-x: J_m(x) ~ (x/2)^m / m! for small x, so J10(0.1) is a tiny
        // but well-defined value. The closed-form small-argument series gives
        // 2.690_532_895e-20 (verified independently); pin against it to catch a
        // recurrence that either blows up or loses the tail's magnitude. (The plan's
        // "~1e-26 / < 1e-20" bound was a wrong hand-estimate — the true leading term
        // (0.05)^10 / 10! is ≈ 2.69e-20, not 1e-26.)
        let v = bessel_jn(10, 0.1);
        assert!(v.is_finite(), "must be finite, got {v}");
        assert!(v.abs() < 1e-19, "must be tiny, got {v}");
        let reference = 2.690_532_895_434_217e-20;
        assert!(
            (v - reference).abs() < reference.abs() * 1e-6,
            "J10(0.1) = {v}, expected ≈ {reference}"
        );
    }
}
