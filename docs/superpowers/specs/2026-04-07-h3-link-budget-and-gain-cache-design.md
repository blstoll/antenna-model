# H3 Link Budget Heatmap & Gain Cache — Design Spec

**Date:** 2026-04-07
**Status:** Approved
**Sprint:** Post-MVP feature additions

---

## Overview

Two related features:

1. **H3 Link Budget Heatmap** — new endpoint `POST /api/v1/h3-heatmap` that computes antenna loss, free-space path loss, azimuth, elevation, and total path loss for each H3 hexagonal cell in an n-ring neighborhood around the feed pointing location.

2. **LRU Gain Cache** — caches aperture integration results keyed on `(az, el, freq)` per antenna-feed to improve throughput, with direct benefit to the H3 heatmap workload and all existing endpoints.

---

## 1. H3 Link Budget Heatmap Endpoint

### 1.1 Endpoint

```
POST /api/v1/h3-heatmap
```

### 1.2 Request Schema

Extends the existing `HeatmapRequest` pattern. All coordinate fields use `Position3D` (ECEF or Geodetic, auto-detected), consistent with `GainRequest` and `HeatmapRequest`. Frequency uses MHz, consistent with all existing endpoints.

```rust
pub struct H3LinkBudgetRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic, auto-detected)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    /// Defines the antenna frame Z-axis (vehicle → boresight direction)
    pub reflector_boresight: Position3D,

    /// Feed pointing location (ECEF or Geodetic)
    /// Geodetic ground intersection of beam peak — becomes the center H3 cell
    pub feed_position: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz (beam squint correction, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Number of H3 neighbor rings around center cell
    /// 0 = center only (1 cell), 1 = 7 cells, 2 = 19 cells, 3 = 37 cells, ...
    pub n_rings: u32,

    /// H3 resolution (0–15). If None, auto-selected from frequency_mhz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h3_resolution: Option<u8>,

    /// System noise temperature in Kelvin (enables G/T output per cell)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_k: Option<f64>,
}
```

### 1.3 Response Schema

Mirrors `HeatmapResponse` structure. Reuses `HeatmapMetadata` and `CalibrationStatusInfo` without modification.

```rust
pub struct H3LinkBudgetResponse {
    pub antenna_id: String,
    pub feed_id: String,
    pub frequency_mhz: f64,

    /// H3 cell ID (hex string) of the center cell (derived from feed_position)
    pub center_cell_id: String,

    /// H3 resolution used (useful when auto-selected from frequency)
    pub h3_resolution: u8,

    /// Per-cell results
    pub cells: Vec<H3CellResult>,

    /// Warnings (extrapolation, out-of-range, etc.)
    pub warnings: Vec<String>,

    /// Aggregate computation metadata — reuses existing HeatmapMetadata
    pub metadata: HeatmapMetadata,

    /// Calibration status — same Option<CalibrationStatusInfo> pattern as all responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

pub struct H3CellResult {
    /// H3 cell identifier (hex string)
    pub cell_id: String,

    /// Cell center longitude (degrees)
    pub center_lon: f64,

    /// Cell center latitude (degrees)
    pub center_lat: f64,

    /// Azimuth in antenna frame (degrees) — mirrors GeometryInfo.emitter_azimuth_deg
    pub azimuth_deg: f64,

    /// Elevation in antenna frame (degrees) — mirrors GeometryInfo.emitter_elevation_deg
    pub elevation_deg: f64,

    /// Slant range from vehicle to cell center (km)
    pub distance_km: f64,

    /// Antenna gain at this az/el (dB) — mirrors GainResponse.gain_db
    pub gain_db: f64,

    /// Loss relative to boresight (dB, positive = loss) — mirrors GainResponse.loss_db
    pub loss_db: f64,

    /// Free-space path loss (dB): 20·log10(4π·d·f/c)
    pub free_space_path_loss_db: f64,

    /// Combined path loss: loss_db + free_space_path_loss_db
    pub total_path_loss_db: f64,

    /// G/T in dB/K — only present when temperature_k provided in request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub g_over_t_db: Option<f64>,
}
```

### 1.4 Auto H3 Resolution from Frequency

When `h3_resolution` is not provided, the resolution is selected to match approximately one H3 cell edge per beamwidth sample:

| Band  | Frequency      | H3 Resolution | Avg cell edge |
|-------|----------------|---------------|---------------|
| L     | < 2 GHz        | 6             | 3.2 km        |
| S/C   | 2–8 GHz        | 7             | 1.2 km        |
| X/Ku  | 8–20 GHz       | 8             | 0.46 km       |
| Ka+   | > 20 GHz       | 9             | 0.17 km       |

### 1.5 Computation Pipeline

For each H3 cell in `grid_disk(center_cell, n_rings)`:

1. **Cell generation** — `h3o::CellIndex::grid_disk(center, n_rings)` → cell iterator
2. **Center coordinates** — `LatLng::from(cell)` → lat/lon → convert to ECEF at `feed_position.alt` (or 0m if no alt)
3. **Slant range** — `distance = |cell_ecef − vehicle_ecef|`
4. **Direction** — `unit_vec = normalize(cell_ecef − vehicle_ecef)`
5. **Antenna frame transform** — boresight axis = `normalize(reflector_boresight_ecef − vehicle_ecef)`; frame X/Y constructed from ECEF Z reference (or ECEF X if boresight within 5° of Z)
6. **Az/el extraction** — project `unit_vec` into antenna frame → az/el in degrees
7. **Gain computation** — existing evaluator pipeline at (az, el, freq_mhz), with cache lookup (Section 2)
8. **Loss** — `loss_db = boresight_gain_db − gain_db`
9. **FSPL** — `20·log10(4π·d·f/c)` where d in meters, f in Hz
10. **Total path loss** — `loss_db + free_space_path_loss_db`
11. **G/T** — if `temperature_k` provided: `gain_db − 10·log10(temperature_k)`

Steps 1–10 executed in parallel via `rayon` (consistent with existing batch and heatmap processing).

### 1.6 Antenna Frame Derivation

The antenna attitude is not explicitly provided in this endpoint — it is derived from geometry:

- **Boresight axis (Z)**: `normalize(reflector_boresight_ecef − vehicle_ecef)`
- **Reference up vector**: ECEF Z-axis (`[0, 0, 1]`), or ECEF X-axis if boresight is within 5° of Z
- **Frame X**: `normalize(up × boresight)`
- **Frame Y**: `boresight × frame_X`

Since dish gain is rotationally symmetric, az affects only which direction in the plane off-boresight, not the gain magnitude — only elevation off-boresight determines gain.

### 1.7 H3 Library

Use the `h3o` crate (pure Rust, no FFI, no C dependency). Operations needed:
- `LatLng::new(lat, lon).to_cell(resolution)` → center cell
- `CellIndex::grid_disk(k)` → k-ring neighborhood iterator
- `LatLng::from(cell)` → cell center coordinates

---

## 2. LRU Gain Cache

### 2.1 What Is Cached

The gain value (f64, in dB) for a given `(az, el, freq)` tuple within a specific `(antenna_id, feed_id)` context. This is the output of the aperture integration pipeline — the computational bottleneck at 0.5–17ms per evaluation.

Not cached: FSPL (trivial arithmetic), coordinate transforms, per-cell geometry.

### 2.2 Cache Key

```rust
#[derive(Hash, Eq, PartialEq, Clone)]
pub struct GainCacheKey {
    /// (az_deg * 1000).round() as i32 — 0.001° resolution
    az_millideg:  i32,
    /// (el_deg * 1000).round() as i32
    el_millideg:  i32,
    /// (freq_mhz * 1000).round() as u32 — 1 kHz resolution
    freq_khz:     u32,
}
```

0.001° angular quantization produces negligible gain error (<0.01 dB for any operational dish size) and is finer than calibration measurement accuracy.

### 2.3 Cache Structure

```rust
pub struct GainCache {
    /// Per (antenna_id, feed_id) cache — avoids cross-antenna lock contention
    caches: DashMap<(String, String), Mutex<LruCache<GainCacheKey, f64>>>,
    max_entries_per_feed: usize,
}
```

`DashMap` provides concurrent access across antenna-feed pairs. `Mutex<LruCache>` serializes access within a single feed's cache.

**Dependencies:** `lru` crate (LRU eviction), `dashmap` crate (already used in codebase).

### 2.4 Integration Point

`GainCache` is held in `AppState` alongside `CalibrationRepository`. The existing evaluator in `service/evaluator.rs` routes through a new `cached_evaluate()` wrapper:

```
Request az/el/freq
    → GainCache::get(antenna_id, feed_id, az, el, freq)
        → hit:  return cached gain_db
        → miss: existing physics pipeline → GainCache::insert → return gain_db
```

All existing endpoints (`/gain`, `/gain/batch`, `/heatmap`) benefit automatically with no schema changes.

### 2.5 Cache Invalidation

`GainCache::invalidate(antenna_id, feed_id)` removes the entry for that feed from the `DashMap`. Called when calibration data is reloaded. No TTL — calibration updates are explicit events.

### 2.6 Configuration

In `config/service.yaml`:

```yaml
cache:
  enabled: true
  max_entries_per_feed: 10000
```

### 2.7 Expected Throughput Impact

| Scenario | Without cache | With cache (hit) | Improvement |
|----------|--------------|------------------|-------------|
| H3 heatmap, first request | ~10ms (19 cells × 0.5ms) | ~10ms | 1x |
| H3 heatmap, repeated request | ~10ms | ~0.1ms | ~100x |
| Batch with repeated az/el | linear | near-constant | up to 100x |
| Single unique query | 0.5–17ms | 0.5–17ms | 1x |

---

## 3. Testing

### 3.1 H3 Link Budget — Unit Tests (`src/service/h3_link_budget.rs`)

- Auto H3 resolution selection: one test per frequency band (L/S-C/X-Ku/Ka)
- Center cell derivation: `feed_position` geodetic → correct H3 cell at given resolution
- Cell count: n=0 → 1, n=1 → 7, n=2 → 19, n=3 → 37
- FSPL formula: verify against known values at specific distance + frequency
- Boresight cell (center) has `azimuth_deg ≈ 0`, `elevation_deg ≈ 0`
- Center cell has lowest `loss_db` in response; `loss_db` generally increases with ring distance
- `total_path_loss_db = loss_db + free_space_path_loss_db` (per cell)
- `g_over_t_db` absent when `temperature_k` not in request; present when provided

### 3.2 H3 Link Budget — Integration Tests (`tests/integration/`)

- Full end-to-end `POST /api/v1/h3-heatmap` with a test antenna
- `n_rings=0` → exactly 1 cell in response
- `n_rings=2` → exactly 19 cells in response
- Center cell has minimum `loss_db`
- `calibration_status` present in response
- Invalid `antenna_id` → 404
- `n_rings` above a configured maximum → 400

### 3.3 Gain Cache — Unit Tests (`src/service/cache.rs`)

- Cache miss: physics pipeline called once, result stored
- Cache hit: physics pipeline not called (verified via call counter)
- LRU eviction: fill beyond `max_entries_per_feed`, verify oldest entry evicted
- Key quantization: az values within 0.0005° map to the same key
- Per-feed isolation: entries for feed A do not appear under feed B
- `invalidate(antenna_id, feed_id)` clears only that feed's entries

### 3.4 Gain Cache — Integration Tests

- Repeated identical H3 heatmap request produces identical results (cache consistency)
- Cache disabled via `cache.enabled: false` → correct results still returned
- Concurrent H3 requests for the same area produce no races or deadlocks (`--test-threads=8`)

---

## 4. New Dependencies

| Crate | Purpose | Notes |
|-------|---------|-------|
| `h3o` | H3 cell operations (pure Rust) | No FFI, no C dependency |
| `lru` | LRU cache eviction | Lightweight, well-maintained |
| `dashmap` | Concurrent map for per-feed caches | New dependency |

---

## 5. Files to Create / Modify

| File | Change |
|------|--------|
| `antenna-model/src/api/schemas.rs` | Add `H3LinkBudgetRequest`, `H3LinkBudgetResponse`, `H3CellResult` |
| `antenna-model/src/service/h3_link_budget.rs` | New — H3 grid generation, pipeline orchestration |
| `antenna-model/src/service/cache.rs` | New — `GainCache`, `GainCacheKey` |
| `antenna-model/src/service/evaluator.rs` | Add `cached_evaluate()` wrapper |
| `antenna-model/src/service/mod.rs` | Export new modules |
| `antenna-model/src/api/routes.rs` | Register `POST /api/v1/h3-heatmap` handler |
| `antenna-model/src/main.rs` | Add `GainCache` to `AppState` |
| `antenna-model/src/config/settings.rs` | Add `CacheConfig` struct + `cache` key |
| `config/service.yaml` | Add `cache` section |
| `antenna-model/Cargo.toml` | Add `h3o`, `lru` dependencies |

---

## 6. Out of Scope

- Physics model extensions (polarization, atmospheric propagation, pointing error) — deferred to a separate spec
- SIMD vectorization — deferred; current throughput already exceeds targets, cache provides cross-request gains first
- GPU acceleration — deferred; significant infrastructure investment not warranted at current scale
- H3 cell boundary polygon output — not needed for link budget use case
- Cache warming / pre-population — deferred; on-demand population sufficient
