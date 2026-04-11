//! LRU Gain Cache
//!
//! Caches physics model results keyed on quantized (az, el, freq, feed_position).
//! Per-feed caches are stored in a DashMap to avoid cross-feed lock contention.

use dashmap::DashMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type FeedCache = Mutex<LruCache<GainCacheKey, f64>>;

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
    pub fn new(az_deg: f64, el_deg: f64, freq_mhz: f64, feed_x: f64, feed_y: f64, feed_z: f64) -> Self {
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
    pub fn get_or_compute<F>(
        &self,
        antenna_id: &str,
        feed_id: &str,
        key: GainCacheKey,
        compute: F,
    ) -> crate::error::Result<f64>
    where
        F: FnOnce() -> crate::error::Result<f64>,
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
        self.caches.remove(&(antenna_id.to_string(), feed_id.to_string()));
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

        let result: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(12.5)
        });

        assert_eq!(result.unwrap(), 12.5);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cache_hit_skips_compute() {
        let cache = GainCache::new(true, 100);
        let call_count = Arc::new(AtomicUsize::new(0));

        // Prime the cache
        let cc = call_count.clone();
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(12.5)
        });

        // Second call should hit cache
        let cc = call_count.clone();
        let result: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(99.0)
        });

        assert_eq!(result.unwrap(), 12.5); // got cached value, not 99.0
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // compute only called once
    }

    #[test]
    fn test_lru_eviction() {
        let cache = GainCache::new(true, 2); // max 2 entries

        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(1.0), || Ok(1.0));
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(2.0), || Ok(2.0));
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(3.0), || Ok(3.0)); // evicts key(1.0)

        let call_count = Arc::new(AtomicUsize::new(0));
        // key(1.0) should be evicted — compute should be called again
        let cc = call_count.clone();
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(1.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(1.0)
        });
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_key_quantization_same_bucket() {
        let cache = GainCache::new(true, 100);
        let call_count = Arc::new(AtomicUsize::new(0));

        // 45.0000 and 45.0004 are within 0.0005° → same quantized key (both round to 45000 millideg)
        let cc = call_count.clone();
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", GainCacheKey::new(45.0000, 10.0, 12000.0, 0.0, 0.0, 0.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(5.0)
        });

        let cc = call_count.clone();
        let result: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", GainCacheKey::new(45.0004, 10.0, 12000.0, 0.0, 0.0, 0.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(9.0)
        });

        assert_eq!(result.unwrap(), 5.0); // cache hit, same bucket
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_per_feed_isolation() {
        let cache = GainCache::new(true, 100);
        let key = test_key(45.0);

        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", key.clone(), || Ok(10.0));

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        // Different feed_id — should be a miss
        let result: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed2", key.clone(), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(20.0)
        });

        assert_eq!(result.unwrap(), 20.0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_invalidate_clears_feed() {
        let cache = GainCache::new(true, 100);

        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || Ok(7.0));
        cache.invalidate("ant1", "feed1");

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(7.0)
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1); // had to recompute
    }

    #[test]
    fn test_invalidate_does_not_clear_other_feeds() {
        let cache = GainCache::new(true, 100);
        let key = test_key(45.0);

        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", key.clone(), || Ok(1.0));
        let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed2", key.clone(), || Ok(2.0));

        cache.invalidate("ant1", "feed1"); // only clear feed1

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        // feed2 should still be in cache
        let result: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed2", key.clone(), || {
            cc.fetch_add(1, Ordering::SeqCst);
            Ok(99.0)
        });
        assert_eq!(result.unwrap(), 2.0);
        assert_eq!(call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_disabled_always_computes() {
        let cache = GainCache::new(false, 100); // disabled
        let call_count = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let cc = call_count.clone();
            let _: crate::error::Result<f64> = cache.get_or_compute("ant1", "feed1", test_key(45.0), || {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(5.0)
            });
        }

        assert_eq!(call_count.load(Ordering::SeqCst), 3); // called every time
    }
}
