# Code Review Checklist

This checklist should be used when reviewing code changes to the antenna-model service and calibration tool to ensure consistent quality standards.

## General Code Quality

- [ ] **Code follows Rust idioms and best practices**
  - Uses iterator methods instead of manual loops where appropriate
  - Leverages the type system for safety
  - Follows naming conventions (snake_case for functions/variables, PascalCase for types)
  - Proper use of `Option` and `Result` types

- [ ] **All public APIs have documentation comments**
  - Module-level `//!` documentation for all public modules
  - Function-level `///` documentation for all public functions
  - Includes examples where helpful
  - Documents parameters, return values, and errors
  - Explains edge cases and limitations

- [ ] **Error handling is comprehensive**
  - No `unwrap()` or `expect()` in production code (except in tests)
  - Errors provide sufficient context for debugging
  - Error types are well-defined using `thiserror`
  - Error propagation uses `?` operator appropriately
  - Proper error recovery and fallback strategies

- [ ] **Tests cover happy path and error cases**
  - Unit tests for all non-trivial logic
  - Integration tests for API endpoints
  - Edge case testing (boundary conditions, invalid inputs)
  - Property-based tests where appropriate
  - Test coverage >80% for modified modules

## Performance

- [ ] **Performance-critical code is benchmarked**
  - Aperture integration paths have benchmark coverage
  - No obvious algorithmic inefficiencies (e.g., O(n²) where O(n) is possible)
  - Allocations are minimized in hot paths
  - No unnecessary clones or copies

- [ ] **No performance regressions**
  - Benchmarks run before and after changes
  - No significant degradation (<10%) in critical paths
  - New algorithms meet performance targets

## Logging and Observability

- [ ] **Logging uses structured fields**
  - Uses `tracing` with structured field logging, not format strings
  - Appropriate log levels (DEBUG for details, INFO for requests, WARN for issues, ERROR for failures)
  - Request IDs included for correlation
  - No sensitive data logged (credentials, PII)

- [ ] **Errors are traceable**
  - Error messages are actionable
  - Stack traces preserved where relevant
  - Context includes file paths, parameter values, etc.

## Security

- [ ] **Security considerations addressed**
  - Input validation for all user-provided data
  - No SQL injection vectors (not applicable currently)
  - No command injection vectors
  - No XSS vulnerabilities (API returns JSON, not HTML)
  - File path validation prevents directory traversal
  - Rate limiting considered for resource-intensive operations

- [ ] **Dependency security**
  - `cargo audit` passes with no known vulnerabilities
  - Dependencies are from trusted sources
  - Minimal dependency footprint

## API Stability

- [ ] **Breaking changes documented**
  - API version changes noted
  - Migration guide provided if applicable
  - Deprecation warnings added before removal

- [ ] **Backward compatibility maintained** (if applicable)
  - Existing API contracts preserved
  - New optional fields don't break existing clients
  - Configuration changes are backward compatible

## Code Maintainability

- [ ] **Complex logic is well-documented**
  - Non-obvious algorithms explained with comments
  - Mathematical formulas referenced (e.g., Ruze equation, Zernike polynomials)
  - Physics concepts clarified for non-experts
  - TODOs and FIXMEs are tracked or resolved

- [ ] **Code is modular and reusable**
  - Single Responsibility Principle followed
  - Functions are focused and composable
  - Avoid code duplication (DRY principle)
  - Appropriate abstraction levels

## Rust-Specific Checks

- [ ] **No unsafe code** (unless absolutely necessary and justified)
  - Safe alternatives explored first
  - Unsafe blocks have safety comments
  - Invariants clearly documented

- [ ] **Lifetime annotations are correct**
  - No over-constraining lifetimes
  - Borrows vs. ownership clarified

- [ ] **Trait implementations are appropriate**
  - Common traits implemented where useful (Debug, Clone, etc.)
  - Custom traits have clear contracts

## Testing

- [ ] **Tests are deterministic**
  - No flaky tests due to timing or randomness
  - Test fixtures are repeatable
  - Cleanup after tests (no state pollution)

- [ ] **Test names are descriptive**
  - Clear what is being tested
  - Clear what the expected behavior is

## Documentation

- [ ] **README updates (if needed)**
  - New features documented
  - Installation instructions current
  - Examples work and are up-to-date

- [ ] **API documentation (if changed)**
  - OpenAPI spec updated
  - Request/response examples current
  - Error codes documented

- [ ] **Architecture docs (if changed)**
  - Design decisions recorded (ADRs)
  - Diagrams updated if applicable
  - Migration guides for breaking changes

## Deployment Readiness

- [ ] **Configuration is externalized**
  - No hardcoded values in production code
  - Environment variables or config files used
  - Sensible defaults provided

- [ ] **Resource limits are appropriate**
  - Memory usage bounded
  - CPU usage reasonable
  - File descriptors managed properly

- [ ] **Health checks work correctly**
  - `/health` and `/ready` endpoints functional
  - Liveness and readiness probes accurate

## Final Checks

- [ ] **All CI/CD checks pass**
  - `cargo test --all --all-features` succeeds
  - `cargo clippy -- -D warnings` has zero warnings
  - `cargo fmt --check` passes
  - `cargo audit` has no vulnerabilities
  - Benchmarks complete successfully

- [ ] **No compiler warnings**
  - All code compiles cleanly
  - No deprecation warnings

- [ ] **Code builds in release mode**
  - `cargo build --release` succeeds
  - Release optimizations don't break functionality

---

## Checklist Usage

### For Code Authors
Review your own code against this checklist before submitting for review. Address any items that don't pass.

### For Code Reviewers
Use this checklist to guide your review. Not all items apply to every change, but most should be considered.

### For Maintainers
Periodically review this checklist and update it as project standards evolve.

---

**Last Updated:** 2025-12-07
**Version:** 1.0
