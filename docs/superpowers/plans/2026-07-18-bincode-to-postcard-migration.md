# Migrate Binary Serialization: bincode → postcard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the archived, unmaintained `bincode` crate (v2.0.1) with `postcard` (v1.1.3) as the binary serialization format for calibration `.bin` artifacts, across both the `antenna-model` service (reader) and the `calibrate` CLI (writer). Do it cleanly while there are **no production `.bin` artifacts** to migrate.

**Architecture:** Five mechanical layers, no physics or API-schema changes:
1. **Dependencies** — swap `bincode` → `postcard` in both crate manifests.
2. **Derives** — drop `bincode::{Encode, Decode}` from the 12 artifact types in `antenna-model/src/data/types.rs`; every one *already* derives serde `Serialize`/`Deserialize`, so postcard works with zero new trait plumbing.
3. **Call sites** — `bincode::encode_to_vec(x, cfg)` → `postcard::to_allocvec(&x)`; `bincode::decode_from_slice(b, cfg)` → `postcard::from_bytes(b)` (note: returns `T`, not `(T, usize)`). The serde-compat variant (`bincode::serde::*`) disappears with the dead path (see below).
4. **Errors** — collapse the two `From<bincode::error::{Encode,Decode}Error>` impls into one `From<postcard::Error>`.
5. **Format hygiene** — bump the ANTC artifact version `1 → 2` (old files fail loudly), regenerate the 2 headerless test fixtures, delete the dead deprecated serde-compat path.

**Tech Stack:** Rust (cargo workspace: `antenna-model` service + `calibrate` CLI). `postcard` is pure Rust, satisfies the repo's "no system BLAS / pure-Rust build" constraint. The ANTC header + CRC32 (`crc32fast`) framing is hand-rolled and **serialization-crate-independent** — it is untouched except for the version constant.

---

## Why postcard (verification summary — do NOT re-litigate)

Recorded 2026-07-18 after research + full surface audit:

- **bincode is unmaintained:** GitHub repo `bincode-org/bincode` was **archived read-only on Aug 15 2025**; only a low-activity sourcehut mirror remains.
- **postcard is the right replacement for *this* codebase:**
  - Actively maintained (v1.1.3, Jul 2025; Mozilla-sponsored).
  - **Has a documented, versioned wire-format spec** (postcard.jamesmunns.com) since v1.0 — bincode never had one. This directly satisfies the "not locked into an opaque binary format" goal.
  - serde-native; all 12 artifact types already derive serde → near drop-in.
  - Pure Rust, compact, std-capable.
- **The one postcard limitation** (not self-describing → no `deserialize_any`, so no `#[serde(flatten)]` / untagged enums) was checked and **does not apply**: no artifact type uses those attributes. The only `#[serde(tag=…)]` enums (`api/schemas.rs:455,532`) are JSON API schemas, never binary-serialized — leave them alone.
- **Rejected alternatives:** `bitcode` (format explicitly unstable across versions — opposite of the goal); `rmp-serde`/`ciborium` (fine, cross-language, but larger/slower and cross-language artifact reads are not a requirement); `oxicode` (too new/unproven). If cross-language artifact reads ever become a requirement, revisit CBOR — but not now.

**User decisions (maintainer, 2026-07-18 — do NOT re-ask):**
- **Delete** the deprecated `calibrate/src/serializer.rs` save/load path (Path A) rather than migrate it — it is dead code the service cannot load.
- **Bump the ANTC artifact version `1 → 2`** so any stray old-format `.bin` fails with a clear "unsupported version" error instead of risking a garbled decode.

---

## Context an implementer MUST read first

**Repo conventions:**
1. The shell is zsh: always quote glob patterns (`--include='*.rs'`).
2. After changes, run `cargo test --workspace` (both crates share these types) — not just the touched module.
3. Never use `unwrap()`/`expect()` in production code; the existing bincode call sites already use `?` / `map_err`. Preserve that. (Test code may keep `unwrap()`.)
4. `openapi.yaml` and the API schemas are **not affected** — this migration never touches request/response serialization (that is serde_json), only the on-disk `.bin` artifact format.
5. File:line references below were verified 2026-07-18 on branch `deps-upgrade`. Line numbers drift — re-verify each reference before editing; if a cited line no longer matches its description, STOP and re-locate, do not guess.

**postcard API differences from bincode 2.x (the traps):**
- **No config object.** Every `bincode::config::standard()` argument (and `use bincode::config;`) disappears.
- **`to_allocvec(&value) -> Result<Vec<u8>, postcard::Error>`** replaces `encode_to_vec(value, cfg)`. Requires the `alloc` feature (enabled transitively by `use-std` — see manifest task). Takes a **reference**.
- **`from_bytes(&[u8]) -> Result<T, postcard::Error>`** replaces `decode_from_slice(b, cfg)`. **Returns `T` directly, NOT `(T, usize)`.** Every `let (x, _) = bincode::decode_from_slice(...)?;` / `.0` must drop the tuple destructure. `from_bytes` silently **ignores trailing bytes** — safe here because ANTC payloads are sliced to exact length (`loader.rs`) and headerless files use the whole buffer.
- **Single unified `postcard::Error`** for both directions — the two `From` impls collapse to one.
- postcard is **not wire-compatible** with bincode. This is the intended clean cutover; the version bump guards headered files, and the only headerless files are regenerated fixtures.

**Serialization surface (verified 2026-07-18) — the complete call-site inventory:**

| Location | Current call | Type | Path |
|---|---|---|---|
| `antenna-model/src/data/loader.rs:115` | `decode_from_slice` (native) | `AntennaCalibration` | **LIVE runtime read** |
| `antenna-model/src/data/loader.rs:9` | `use bincode::config;` | — | import to remove |
| `calibrate/src/main.rs:150` | `encode_to_vec` (native) | `AntennaCalibration` | **LIVE write — full mode, ANTC header** |
| `calibrate/src/main.rs:342` | `encode_to_vec` (native) | `AntennaCalibration` | **LIVE write — boresight mode, HEADERLESS** |
| `calibrate/src/serializer.rs:222,322` | `bincode::serde::*` | `CalibrationArtifact` | **DEAD (delete — Path A)** |
| `antenna-model/src/error.rs:384-394` | `From<Encode/DecodeError>` | — | collapse to `From<postcard::Error>` |
| `antenna-model/src/data/types.rs:6` | `use bincode::{Decode, Encode};` | — | import to remove |
| `antenna-model/src/data/types.rs` (12 derive lines: 31,120,194,337,415,457,505,537,624,699,776,825) | `#[derive(… Encode, Decode …)]` | — | drop `Encode, Decode` |
| `antenna-model/src/data/repository.rs:530` | `encode_to_vec` (native) | test helper | test |
| `antenna-model/src/data/types.rs:1834/1836, 2172/2174, 2199/2201` | encode+decode | tests | test |
| `antenna-model/src/data/loader.rs:362,441,460,493,532,576` | `encode_to_vec` | tests | test |
| `calibrate/tests/artifact_export_integration_test.rs:40` | `encode_to_vec` (native) | e2e | **key e2e test** |

**Path facts (from surface audit):**
- The `.bin` files the service actually loads are produced by `calibrate/src/main.rs` (full mode = ANTC-headered, boresight mode = headerless) and read by `antenna-model/src/data/loader.rs:41` (`load_calibration_artifact`, called in prod from `repository.rs:154`).
- `calibrate/src/serializer.rs` `save_artifact`/`load_artifact` (Path A) are **deprecated dead code** — the service cannot load their `CalibrationArtifact` output; only their own unit tests call them. **Being deleted.** But `serializer.rs` *also* holds `CalibrationArtifact`, `ArtifactMetadata`, `export_metadata_json`, `export_validation_json`, `SerializationError` — these ARE used by `main.rs` (`:654,:734`) and **must be kept**.
- The `From<bincode::error::…>` impls in `error.rs` have **no live caller** (the loader maps decode errors inline to `DataError::LoadError`). Convert them anyway for correctness/completeness; do not rely on them being exercised.

**Checked-in binary artifacts (the only files needing regeneration):**
- `antenna-model/tests/fixtures/calibration_data/test_uncalibrated_sband_boresight.bin`
- `antenna-model/tests/fixtures/calibration_data/test_uncalibrated_xband_boresight.bin`

Both are **headerless legacy-format** bincode (verified: first byte is a bincode length prefix, not `ANTC`). Referenced by `antenna-model/tests/fixtures/test_antennas.yaml:13,30`, loaded by integration tests (`tests/integration/helpers.rs:57`) and benches (`benches/heatmap_benchmarks.rs`). After the format switch they will fail to decode and **must be regenerated as postcard bytes** (see Task 6). Note: `calibration_data/design_specs/small_groundstation.yaml` and the `enabled: true` antennas load from **YAML** (serde_yaml) — unaffected.

---

## Tasks

### Task 1 — Swap dependencies in both manifests
- [ ] `antenna-model/Cargo.toml:9`: replace `bincode = "2.0.1"` with `postcard = { version = "1.1.3", features = ["use-std"] }`. (`use-std` implies `alloc` — required for `to_allocvec` — and adds `std::error::Error for postcard::Error`, which the `From` impl and `?` benefit from.)
- [ ] `calibrate/Cargo.toml:13`: replace `bincode = { version = "2.0.1", features = ["serde"] }` with `postcard = { version = "1.1.3", features = ["use-std"] }`.
- [ ] `cargo update -p postcard` / let the lockfile resolve; confirm `bincode` and `bincode_derive` are **gone** from `Cargo.lock` after the code changes land (do a final `grep -n bincode Cargo.lock` — expect no hits).
- [ ] Do NOT touch `crc32fast`, `serde`, `serde_json`, `serde_yaml` — all still needed.

### Task 2 — Drop native derives from the 12 artifact types
- [ ] `antenna-model/src/data/types.rs:6`: remove `use bincode::{Decode, Encode};`.
- [ ] For each of the 12 derive lines (31,120,194,337,415,457,505,537,624,699,776,825), change `#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]` → `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]`. Leave serde `Serialize, Deserialize` intact.
- [ ] Update the doc comment at `types.rs:177` (mentions "changed the bincode layout") to reference postcard, or generalize to "the serialized layout".
- [ ] `cargo build -p antenna-model` — expect it to fail only at the not-yet-migrated call sites (Tasks 3–4), not on the derives.

### Task 3 — Migrate the LIVE reader (`loader.rs`) + bump ANTC version
- [ ] `loader.rs:9`: remove `use bincode::config;`.
- [ ] `loader.rs:17`: bump `const ANTC_SUPPORTED_VERSION: u32 = 1;` → `2`.
- [ ] `loader.rs:114-119`: replace
  ```rust
  let config = config::standard();
  let (calibration, _): (AntennaCalibration, usize) = bincode::decode_from_slice(payload, config)
      .map_err(|e| DataError::LoadError { path: …, reason: format!("Failed to deserialize calibration data: {}", e) })?;
  ```
  with
  ```rust
  let calibration: AntennaCalibration = postcard::from_bytes(payload)
      .map_err(|e| DataError::LoadError { path: …, reason: format!("Failed to deserialize calibration data: {}", e) })?;
  ```
  Keep the `"Failed to deserialize"` wording — a test asserts `reason.contains("deserialize")` (`loader.rs:387`).
- [ ] Keep the legacy-headerless fallback branch (`loader.rs:108-110`) — boresight mode still writes headerless. (It now decodes postcard bytes; old bincode headerless files will error at decode, which is acceptable since none exist in production.)

### Task 4 — Migrate the LIVE writers (`calibrate/src/main.rs`)
- [ ] `main.rs:150` (`write_antc_artifact`, full mode): `bincode::encode_to_vec(calibration, bincode::config::standard())` → `postcard::to_allocvec(calibration)` (adjust `&` to match — `to_allocvec` takes a reference). Keep the `.context("Failed to … encode calibration artifact")?` (reword "bincode-encode" → "postcard-encode").
- [ ] `main.rs:155-159` (`write_antc_artifact` header assembly): bump the hardcoded version `1u32` → `2u32` so written headers match the new `ANTC_SUPPORTED_VERSION`.
- [ ] `main.rs:342` (`run_boresight_calibration`, headerless): `bincode::encode_to_vec(&calibration, …)` → `postcard::to_allocvec(&calibration)`; reword the `.context(...)`.
- [ ] Update the doc comment at `main.rs:144` (mentions "bincode payload").

### Task 5 — Collapse the error `From` impls
- [ ] `antenna-model/src/error.rs:384-394`: replace the two impls (`From<bincode::error::EncodeError>` and `From<bincode::error::DecodeError>`) with a single:
  ```rust
  impl From<postcard::Error> for DataError {
      fn from(err: postcard::Error) -> Self {
          DataError::Serialization(err.to_string())
      }
  }
  ```
  (`DataError::Serialization(String)` at `error.rs:93` is unchanged.)

### Task 6 — Delete the dead serde-compat path (Path A) — SURGICAL
Keep `CalibrationArtifact`, `ArtifactMetadata`, `SerializationError`, `export_metadata_json`, `export_validation_json`, and the `summary()` impl (all used by `main.rs`). Delete only the bincode-bearing dead code:
- [ ] `calibrate/src/serializer.rs`: delete `save_artifact` (~:216-250), `load_artifact` (~:264-332), and `artifact_format_info` (~:405-436, the fn whose string literal says "bincode").
- [ ] Delete the constants `MAGIC_NUMBER` / `ARTIFACT_VERSION` (~:96-99) **iff** they become unused after the above (they are only used by save/load_artifact). Verify with grep before deleting.
- [ ] Delete the now-dead tests in `serializer.rs`: `test_save_and_load_artifact` (:508), `test_checksum_validation` (:535), `test_invalid_magic_number` (:559), `test_version_mismatch` (:577), `test_format_info` (:605, the one asserting `contains("bincode")`). Keep `test_artifact_summary` (:596) if it doesn't touch save/load.
- [ ] `calibrate/src/lib.rs:57-59`: remove `artifact_format_info`, `load_artifact`, `save_artifact` from the `pub use serializer::{…}` re-export. Keep `export_metadata_json`, `export_validation_json`, `ArtifactMetadata`, `CalibrationArtifact`, `SerializationError`.
- [ ] Update the module-header doc (`serializer.rs:1-25`) — it describes the deleted save/load bincode format.

### Task 7 — Migrate all test / bench helpers
- [ ] `antenna-model/src/data/repository.rs:530` (`write_calibration_file` helper): `encode_to_vec` → `to_allocvec`. This helper feeds many repo tests (`:683,:727,:1084`) — they should pass once the helper produces postcard bytes read by the migrated loader.
- [ ] `antenna-model/src/data/types.rs` round-trip tests: `:1834/1836` (`test_serialization_round_trip`), `:2172/2174` (`test_serialization_backward_compatibility`), `:2199/2201` (`test_partial_calibration_serialization_round_trip`) — swap encode/decode to postcard, drop the `(T, usize)` tuple / `.0`, drop `config`. `test_serialization_backward_compatibility` asserts optional fields (`correction_surface`, `calibration_status`, `calibration_coverage`, which carry `#[serde(default, skip_serializing_if=…)]`) round-trip as `None` — postcard honors those serde attributes, so this should hold; confirm.
- [ ] `antenna-model/src/data/loader.rs` tests: `:362,441,460,493,532,576` — swap `encode_to_vec` → `to_allocvec`. The `make_antc_bytes` helper (~:294-303) writes the version byte — update it to write `2` to match `ANTC_SUPPORTED_VERSION`. `test_load_antc_unsupported_version_rejected` (:527) must now encode a version ≠ 2 (e.g. `1` or `3`) to still exercise the reject path.
- [ ] `calibrate/tests/artifact_export_integration_test.rs`: `:40` (`write_antc` helper) `encode_to_vec` → `to_allocvec`; update its ANTC header version to `2` (:36-49). This is the true full-export → ANTC → service-load e2e test — it MUST stay green.

### Task 8 — Regenerate the two `.bin` test fixtures as postcard
- [ ] Regenerate `test_uncalibrated_sband_boresight.bin` and `test_uncalibrated_xband_boresight.bin` (headerless postcard bytes of the same `AntennaCalibration` values they currently hold). Options, pick one:
  - **(a) Preferred — generate on the fly:** add a small `#[test]`-gated or `build.rs`-free helper / one-shot `cargo run` snippet in `calibrate` that constructs the two `AntennaCalibration` fixtures and writes headerless `postcard::to_allocvec` bytes to the fixture paths. Reuse the exact field values encoded in the originals (decode the originals with a throwaway bincode snippet first, or reconstruct from `test_antennas.yaml` + the boresight defaults).
  - **(b) Alternative — eliminate the checked-in binaries:** change the integration harness (`tests/integration/helpers.rs`) to build these fixtures in a `tempfile` dir at test time via the migrated writer, and delete the two `.bin` files + their `test_antennas.yaml` references. Cleaner long-term (no binary blobs in git), slightly larger diff.
- [ ] Whichever path: confirm the regenerated fixtures decode via `load_calibration_artifact` and the integration tests that consume `test_antennas.yaml` pass.
- [ ] If keeping checked-in binaries, note in the commit that they are now postcard-format.

### Task 9 — Docs & comment sweep
- [ ] `grep -rn 'bincode' --include='*.rs' --include='*.toml' .` (exclude `target/`) → expect **zero** hits except intentional historical mentions. Fix any stragglers.
- [ ] Update `CLAUDE.md`: the Data Layer / Calibration Workflow / serializer bullets that say "bincode" → "postcard". Mention the wire format is now the documented postcard spec.
- [ ] Update `docs/domain-contract.md` / `docs/architecture.md` if either names bincode as the artifact format (grep first).

### Task 10 — Verify end-to-end
- [ ] `cargo build --release` (both bins).
- [ ] `cargo test --workspace` — all green, especially `artifact_export_integration_test`, the `loader.rs` ANTC/CRC/version/headerless tests, and the `types.rs` round-trip tests.
- [ ] `cargo clippy --workspace -- -D warnings`.
- [ ] `cargo fmt`.
- [ ] Manual smoke: run the `calibrate` full mode to produce a `.bin`, then start the service (or a loader test) pointed at it, and confirm it loads and validates. Confirm an *old* bincode `.bin` (e.g. `git show` an original fixture into a temp path) now fails with a clear error (version reject for headered / decode error for headerless), not a silent garbled load.
- [ ] `grep -n bincode Cargo.lock` → no hits (dependency fully removed).

---

## Risk / rollback notes
- **No production data at risk:** no `.bin` artifacts are checked in except the 2 regenerated fixtures; enabled antennas load from YAML. This is why the cutover is clean and needs no dual-read compatibility shim.
- **Biggest failure mode:** forgetting the `(T, usize)` → `T` change at a `decode_from_slice` site, or passing a value instead of a reference to `to_allocvec` — both are compile errors, so the compiler catches them.
- **Subtle behavior change:** `postcard::from_bytes` ignores trailing bytes (bincode reported consumed length). Harmless here (payloads are exact-length), but do not add a "leftover bytes" assertion expecting the old tuple.
- **Rollback:** the change is contained to serialization plumbing; reverting the commit restores bincode. No schema/migration state to unwind.
