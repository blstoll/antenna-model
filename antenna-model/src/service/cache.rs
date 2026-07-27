//! LRU Gain Cache
//!
//! Caches physics model results keyed on quantized (az, el, freq, physical feed
//! position). The feed coordinates in the key are the *physical* feed position in
//! the antenna frame, relative to the reflector vertex — the steering displacement
//! `compute_feed_position_from_pointing` derives from the request's
//! `feed_pointing_location`, plus the feed's design offset — not the aim point
//! itself. Note this is a vertex-relative position, not a displacement from the
//! focus: `evaluator.rs` computes the reported `feed_offset_meters` as
//! `feed_z - focal_length_m` precisely because the two differ.
//!
//! Per-feed caches are stored in a DashMap to avoid cross-feed lock contention.

use dashmap::DashMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type FeedCache = Mutex<LruCache<GainCacheKey, CachedGain>>;

/// A cached physics-gain value together with the one piece of its provenance that
/// cannot be re-derived without redoing the integration a cache hit exists to skip.
///
/// The cache deliberately stores **only** what is specific to the cached
/// `(az, el, freq, feed)` point. Warnings that are a pure function of the antenna
/// configuration — spillover, the feed-offset band, the ray-tracing stub — are
/// re-derived by the caller on every request, hit or miss, so a warm cache cannot
/// swallow them (`analyze_edge_cases` ignores `(theta, phi)` outright, so they are
/// identical at every point anyway).
///
/// Convergence is the exception that forces this type to exist: it describes *this*
/// number, produced by *this* integration, and it is exactly what a cache hit skips
/// recomputing — so it has to ride along with the value. Before roadmap unit C10 it
/// did not, and a warm `/h3-heatmap` served non-converged gains with no warning at
/// all, silently breaking the "never silent" guarantee the P10 self-check exists to
/// provide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedGain {
    /// Physics-only gain (dB) at the cached point.
    pub value: f64,
    /// `false` when the integrator's self-check did not converge at this point.
    pub converged: bool,
}

impl CachedGain {
    /// A value whose integration converged — the ordinary case.
    pub fn converged(value: f64) -> Self {
        Self {
            value,
            converged: true,
        }
    }

    pub fn new(value: f64, converged: bool) -> Self {
        Self { value, converged }
    }
}

/// Quantized cache key for a gain lookup.
/// All floats are rounded to integers to make them hashable.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct GainCacheKey {
    /// (az_deg * 1000).round() as i32 — 0.001° resolution
    pub az_millideg: i32,
    /// (el_deg * 1000).round() as i32
    pub el_millideg: i32,
    /// (freq_mhz * 1000).round() as u32 — 1 kHz resolution
    pub freq_khz: u32,
    /// (feed_x_m * 1000).round() as i32 — 1 mm resolution
    pub feed_x_mm: i32,
    /// (feed_y_m * 1000).round() as i32
    pub feed_y_mm: i32,
    /// (feed_z_m * 1000).round() as i32
    pub feed_z_mm: i32,
}

impl GainCacheKey {
    pub fn new(
        az_deg: f64,
        el_deg: f64,
        freq_mhz: f64,
        feed_x: f64,
        feed_y: f64,
        feed_z: f64,
    ) -> Self {
        Self {
            az_millideg: (az_deg * 1000.0).round() as i32,
            el_millideg: (el_deg * 1000.0).round() as i32,
            freq_khz: (freq_mhz * 1000.0).round() as u32,
            feed_x_mm: (feed_x * 1000.0).round() as i32,
            feed_y_mm: (feed_y * 1000.0).round() as i32,
            feed_z_mm: (feed_z * 1000.0).round() as i32,
        }
    }
}

/// Thread-safe per-feed LRU gain cache.
pub struct GainCache {
    /// Per-(antenna_id, feed_id) LRU caches
    caches: DashMap<(String, String), Arc<FeedCache>>,
    max_entries_per_feed: usize,
    pub enabled: bool,
}

impl GainCache {
    pub fn new(enabled: bool, max_entries_per_feed: usize) -> Self {
        Self {
            caches: DashMap::new(),
            max_entries_per_feed,
            enabled,
        }
    }

    /// Get a cached gain value, or compute and cache it.
    /// If cache is disabled, always calls compute.
    ///
    /// The closure returns a [`CachedGain`] rather than a bare `f64` so that
    /// convergence — which only the computation knows and only the *miss* path runs
    /// — survives into every later hit. See [`CachedGain`] for why nothing else about
    /// the computation belongs in here.
    pub fn get_or_compute<F>(
        &self,
        antenna_id: &str,
        feed_id: &str,
        key: GainCacheKey,
        compute: F,
    ) -> crate::error::Result<CachedGain>
    where
        F: FnOnce() -> crate::error::Result<CachedGain>,
    {
        if !self.enabled {
            return compute();
        }

        let feed_key = (antenna_id.to_string(), feed_id.to_string());

        // Atomically get-or-create the Arc<FeedCache>, then clone it and
        // release the DashMap shard lock before taking the LRU mutex.
        let arc = self
            .caches
            .entry(feed_key)
            .or_insert_with(|| {
                Arc::new(Mutex::new(LruCache::new(
                    NonZeroUsize::new(self.max_entries_per_feed).unwrap_or(NonZeroUsize::MIN),
                )))
            })
            .clone();

        let mut cache = arc.lock().map_err(|_| {
            crate::error::AntennaModelError::Computation(
                crate::error::ComputationError::InvalidModelState(
                    "cache mutex poisoned".to_string(),
                ),
            )
        })?;

        if let Some(&val) = cache.get(&key) {
            return Ok(val);
        }

        // Release the lock before calling compute (which may be slow).
        drop(cache);

        let value = compute()?;

        // Re-lock to insert the computed value.
        let mut cache = arc.lock().map_err(|_| {
            crate::error::AntennaModelError::Computation(
                crate::error::ComputationError::InvalidModelState(
                    "cache mutex poisoned".to_string(),
                ),
            )
        })?;
        cache.put(key, value);

        Ok(value)
    }

    /// Invalidate all cached entries for a specific feed.
    pub fn invalidate(&self, antenna_id: &str, feed_id: &str) {
        self.caches
            .remove(&(antenna_id.to_string(), feed_id.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_key(az: f64) -> GainCacheKey {
        GainCacheKey::new(az, 10.0, 12000.0, 0.1, 0.0, 0.0)
    }

    #[test]
    fn test_cache_miss_calls_compute() {
        let cache = GainCache::new(true, 100);
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        let result: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(12.5))
            });

        assert_eq!(result.unwrap().value, 12.5);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cache_hit_skips_compute() {
        let cache = GainCache::new(true, 100);
        let call_count = Arc::new(AtomicUsize::new(0));

        // Prime the cache
        let cc = call_count.clone();
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(12.5))
            });

        // Second call should hit cache
        let cc = call_count.clone();
        let result: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(99.0))
            });

        assert_eq!(result.unwrap().value, 12.5); // got cached value, not 99.0
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // compute only called once
    }

    #[test]
    fn test_lru_eviction() {
        let cache = GainCache::new(true, 2); // max 2 entries

        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(1.0), || {
                Ok(CachedGain::converged(1.0))
            });
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(2.0), || {
                Ok(CachedGain::converged(2.0))
            });
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(3.0), || {
                Ok(CachedGain::converged(3.0))
            }); // evicts key(1.0)

        let call_count = Arc::new(AtomicUsize::new(0));
        // key(1.0) should be evicted — compute should be called again
        let cc = call_count.clone();
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(1.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(1.0))
            });
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_key_quantization_same_bucket() {
        let cache = GainCache::new(true, 100);
        let call_count = Arc::new(AtomicUsize::new(0));

        // 45.0000 and 45.0004 are within 0.0005° → same quantized key (both round to 45000 millideg)
        let cc = call_count.clone();
        let _: crate::error::Result<CachedGain> = cache.get_or_compute(
            "ant1",
            "feed1",
            GainCacheKey::new(45.0000, 10.0, 12000.0, 0.0, 0.0, 0.0),
            || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(5.0))
            },
        );

        let cc = call_count.clone();
        let result: crate::error::Result<CachedGain> = cache.get_or_compute(
            "ant1",
            "feed1",
            GainCacheKey::new(45.0004, 10.0, 12000.0, 0.0, 0.0, 0.0),
            || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(9.0))
            },
        );

        assert_eq!(result.unwrap().value, 5.0); // cache hit, same bucket
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_per_feed_isolation() {
        let cache = GainCache::new(true, 100);
        let key = test_key(45.0);

        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", key.clone(), || {
                Ok(CachedGain::converged(10.0))
            });

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        // Different feed_id — should be a miss
        let result: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed2", key.clone(), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(20.0))
            });

        assert_eq!(result.unwrap().value, 20.0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_invalidate_clears_feed() {
        let cache = GainCache::new(true, 100);

        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                Ok(CachedGain::converged(7.0))
            });
        cache.invalidate("ant1", "feed1");

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(7.0))
            });

        assert_eq!(call_count.load(Ordering::SeqCst), 1); // had to recompute
    }

    #[test]
    fn test_invalidate_does_not_clear_other_feeds() {
        let cache = GainCache::new(true, 100);
        let key = test_key(45.0);

        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", key.clone(), || {
                Ok(CachedGain::converged(1.0))
            });
        let _: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed2", key.clone(), || {
                Ok(CachedGain::converged(2.0))
            });

        cache.invalidate("ant1", "feed1"); // only clear feed1

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        // feed2 should still be in cache
        let result: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed2", key.clone(), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(99.0))
            });
        assert_eq!(result.unwrap().value, 2.0);
        assert_eq!(call_count.load(Ordering::SeqCst), 0);
    }

    /// The convergence flag must survive a cache hit (roadmap C10).
    ///
    /// This is the property the whole [`CachedGain`] type exists for: a hit skips the
    /// integration, so if the flag did not ride along with the value there would be no
    /// way to know the served number came from a non-converged integral. Asserting the
    /// hit's `value` alone would pass even with the flag dropped, which is precisely how
    /// the pre-C10 bug survived.
    #[test]
    fn test_nonconvergence_flag_survives_cache_hit() {
        let cache = GainCache::new(true, 100);

        // Prime with a NON-converged value.
        let primed: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                Ok(CachedGain::new(12.5, false))
            });
        assert!(!primed.unwrap().converged);

        // Hit: the closure must not run, and the flag must come back false.
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let hit: crate::error::Result<CachedGain> =
            cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(CachedGain::converged(99.0))
            });

        let hit = hit.unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 0, "expected a cache hit");
        assert_eq!(hit.value, 12.5);
        assert!(
            !hit.converged,
            "a cache hit must report the stored convergence flag, not assume convergence"
        );
    }

    #[test]
    fn test_disabled_always_computes() {
        let cache = GainCache::new(false, 100); // disabled
        let call_count = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let cc = call_count.clone();
            let _: crate::error::Result<CachedGain> =
                cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                    cc.fetch_add(1, Ordering::SeqCst);
                    Ok(CachedGain::converged(5.0))
                });
        }

        assert_eq!(call_count.load(Ordering::SeqCst), 3); // called every time
    }
}
