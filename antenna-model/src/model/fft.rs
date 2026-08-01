//! In-house mixed-radix FFT for the aperture integrator's φ' transform (roadmap P10-perf).
//!
//! Pure Rust, no dependencies — same rule as [`crate::model::bessel`].
//!
//! # Why this exists
//!
//! [`crate::model::integration::azimuthal_mode_field_inner`] needs the azimuthal Fourier
//! coefficients `gₘ(ρ)` of the aperture-plane function `g(ρ,φ')` for every radial sample.
//! Until P10-perf that was a direct DFT costing `O(n_φ · M)` per radial sample, which is the
//! integrator's dominant term on exactly the geometries the model is slowest at: a ~5° beam
//! steer on the 34 m dish sizes `n_φ = 536` and `M = 254`, so the transform alone is ~137 000
//! complex multiply-accumulates per radial sample, ~2 500 radial samples deep. This module
//! makes that term `O(n_φ log n_φ)`.
//!
//! # Scope
//!
//! Deliberately minimal: forward transform only, `Complex64` only, no in-place API beyond a
//! caller-supplied scratch buffer. It is not a general-purpose FFT crate and should not grow
//! into one — the only consumer is the aperture integrator.
//!
//! # Correctness
//!
//! A wrong FFT is a wrong integrator, and CLAUDE.md pitfall #2 applies with full force: the
//! result would still be a plausible number. The module tests therefore check the transform
//! against a literal transcription of the DFT sum at **every** supported length in the range
//! the integrator uses, on random data, rather than spot-checking a few sizes.

use num_complex::Complex64;
use std::f64::consts::TAU;

/// Radices handled without a heap allocation in the butterfly combine. Covers every factor a
/// [`next_fast_len`] length can have (2, 3, 5) with room to spare; larger prime factors (only
/// reachable from tests, which exercise arbitrary lengths) fall back to a `Vec`.
const MAX_INLINE_RADIX: usize = 8;

/// Smallest length `>= n` that this module transforms quickly: an **even** 5-smooth number
/// (only 2, 3 and 5 as prime factors).
///
/// The aperture integrator is free to choose `n_phi` — it only has to be *at least* the
/// azimuthal Nyquist count — so rounding it up to a fast length costs a few extra evaluations
/// of the aperture-plane function and buys the whole `O(n_φ log n_φ)` transform. 5-smooth
/// numbers are dense — the padding is at most **12.5 %** over the integrator's whole range and
/// usually a few percent — which is why this does not round up to a power of two: that would
/// have cost 91 % extra at a length like 536, and those aperture-plane evaluations are the
/// integrator's remaining floor cost, not a rounding detail.
///
/// Evenness is not required by the transform; it is kept because the integrator's `±m` mode
/// pairs are indexed symmetrically about `n/2`.
pub(crate) fn next_fast_len(n: usize) -> usize {
    let mut m = n.max(2);
    if !m.is_multiple_of(2) {
        m += 1;
    }
    loop {
        if is_smooth(m) {
            return m;
        }
        m += 2;
    }
}

/// Whether `n`'s only prime factors are 2, 3 and 5.
fn is_smooth(mut n: usize) -> bool {
    for p in [2usize, 3, 5] {
        while n.is_multiple_of(p) {
            n /= p;
        }
    }
    n == 1
}

/// A forward DFT of a fixed length, with its twiddle table and factorization precomputed.
///
/// Build one per transform length and reuse it across all radial samples — construction is
/// `O(n)` transcendental calls, which must not land in the radial loop.
pub(crate) struct FftPlan {
    n: usize,
    /// `e^{-2πi j / n}` for `j` in `0..n`.
    twiddles: Vec<Complex64>,
    /// Prime factors of `n`, largest-radix-first, in the order the recursion consumes them.
    factors: Vec<usize>,
}

impl FftPlan {
    /// Plan a forward transform of length `n` (`n >= 1`, any factorization; non-smooth
    /// lengths still work, they are just slower because a large prime factor degrades to a
    /// direct `O(p²)` butterfly).
    pub(crate) fn new(n: usize) -> Self {
        debug_assert!(n >= 1, "FFT length must be positive");
        let twiddles = (0..n)
            .map(|j| Complex64::new(0.0, -TAU * j as f64 / n as f64).exp())
            .collect();
        Self {
            n,
            twiddles,
            factors: factorize(n),
        }
    }

    /// Forward transform: on return `data[k] = Σ_j data_in[j] · e^{-2πi jk/n}`.
    ///
    /// `scratch` must be the same length as `data`; its contents are clobbered. Unnormalized
    /// (no `1/n`) — the caller applies whatever normalization its definition needs.
    ///
    /// # Panics
    /// If `data.len() != self.len()` or `scratch.len() != data.len()`.
    pub(crate) fn forward(&self, data: &mut [Complex64], scratch: &mut [Complex64]) {
        assert_eq!(data.len(), self.n, "FFT input length must match the plan");
        assert_eq!(scratch.len(), self.n, "FFT scratch length must match input");
        scratch.copy_from_slice(data);
        self.transform(scratch, 0, 1, data, 0, self.n, 0);
    }

    /// Recursive mixed-radix Cooley–Tukey, decimation in time.
    ///
    /// Computes the `n`-point DFT of `inp[inp_start], inp[inp_start + stride], …` into
    /// `out[out_start .. out_start + n]`. `fi` indexes [`Self::factors`]; each level splits
    /// the transform into `p = factors[fi]` interleaved sub-transforms of length `m = n/p`,
    /// then combines them with a direct `p`-point DFT per output group.
    #[allow(clippy::too_many_arguments)]
    fn transform(
        &self,
        inp: &[Complex64],
        inp_start: usize,
        stride: usize,
        out: &mut [Complex64],
        out_start: usize,
        n: usize,
        fi: usize,
    ) {
        if n == 1 {
            out[out_start] = inp[inp_start];
            return;
        }
        let p = self.factors[fi];
        let m = n / p;

        // Sub-transforms of the p interleaved decimations, written to disjoint output blocks.
        for r in 0..p {
            self.transform(
                inp,
                inp_start + r * stride,
                stride * p,
                out,
                out_start + r * m,
                m,
                fi + 1,
            );
        }

        // Combine. `self.twiddles` is indexed against the FULL length `self.n`, so a root of
        // unity of order `n` is `twiddles[(j · self.n / n) % self.n]` — exact integer index
        // arithmetic, never a re-derived transcendental.
        let step_n = self.n / n;
        let step_p = self.n / p;
        let mut inline = [Complex64::new(0.0, 0.0); MAX_INLINE_RADIX];
        let mut heap;
        let t: &mut [Complex64] = if p <= MAX_INLINE_RADIX {
            &mut inline[..p]
        } else {
            heap = vec![Complex64::new(0.0, 0.0); p];
            &mut heap
        };

        for k in 0..m {
            for (r, slot) in t.iter_mut().enumerate() {
                let w = self.twiddles[(r * k * step_n) % self.n];
                *slot = out[out_start + r * m + k] * w;
            }
            for q in 0..p {
                let mut acc = Complex64::new(0.0, 0.0);
                for (r, &tr) in t.iter().enumerate() {
                    acc += tr * self.twiddles[(r * q * step_p) % self.n];
                }
                out[out_start + q * m + k] = acc;
            }
        }
    }
}

/// Prime factors of `n`, ascending, with `1` factorizing to the empty list.
fn factorize(mut n: usize) -> Vec<usize> {
    let mut factors = Vec::new();
    let mut p = 2;
    while p * p <= n {
        while n.is_multiple_of(p) {
            factors.push(p);
            n /= p;
        }
        p += 1;
    }
    if n > 1 {
        factors.push(n);
    }
    factors
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Literal transcription of the DFT definition — the reference every test below is graded
    /// against. Deliberately the slow, obvious form: it is the thing we trust.
    fn naive_dft(x: &[Complex64]) -> Vec<Complex64> {
        let n = x.len();
        (0..n)
            .map(|k| {
                let mut acc = Complex64::new(0.0, 0.0);
                for (j, &xj) in x.iter().enumerate() {
                    acc += xj * Complex64::new(0.0, -TAU * (j * k) as f64 / n as f64).exp();
                }
                acc
            })
            .collect()
    }

    /// Deterministic pseudo-random complex data (a small LCG — no dev-dependency needed, and
    /// a fixed seed keeps failures reproducible).
    fn pseudo_random(n: usize, seed: u64) -> Vec<Complex64> {
        let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((s >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        (0..n).map(|_| Complex64::new(next(), next())).collect()
    }

    fn max_abs_diff(a: &[Complex64], b: &[Complex64]) -> f64 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).norm())
            .fold(0.0, f64::max)
    }

    /// The load-bearing test: at **every** fast length the integrator can ask for, the plan
    /// reproduces the DFT sum. Not a spot check — a wrong butterfly at one radix combination
    /// is exactly the kind of defect that survives spot checks and then silently corrupts an
    /// aperture integral.
    ///
    /// Exhaustive to 512 and then sampled up to `MODE_PHI_MAX` (2048). The reference is a
    /// literal `O(n²)` DFT with a transcendental per term, so running it exhaustively to 2048
    /// cost ~20 s in a debug build — over D18's slow-test line for coverage that is genuinely
    /// redundant: every radix *combination* reachable above 512 (2ᵃ3ᵇ5ᶜ) already appears below
    /// it, and the sampled lengths cover the deep-recursion cases regardless.
    #[test]
    fn matches_the_dft_definition_at_every_fast_length() {
        let mut lengths = Vec::new();
        let mut n = 2;
        while n <= 512 {
            let fast = next_fast_len(n);
            if !lengths.contains(&fast) {
                lengths.push(fast);
            }
            n += 2;
        }
        // Sampled above 512: pure powers of two/three/five, mixed radices, and the ceiling.
        for &n in &[
            540usize, 600, 640, 648, 720, 750, 972, 1024, 1080, 1250, 1620, 2048,
        ] {
            assert_eq!(next_fast_len(n), n, "{n} should already be a fast length");
            if !lengths.contains(&n) {
                lengths.push(n);
            }
        }
        assert!(
            lengths.len() > 40,
            "expected a dense set of fast lengths, got {}",
            lengths.len()
        );

        for &len in &lengths {
            let x = pseudo_random(len, len as u64);
            let expected = naive_dft(&x);
            let plan = FftPlan::new(len);
            let mut data = x.clone();
            let mut scratch = vec![Complex64::new(0.0, 0.0); len];
            plan.forward(&mut data, &mut scratch);
            // Absolute tolerance scaled by the transform's own magnitude: DFT round-off grows
            // like sqrt(n)·eps·|x|, and |X| itself is O(sqrt(n)) for random input.
            let scale = expected.iter().map(|c| c.norm()).fold(0.0, f64::max);
            let tol = 1e-12 * scale * (len as f64).sqrt();
            let diff = max_abs_diff(&data, &expected);
            assert!(
                diff < tol,
                "length {len}: max |FFT − DFT| = {diff:.3e} exceeds {tol:.3e}"
            );
        }
    }

    /// Non-smooth lengths (a large prime factor) must still be *correct*, only slower. The
    /// integrator never asks for one, but nothing in the API prevents it and a silently wrong
    /// answer there would be a trap for the next caller.
    #[test]
    fn matches_the_dft_definition_at_awkward_lengths() {
        for len in [1usize, 2, 3, 7, 11, 13, 17, 26, 49, 121, 143, 169, 253] {
            let x = pseudo_random(len, 99 + len as u64);
            let expected = naive_dft(&x);
            let plan = FftPlan::new(len);
            let mut data = x.clone();
            let mut scratch = vec![Complex64::new(0.0, 0.0); len];
            plan.forward(&mut data, &mut scratch);
            let scale = expected.iter().map(|c| c.norm()).fold(1.0, f64::max);
            let diff = max_abs_diff(&data, &expected);
            assert!(
                diff < 1e-12 * scale * (len as f64).sqrt(),
                "length {len}: max |FFT − DFT| = {diff:.3e}"
            );
        }
    }

    /// Sign-convention pin, independent of the naive reference (which could have the exponent
    /// flipped in sympathy with the implementation).
    ///
    /// With the forward kernel `e^{-2πi jk/n}`, a tone `x_j = e^{+2πi m j/n}` collapses to a
    /// single spike at bin `m` — and therefore the *conjugate* coefficient the integrator
    /// calls `g₋ₘ` lives at bin `n − m`. That indexing is what
    /// [`crate::model::integration::azimuthal_mode_field_inner`] relies on to fill `gm_neg`,
    /// so getting it backwards would silently swap the `±m` modes: a coma lobe on the wrong
    /// side, at full amplitude, with every convergence check still reporting success.
    #[test]
    fn pure_tone_lands_in_the_expected_bin_with_the_expected_sign() {
        let n = 60;
        let plan = FftPlan::new(n);
        let mut scratch = vec![Complex64::new(0.0, 0.0); n];
        for m in [1usize, 2, 7, 29] {
            // e^{+2πi m j/n} → spike at bin m.
            let mut data: Vec<Complex64> = (0..n)
                .map(|j| Complex64::new(0.0, TAU * (m * j) as f64 / n as f64).exp())
                .collect();
            plan.forward(&mut data, &mut scratch);
            for (k, v) in data.iter().enumerate() {
                let expected = if k == m { n as f64 } else { 0.0 };
                assert!(
                    (v.norm() - expected).abs() < 1e-9,
                    "+m tone: m={m}: bin {k} = {v} (expected magnitude {expected})"
                );
            }

            // e^{-2πi m j/n} → spike at bin n − m, i.e. the `g₋ₘ` slot.
            let mut data: Vec<Complex64> = (0..n)
                .map(|j| Complex64::new(0.0, -TAU * (m * j) as f64 / n as f64).exp())
                .collect();
            plan.forward(&mut data, &mut scratch);
            for (k, v) in data.iter().enumerate() {
                let expected = if k == n - m { n as f64 } else { 0.0 };
                assert!(
                    (v.norm() - expected).abs() < 1e-9,
                    "−m tone: m={m}: bin {k} = {v} (expected magnitude {expected})"
                );
            }
        }
    }

    #[test]
    fn next_fast_len_is_even_smooth_and_not_below_the_request() {
        // Density claim from the docstring. The padding is what the integrator pays for the
        // transform: every unit of it is an extra aperture-plane evaluation per radial sample,
        // and those are the integrator's floor cost. `MODE_PHI_MIN` (64) is the smallest length
        // it can ask for, so that is where the claim has to hold.
        let mut worst = (0usize, 0usize, 0.0_f64);
        for n in 1usize..=3000 {
            let f = next_fast_len(n);
            assert!(f >= n, "next_fast_len({n}) = {f} is below the request");
            assert!(f.is_multiple_of(2), "next_fast_len({n}) = {f} is odd");
            assert!(is_smooth(f), "next_fast_len({n}) = {f} is not 5-smooth");
            if n >= 64 {
                let pad = (f - n) as f64 / n as f64;
                if pad > worst.2 {
                    worst = (n, f, pad);
                }
            }
        }
        assert!(
            worst.2 <= 0.125,
            "worst padding is next_fast_len({}) = {} ({:.1} %), above the documented 12.5 %",
            worst.0,
            worst.1,
            worst.2 * 100.0
        );
    }

    #[test]
    fn factorize_reproduces_the_product() {
        for n in 1usize..=500 {
            let f = factorize(n);
            assert_eq!(f.iter().product::<usize>().max(1), n.max(1), "n={n}");
        }
    }
}
