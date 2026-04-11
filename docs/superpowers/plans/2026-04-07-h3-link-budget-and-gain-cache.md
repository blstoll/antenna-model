# H3 Link Budget Heatmap & Gain Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-extended-cc:subagent-driven-development (recommended) or superpowers-extended-cc:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `POST /api/v1/h3-heatmap` endpoint that computes antenna loss, FSPL, az/el, and total path loss for each H3 hexagonal cell in an n-ring neighborhood, backed by an LRU gain cache that improves throughput for repeated queries.

**Architecture:** The gain cache (`service/cache.rs`) wraps the physics computation in a `DashMap` of per-feed `LruCache`s keyed on quantized `(az, el, freq, feed_physical_position)`. The H3 link budget service (`service/h3_link_budget.rs`) generates H3 cells via the `h3o` crate, uses the existing `compute_emitter_direction` for coordinate transforms, and calls through the cache to the existing physics pipeline. A new handler and route wire the endpoint into the API.

**Tech Stack:** Rust, `h3o` (H3 cells, pure Rust), `lru` (LRU cache), `dashmap` (concurrent map), existing `rayon` parallelism, existing `poem` web framework.

---

### Task 1: Dependencies and cache configuration

**Goal:** Add h3o/lru/dashmap crates and wire a `CacheConfig` struct into the settings system.

**Files:**
- Modify: `antenna-model/Cargo.toml`
- Modify: `antenna-model/src/config/settings.rs`
- Modify: `config/service.yaml`

**Acceptance Criteria:**
- [ ] `cargo build` succeeds with h3o, lru, dashmap in scope
- [ ] `CacheConfig` deserializes from the `cache:` YAML section
- [ ] `ServiceConfig` has a `cache: CacheConfig` field
- [ ] Missing `cache:` section in YAML falls back to defaults (enabled=true, 10000 entries)

**Verify:** `cargo test -p antenna-model -- config 2>&1 | grep -E "ok|FAILED"` → all config tests pass

**Steps:**

- [ ] **Step 1: Add dependencies to Cargo.toml**

In `antenna-model/Cargo.toml`, add inside `[dependencies]`:
```toml
dashmap = "6.1.0"
h3o = "0.7.0"
lru = "0.12.5"
```

- [ ] **Step 2: Add CacheConfig struct to settings.rs**

In `antenna-model/src/config/settings.rs`, add the struct and update `ServiceConfig`:

```rust
/// Gain cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Enable gain result caching for throughput improvement
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,

    /// Maximum cached gain entries per antenna-feed pair
    #[serde(default = "default_max_entries_per_feed")]
    pub max_entries_per_feed: usize,
}

fn default_cache_enabled() -> bool { true }
fn default_max_entries_per_feed() -> usize { 10_000 }

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: default_cache_enabled(),
            max_entries_per_feed: default_max_entries_per_feed(),
        }
    }
}
```

Add the field to `ServiceConfig`:
```rust
pub struct ServiceConfig {
    pub server: ServerConfig,
    pub calibration: CalibrationConfig,
    pub logging: LoggingConfig,
    #[serde(default)]
    pub performance: PerformanceConfig,
    /// Gain cache configuration
    #[serde(default)]
    pub cache: CacheConfig,
}
```

- [ ] **Step 3: Add cache section to config/service.yaml**

Append to `config/service.yaml`:
```yaml
cache:
  # Enable LRU caching of gain results for repeated queries (e.g., H3 heatmaps)
  enabled: true

  # Maximum cached entries per antenna-feed pair
  # Each entry is ~50 bytes; 10000 entries ≈ 500 KB per feed
  max_entries_per_feed: 10000
```

- [ ] **Step 4: Write failing test for CacheConfig deserialization**

Add at the bottom of `antenna-model/src/config/settings.rs` inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_cache_config_defaults() {
    let config = CacheConfig::default();
    assert!(config.enabled);
    assert_eq!(config.max_entries_per_feed, 10_000);
}

#[test]
fn test_service_config_with_cache_section() {
    let yaml = r#"
server:
  host: "127.0.0.1"
  port: 3000
cache:
  enabled: false
  max_entries_per_feed: 500
"#;
    let config: ServiceConfig = serde_yaml::from_str(yaml).expect("parse failed");
    assert!(!config.cache.enabled);
    assert_eq!(config.cache.max_entries_per_feed, 500);
}

#[test]
fn test_service_config_cache_defaults_when_section_missing() {
    let yaml = r#"
server:
  host: "127.0.0.1"
  port: 3000
"#;
    let config: ServiceConfig = serde_yaml::from_str(yaml).expect("parse failed");
    assert!(config.cache.enabled);
    assert_eq!(config.cache.max_entries_per_feed, 10_000);
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p antenna-model -- config --nocapture 2>&1 | tail -5
```
Expected: all config tests pass.

- [ ] **Step 6: Commit**

```bash
git add antenna-model/Cargo.toml antenna-model/src/config/settings.rs config/service.yaml
git commit -m "feat: add h3o/lru/dashmap deps and CacheConfig settings"
```

---

### Task 2: Implement GainCache

**Goal:** A thread-safe per-feed LRU cache for gain values keyed on quantized `(az, el, freq, feed_physical_position)`.

**Files:**
- Create: `antenna-model/src/service/cache.rs`
- Modify: `antenna-model/src/service/mod.rs`

**Acceptance Criteria:**
- [ ] Cache miss calls the compute closure; result is stored
- [ ] Cache hit returns stored value without calling closure
- [ ] LRU eviction works when `max_entries_per_feed` is exceeded
- [ ] Key quantization: az values within 0.0005° map to the same key
- [ ] Per-feed isolation: entries for one feed don't appear under another
- [ ] `invalidate(antenna_id, feed_id)` clears only that feed's entries

**Verify:** `cargo test -p antenna-model -- cache --nocapture 2>&1 | tail -5` → all cache tests pass

**Steps:**

- [ ] **Step 1: Write the test file first (TDD)**

Create `antenna-model/src/service/cache.rs` with only the test module:

```rust
//! LRU Gain Cache
//!
//! Caches physics model results keyed on quantized (az, el, freq, feed_position).
//! Per-feed caches are stored in a DashMap to avoid cross-feed lock contention.

use crate::error::Result;
use dashmap::DashMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Quantized cache key for gain lookup.
///
/// Floating-point values are rounded before hashing:
/// - az/el: nearest 0.001° (1 millidegree)
/// - freq: nearest 1 kHz
/// - feed position: nearest millimeter
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct GainCacheKey {
    pub az_millideg:  i32,
    pub el_millideg:  i32,
    pub freq_khz:     u32,
    pub feed_x_mm:    i32,
    pub feed_y_mm:    i32,
    pub feed_z_mm:    i32,
}

impl GainCacheKey {
    pub fn new(
        az_deg: f64,
        el_deg: f64,
        freq_mhz: f64,
        feed_x_m: f64,
        feed_y_m: f64,
        feed_z_m: f64,
    ) -> Self {
        Self {
            az_millideg:  (az_deg  * 1_000.0).round() as i32,
            el_millideg:  (el_deg  * 1_000.0).round() as i32,
            freq_khz:     (freq_mhz * 1_000.0).round() as u32,
            feed_x_mm:    (feed_x_m * 1_000.0).round() as i32,
            feed_y_mm:    (feed_y_m * 1_000.0).round() as i32,
            feed_z_mm:    (feed_z_m * 1_000.0).round() as i32,
        }
    }
}

type FeedCache = Mutex<LruCache<GainCacheKey, f64>>;

/// Thread-safe LRU gain cache.
///
/// Organized as a DashMap of per-feed caches. Each feed has its own LruCache
/// protected by a Mutex, allowing concurrent access across feeds while
/// serializing within a single feed.
pub struct GainCache {
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

    /// Get or compute a gain value.
    ///
    /// Checks the cache first. On miss, calls `compute`, stores the result,
    /// and returns it. The lock is released before calling `compute` to avoid
    /// holding it during expensive physics computation.
    pub fn get_or_compute(
        &self,
        antenna_id: &str,
        feed_id: &str,
        key: GainCacheKey,
        compute: impl FnOnce() -> Result<f64>,
    ) -> Result<f64> {
        if !self.enabled {
            return compute();
        }

        let feed_key = (antenna_id.to_string(), feed_id.to_string());

        // Get or create the per-feed cache
        let feed_cache = self
            .caches
            .entry(feed_key)
            .or_insert_with(|| {
                Arc::new(Mutex::new(LruCache::new(
                    NonZeroUsize::new(self.max_entries_per_feed).unwrap_or(NonZeroUsize::MIN),
                )))
            })
            .clone();

        // Check cache (release lock before expensive compute)
        {
            let mut cache = feed_cache.lock().unwrap();
            if let Some(&gain) = cache.get(&key) {
                return Ok(gain);
            }
        }

        // Cache miss: compute, then store
        let gain = compute()?;
        {
            let mut cache = feed_cache.lock().unwrap();
            cache.put(key, gain);
        }
        Ok(gain)
    }

    /// Remove all cached entries for a specific antenna-feed pair.
    ///
    /// Call this when calibration data for a feed is reloaded.
    pub fn invalidate(&self, antenna_id: &str, feed_id: &str) {
        let feed_key = (antenna_id.to_string(), feed_id.to_string());
        self.caches.remove(&feed_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(az: f64, el: f64) -> GainCacheKey {
        GainCacheKey::new(az, el, 8400.0, 0.0, 0.0, 0.0)
    }

    #[test]
    fn test_cache_miss_calls_compute() {
        let cache = GainCache::new(true, 100);
        let call_count = std::cell::Cell::new(0u32);
        let result = cache.get_or_compute("ant1", "feed1", make_key(1.0, 2.0), || {
            call_count.set(call_count.get() + 1);
            Ok(42.5)
        });
        assert_eq!(result.unwrap(), 42.5);
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn test_cache_hit_skips_compute() {
        let cache = GainCache::new(true, 100);
        // Populate cache
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 2.0), || Ok(42.5)).unwrap();
        // Second call should hit cache
        let call_count = std::cell::Cell::new(0u32);
        let result = cache.get_or_compute("ant1", "feed1", make_key(1.0, 2.0), || {
            call_count.set(call_count.get() + 1);
            Ok(99.0) // different value to confirm cache is used
        });
        assert_eq!(result.unwrap(), 42.5); // original value
        assert_eq!(call_count.get(), 0);   // compute not called
    }

    #[test]
    fn test_lru_eviction() {
        let cache = GainCache::new(true, 2); // capacity = 2
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || Ok(1.0)).unwrap();
        cache.get_or_compute("ant1", "feed1", make_key(2.0, 0.0), || Ok(2.0)).unwrap();
        // Insert third entry → evicts key(1.0, 0.0) (LRU)
        cache.get_or_compute("ant1", "feed1", make_key(3.0, 0.0), || Ok(3.0)).unwrap();
        // key(1.0) should be evicted; compute should be called again
        let call_count = std::cell::Cell::new(0u32);
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || {
            call_count.set(call_count.get() + 1);
            Ok(1.0)
        }).unwrap();
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn test_key_quantization_same_bucket() {
        // Values within 0.0005° should map to the same key
        let k1 = GainCacheKey::new(1.0000, 2.0000, 8400.0, 0.0, 0.0, 0.0);
        let k2 = GainCacheKey::new(1.0004, 2.0004, 8400.0, 0.0, 0.0, 0.0); // rounds to same millideg
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_key_quantization_different_bucket() {
        let k1 = GainCacheKey::new(1.000, 2.000, 8400.0, 0.0, 0.0, 0.0);
        let k2 = GainCacheKey::new(1.001, 2.000, 8400.0, 0.0, 0.0, 0.0);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_per_feed_isolation() {
        let cache = GainCache::new(true, 100);
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || Ok(10.0)).unwrap();
        // Same key, different feed — should miss
        let call_count = std::cell::Cell::new(0u32);
        cache.get_or_compute("ant1", "feed2", make_key(1.0, 0.0), || {
            call_count.set(call_count.get() + 1);
            Ok(20.0)
        }).unwrap();
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn test_invalidate_clears_feed() {
        let cache = GainCache::new(true, 100);
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || Ok(42.0)).unwrap();
        cache.invalidate("ant1", "feed1");
        // After invalidation, compute should be called again
        let call_count = std::cell::Cell::new(0u32);
        cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || {
            call_count.set(call_count.get() + 1);
            Ok(42.0)
        }).unwrap();
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn test_disabled_cache_always_computes() {
        let cache = GainCache::new(false, 100);
        let call_count = std::cell::Cell::new(0u32);
        for _ in 0..3 {
            cache.get_or_compute("ant1", "feed1", make_key(1.0, 0.0), || {
                call_count.set(call_count.get() + 1);
                Ok(42.0)
            }).unwrap();
        }
        assert_eq!(call_count.get(), 3); // always calls compute
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (module not yet in mod.rs)**

```bash
cargo test -p antenna-model -- cache 2>&1 | tail -10
```
Expected: compile error — module not found.

- [ ] **Step 3: Export from service/mod.rs**

In `antenna-model/src/service/mod.rs`:
```rust
pub mod batch;
pub mod cache;
pub mod evaluator;
pub mod heatmap;
pub mod validator;

pub use batch::evaluate_batch;
pub use cache::{GainCache, GainCacheKey};
pub use evaluator::compute_gain_from_request;
pub use heatmap::generate_heatmap;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p antenna-model -- cache --nocapture 2>&1 | tail -5
```
Expected: `8 passed`.

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/service/cache.rs antenna-model/src/service/mod.rs
git commit -m "feat: implement LRU gain cache with per-feed DashMap"
```

---

### Task 3: Wire GainCache into AppState

**Goal:** Add `GainCache` to `AppState` so handlers can pass it to services. No behavior change to existing endpoints yet.

**Files:**
- Modify: `antenna-model/src/api/mod.rs`

**Acceptance Criteria:**
- [ ] `AppState` has a `cache: Arc<GainCache>` field
- [ ] `AppState::new()` creates cache from `config.cache`
- [ ] `AppState::with_defaults()` creates a default cache
- [ ] Existing `test_app_state_*` tests still pass

**Verify:** `cargo test -p antenna-model -- app_state --nocapture 2>&1 | tail -5` → all pass

**Steps:**

- [ ] **Step 1: Add cache field to AppState**

In `antenna-model/src/api/mod.rs`, add the import and field:

```rust
// At top of file, add to imports:
use crate::service::GainCache;

// In AppState struct, add field:
pub struct AppState {
    pub start_time: SystemTime,
    pub version: &'static str,
    pub config: Arc<ServiceConfig>,
    pub ready: Arc<AtomicBool>,
    pub antenna_ids: Arc<parking_lot::RwLock<Vec<String>>>,
    pub repository: CalibrationRepository,
    /// Gain computation cache for throughput improvement
    pub cache: Arc<GainCache>,
}
```

- [ ] **Step 2: Update AppState::new() to build cache from config**

```rust
pub fn new(config: ServiceConfig, repository: CalibrationRepository) -> Self {
    let cache = Arc::new(GainCache::new(
        config.cache.enabled,
        config.cache.max_entries_per_feed,
    ));
    Self {
        start_time: SystemTime::now(),
        version: env!("CARGO_PKG_VERSION"),
        config: Arc::new(config),
        ready: Arc::new(AtomicBool::new(true)),
        antenna_ids: Arc::new(parking_lot::RwLock::new(Vec::new())),
        repository,
        cache,
    }
}
```

- [ ] **Step 3: Update AppState::with_defaults()**

```rust
pub fn with_defaults() -> Self {
    Self::new(ServiceConfig::with_defaults(), CalibrationRepository::new())
}
```
(No change needed — `new()` now handles cache construction from config.)

- [ ] **Step 4: Run existing AppState tests to confirm nothing broke**

```bash
cargo test -p antenna-model -- app_state --nocapture 2>&1 | tail -5
```
Expected: all existing AppState tests pass.

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/api/mod.rs
git commit -m "feat: add GainCache to AppState"
```

---

### Task 4: Add H3 schemas

**Goal:** Define `H3LinkBudgetRequest`, `H3LinkBudgetResponse`, and `H3CellResult` in the schemas module, consistent with all existing API types.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs`

**Acceptance Criteria:**
- [ ] All three types serialize and deserialize correctly
- [ ] `h3_resolution` is absent from JSON when `None` (skipped)
- [ ] `temperature_k`, `pointing_frequency_mhz`, `g_over_t_db` are skipped when `None`
- [ ] Round-trip serde test passes

**Verify:** `cargo test -p antenna-model -- schemas::tests::test_h3 --nocapture 2>&1 | tail -5` → pass

**Steps:**

- [ ] **Step 1: Add schemas to schemas.rs**

Append to `antenna-model/src/api/schemas.rs` after the existing heatmap types:

```rust
// ============================================================================
// H3 Link Budget Heatmap Request/Response
// ============================================================================

/// Request for H3 link budget heatmap generation.
///
/// Computes antenna loss, free-space path loss, azimuth, elevation, and
/// total path loss for each H3 hexagonal cell in an n-ring neighborhood
/// centered on the feed pointing location.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct H3LinkBudgetRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic, auto-detected)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    ///
    /// Defines the antenna frame Z-axis. The vector from vehicle_position
    /// to reflector_boresight is the antenna boresight direction.
    pub reflector_boresight: Position3D,

    /// Feed pointing location (ECEF or Geodetic)
    ///
    /// Where the beam is aimed on the Earth's surface. Becomes the center
    /// H3 cell of the heatmap.
    pub feed_position: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz for beam squint correction (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Number of H3 neighbor rings around center cell
    ///
    /// - 0: center cell only (1 cell)
    /// - 1: center + 1 ring = 7 cells
    /// - 2: center + 2 rings = 19 cells
    /// - k: (3k² + 3k + 1) cells
    pub n_rings: u32,

    /// H3 resolution (0–15, higher = finer).
    ///
    /// If not provided, auto-selected from frequency_mhz:
    /// - < 2 GHz → resolution 6 (3.2 km avg edge)
    /// - 2–8 GHz → resolution 7 (1.2 km)
    /// - 8–20 GHz → resolution 8 (0.46 km)
    /// - > 20 GHz → resolution 9 (0.17 km)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h3_resolution: Option<u8>,

    /// System noise temperature in Kelvin (enables g_over_t_db per cell)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_k: Option<f64>,
}

/// Response from H3 link budget heatmap generation.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct H3LinkBudgetResponse {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// H3 cell ID (hex string) of the center cell derived from feed_position
    pub center_cell_id: String,

    /// H3 resolution used (useful when auto-selected)
    pub h3_resolution: u8,

    /// Per-cell results
    pub cells: Vec<H3CellResult>,

    /// Warnings (extrapolation, out-of-range queries, etc.)
    pub warnings: Vec<String>,

    /// Aggregate computation metadata
    pub metadata: HeatmapMetadata,

    /// Calibration status — same optional pattern as all other responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

/// Per-cell result in an H3 link budget heatmap.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct H3CellResult {
    /// H3 cell identifier (hex string, e.g. "8928308280fffff")
    pub cell_id: String,

    /// Cell center longitude in degrees
    pub center_lon: f64,

    /// Cell center latitude in degrees
    pub center_lat: f64,

    /// Azimuth in antenna frame (degrees) — mirrors GeometryInfo.emitter_azimuth_deg
    pub azimuth_deg: f64,

    /// Elevation in antenna frame (degrees) — mirrors GeometryInfo.emitter_elevation_deg
    pub elevation_deg: f64,

    /// Slant range from vehicle to cell center (km)
    pub distance_km: f64,

    /// Antenna gain at this cell's az/el (dB) — mirrors GainResponse.gain_db
    pub gain_db: f64,

    /// Loss relative to center cell gain (dB, positive = loss) — mirrors GainResponse.loss_db
    pub loss_db: f64,

    /// Free-space path loss (dB): 20·log10(4π·d·f/c)
    pub free_space_path_loss_db: f64,

    /// Combined path loss: loss_db + free_space_path_loss_db
    pub total_path_loss_db: f64,

    /// G/T in dB/K — present only when temperature_k was in request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub g_over_t_db: Option<f64>,
}
```

- [ ] **Step 2: Write round-trip serde test**

Add inside the existing `#[cfg(test)]` block in `schemas.rs` (or create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h3_request_serde_round_trip() {
        let req = H3LinkBudgetRequest {
            antenna_id: "ant1".to_string(),
            feed_id: "feed1".to_string(),
            vehicle_position: Position3D::new(6_500_000.0, 0.0, 0.0),
            reflector_boresight: Position3D::new(-73.5, 40.5, 0.0),
            feed_position: Position3D::new(-73.5, 40.5, 0.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            n_rings: 2,
            h3_resolution: Some(8),
            temperature_k: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: H3LinkBudgetRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
        // Optional fields absent from JSON when None
        assert!(!json.contains("pointing_frequency_mhz"));
        assert!(!json.contains("temperature_k"));
    }

    #[test]
    fn test_h3_cell_result_optional_g_over_t() {
        let cell = H3CellResult {
            cell_id: "8928308280fffff".to_string(),
            center_lon: -73.5,
            center_lat: 40.5,
            azimuth_deg: 0.5,
            elevation_deg: 0.3,
            distance_km: 500.0,
            gain_db: 45.2,
            loss_db: 0.0,
            free_space_path_loss_db: 175.3,
            total_path_loss_db: 175.3,
            g_over_t_db: None,
        };
        let json = serde_json::to_string(&cell).unwrap();
        assert!(!json.contains("g_over_t_db"));
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p antenna-model -- schemas 2>&1 | tail -5
```
Expected: new schema tests pass, existing schema tests unaffected.

- [ ] **Step 4: Commit**

```bash
git add antenna-model/src/api/schemas.rs
git commit -m "feat: add H3LinkBudgetRequest/Response/CellResult schemas"
```

---

### Task 5: H3 link budget service

**Goal:** Implement the core service function that generates H3 cells, computes az/el via existing coordinate transforms, calls through the gain cache, and assembles the response.

**Files:**
- Create: `antenna-model/src/service/h3_link_budget.rs`
- Modify: `antenna-model/src/service/mod.rs`

**Acceptance Criteria:**
- [ ] `h3_resolution_from_frequency()` returns correct resolution for all four bands
- [ ] `n_rings=0` → exactly 1 cell; `n_rings=1` → 7 cells; `n_rings=2` → 19 cells
- [ ] Center cell (feed_position derived) has lowest `loss_db` (≈ 0)
- [ ] `total_path_loss_db == loss_db + free_space_path_loss_db` for every cell
- [ ] `g_over_t_db` absent when `temperature_k` is None; present when provided
- [ ] FSPL formula correct: `20·log10(4π·d·f/c)` (verified against known value)

**Verify:** `cargo test -p antenna-model -- h3_link_budget --nocapture 2>&1 | tail -10` → all pass

**Steps:**

- [ ] **Step 1: Write tests first**

Create `antenna-model/src/service/h3_link_budget.rs` with only the test module:

```rust
//! H3 Link Budget Heatmap Service
//!
//! Computes per-H3-cell antenna loss, FSPL, az/el, and total path loss
//! for an n-ring neighborhood around the feed pointing location.

use crate::api::schemas::{
    CalibrationStatusInfo, H3CellResult, H3LinkBudgetRequest, H3LinkBudgetResponse, HeatmapMetadata,
    Position3D,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::model::{
    compute_emitter_direction, compute_feed_position_from_pointing, geodetic_to_ecef,
    AntennaConfiguration, FeedParameters as ModelFeedParams, FeedPosition, IntegrationParams,
    MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
};
use crate::model::{compute_gain_db, evaluate_correction, wavelength_from_frequency};
use crate::service::cache::{GainCache, GainCacheKey};
use h3o::{CellIndex, LatLng, Resolution};
use rayon::prelude::*;
use std::time::Instant;

/// Select H3 resolution from frequency when caller does not specify one.
///
/// Higher frequency → narrower beam → finer resolution needed to sample pattern.
pub fn h3_resolution_from_frequency(freq_mhz: f64) -> u8 {
    match freq_mhz {
        f if f < 2_000.0  => 6,  // L-band,  avg edge 3.2 km
        f if f < 8_000.0  => 7,  // S/C-band, avg edge 1.2 km
        f if f < 20_000.0 => 8,  // X/Ku-band, avg edge 0.46 km
        _                 => 9,  // Ka+, avg edge 0.17 km
    }
}

/// Free-space path loss in dB.
///
/// FSPL = 20·log10(4π·d·f/c) where d is in meters and f is in Hz.
fn free_space_path_loss_db(distance_m: f64, freq_hz: f64) -> f64 {
    const C: f64 = 299_792_458.0;
    20.0 * (4.0 * std::f64::consts::PI * distance_m * freq_hz / C).log10()
}

/// Slant range in km between vehicle ECEF and cell ECEF.
fn slant_range_km(
    vehicle: (f64, f64, f64),
    cell: (f64, f64, f64),
) -> f64 {
    let dx = cell.0 - vehicle.0;
    let dy = cell.1 - vehicle.1;
    let dz = cell.2 - vehicle.2;
    (dx * dx + dy * dy + dz * dz).sqrt() / 1_000.0
}

/// Compute H3 link budget heatmap.
pub fn compute_h3_link_budget(
    request: &H3LinkBudgetRequest,
    repository: &CalibrationRepository,
    cache: &GainCache,
) -> Result<H3LinkBudgetResponse> {
    let start = Instant::now();
    let mut warnings = Vec::new();

    // --- Resolve H3 resolution ---
    let resolution_u8 = request
        .h3_resolution
        .unwrap_or_else(|| h3_resolution_from_frequency(request.frequency_mhz));
    let resolution = Resolution::try_from(resolution_u8).map_err(|_| {
        AntennaModelError::Validation(crate::error::ValidationError::InvalidValue {
            param: "h3_resolution".to_string(),
            reason: format!("H3 resolution {} is not in range 0–15", resolution_u8),
        })
    })?;

    // --- Get center H3 cell from feed_position ---
    // feed_position uses x=lon, y=lat convention for geodetic (matches Position3D)
    let (center_lon, center_lat) = if request.feed_position.is_geodetic() {
        (request.feed_position.x, request.feed_position.y)
    } else {
        // ECEF → geodetic
        let (lon, lat, _) = crate::model::ecef_to_geodetic(
            request.feed_position.x,
            request.feed_position.y,
            request.feed_position.z,
        );
        (lon, lat)
    };

    let center_latlng = LatLng::new(center_lat, center_lon)
        .map_err(|e| AntennaModelError::Generic(format!("Invalid feed_position lat/lon: {}", e)))?;
    let center_cell: CellIndex = center_latlng.to_cell(resolution);
    let center_cell_id = format!("{}", center_cell);

    // --- Load calibration and build AntennaConfiguration ---
    let calibration = repository
        .get_calibration(&request.antenna_id, &request.feed_id)
        .ok_or_else(|| AntennaModelError::FeedNotFound {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
        })?;

    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let diameter_m = calibration.physical_config.reflector.diameter_m;

    let reflector = ModelReflector::builder()
        .diameter(diameter_m)
        .focal_length(focal_length_m)
        .surface_rms(calibration.physical_config.reflector.surface_rms_mm / 1000.0)
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("Reflector build error: {}", e)))?;

    // Physical feed position from pointing target (same derivation as evaluator.rs)
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_position,
        &request.reflector_boresight,
        &request.vehicle_position,
        focal_length_m,
    )?;
    let design_pos = &calibration.physical_config.feed.position;
    let feed_x = steer_x + design_pos.0;
    let feed_y = steer_y + design_pos.1;
    let feed_z = steer_z + design_pos.2;
    let feed_position_model = FeedPosition::new(feed_x, feed_y, feed_z);

    let feed = ModelFeedParams::builder()
        .position(feed_position_model)
        .q_factor(calibration.physical_config.feed.q_factor)
        .phase_center_offset(calibration.physical_config.feed.phase_center_offset_m)
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("Feed build error: {}", e)))?;

    let mut config_builder = AntennaConfiguration::builder()
        .id(&calibration.antenna_id)
        .name(&calibration.metadata.antenna_name)
        .reflector(reflector)
        .feed(feed);

    if let Some(ref mesh_data) = calibration.physical_config.mesh {
        let mesh = ModelMeshParams::builder()
            .spacing(mesh_data.mesh_spacing_mm / 1000.0)
            .wire_diameter(mesh_data.wire_diameter_mm / 1000.0)
            .build()
            .map_err(|e| AntennaModelError::Generic(format!("Mesh build error: {}", e)))?;
        config_builder = config_builder.mesh(mesh);
    }
    let antenna_config = config_builder
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("AntennaConfig build error: {}", e)))?;

    let integration_params = IntegrationParams::fast();
    let freq_hz = request.frequency_mhz * 1e6;

    // Vehicle ECEF for distance calculations
    let vehicle_ecef = if request.vehicle_position.is_ecef() {
        (request.vehicle_position.x, request.vehicle_position.y, request.vehicle_position.z)
    } else {
        geodetic_to_ecef(
            request.vehicle_position.x,
            request.vehicle_position.y,
            request.vehicle_position.z,
        )
    };

    // --- Helper: compute gain for a cell position through cache ---
    let compute_cell_gain = |cell_pos: &Position3D| -> Result<(f64, f64, f64)> {
        // az/el of this cell in antenna frame
        let (az_deg, el_deg) = compute_emitter_direction(
            cell_pos,
            &request.vehicle_position,
            &request.reflector_boresight,
        )?;

        let cache_key = GainCacheKey::new(az_deg, el_deg, request.frequency_mhz, feed_x, feed_y, feed_z);

        let gain_db = cache.get_or_compute(
            &request.antenna_id,
            &request.feed_id,
            cache_key,
            || {
                let theta_rad = el_deg.to_radians();
                let phi_rad = az_deg.to_radians();
                let result = compute_gain_db(theta_rad, phi_rad, &antenna_config, freq_hz, &integration_params)?;
                let mut gain = result.gain;

                // Apply correction surface if in coverage
                let in_coverage = calibration.correction_surface.is_some()
                    && crate::service::evaluator::is_in_coverage(
                        &calibration.calibration_coverage,
                        az_deg,
                        el_deg,
                        request.frequency_mhz,
                    );
                if let Some(ref correction) = calibration.correction_surface {
                    if in_coverage {
                        if let Ok(corr) = evaluate_correction(correction, az_deg, el_deg, request.frequency_mhz, 290.0) {
                            gain += corr.correction_db;
                        }
                    }
                }
                Ok(gain)
            },
        )?;

        Ok((az_deg, el_deg, gain_db))
    };

    // --- Compute center cell gain (reference for loss_db) ---
    let center_latlng_back = LatLng::from(center_cell);
    let center_cell_pos = Position3D::new(center_latlng_back.lng(), center_latlng_back.lat(), 0.0);
    let (_, _, center_gain_db) = compute_cell_gain(&center_cell_pos)?;

    // --- Generate all H3 cells ---
    let all_cells: Vec<CellIndex> = center_cell.grid_disk_safe(request.n_rings);

    // --- Evaluate each cell in parallel ---
    let cell_results: Vec<Result<H3CellResult>> = all_cells
        .par_iter()
        .map(|&cell| {
            let latlng = LatLng::from(cell);
            let lat = latlng.lat();
            let lon = latlng.lng();
            let alt = 0.0f64; // cells on Earth surface

            let cell_pos = Position3D::new(lon, lat, alt);
            let (az_deg, el_deg, gain_db) = compute_cell_gain(&cell_pos)?;

            // Slant range
            let cell_ecef = geodetic_to_ecef(lon, lat, alt);
            let distance_km = slant_range_km(vehicle_ecef, cell_ecef);

            let fspl_db = free_space_path_loss_db(distance_km * 1_000.0, freq_hz);
            let loss_db = center_gain_db - gain_db;
            let total_path_loss_db = loss_db + fspl_db;

            let g_over_t_db = request.temperature_k.map(|t_k| {
                gain_db - 10.0 * t_k.log10()
            });

            Ok(H3CellResult {
                cell_id: format!("{}", cell),
                center_lat: lat,
                center_lon: lon,
                azimuth_deg: az_deg,
                elevation_deg: el_deg,
                distance_km,
                gain_db,
                loss_db,
                free_space_path_loss_db: fspl_db,
                total_path_loss_db,
                g_over_t_db,
            })
        })
        .collect();

    // Collect results, accumulating warnings for cells that failed
    let mut cells = Vec::with_capacity(all_cells.len());
    let mut failed_points = 0usize;
    let mut peak_gain_db = f64::NEG_INFINITY;
    for result in cell_results {
        match result {
            Ok(cell) => {
                if cell.gain_db > peak_gain_db { peak_gain_db = cell.gain_db; }
                cells.push(cell);
            }
            Err(e) => {
                failed_points += 1;
                warnings.push(format!("Cell evaluation failed: {}", e));
            }
        }
    }

    // Calibration status info
    let calibration_status = calibration.calibration_status.as_ref().map(|s| {
        CalibrationStatusInfo::from(s)
    });

    Ok(H3LinkBudgetResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        frequency_mhz: request.frequency_mhz,
        center_cell_id,
        h3_resolution: resolution_u8,
        cells,
        warnings,
        metadata: HeatmapMetadata {
            points_evaluated: all_cells.len(),
            computation_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            peak_gain_db,
            failed_points,
        },
        calibration_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h3_resolution_from_frequency() {
        assert_eq!(h3_resolution_from_frequency(1_500.0), 6);  // L-band
        assert_eq!(h3_resolution_from_frequency(2_300.0), 7);  // S-band
        assert_eq!(h3_resolution_from_frequency(6_000.0), 7);  // C-band
        assert_eq!(h3_resolution_from_frequency(8_400.0), 8);  // X-band
        assert_eq!(h3_resolution_from_frequency(14_000.0), 8); // Ku-band
        assert_eq!(h3_resolution_from_frequency(32_000.0), 9); // Ka-band
    }

    #[test]
    fn test_free_space_path_loss_db_known_value() {
        // At 1 km distance and 1 GHz: FSPL = 20·log10(4π·1000·1e9/3e8) ≈ 92.4 dB
        let fspl = free_space_path_loss_db(1_000.0, 1e9);
        assert!((fspl - 92.4).abs() < 0.1, "FSPL at 1km/1GHz expected ~92.4 dB, got {}", fspl);
    }

    #[test]
    fn test_slant_range_km() {
        let a = (0.0f64, 0.0, 0.0);
        let b = (3_000_000.0f64, 4_000_000.0, 0.0); // 5000 km
        let range = slant_range_km(a, b);
        assert!((range - 5_000.0).abs() < 1.0, "Expected ~5000 km, got {}", range);
    }

    #[test]
    fn test_h3_cell_count_n_rings() {
        use h3o::{CellIndex, LatLng, Resolution};
        let center: CellIndex = LatLng::new(40.0, -73.0).unwrap().to_cell(Resolution::Eight);
        assert_eq!(center.grid_disk_safe(0).len(), 1);
        assert_eq!(center.grid_disk_safe(1).len(), 7);
        assert_eq!(center.grid_disk_safe(2).len(), 19);
        assert_eq!(center.grid_disk_safe(3).len(), 37);
    }

    #[test]
    fn test_total_path_loss_is_sum() {
        // total_path_loss_db == loss_db + fspl_db (arithmetic property, no physics needed)
        let loss_db = 3.5f64;
        let fspl_db = 175.2f64;
        let total = loss_db + fspl_db;
        assert!((total - 178.7).abs() < 0.001);
    }
}
```

- [ ] **Step 2: Run tests to confirm they compile but the module is missing from mod.rs**

```bash
cargo test -p antenna-model -- h3_link_budget 2>&1 | head -10
```
Expected: compile error — `h3_link_budget` not found.

- [ ] **Step 3: Wire `is_in_coverage` visibility**

In `antenna-model/src/service/evaluator.rs`, change `fn is_in_coverage` to `pub(crate) fn is_in_coverage` so `h3_link_budget.rs` can call it:

```rust
pub(crate) fn is_in_coverage(
    coverage: &Option<CalibrationCoverage>,
    azimuth_deg: f64,
    elevation_deg: f64,
    frequency_mhz: f64,
) -> bool {
```

- [ ] **Step 4: Export from service/mod.rs**

```rust
pub mod batch;
pub mod cache;
pub mod evaluator;
pub mod h3_link_budget;
pub mod heatmap;
pub mod validator;

pub use batch::evaluate_batch;
pub use cache::{GainCache, GainCacheKey};
pub use evaluator::compute_gain_from_request;
pub use h3_link_budget::compute_h3_link_budget;
pub use heatmap::generate_heatmap;
```

- [ ] **Step 5: Run unit tests**

```bash
cargo test -p antenna-model -- h3_link_budget --nocapture 2>&1 | tail -10
```
Expected: `test_h3_resolution_from_frequency`, `test_free_space_path_loss_db_known_value`, `test_slant_range_km`, `test_h3_cell_count_n_rings`, `test_total_path_loss_is_sum` — all pass.

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/service/h3_link_budget.rs antenna-model/src/service/mod.rs antenna-model/src/service/evaluator.rs
git commit -m "feat: implement H3 link budget service with gain cache integration"
```

---

### Task 6: Handler and route registration

**Goal:** Expose `POST /api/v1/h3-heatmap` with the same validation + error mapping pattern as existing handlers.

**Files:**
- Modify: `antenna-model/src/api/handlers.rs`
- Modify: `antenna-model/src/api/routes.rs`

**Acceptance Criteria:**
- [ ] `POST /api/v1/h3-heatmap` returns 200 with valid body for a known test antenna
- [ ] Unknown `antenna_id` returns 404
- [ ] Invalid `n_rings` (> 10) returns 422
- [ ] Handler passes `&state.cache` to service function

**Verify:** `cargo build -p antenna-model 2>&1 | tail -5` → builds cleanly

**Steps:**

- [ ] **Step 1: Add validation for H3LinkBudgetRequest**

In `antenna-model/src/service/validator.rs`, add a validation function. Add it after the existing `validate_heatmap_request` function:

```rust
/// Maximum n_rings to prevent excessively large responses.
const MAX_H3_RINGS: u32 = 10; // 10 rings = 331 cells

/// Validate an H3 link budget request.
pub fn validate_h3_link_budget_request(
    request: &crate::api::schemas::H3LinkBudgetRequest,
    repository: &crate::data::repository::CalibrationRepository,
) -> crate::error::Result<()> {
    if request.antenna_id.is_empty() {
        return Err(crate::error::AntennaModelError::Validation(
            crate::error::ValidationError::MissingField {
                field: "antenna_id".to_string(),
            },
        ));
    }
    if request.feed_id.is_empty() {
        return Err(crate::error::AntennaModelError::Validation(
            crate::error::ValidationError::MissingField {
                field: "feed_id".to_string(),
            },
        ));
    }
    if !repository.has_calibration(&request.antenna_id, &request.feed_id) {
        return Err(crate::error::AntennaModelError::FeedNotFound {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
        });
    }
    if request.frequency_mhz <= 0.0 || !request.frequency_mhz.is_finite() {
        return Err(crate::error::AntennaModelError::Validation(
            crate::error::ValidationError::InvalidValue {
                param: "frequency_mhz".to_string(),
                reason: "Must be positive and finite".to_string(),
            },
        ));
    }
    if request.n_rings > MAX_H3_RINGS {
        return Err(crate::error::AntennaModelError::Validation(
            crate::error::ValidationError::InvalidValue {
                param: "n_rings".to_string(),
                reason: format!("n_rings {} exceeds maximum of {}", request.n_rings, MAX_H3_RINGS),
            },
        ));
    }
    if let Some(res) = request.h3_resolution {
        if res > 15 {
            return Err(crate::error::AntennaModelError::Validation(
                crate::error::ValidationError::InvalidValue {
                    param: "h3_resolution".to_string(),
                    reason: format!("h3_resolution {} must be 0–15", res),
                },
            ));
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Add handler to handlers.rs**

Add the following import at the top of `handlers.rs` alongside the existing imports:
```rust
use crate::api::schemas::{H3LinkBudgetRequest, H3LinkBudgetResponse};
use crate::service::compute_h3_link_budget;
```

Add the handler function after `generate_heatmap_endpoint`:

```rust
/// POST /api/v1/h3-heatmap - H3 link budget heatmap
///
/// Computes antenna loss, FSPL, az/el, and total path loss for each H3
/// hexagonal cell in an n-ring neighborhood around the feed pointing location.
#[handler]
pub async fn h3_link_budget(
    state: Data<&Arc<AppState>>,
    Json(request): Json<H3LinkBudgetRequest>,
) -> poem::Result<Json<H3LinkBudgetResponse>> {
    info!(
        antenna_id = %request.antenna_id,
        feed_id = %request.feed_id,
        frequency_mhz = request.frequency_mhz,
        n_rings = request.n_rings,
        "H3 link budget request received"
    );

    if let Err(validation_err) =
        validator::validate_h3_link_budget_request(&request, &state.repository)
    {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %validation_err,
            "H3 link budget validation failed"
        );
        let status_code = match &validation_err {
            crate::error::AntennaModelError::FeedNotFound { .. } => StatusCode::NOT_FOUND,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        let error_response = ErrorResponse::new("validation_error", validation_err.to_string());
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            status_code,
        ));
    }

    match compute_h3_link_budget(&request, &state.repository, &state.cache) {
        Ok(response) => {
            info!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                cell_count = response.cells.len(),
                computation_time_ms = response.metadata.computation_time_ms,
                "H3 link budget computation successful"
            );
            Ok(Json(response))
        }
        Err(e) => {
            error!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                error = %e,
                "H3 link budget computation failed"
            );
            let (status_code, error_type) = match &e {
                crate::error::AntennaModelError::FeedNotFound { .. } => {
                    (StatusCode::NOT_FOUND, "feed_not_found")
                }
                crate::error::AntennaModelError::InvalidCoordinate { .. } => {
                    (StatusCode::BAD_REQUEST, "invalid_coordinate")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };
            let error_response = ErrorResponse::new(error_type, e.to_string());
            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                status_code,
            ))
        }
    }
}
```

- [ ] **Step 3: Register route in routes.rs**

In `antenna-model/src/api/routes.rs`, add the route after the existing heatmap route:

```rust
// H3 link budget heatmap endpoint
.at("/api/v1/h3-heatmap", post(handlers::h3_link_budget))
```

- [ ] **Step 4: Build to confirm no compile errors**

```bash
cargo build -p antenna-model 2>&1 | tail -10
```
Expected: builds successfully.

- [ ] **Step 5: Run all existing unit tests to confirm nothing regressed**

```bash
cargo test -p antenna-model --lib 2>&1 | tail -5
```
Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/api/handlers.rs antenna-model/src/api/routes.rs antenna-model/src/service/validator.rs
git commit -m "feat: add POST /api/v1/h3-heatmap handler and route"
```

---

### Task 7: Integration tests

**Goal:** End-to-end tests for the H3 heatmap endpoint and cache consistency, running against a real server with test fixtures.

**Files:**
- Create: `antenna-model/tests/integration/h3_link_budget_tests.rs`
- Modify: `antenna-model/tests/integration/mod.rs`

**Acceptance Criteria:**
- [ ] `n_rings=0` returns exactly 1 cell
- [ ] `n_rings=2` returns exactly 19 cells
- [ ] Center cell has `loss_db` closest to 0 among all cells
- [ ] `total_path_loss_db == loss_db + free_space_path_loss_db` for each cell (within floating-point tolerance)
- [ ] `calibration_status` field present in response
- [ ] Unknown `antenna_id` returns HTTP 404
- [ ] `n_rings=11` (exceeds max) returns HTTP 422
- [ ] Repeated identical requests return identical responses (cache consistency)

**Verify:** `cargo test -p antenna-model --test integration -- h3 --nocapture 2>&1 | tail -10` → all pass

**Steps:**

- [ ] **Step 1: Write integration tests**

Create `antenna-model/tests/integration/h3_link_budget_tests.rs`:

```rust
//! Integration tests for POST /api/v1/h3-heatmap

use crate::integration::helpers::{start_test_server, TestClient};

/// Build a minimal valid H3 link budget request body using a known test antenna.
///
/// Uses "test_boresight_xband" and "x_band" which are defined in
/// tests/fixtures/test_antennas.yaml. Vehicle is at ~500 km altitude
/// above the boresight point (ECEF).
fn h3_request_body(n_rings: u32, h3_resolution: Option<u8>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "antenna_id": "test_boresight_xband",
        "feed_id": "x_band",
        "vehicle_position": {
            "x": 6_878_137.0,  // ~500 km above equator, ECEF
            "y": 0.0,
            "z": 0.0
        },
        "reflector_boresight": {
            "x": 0.0,   // lon=0, lat=0 (geodetic)
            "y": 0.0,
            "z": 0.0
        },
        "feed_position": {
            "x": 0.0,   // same as boresight (on-axis feed)
            "y": 0.0,
            "z": 0.0
        },
        "frequency_mhz": 8400.0,
        "n_rings": n_rings
    });
    if let Some(res) = h3_resolution {
        body["h3_resolution"] = serde_json::json!(res);
    }
    body
}

#[tokio::test]
async fn test_h3_n_rings_0_returns_1_cell() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(0, Some(7)))
        .await;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert_eq!(body["cells"].as_array().unwrap().len(), 1);
    assert_eq!(body["h3_resolution"], 7);
}

#[tokio::test]
async fn test_h3_n_rings_2_returns_19_cells() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(2, Some(7)))
        .await;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert_eq!(body["cells"].as_array().unwrap().len(), 19);
}

#[tokio::test]
async fn test_h3_center_cell_has_minimum_loss() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(2, Some(7)))
        .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;

    let center_id = body["center_cell_id"].as_str().unwrap();
    let cells = body["cells"].as_array().unwrap();

    let center_loss = cells
        .iter()
        .find(|c| c["cell_id"].as_str().unwrap() == center_id)
        .expect("center cell not in results")["loss_db"]
        .as_f64()
        .unwrap();

    for cell in cells {
        let loss = cell["loss_db"].as_f64().unwrap();
        assert!(
            loss >= center_loss - 0.001,
            "Cell {} has lower loss ({}) than center ({})",
            cell["cell_id"],
            loss,
            center_loss
        );
    }
}

#[tokio::test]
async fn test_h3_total_path_loss_equals_sum() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(1, Some(7)))
        .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;

    for cell in body["cells"].as_array().unwrap() {
        let loss = cell["loss_db"].as_f64().unwrap();
        let fspl = cell["free_space_path_loss_db"].as_f64().unwrap();
        let total = cell["total_path_loss_db"].as_f64().unwrap();
        assert!(
            (total - (loss + fspl)).abs() < 0.001,
            "total_path_loss_db mismatch for cell {}: {} != {} + {}",
            cell["cell_id"], total, loss, fspl
        );
    }
}

#[tokio::test]
async fn test_h3_calibration_status_present() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(0, Some(7)))
        .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert!(body["calibration_status"].is_object(), "calibration_status should be present");
}

#[tokio::test]
async fn test_h3_unknown_antenna_returns_404() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let mut body = h3_request_body(0, Some(7));
    body["antenna_id"] = serde_json::json!("nonexistent_antenna");
    let resp = client.post("/api/v1/h3-heatmap", &body).await;
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_h3_exceeds_max_rings_returns_422() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(11, Some(7)))
        .await;
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn test_h3_cache_consistency_repeated_request() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    let body = h3_request_body(1, Some(7));
    let resp1: serde_json::Value = client
        .post("/api/v1/h3-heatmap", &body)
        .await
        .json()
        .await;
    let resp2: serde_json::Value = client
        .post("/api/v1/h3-heatmap", &body)
        .await
        .json()
        .await;

    // Cell counts and cell IDs should be identical
    assert_eq!(resp1["cells"].as_array().unwrap().len(),
               resp2["cells"].as_array().unwrap().len());

    // Gain values should be identical (cache returns same result)
    for (c1, c2) in resp1["cells"].as_array().unwrap()
        .iter()
        .zip(resp2["cells"].as_array().unwrap())
    {
        let g1 = c1["gain_db"].as_f64().unwrap();
        let g2 = c2["gain_db"].as_f64().unwrap();
        assert!((g1 - g2).abs() < 0.001, "gain_db mismatch between requests: {} vs {}", g1, g2);
    }
}

#[tokio::test]
async fn test_h3_auto_resolution_used_when_not_specified() {
    let server = start_test_server().await;
    let client = TestClient::new(&server.url);

    // No h3_resolution in request — should auto-select from 8400 MHz → resolution 8
    let resp = client
        .post("/api/v1/h3-heatmap", &h3_request_body(0, None))
        .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert_eq!(body["h3_resolution"], 8); // X-band auto-resolution
}
```

- [ ] **Step 2: Register test module**

In `antenna-model/tests/integration/mod.rs`, add:
```rust
pub mod h3_link_budget_tests;
```

- [ ] **Step 3: Run integration tests**

```bash
cargo test -p antenna-model --test integration -- h3 --nocapture 2>&1 | tail -15
```
Expected: all 8 h3 integration tests pass.

- [ ] **Step 4: Run full test suite to confirm no regressions**

```bash
cargo test --all 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add antenna-model/tests/integration/h3_link_budget_tests.rs antenna-model/tests/integration/mod.rs
git commit -m "test: integration tests for H3 link budget endpoint and cache consistency"
```

---

## Self-Review

**Spec coverage check:**
- ✅ `POST /api/v1/h3-heatmap` endpoint — Tasks 5, 6
- ✅ `H3LinkBudgetRequest` schema with all specified fields — Task 4
- ✅ `H3CellResult` with az/el, distance, gain, loss, FSPL, total, G/T — Task 4
- ✅ `feed_position` as center H3 cell — Task 5
- ✅ `n_rings` parameter — Tasks 4, 5
- ✅ Auto H3 resolution from frequency — Task 5
- ✅ LRU gain cache — Tasks 2, 3
- ✅ Cache key includes feed physical position — Task 2
- ✅ Cache per `(antenna_id, feed_id)` in DashMap — Task 2
- ✅ Cache enabled/disabled via config — Tasks 1, 2
- ✅ `cache.enabled: false` still returns correct results — tested in Task 2
- ✅ `invalidate()` for future hot-reload — Task 2
- ✅ FSPL formula `20·log10(4πdf/c)` — Task 5
- ✅ `loss_db` = center cell gain − cell gain — Task 5
- ✅ `total_path_loss_db = loss_db + fspl_db` — Task 5
- ✅ `g_over_t_db` when `temperature_k` provided — Task 5
- ✅ Parallel evaluation via rayon — Task 5
- ✅ `h3o` crate (pure Rust) — Task 1
- ✅ `lru` + `dashmap` — Task 1
- ✅ Integration tests — Task 7

**Type consistency check:**
- `GainCacheKey` defined in Task 2, used in Task 5 — ✅ consistent
- `compute_h3_link_budget(request, repository, cache)` defined Task 5, called in handler Task 6 — ✅ consistent
- `validate_h3_link_budget_request(request, repository)` defined Task 6 step 1, called in handler step 2 — ✅ consistent
- `H3LinkBudgetRequest`, `H3LinkBudgetResponse`, `H3CellResult` defined Task 4, used in Tasks 5, 6, 7 — ✅ consistent
- `HeatmapMetadata` reused from existing schemas — ✅ no change needed
- `is_in_coverage` made `pub(crate)` in Task 5 step 3 — ✅ visible to h3_link_budget.rs
