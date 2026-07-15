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
/// two-branch form fixes accuracy, cost, and the overflow together.
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
