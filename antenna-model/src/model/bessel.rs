//! In-house cylindrical Bessel functions Jₘ(x) for real argument.
//!
//! Pure Rust (no BLAS / no external crate — matches the repo's dependency rule).
//! `bessel_j0`/`bessel_j1` use the Numerical Recipes (Press et al.) rational
//! approximations: a polynomial ratio for |x| < 8 and an asymptotic amplitude/phase
//! form for |x| >= 8. `bessel_jn` uses Miller's downward recurrence, the stable
//! direction for Jₙ (upward recurrence amplifies round-off catastrophically).
//!
//! Validated in BOTH branches — see the module tests. A special-function routine
//! that is wrong is *confidently* wrong: the coefficients below are pinned by tests
//! at |x|<8 and |x|>=8 independently.

/// Bessel function of the first kind, order 0.
pub fn bessel_j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let p1 = 57_568_490_574.0
            + y * (-13_362_590_354.0
                + y * (651_619_640.7
                    + y * (-11_214_424.18 + y * (77_392.330_17 + y * (-184.905_245_6)))));
        let p2 = 57_568_490_411.0
            + y * (1_029_532_985.0
                + y * (9_494_680.718 + y * (59_272.648_53 + y * (267.853_271_2 + y))));
        p1 / p2
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
    if ax < 8.0 {
        // Small-argument branch: the leading `x` factor in `p1` already carries the
        // sign of x (J1 is odd), so no separate sign correction is needed here.
        let y = x * x;
        let p1 = x
            * (72_362_614_232.0
                + y * (-7_895_059_235.0
                    + y * (242_396_853.1
                        + y * (-2_972_611.439 + y * (15_704.482_60 + y * (-30.160_366_06))))));
        let p2 = 144_725_228_442.0
            + y * (2_300_535_178.0
                + y * (18_583_304.74 + y * (99_447.433_94 + y * (376.999_139_7 + y))));
        p1 / p2
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
/// n=0,1 delegate to the rational approximations. For n>=2 this follows the
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
/// two-branch form fixes cost and the overflow outright, and fixes accuracy
/// everywhere except **at** the turning point itself, where downward is still the
/// only stable direction and `acc` is still a constant: measured 2026-08-01, the
/// recurrence identity closes to 2e-8 at `m = x = 255` but only 9e-3 at
/// `m = x = 10⁴` (it is exact to ~1e-15 at `m = 0.9x` and `m = 1.1x` throughout).
/// Harmless for every current caller — the mode integrator's `MODE_M_MAX` caps the
/// order at 254, where the error is 2e-8 — and pinned by
/// `jn_turning_point_accuracy_degrades_far_above_the_served_order_ceiling` so it
/// cannot silently become relevant if that cap is ever raised.
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
        // DOWNWARD Miller recurrence: J_{m-1}(x) = (2m/x) J_m(x) - J_{m+1}(x).
        // Only runs for |x| <= n, so the argument is small and `ax as usize` is
        // bounded by n (no overflow). The seed starts an even number of orders
        // above n so the arbitrary seed decays into negligibility before order n.
        let acc = 40; // extra iterations above n for accuracy
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
    };

    // Jₙ(−x) = (−1)ⁿ Jₙ(x): correct the sign for negative x, odd n.
    if x < 0.0 && n % 2 == 1 {
        -ans
    } else {
        ans
    }
}

/// Extra recurrence steps started above the highest wanted order in the downward (Miller)
/// branch, so the arbitrary seed has decayed into negligibility by the time the recurrence
/// reaches an order we keep. Matches the per-call [`bessel_jn`]'s `acc`.
const MILLER_ACC: usize = 40;

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
        let m_start = 2 * ((top + MILLER_ACC) / 2 + 1); // force even

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
        // Bounded at 300 deliberately: `MODE_M_MAX` caps the served integrator at order 254,
        // and above ~300 the identity stops closing to machine precision *at* `m = x` — see
        // `jn_turning_point_accuracy_degrades_far_above_the_served_order_ceiling`, which pins
        // that separately rather than letting this test's tolerance be loosened to hide it.
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
                // 1e-7, not machine precision, and the difference is a MEASUREMENT: exactly at
                // the turning point (`m = x = 255`) the downward branch starts only
                // `MILLER_ACC = 40` orders above the wanted one, where `J₂₉₆(255)/J₂₅₅(255)`
                // is still ~4e-5, so the seed has not fully decayed and the identity closes to
                // 2e-8 rather than 1e-15. That is the accuracy `bessel_jn` has always had here
                // — this test is the first to look — and 2e-8 is seven orders inside the
                // integrator's 0.5 % mode-truncation budget, so it is recorded, not fixed.
                let scale = lhs.abs().max(jm1.abs()).max(jp1.abs()).max(1e-300);
                assert!(
                    (lhs - rhs).abs() <= scale * 1e-7,
                    "recurrence identity broken at m={m}, x={x}: {lhs} vs {rhs}"
                );
            }
        }
    }

    /// **Known accuracy cliff, pinned rather than fixed** (measured 2026-08-01 by P10-perf's
    /// new turning-point coverage; filed to the roadmap, not repaired here).
    ///
    /// Exactly at the turning point `m = x`, [`bessel_jn`]'s downward branch starts only
    /// `acc = 40` orders above the wanted one. That constant offset is the very scheme this
    /// module's header warns about — "the turning-point transition width grows like `x^(1/3)`,
    /// so a constant seed offset fails to reach the decaying tail" — and while the two-branch
    /// design removed the problem for `m ≪ x` (which now takes the upward recurrence), it
    /// left it in place *at* `m ≈ x`, where downward is still the only stable direction.
    ///
    /// Measured relative closure of `(2m/x)·Jₘ = J_{m−1} + J_{m+1}` at `m = x`:
    ///
    /// | x | 50 | 200 | 255 | 400 | 700 | 1000 | 3000 | 10000 |
    /// |---|----|-----|-----|-----|-----|------|------|-------|
    /// | rel. err | 3e-10 | 1e-9 | 2e-8 | 2e-7 | 4e-6 | 2e-5 | 9e-4 | **9e-3** |
    ///
    /// At `m = 0.9x` and `m = 1.1x` the same identity closes to ~1e-15 at every one of those
    /// arguments, so the defect is sharply localized to the turning point.
    ///
    /// **Why this is not a served-path defect today:** the only caller that reaches the
    /// downward branch is the azimuthal-mode integrator, whose order is capped by
    /// `MODE_M_MAX = 254`. The turning point is therefore never crossed above `x ≈ 254`, where
    /// the error is 2e-8 — seven orders inside the mode-truncation budget. It becomes real the
    /// moment that cap is raised, which is why the behavior is pinned here instead of left to
    /// be rediscovered.
    #[test]
    fn jn_turning_point_accuracy_degrades_far_above_the_served_order_ceiling() {
        // (x, tolerance) — the measured value with ~3× headroom, so this catches a regression
        // while documenting the real number.
        for &(x, tol) in &[
            (255.0_f64, 1e-7),
            (400.0, 1e-6),
            (700.0, 2e-5),
            (1000.0, 1e-4),
            (3000.0, 3e-3),
            (10000.0, 3e-2),
        ] {
            let m = x as u32;
            let lhs = (2.0 * m as f64 / x) * bessel_jn(m, x);
            let rhs = bessel_jn(m - 1, x) + bessel_jn(m + 1, x);
            let rel = (lhs - rhs).abs() / lhs.abs().max(1e-300);
            assert!(
                rel <= tol,
                "J_{m}({x}) turning-point closure degraded to {rel:.3e} (documented ≤ {tol:.0e})"
            );
            // Away from the turning point the same identity must still hold to near machine
            // precision — that is what localizes the defect and keeps this test from being
            // satisfied by a routine that is simply bad everywhere.
            let off = (0.9 * x) as u32;
            let lhs_off = (2.0 * off as f64 / x) * bessel_jn(off, x);
            let rhs_off = bessel_jn(off - 1, x) + bessel_jn(off + 1, x);
            let rel_off = (lhs_off - rhs_off).abs() / lhs_off.abs().max(1e-300);
            assert!(
                rel_off < 1e-12,
                "J_{off}({x}) is off the turning point and must be exact, got {rel_off:.3e}"
            );
        }
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
