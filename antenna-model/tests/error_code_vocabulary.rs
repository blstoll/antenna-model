//! Guards that the served error-code vocabulary, the OpenAPI spec, and the API
//! documentation all describe the same set of codes (roadmap unit C3).
//!
//! `api::schemas::ErrorCode` is the source of truth and is compiler-enforced at
//! every emission site. The spec and the docs are hand-maintained, so they are
//! pinned here instead — adding a code without documenting it fails the build.
//!
//! This is deliberately narrow: it checks the *vocabulary*, not the status codes
//! each one is served with. Roadmap unit C2 owns the statuses, and unit C7 adds the
//! general path/method drift guard.

use antenna_model::api::schemas::ErrorCode;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Read a `components.schemas.<schema>.properties.<property>.enum` string list.
fn spec_enum<'a>(spec: &'a serde_yaml::Value, schema: &str, property: &str) -> Vec<&'a str> {
    let enum_node = spec
        .get("components")
        .and_then(|n| n.get("schemas"))
        .and_then(|n| n.get(schema))
        .and_then(|n| n.get("properties"))
        .and_then(|n| n.get(property))
        .and_then(|n| n.get("enum"))
        .and_then(|n| n.as_sequence())
        .unwrap_or_else(|| {
            panic!(
                "openapi.yaml must define components.schemas.{schema}.properties.{property}.enum"
            )
        });

    enum_node
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("every {schema}.{property} enum entry must be a string"))
        })
        .collect()
}

/// Every code in `ErrorCode::ALL` appears in openapi.yaml's `ErrorResponse.error`
/// enum **and** in `GainError.code`, and neither enum contains anything else.
///
/// Both are checked because both are hand-maintained copies of the same vocabulary.
/// `GainError` (the typed per-item failure C8 stage 3 added to batch responses) draws
/// on `ErrorResponse.error`'s codes but re-lists them, so before this test covered it,
/// renaming a code updated the enum with a drift guard and silently rotted the one
/// without — the exact failure mode C3 introduced this file to prevent.
#[test]
fn openapi_error_enum_matches_the_served_vocabulary() {
    let path = repo_root().join("openapi.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let spec: serde_yaml::Value =
        serde_yaml::from_str(&text).expect("openapi.yaml must be valid YAML");

    for (schema, property) in [("ErrorResponse", "error"), ("GainError", "code")] {
        let documented = spec_enum(&spec, schema, property);

        for code in ErrorCode::ALL {
            assert!(
                documented.contains(&code.as_str()),
                "error code {code:?} is served but missing from openapi.yaml's \
                 {schema}.{property} enum"
            );
        }
        for code in &documented {
            assert!(
                ErrorCode::ALL.iter().any(|c| c.as_str() == *code),
                "openapi.yaml's {schema}.{property} enum documents error code {code:?}, \
                 which the service never emits"
            );
        }
    }
}

/// Every code has a row in the error-code table in `docs/api-documentation.md`.
///
/// Matches on the table-cell form (`` | `code` | ``) rather than a bare substring, so
/// a passing mention in prose does not satisfy the check — and so that
/// `validation_error` cannot stand in for a missing `invalid_request_body` row by
/// virtue of one code being a substring of some other line.
#[test]
fn every_error_code_is_documented_in_the_api_docs() {
    let path = repo_root().join("docs/api-documentation.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    for code in ErrorCode::ALL {
        let row = format!("| `{code}` |");
        assert!(
            text.contains(&row),
            "error code {code:?} has no row in the error-code table in \
             docs/api-documentation.md (looked for {row:?})"
        );
    }
}

/// No `PascalCase` error code survives in the spec or the published examples.
///
/// Before C3, `openapi.yaml`, `docs/architecture.md`, and `examples/api_requests.json`
/// all advertised codes like `"AntennaNotFound"` that no emission site ever produced —
/// they came from a set of unused `ErrorResponse` convenience constructors. The
/// constructors are gone; this keeps the documentation from regrowing them.
#[test]
fn no_pascal_case_error_codes_remain_in_the_published_contract() {
    let files = [
        "openapi.yaml",
        "docs/api-documentation.md",
        "docs/architecture.md",
        "examples/api_requests.json",
    ];

    for file in files {
        let path = repo_root().join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        for (lineno, line) in text.lines().enumerate() {
            // Only the wire field matters: `error: "Foo"` (YAML) or `"error": "Foo"`
            // (JSON/Markdown snippets). Rust type names in prose are fine.
            for marker in ["error: \"", "\"error\": \""] {
                let Some(rest) = line.split_once(marker).map(|(_, r)| r) else {
                    continue;
                };
                let Some(value) = rest.split('"').next() else {
                    continue;
                };
                assert!(
                    !value.starts_with(char::is_uppercase),
                    "{file}:{} advertises PascalCase error code {value:?}; the served \
                     vocabulary is snake_case (see api/schemas.rs, enum ErrorCode)",
                    lineno + 1
                );
            }
        }
    }
}
